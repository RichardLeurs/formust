//! Runs the language server (rust-analyzer) on its own thread and talks to the UI over channels.

use eframe::egui;
use formust_build::{Diagnostic, Severity};
use formust_lsp::{Client, CompletionItem, Config, FileEdit, Location, Symbol};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};

pub enum Req {
    /// The tab for `path` was closed: tell the server and drop its diagnostics.
    Close { path: PathBuf },
    /// Format the file on disk (rustfmt through the server; see `Client::format_file`).
    Format { path: PathBuf },
    Complete {
        path: PathBuf,
        text: String,
        line: u32,
        col: u32,
    },
    Definition {
        path: PathBuf,
        text: String,
        line: u32,
        col: u32,
    },
    Hover {
        path: PathBuf,
        text: String,
        line: u32,
        col: u32,
    },
    References {
        path: PathBuf,
        text: String,
        line: u32,
        col: u32,
    },
    /// The signature of the call at the caret: `at` is the caret's char index and `key` the text's,
    /// both echoed back so a late answer can be recognised.
    Signature {
        path: PathBuf,
        text: String,
        line: u32,
        col: u32,
        at: usize,
        key: u64,
    },
    /// The declarations of a file. `key` identifies the text asked about, echoed back.
    Symbols {
        path: PathBuf,
        text: String,
        key: u64,
    },
    Rename {
        path: PathBuf,
        text: String,
        line: u32,
        col: u32,
        new_name: String,
    },
}

pub enum Resp {
    Ready,
    /// The server has closed a file.
    Closed,
    /// The server published diagnostics for an open file; empty when the file is clean.
    Diagnostics {
        path: PathBuf,
        list: Vec<formust_lsp::Diagnostic>,
    },
    Completion(Vec<CompletionItem>),
    Definition(Option<Location>),
    /// The position the answer is for (0-based line, column), so late answers can be ignored.
    Hover(u32, u32, Option<String>),
    References(Vec<Location>),
    Signature {
        at: usize,
        key: u64,
        sig: Option<formust_lsp::Signature>,
    },
    Symbols {
        path: PathBuf,
        key: u64,
        symbols: Vec<Symbol>,
    },
    /// The replacements of a rename, per file.
    Rename(Vec<FileEdit>),
    /// The server has formatted the file on disk (or found it already formatted), or says why not (for
    /// instance rustfmt is not installed). Apart from `Error`, which stops the
    /// language server, because a file that cannot be formatted is not a reason to switch it off.
    Formatted {
        error: Option<String>,
    },
    Error(String),
}

pub struct Handle {
    pub requests: Sender<Req>,
    pub responses: Receiver<Resp>,
}

