//! Minimal Language Server Protocol client for rust-analyzer.
//!
//! Synchronous: requests block until answered. Run it on its own thread. Nothing here is specific to
//! one server except where it is found (`Config`) and the options it is started with
//! (`initialization_options`).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// Where rust-analyzer is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub rust_analyzer: PathBuf,
}

impl Config {
    /// Looks for a working rust-analyzer: the one `rustup which rust-analyzer` names (the component
    /// of the active toolchain), then `PATH`, then `~/.cargo/bin`. `None` when there is none; the
    /// editor then runs without language intelligence (`rustup component add rust-analyzer`).
    pub fn locate() -> Option<Self> {
        find_tool("rust-analyzer").map(|rust_analyzer| Self { rust_analyzer })
    }
}

/// Finds an executable tool of the Rust toolchain that actually runs (`--version` succeeds). That
/// matters for rust-analyzer: `~/.cargo/bin/rust-analyzer` is a rustup shim that exists even when the
/// component is not installed, and then only prints an error.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    tool_candidates(name).into_iter().find(|p| runs(p))
}

fn runs(path: &Path) -> bool {
    Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The places to try, in order and without duplicates.
fn tool_candidates(name: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    };
    if let Ok(o) = Command::new("rustup")
        .args(["which", name])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        && o.status.success()
    {
        let line = String::from_utf8_lossy(&o.stdout);
        if let Some(l) = line.lines().next().filter(|l| !l.trim().is_empty()) {
            add(PathBuf::from(l.trim()));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            add(dir.join(name));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        add(PathBuf::from(home).join(".cargo/bin").join(name));
    }
    out
}

/// The folder rust-analyzer should treat as the workspace for a file or folder `start`: the
/// outermost enclosing folder with a `Cargo.toml` that declares `[workspace]`, else the nearest
/// folder with a `Cargo.toml`, else `start` itself (a folder) or its parent (a file).
pub fn cargo_root(start: &Path) -> PathBuf {
    let base = if start.is_dir() {
        start.to_path_buf()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| start.to_path_buf())
    };
    let mut nearest = None;
    let mut workspace = None;
    for dir in base.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            nearest.get_or_insert_with(|| dir.to_path_buf());
            if text.lines().any(|l| l.trim() == "[workspace]") {
                workspace = Some(dir.to_path_buf());
            }
        }
    }
    workspace.or(nearest).unwrap_or(base)
}

/// What the server is started with. Check on save only (not on every keystroke); no build scripts or
/// proc macros are run on the user's behalf; cargo never goes online; the server's `cargo check`
/// uses its own target folder, so it does not block the user's builds on the target lock.
pub fn initialization_options() -> Value {
    json!({
        "checkOnSave": true,
        "check": {"command": "check", "allTargets": true},
        "cargo": {
            "buildScripts": {"enable": false},
            "autoreload": true,
            "targetDir": true,
            "extraEnv": {"CARGO_NET_OFFLINE": "true"},
        },
        "procMacro": {"enable": false},
        "diagnostics": {"enable": true},
        "lens": {"enable": false},
        "inlayHints": {"enable": false},
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
}

/// A 0-based position in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
}

/// A declaration in a file: its name, its LSP `SymbolKind` number, and its 0-based position. `depth`
/// is 0 for a top-level declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: u32,
    pub line: u32,
    pub col: u32,
    pub depth: u32,
}

/// The signature of the call being typed: its label (`WriteLn(...)`), the labels of its
/// parameters, and which parameter the caret is in (`None` when the server does not say).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    pub parameters: Vec<String>,
    pub active: Option<usize>,
}

/// A replacement in a file: the range (0-based line and character column, end exclusive) and the
/// new text. What a rename sends back for each file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub new_text: String,
}

/// All the replacements of a rename in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    pub path: PathBuf,
    pub edits: Vec<TextEdit>,
}

pub struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    id: i64,
    versions: std::collections::HashMap<PathBuf, i64>,
    /// Last text sent per file, to convert columns between chars and UTF-16 units.
    texts: std::collections::HashMap<PathBuf, String>,
    encoding: Encoding,
    /// Latest `publishDiagnostics` per file.
    diagnostics: std::collections::HashMap<PathBuf, Vec<Diagnostic>>,
    /// Files whose diagnostics were published since `take_published` last looked.
    published: Vec<PathBuf>,
    /// The server's last `serverStatus` said it was idle (loaded, indexed, nothing queued).
    quiescent: bool,
}

/// A problem the server reported; positions are 0-based, columns in chars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// LSP severity: 1 error, 2 warning, 3 information, 4 hint.
    pub severity: u8,
    pub message: String,
}