/// Starts rust-analyzer for the Cargo project around `root`; `None` when it is not installed
/// (`rustup component add rust-analyzer`).
pub fn spawn(root: &Path, ctx: egui::Context) -> Option<Handle> {
    let config = Config::locate()?;
    let root = formust_lsp::cargo_root(root);
    let (requests, req_rx) = channel::<Req>();
    let (resp_tx, responses) = channel();
    std::thread::spawn(move || {
        let reply = |r: Resp| {
            let _ = resp_tx.send(r);
            ctx.request_repaint();
        };
        let mut client = match Client::start(&config, &root) {
            Ok(c) => c,
            Err(e) => return reply(Resp::Error(e)),
        };
        reply(Resp::Ready);
        loop {
            // Wake now and then to pass on what the server published without being asked.
            let req = match req_rx.recv_timeout(std::time::Duration::from_millis(300)) {
                Ok(req) => req,
                Err(RecvTimeoutError::Timeout) => {
                    for (path, list) in client.take_published() {
                        reply(Resp::Diagnostics { path, list });
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let result = match req {
                Req::Close { path } => client.close(&path).map(|_| Resp::Closed),
                Req::Complete {
                    path,
                    text,
                    line,
                    col,
                } => client
                    .sync(&path, &text)
                    .and_then(|_| client.completion(&path, line, col))
                    .map(Resp::Completion),
                Req::Definition {
                    path,
                    text,
                    line,
                    col,
                } => client
                    .sync(&path, &text)
                    .and_then(|_| client.definition(&path, line, col))
                    .map(Resp::Definition),
                Req::Hover {
                    path,
                    text,
                    line,
                    col,
                } => client
                    .sync(&path, &text)
                    .and_then(|_| client.hover(&path, line, col))
                    .map(|h| Resp::Hover(line, col, h.filter(|t| !t.is_empty()))),
                Req::References {
                    path,
                    text,
                    line,
                    col,
                } => client
                    .sync(&path, &text)
                    .and_then(|_| client.references(&path, line, col))
                    .map(Resp::References),
                Req::Signature {
                    path,
                    text,
                    line,
                    col,
                    at,
                    key,
                } => client
                    .sync(&path, &text)
                    .and_then(|_| client.signature_help(&path, line, col))
                    .map(|sig| Resp::Signature { at, key, sig }),
                Req::Symbols { path, text, key } => client
                    .sync(&path, &text)
                    .and_then(|_| client.document_symbols(&path))
                    .map(|symbols| Resp::Symbols { path, key, symbols }),
                Req::Format { path } => Ok(Resp::Formatted {
                    error: client.format_file(&path).err(),
                }),
                Req::Rename {
                    path,
                    text,
                    line,
                    col,
                    new_name,
                } => client
                    .sync(&path, &text)
                    .and_then(|_| client.rename(&path, line, col, &new_name))
                    .map(Resp::Rename),
            };
            for (path, list) in client.take_published() {
                reply(Resp::Diagnostics { path, list });
            }
            reply(result.unwrap_or_else(Resp::Error));
        }
        client.stop();
    });
    Some(Handle {
        requests,
        responses,
    })
}

/// The server's diagnostic as a Problems entry (the list counts lines and columns from 1).
pub fn problem_for(path: &Path, d: &formust_lsp::Diagnostic) -> Diagnostic {
    Diagnostic {
        file: path.to_path_buf(),
        line: d.line + 1,
        col: d.col + 1,
        severity: match d.severity {
            1 => Severity::Error,
            2 => Severity::Warning,
            3 => Severity::Note,
            _ => Severity::Hint,
        },
        message: d.message.clone(),
    }
}

/// What rust-analyzer added to the Problems list, per file, so a republished or closed file takes
/// back exactly its own entries. An entry that the list already had (cargo reported the same) is not
/// added, and so is not taken back.
#[derive(Default)]
pub struct Published {
    added: HashMap<PathBuf, Vec<Diagnostic>>,
}

impl Published {
    /// Replaces the entries of `path` in `list` with `diags`, skipping identical ones.
    pub fn set(
        &mut self,
        list: &mut Vec<Diagnostic>,
        path: &Path,
        diags: &[formust_lsp::Diagnostic],
    ) {
        self.clear(list, path);
        let mut added = Vec::new();
        for d in diags {
            let problem = problem_for(path, d);
            if !list.contains(&problem) {
                list.push(problem.clone());
                added.push(problem);
            }
        }
        if !added.is_empty() {
            self.added.insert(path.to_path_buf(), added);
        }
    }

    /// Takes back the entries added for `path`.
    pub fn clear(&mut self, list: &mut Vec<Diagnostic>, path: &Path) {
        if let Some(old) = self.added.remove(path) {
            list.retain(|d| !old.contains(d));
        }
    }

    /// Takes back every entry (the server restarted or stopped).
    pub fn clear_all(&mut self, list: &mut Vec<Diagnostic>) {
        for (_, old) in self.added.drain() {
            list.retain(|d| !old.contains(d));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lsp_diag(line: u32, severity: u8, message: &str) -> formust_lsp::Diagnostic {
        formust_lsp::Diagnostic {
            line,
            col: 4,
            end_line: line,
            end_col: 6,
            severity,
            message: message.into(),
        }
    }

    #[test]
    fn a_server_diagnostic_becomes_a_one_based_problem() {
        let p = problem_for(Path::new("/p/a.rs"), &lsp_diag(2, 2, "unused"));
        assert_eq!((p.line, p.col, p.severity), (3, 5, Severity::Warning));
        assert_eq!(p.file, PathBuf::from("/p/a.rs"));
    }

    #[test]
    fn published_problems_merge_dedupe_and_clear() {
        let a = Path::new("/p/a.rs");
        let b = Path::new("/p/b.rs");
        let cargo = problem_for(a, &lsp_diag(0, 1, "boom"));
        let mut list = vec![cargo.clone()];
        let mut published = Published::default();
        published.set(
            &mut list,
            a,
            &[lsp_diag(0, 1, "boom"), lsp_diag(1, 2, "warn")],
        );
        published.set(&mut list, b, &[lsp_diag(0, 1, "other")]);
        assert_eq!(list.len(), 3, "the identical one is listed once");
        // Republished with fewer entries: the old ones go.
        published.set(&mut list, a, &[lsp_diag(1, 2, "warn")]);
        assert_eq!(list.len(), 3);
        // Republished empty: its own entries go, cargo's stays.
        published.set(&mut list, a, &[]);
        assert_eq!(list.len(), 2);
        assert!(list.contains(&cargo));
        // Closing b clears b.
        published.clear(&mut list, b);
        assert_eq!(list, vec![cargo.clone()]);
        published.set(&mut list, a, &[lsp_diag(1, 2, "warn")]);
        published.clear_all(&mut list);
        assert_eq!(list, vec![cargo]);
    }
}