/// How a server counts the `character` of a position. The spec says UTF-16 code units, which
/// rust-analyzer uses; `Utf8` is kept for servers that count bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
}

impl Encoding {
    /// Wire column for `chars` chars into `line`.
    pub fn to_wire(self, line: &str, chars: u32) -> u32 {
        match self {
            Self::Utf16 => utf16_col(line, chars),
            Self::Utf8 => line
                .chars()
                .take(chars as usize)
                .map(|c| c.len_utf8() as u32)
                .sum(),
        }
    }

    /// Chars into `line` for the wire column `units` (inside a char rounds up past it).
    pub fn from_wire(self, line: &str, units: u32) -> u32 {
        match self {
            Self::Utf16 => char_col(line, units),
            Self::Utf8 => {
                let mut used = 0;
                for (n, c) in line.chars().enumerate() {
                    if used >= units {
                        return n as u32;
                    }
                    used += c.len_utf8() as u32;
                }
                line.chars().count() as u32
            }
        }
    }
}

/// UTF-16 code units in the first `chars` chars of `line`.
pub fn utf16_col(line: &str, chars: u32) -> u32 {
    line.chars()
        .take(chars as usize)
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// Chars covered by the first `units` UTF-16 code units of `line` (a position inside a
/// surrogate pair rounds up to the end of that char).
pub fn char_col(line: &str, units: u32) -> u32 {
    let mut used = 0;
    for (n, c) in line.chars().enumerate() {
        if used >= units {
            return n as u32;
        }
        used += c.len_utf16() as u32;
    }
    line.chars().count() as u32
}

/// The name to insert (`filterText`, which has no call parentheses, else the label) and what to show
/// beside it: the signature or type the server gives.
fn completion_item(i: &Value) -> CompletionItem {
    let label = i["filterText"]
        .as_str()
        .or_else(|| i["label"].as_str())
        .unwrap_or_default();
    let detail = i["detail"]
        .as_str()
        .or_else(|| i["labelDetails"]["detail"].as_str())
        .or_else(|| i["labelDetails"]["description"].as_str())
        .unwrap_or_default();
    CompletionItem {
        label: label.to_string(),
        detail: detail.to_string(),
    }
}

fn nth_line(text: &str, line: u32) -> &str {
    text.split('\n')
        .nth(line as usize)
        .unwrap_or_default()
        .trim_end_matches('\r')
}

impl Client {
    /// Starts the server for the workspace at `root` (a Cargo project folder, see `cargo_root`) and
    /// completes the handshake.
    pub fn start(config: &Config, root: &Path) -> Result<Self, String> {
        let mut child = Command::new(&config.rust_analyzer)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start rust-analyzer: {e}"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, messages) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(msg) = read_message(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            stdin,
            messages,
            id: 0,
            versions: Default::default(),
            texts: Default::default(),
            encoding: Encoding::Utf16,
            diagnostics: Default::default(),
            published: Vec::new(),
            quiescent: false,
        };
        client.request("initialize", initialize_params(root))?;
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    /// Waits until rust-analyzer says it has finished loading and indexing the workspace (its
    /// `experimental/serverStatus` with `quiescent`), up to `timeout`. Returns whether it did.
    pub fn wait_quiescent(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while !self.quiescent {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.messages.recv_timeout(left) {
                Ok(msg) if msg.get("method").is_some() => {
                    let _ = self.handle_server_message(&msg);
                }
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        true
    }

    fn write(&mut self, body: Value) -> Result<(), String> {
        let body = body.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body)
            .and_then(|_| self.stdin.flush())
            .map_err(|e| e.to_string())
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = self
                .messages
                .recv_timeout(left)
                .map_err(|_| format!("no answer to {method}"))?;
            if msg.get("method").is_some() {
                self.handle_server_message(&msg)?;
            } else if msg["id"] == id {
                return match msg.get("error") {
                    Some(e) => Err(format!(
                        "{method}: {}",
                        e["message"].as_str().unwrap_or("error")
                    )),
                    None => Ok(msg["result"].clone()),
                };
            }
        }
    }

    /// Notifications are recorded; requests from the server (progress, registration) just
    /// get an empty answer.
    fn handle_server_message(&mut self, msg: &Value) -> Result<(), String> {
        if let Some(req_id) = msg.get("id") {
            return self.write(json!({"jsonrpc": "2.0", "id": req_id, "result": null}));
        }
        if msg["method"] == "experimental/serverStatus" {
            self.quiescent = msg["params"]["quiescent"].as_bool().unwrap_or(false);
        }
        if msg["method"] == "textDocument/publishDiagnostics" {
            let params = &msg["params"];
            let path = path_of(params["uri"].as_str().unwrap_or_default());
            let text = self.texts.get(&path).cloned().unwrap_or_default();
            let encoding = self.encoding;
            let list = params["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| parse_diagnostic(d, &text, encoding))
                .collect();
            if !self.published.contains(&path) {
                self.published.push(path.clone());
            }
            self.diagnostics.insert(path, list);
        }
        Ok(())
    }

    /// The files whose diagnostics the server published since the last call, with the new list of
    /// each (empty when the file is clean). Reads what has arrived without waiting. Only files
    /// that were opened with `sync` are reported: the server also publishes cargo's results for
    /// files nobody opened, and those come to the app from its own `cargo check`.
    pub fn take_published(&mut self) -> Vec<(PathBuf, Vec<Diagnostic>)> {
        while let Ok(msg) = self.messages.try_recv() {
            if msg.get("method").is_some() {
                let _ = self.handle_server_message(&msg);
            }
        }
        std::mem::take(&mut self.published)
            .into_iter()
            .filter(|p| self.versions.contains_key(p))
            .map(|p| {
                let list = self.diagnostics.get(&p).cloned().unwrap_or_default();
                (p, list)
            })
            .collect()
    }

    /// Tells the server `path` is closed and forgets what it published for it.
    pub fn close(&mut self, path: &Path) -> Result<(), String> {
        self.texts.remove(path);
        self.diagnostics.remove(path);
        self.published.retain(|p| p != path);
        if self.versions.remove(path).is_none() {
            return Ok(());
        }
        self.notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri(path)}}),
        )
    }

    /// Diagnostics the server has published for `path`, waiting up to `wait` for news.
    pub fn diagnostics(&mut self, path: &Path, wait: Duration) -> Vec<Diagnostic> {
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.messages.recv_timeout(left) {
                Ok(msg) if msg.get("method").is_some() => {
                    let _ = self.handle_server_message(&msg);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        self.diagnostics.get(path).cloned().unwrap_or_default()
    }

    /// Tells the server the full current text of `path`. Opens it on first use.
    pub fn sync(&mut self, path: &Path, text: &str) -> Result<(), String> {
        self.texts.insert(path.to_path_buf(), text.to_string());
        let version = self.versions.entry(path.to_path_buf()).or_insert(0);
        *version += 1;
        let version = *version;
        if version == 1 {
            self.notify(
                "textDocument/didOpen",
                json!({"textDocument": {"uri": uri(path), "languageId": "rust", "version": 1, "text": text}}),
            )
        } else {
            self.notify(
                "textDocument/didChange",
                json!({"textDocument": {"uri": uri(path), "version": version}, "contentChanges": [{"text": text}]}),
            )
        }
    }

    /// Request parameters for a position given as (line, column in chars).
    fn at(&self, path: &Path, line: u32, col: u32) -> Value {
        let text = self.texts.get(path).map(String::as_str).unwrap_or_default();
        let col = self.encoding.to_wire(nth_line(text, line), col);
        json!({"textDocument": {"uri": uri(path)}, "position": {"line": line, "character": col}})
    }

    pub fn completion(
        &mut self,
        path: &Path,
        line: u32,
        col: u32,
    ) -> Result<Vec<CompletionItem>, String> {
        let result = self.request("textDocument/completion", self.at(path, line, col))?;
        let items = result.get("items").unwrap_or(&result);
        let mut items: Vec<&Value> = items.as_array().into_iter().flatten().collect();
        items.sort_by(|a, b| a["sortText"].as_str().cmp(&b["sortText"].as_str()));
        Ok(items.into_iter().map(completion_item).collect::<Vec<_>>())
    }

    /// Hover text (markdown with the code fence removed), if the server has any.
    pub fn hover(&mut self, path: &Path, line: u32, col: u32) -> Result<Option<String>, String> {
        let result = self.request("textDocument/hover", self.at(path, line, col))?;
        let contents = &result["contents"];
        let text = contents["value"].as_str().or_else(|| contents.as_str());
        Ok(text.map(strip_fence))
    }

    pub fn definition(
        &mut self,
        path: &Path,
        line: u32,
        col: u32,
    ) -> Result<Option<Location>, String> {
        let result = self.request("textDocument/definition", self.at(path, line, col))?;
        let loc = if result.is_array() {
            &result[0]
        } else {
            &result
        };
        Ok(loc["uri"].as_str().map(|u| {
            let path = path_of(u);
            let line = loc["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
            let units = loc["range"]["start"]["character"].as_u64().unwrap_or(0) as u32;
            let col = self.char_col_in(&path, line, units);
            Location { path, line, col }
        }))
    }

    /// The symbols declared in a file (types, fields, routines, ...), in document order. Nested ones
    /// (a class's fields, a routine's locals) carry their depth. Both answers the spec allows are
    /// read: the nested `DocumentSymbol` tree and the flat `SymbolInformation` list.
    pub fn document_symbols(&mut self, path: &Path) -> Result<Vec<Symbol>, String> {
        let params = json!({"textDocument": {"uri": uri(path)}});
        let result = self.request("textDocument/documentSymbol", params)?;
        let mut out = Vec::new();
        for s in result.as_array().into_iter().flatten() {
            self.push_symbol(path, s, 0, &mut out);
        }
        Ok(out)
    }

    fn push_symbol(&self, path: &Path, s: &Value, depth: u32, out: &mut Vec<Symbol>) {
        let mut name = s["name"].as_str().unwrap_or_default().to_string();
        // A DocumentSymbol has `selectionRange`; a SymbolInformation has `location`, and some servers name
        // members `Class.Member` there instead of nesting them.
        let flat = s.get("selectionRange").is_none() && s.get("range").is_none();
        let (range, file) = if !flat {
            (&s["selectionRange"], path.to_path_buf())
        } else {
            (
                &s["location"]["range"],
                path_of(s["location"]["uri"].as_str().unwrap_or_default()),
            )
        };
        let mut depth = depth;
        if flat && let Some((_, last)) = name.rsplit_once('.') {
            depth += name.matches('.').count() as u32;
            name = last.to_string();
        }
        let line = range["start"]["line"].as_u64().unwrap_or(0) as u32;
        let units = range["start"]["character"].as_u64().unwrap_or(0) as u32;
        let col = self.char_col_in(&file, line, units);
        if !name.is_empty() {
            out.push(Symbol {
                name,
                kind: s["kind"].as_u64().unwrap_or(0) as u32,
                line,
                col,
                depth,
            });
        }
        for child in s["children"].as_array().into_iter().flatten() {
            self.push_symbol(path, child, depth + 1, out);
        }
    }

    /// The signature of the routine called at the position, while a call is being typed. `None`
    /// when the position is not in a call the server knows.
    pub fn signature_help(
        &mut self,
        path: &Path,
        line: u32,
        col: u32,
    ) -> Result<Option<Signature>, String> {
        let result = self.request("textDocument/signatureHelp", self.at(path, line, col))?;
        let sigs = result["signatures"].as_array();
        let Some(sig) = sigs.and_then(|s| {
            let i = result["activeSignature"].as_u64().unwrap_or(0) as usize;
            s.get(i).or_else(|| s.first())
        }) else {
            return Ok(None);
        };
        let label = sig["label"].as_str().unwrap_or_default().to_string();
        let parameters = sig["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| match &p["label"] {
                // A parameter label is either its text or a [start, end) range in the label.
                Value::String(s) => s.clone(),
                Value::Array(r) => {
                    let (a, b) = (
                        r.first().and_then(Value::as_u64),
                        r.get(1).and_then(Value::as_u64),
                    );
                    match (a, b) {
                        (Some(a), Some(b)) => label
                            .chars()
                            .skip(a as usize)
                            .take(b.saturating_sub(a) as usize)
                            .collect(),
                        _ => String::new(),
                    }
                }
                _ => String::new(),
            })
            .collect();
        let active = result["activeParameter"]
            .as_u64()
            .or_else(|| sig["activeParameter"].as_u64())
            .map(|i| i as usize);
        if label.is_empty() {
            return Ok(None);
        }
        Ok(Some(Signature {
            label,
            parameters,
            active,
        }))
    }

    /// Every place the symbol at the position is used, the declaration included.
    pub fn references(
        &mut self,
        path: &Path,
        line: u32,
        col: u32,
    ) -> Result<Vec<Location>, String> {
        let mut params = self.at(path, line, col);
        params["context"] = json!({"includeDeclaration": true});
        let result = self.request("textDocument/references", params)?;
        Ok(result
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|loc| self.location_of(loc))
            .collect())
    }

    /// Renames the symbol at the position to `new_name`. Returns the replacements per file: the
    /// server answers with either `changes` or `documentChanges`.
    pub fn rename(
        &mut self,
        path: &Path,
        line: u32,
        col: u32,
        new_name: &str,
    ) -> Result<Vec<FileEdit>, String> {
        let mut params = self.at(path, line, col);
        params["newName"] = json!(new_name);
        let result = self.request("textDocument/rename", params)?;
        let mut files: Vec<FileEdit> = Vec::new();
        let mut add = |this: &Self, uri: &str, edits: &Value| {
            let path = path_of(uri);
            let edits = edits
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| this.text_edit_in(&path, e))
                .collect();
            files.push(FileEdit { path, edits });
        };
        if let Some(changes) = result["changes"].as_object() {
            for (uri, edits) in changes {
                add(self, uri, edits);
            }
        } else if let Some(docs) = result["documentChanges"].as_array() {
            for d in docs {
                if let Some(uri) = d["textDocument"]["uri"].as_str() {
                    add(self, uri, &d["edits"]);
                }
            }
        }
        Ok(files)
    }

    fn location_of(&self, loc: &Value) -> Option<Location> {
        let path = path_of(loc["uri"].as_str()?);
        let line = loc["range"]["start"]["line"].as_u64()? as u32;
        let units = loc["range"]["start"]["character"].as_u64().unwrap_or(0) as u32;
        let col = self.char_col_in(&path, line, units);
        Some(Location { path, line, col })
    }

    fn text_edit_in(&self, path: &Path, e: &Value) -> TextEdit {
        let point = |p: &Value| {
            let line = p["line"].as_u64().unwrap_or(0) as u32;
            let units = p["character"].as_u64().unwrap_or(0) as u32;
            (line, self.char_col_in(path, line, units))
        };
        TextEdit {
            start: point(&e["range"]["start"]),
            end: point(&e["range"]["end"]),
            new_text: e["newText"].as_str().unwrap_or_default().to_string(),
        }
    }

    /// Converts a UTF-16 column in `path` to chars, using the synced text or else the file.
    fn char_col_in(&self, path: &Path, line: u32, units: u32) -> u32 {
        match self.texts.get(path) {
            Some(t) => self.encoding.from_wire(nth_line(t, line), units),
            None => {
                std::fs::read_to_string(path).map_or(units, |t| char_col(nth_line(&t, line), units))
            }
        }
    }

    /// Tells the server the file was saved, which starts `cargo check` for it (check on save).
    pub fn did_save(&mut self, path: &Path) -> Result<(), String> {
        self.notify(
            "textDocument/didSave",
            json!({"textDocument": {"uri": uri(path)}}),
        )
    }

    /// The edits `textDocument/formatting` (rustfmt) wants for the text last synced for `path`.
    pub fn formatting(&mut self, path: &Path) -> Result<Vec<TextEdit>, String> {
        let params = formatting_params(path);
        let result = self
            .request("textDocument/formatting", params)
            .map_err(friendly_format_error)?;
        Ok(result
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| self.text_edit_in(path, e))
            .collect())
    }

    /// Formats the file at `path` with rustfmt, through the server, and writes it back when it
    /// changed. The file is read from disk, so an editor must save first and read it again
    /// afterwards. `Ok(true)` when the file changed. Without rustfmt this is an error saying so.
    pub fn format_file(&mut self, path: &Path) -> Result<bool, String> {
        if find_tool("rustfmt").is_none() {
            return Err(RUSTFMT_MISSING.to_string());
        }
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("could not read the file: {e}"))?;
        self.sync(path, &text)?;
        let edits = self.formatting(path)?;
        let formatted = apply_edits(&text, &edits);
        if formatted == text {
            return Ok(false);
        }
        std::fs::write(path, &formatted).map_err(|e| format!("could not write the file: {e}"))?;
        // The server's copy is the unformatted one until the next `sync`.
        self.sync(path, &formatted)?;
        Ok(true)
    }

    pub fn stop(&mut self) {
        let _ = self.request("shutdown", Value::Null);
        let _ = self.notify("exit", Value::Null);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What the app says when formatting cannot run for want of rustfmt.
pub const RUSTFMT_MISSING: &str =
    "rustfmt was not found (install it with: rustup component add rustfmt)";

/// Whether `name` can be a Rust identifier written as plain ASCII: letters, digits and `_`, not starting
/// with a digit. Keywords are not checked; the server reports them.
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn friendly_format_error(e: String) -> String {
    if e.to_ascii_lowercase().contains("rustfmt") && !e.contains("rustup component add") {
        format!("{e} ({RUSTFMT_MISSING})")
    } else {
        e
    }
}

/// `initialize` parameters: the Cargo project at `root` as the workspace, plain-text completions and
/// the one extension used (`experimental/serverStatus`).
fn initialize_params(root: &Path) -> Value {
    json!({
        "processId": std::process::id(),
        "clientInfo": {"name": "formust", "version": env!("CARGO_PKG_VERSION")},
        "rootUri": uri(root),
        "workspaceFolders": [{"uri": uri(root), "name": "workspace"}],
        "capabilities": {
            "general": {"positionEncodings": ["utf-16"]},
            "textDocument": {
                "completion": {"completionItem": {"snippetSupport": false}},
                "publishDiagnostics": {"relatedInformation": false},
                "hover": {"contentFormat": ["markdown", "plaintext"]},
                "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
            },
            "experimental": {"serverStatusNotification": true},
        },
        "initializationOptions": initialization_options(),
    })
}

fn formatting_params(path: &Path) -> Value {
    json!({
        "textDocument": {"uri": uri(path)},
        "options": {"tabSize": 4, "insertSpaces": true},
    })
}

/// Applies replacements (positions in chars, as `TextEdit` holds them) to `text`. They must not
/// overlap; they are applied from the end so earlier positions stay valid.
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> String {
    let offset = |(line, col): (u32, u32)| -> usize {
        let mut at = 0;
        for (i, l) in text.split_inclusive('\n').enumerate() {
            if i == line as usize {
                let body = l.trim_end_matches(['\n', '\r']);
                let n = (col as usize).min(body.chars().count());
                return at + body.char_indices().nth(n).map_or(body.len(), |(b, _)| b);
            }
            at += l.len();
        }
        text.len()
    };
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            let (a, b) = (offset(e.start), offset(e.end));
            (a, b.max(a), e.new_text.as_str())
        })
        .collect();
    spans.sort_by_key(|s| std::cmp::Reverse((s.0, s.1)));
    let mut out = text.to_string();
    for (a, b, new) in spans {
        out.replace_range(a..b, new);
    }
    out
}

/// `file://` URI of an absolute path, percent-encoding what a URI cannot hold.
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The path of a `file://` URI (percent escapes decoded). Anything else is taken as a path.
pub fn path_of(uri: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://").unwrap_or(uri);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

fn parse_diagnostic(d: &Value, text: &str, encoding: Encoding) -> Diagnostic {
    let pos = |p: &Value| {
        let line = p["line"].as_u64().unwrap_or(0) as u32;
        let units = p["character"].as_u64().unwrap_or(0) as u32;
        (line, encoding.from_wire(nth_line(text, line), units))
    };
    let (line, col) = pos(&d["range"]["start"]);
    let (end_line, end_col) = pos(&d["range"]["end"]);
    Diagnostic {
        line,
        col,
        end_line,
        end_col,
        severity: d["severity"].as_u64().unwrap_or(1) as u8,
        message: d["message"].as_str().unwrap_or_default().to_string(),
    }
}

/// Hover markdown as plain text: code fence lines (``` with an optional language) and rust-analyzer's
/// `---` separators are removed, and runs of blank lines become one.
pub fn strip_fence(markdown: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in markdown.trim().lines() {
        let t = line.trim_end();
        if t.starts_with("```") || t == "---" {
            continue;
        }
        if t.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(t);
    }
    out.join("\n").trim().to_string()
}

fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse::<usize>().ok();
        }
    }
    let mut buf = vec![0; length?];
    reader.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_accepts_plain_rust_identifiers_only() {
        assert!(is_identifier("form_2"));
        assert!(is_identifier("_hidden"));
        assert!(is_identifier("Login"));
        assert!(!is_identifier("2fast"));
        assert!(!is_identifier("with space"));
        assert!(!is_identifier("dash-name"));
        assert!(!is_identifier(""));
    }

    #[test]
    fn strips_markdown_fence() {
        assert_eq!(strip_fence("```rust\nfn main()\n```"), "fn main()");
        assert_eq!(strip_fence("```\na\nb\n```\n"), "a\nb");
        assert_eq!(strip_fence("plain text"), "plain text");
        // rust-analyzer: signature, a separator, then docs.
        assert_eq!(
            strip_fence("```rust\nlet x: u8\n```\n\n---\n\nsize = 1"),
            "let x: u8\n\nsize = 1"
        );
        assert_eq!(strip_fence("```rust\nfn f()"), "fn f()");
    }

    #[test]
    fn converts_columns_between_chars_and_utf16() {
        // '€' is one UTF-16 unit, the emoji is two.
        let line = "a€😀b";
        assert_eq!(utf16_col(line, 0), 0);
        assert_eq!(utf16_col(line, 2), 2);
        assert_eq!(utf16_col(line, 3), 4);
        assert_eq!(utf16_col(line, 4), 5);
        assert_eq!(utf16_col(line, 99), 5);
        assert_eq!(char_col(line, 4), 3);
        assert_eq!(char_col(line, 3), 3); // inside the pair: rounds up
        assert_eq!(char_col(line, 5), 4);
        assert_eq!(char_col(line, 99), 4);
        for n in 0..=4 {
            assert_eq!(char_col(line, utf16_col(line, n)), n);
        }
    }

    #[test]
    fn utf8_columns_count_bytes() {
        let line = "a€😀b";
        assert_eq!(Encoding::Utf8.to_wire(line, 3), 8);
        assert_eq!(Encoding::Utf8.from_wire(line, 8), 3);
        assert_eq!(Encoding::Utf8.from_wire(line, 99), 4);
        assert_eq!(Encoding::Utf16.to_wire(line, 3), 4);
    }

    #[test]
    fn parses_diagnostics_with_utf16_columns() {
        let d = json!({"range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 4}},
                       "severity": 2, "message": "m"});
        let got = parse_diagnostic(&d, "x\n😀ab\n", Encoding::Utf16);
        assert_eq!((got.col, got.end_col, got.severity), (1, 3, 2));
    }

    #[test]
    fn uris_round_trip_and_escape() {
        let plain = Path::new("/Users/me/proj/src/main.rs");
        assert_eq!(uri(plain), "file:///Users/me/proj/src/main.rs");
        assert_eq!(path_of(&uri(plain)), plain);
        let odd = Path::new("/Users/me/My Proj/é#1/forms/login.frm.rs");
        let u = uri(odd);
        assert!(
            u.starts_with("file:///Users/me/My%20Proj/") && !u.contains(' ') && !u.contains('#'),
            "{u}"
        );
        assert_eq!(path_of(&u), odd);
        assert_eq!(path_of("file:///a/b%2Fc"), Path::new("/a/b/c"));
        assert_eq!(path_of("/plain/path"), Path::new("/plain/path"));
        assert_eq!(
            path_of("file:///100%"),
            Path::new("/100%"),
            "a stray percent stays"
        );
        assert_eq!(path_of("file:///x%zz"), Path::new("/x%zz"));
    }

    #[test]
    fn initialize_names_the_cargo_root_and_no_pascal_leftovers() {
        let p = initialize_params(Path::new("/w/app"));
        assert_eq!(p["rootUri"], "file:///w/app");
        assert_eq!(p["workspaceFolders"][0]["uri"], "file:///w/app");
        assert_eq!(p["clientInfo"]["name"], "formust");
        assert_eq!(
            p["capabilities"]["textDocument"]["completion"]["completionItem"]["snippetSupport"],
            false
        );
        let o = &p["initializationOptions"];
        assert_eq!(o["checkOnSave"], true);
        assert_eq!(o["check"]["command"], "check");
        assert_eq!(o["procMacro"]["enable"], false);
        assert_eq!(o["cargo"]["buildScripts"]["enable"], false);
        assert_eq!(o["cargo"]["extraEnv"]["CARGO_NET_OFFLINE"], "true");
        let all = p.to_string().to_ascii_lowercase();
        for gone in ["pasls", "fpc", "lazarus"] {
            assert!(!all.contains(gone), "{gone}");
        }
    }

    #[test]
    fn requests_are_shaped_as_the_protocol_wants() {
        let f = formatting_params(Path::new("/w/a b.rs"));
        assert_eq!(f["textDocument"]["uri"], "file:///w/a%20b.rs");
        assert_eq!(f["options"]["tabSize"], 4);
        assert_eq!(f["options"]["insertSpaces"], true);
    }

    #[test]
    fn completion_items_use_the_bare_name_and_a_detail() {
        let i = json!({"label": "push(…)", "filterText": "push", "detail": "fn(&mut self, T)"});
        assert_eq!(
            completion_item(&i),
            CompletionItem {
                label: "push".into(),
                detail: "fn(&mut self, T)".into()
            }
        );
        let i = json!({"label": "len", "labelDetails": {"detail": "()"}});
        assert_eq!(
            completion_item(&i),
            CompletionItem {
                label: "len".into(),
                detail: "()".into()
            }
        );
        assert_eq!(
            completion_item(&json!({})),
            CompletionItem {
                label: String::new(),
                detail: String::new()
            }
        );
    }

    fn edit(a: (u32, u32), b: (u32, u32), t: &str) -> TextEdit {
        TextEdit {
            start: a,
            end: b,
            new_text: t.into(),
        }
    }

    #[test]
    fn edits_apply_from_the_end() {
        let text = "fn  main( ){\nlet x=1;\n}\n";
        let edits = [
            edit((0, 2), (0, 4), " "),
            edit((0, 9), (0, 10), ""),
            edit((1, 5), (1, 6), " = "),
        ];
        assert_eq!(apply_edits(text, &edits), "fn main(){\nlet x = 1;\n}\n");
    }

    #[test]
    fn edits_handle_wide_chars_multi_line_ranges_and_the_end() {
        // Columns are chars, not bytes.
        assert_eq!(
            apply_edits("é😀x\n", &[edit((0, 2), (0, 3), "y")]),
            "é😀y\n"
        );
        // A range across lines, and one past the last line (insertion at the end).
        assert_eq!(
            apply_edits("a\nb\nc\n", &[edit((0, 1), (2, 0), "-")]),
            "a-c\n"
        );
        assert_eq!(apply_edits("a\n", &[edit((5, 0), (5, 0), "z")]), "a\nz");
        // CRLF: the column stops before the line break.
        assert_eq!(
            apply_edits("ab\r\ncd\r\n", &[edit((0, 9), (0, 9), "!")]),
            "ab!\r\ncd\r\n"
        );
        assert_eq!(apply_edits("same", &[]), "same");
    }

    #[test]
    fn a_missing_rustfmt_is_explained() {
        let e = friendly_format_error("textDocument/formatting: Failed to spawn rustfmt".into());
        assert!(e.contains("rustup component add rustfmt"), "{e}");
        assert_eq!(friendly_format_error("other".into()), "other");
    }

    #[test]
    fn the_cargo_root_is_the_outermost_workspace_else_the_nearest_crate() {
        let tmp = std::env::temp_dir().join(format!("formust-lsp-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let member = tmp.join("ws/crates/a");
        std::fs::create_dir_all(member.join("src")).unwrap();
        std::fs::write(
            tmp.join("ws/Cargo.toml"),
            "[workspace]\nmembers = [\"crates/a\"]\n",
        )
        .unwrap();
        std::fs::write(member.join("Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();
        std::fs::write(member.join("src/main.rs"), "").unwrap();
        assert_eq!(cargo_root(&member.join("src/main.rs")), tmp.join("ws"));
        assert_eq!(cargo_root(&member), tmp.join("ws"));
        let lone = tmp.join("lone");
        std::fs::create_dir_all(lone.join("src")).unwrap();
        std::fs::write(lone.join("Cargo.toml"), "[package]\nname = \"l\"\n").unwrap();
        assert_eq!(cargo_root(&lone.join("src")), lone);
        // No Cargo.toml at all: the folder itself, or a file's folder.
        let none = tmp.join("none");
        std::fs::create_dir_all(&none).unwrap();
        std::fs::write(none.join("x.rs"), "").unwrap();
        assert_eq!(cargo_root(&none), none);
        assert_eq!(cargo_root(&none.join("x.rs")), none);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_candidates_are_unique_existing_files() {
        let list = tool_candidates("cargo");
        let mut sorted = list.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len());
        assert!(list.iter().all(|p| p.is_file()));
        assert!(tool_candidates("no-such-tool-xyz").is_empty());
    }

    /// A client over a process that does nothing: only the message handling is exercised.
    fn test_client(messages: Receiver<Value>) -> Client {
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        Client {
            child,
            stdin,
            messages,
            id: 0,
            versions: Default::default(),
            texts: Default::default(),
            encoding: Encoding::Utf16,
            diagnostics: Default::default(),
            published: Vec::new(),
            quiescent: false,
        }
    }

    #[test]
    fn server_status_marks_the_client_quiescent() {
        let (_tx, messages) = channel();
        let mut c = test_client(messages);
        c.handle_server_message(&json!({"method": "experimental/serverStatus", "params": {"quiescent": true, "health": "ok"}})).unwrap();
        assert!(c.quiescent);
        c.texts.insert(PathBuf::from("/w/a.rs"), "x\n".into());
        c.handle_server_message(&json!({"method": "textDocument/publishDiagnostics", "params": {"uri": "file:///w/a.rs",
            "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "severity": 1, "message": "boom"}]}})).unwrap();
        let got = c.diagnostics(Path::new("/w/a.rs"), Duration::from_millis(1));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "boom");
    }

    #[test]
    fn published_diagnostics_are_reported_once_for_open_files_only() {
        let (_tx, messages) = channel();
        let mut c = test_client(messages);
        let publish = |c: &mut Client, uri: &str, n: usize| {
            let d: Vec<Value> = (0..n).map(|_| json!({"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "severity": 2, "message": "w"})).collect();
            c.handle_server_message(&json!({"method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": d}})).unwrap();
        };
        c.versions.insert(PathBuf::from("/w/a.rs"), 1);
        publish(&mut c, "file:///w/a.rs", 1);
        publish(&mut c, "file:///w/other.rs", 1);
        let got = c.take_published();
        assert_eq!(got.len(), 1, "unopened files are left out");
        assert_eq!(got[0].0, PathBuf::from("/w/a.rs"));
        assert_eq!(got[0].1.len(), 1);
        assert!(c.take_published().is_empty(), "reported once");
        publish(&mut c, "file:///w/a.rs", 0);
        let got = c.take_published();
        assert_eq!(got, vec![(PathBuf::from("/w/a.rs"), vec![])]);
    }
}
