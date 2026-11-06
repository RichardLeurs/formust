//! Run Form: Run on a Rust file with a form builds a small program that shows that one form in a
//! window. Nothing is written into the project. The program is generated in the project's cargo
//! target folder (`<target>/formust-run/<hash>/`): one Rust file, a `Cargo.toml` with the project's
//! dependencies, and a `main.rs` that opens the window. Only that package is built, not the
//! project's binary.
//!
//! The Rust file is the form file as Save would write it, with the unsaved edits: a standalone copy
//! of the source, so nothing else is needed. While a form is unedited and has no form file, it is the
//! source's text (an open tab's, unsaved). That text gets a `formust_run` module appended. It sits inside the owner's module, so
//! it can use the owner's private items, and it calls the form's function every frame: a `Ui` form
//! inside a `CentralPanel`, a dialog (a `Context` form) with the `ctx`, a free dialog as
//! `owner::fn(ctx)`, a free `fn f(ui: &mut egui::Ui)` as `owner::f(ui)` inside a `CentralPanel`. A
//! dialog starts open: the `bool` fields that keep it closed are set to `true` when the run starts
//! (see [`formust_rust::run::open_flags`]). A headless run (`FORMUST_RUN_FORM_TEST=1`) draws one frame
//! instead and prints the texts it drew; the tests use it.
//!
//! The window has no native title bar: the program draws a strip in its place with the title and a
//! Light/Dark switch on the right (see [`MAIN`]). It starts in the designer canvas's theme, passed in
//! [`THEME_VAR`]; Live Run also keeps the theme last chosen in its window (see [`live`]).
//!
//! Each run also removes old program folders of other sources under `<target>/formust-run/` (see
//! [`clean_old`]); nothing outside that folder is removed.
//!
//! The pure parts (manifest, copies, placement, the choice of form) are here and unit-tested. Reading
//! the project and running cargo happen on a worker thread ([`prepare`]), so the window never waits
//! for them.

use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use crate::bottom;
use crate::cargo::{self, Session};
use crate::designer;
use eframe::egui;
use formust_build::{Action, Diagnostic, Request, Severity};
use formust_model::Form;
use formust_rust::{FileKind, Origin, file_kind};

/// The folder under the target folder where the generated programs live.
const FOLDER: &str = "formust-run";
/// The file in a program folder whose time says when it last ran (see [`clean_old`]). Outside `src/`,
/// so cargo's build of the program does not depend on it.
const STAMP: &str = "formust-last-run";
/// Program folders not run for this long are removed (see [`clean_old`]).
const KEEP_FOR: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 60 * 60);
/// At most this many program folders are kept, the most recently run first (see [`clean_old`]).
const KEEP_COUNT: usize = 16;

/// What the designer shows, as Run Form needs it. Built on the UI thread from the open tabs and the
/// designer's state, so unsaved text is what the program gets.
#[derive(Clone, Debug)]
pub struct Inputs {
    /// The file that runs, where the form's owner type lives: the form file (the copy), or the
    /// source while the form is unedited and has no form file.
    pub owner: PathBuf,
    /// Its text: the copy as Save would write it, or the source's open tab, else the disk's.
    pub owner_text: String,
    /// The form's identity, `owner::method`.
    pub form_id: String,
    /// The owner type; empty for a free function ([`Kind::FreeDialog`] or [`Kind::FreeUi`]).
    pub owner_type: String,
    pub method: String,
    pub kind: Kind,
}

/// How the form is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `fn m(&mut self, ui: &mut egui::Ui)`, drawn inside a `CentralPanel`.
    Ui,
    /// `fn m(&mut self, ctx: &egui::Context)`: a dialog, called every frame.
    Context,
    /// The old `fn update(&mut self, ctx, frame)`: called with the frame, in the window only.
    Legacy,
    /// `fn ui(&mut self, ui, frame)` of an `eframe::App` (a legacy `update` in the copy is one):
    /// called through the trait with the frame, in the window only.
    App,
    /// A free dialog, `fn m(ctx: &egui::Context)` with no owner type: called as `owner::m(ctx)` every
    /// frame. The copy keeps the function, so it must compile on its own (see [`crate_note`]).
    FreeDialog,
    /// A free `fn m(ui: &mut egui::Ui)` with no owner type: called as `owner::m(ui)` inside a
    /// `CentralPanel`. Only a function whose one parameter is the `ui` runs (see
    /// [`formust_rust::run::takes_only_ui`]).
    FreeUi,
}

/// The project as the generated manifest needs it.
#[derive(Clone, Debug, Default)]
pub struct ProjectInfo {
    /// The project's `Cargo.toml`.
    pub manifest: PathBuf,
    pub manifest_text: String,
    /// The workspace root and its `Cargo.toml`, for dependencies inherited from it.
    pub workspace: Option<(PathBuf, String)>,
    /// The `Cargo.lock` of the workspace, so the program builds with the project's versions.
    pub lock: Option<String>,
}

/// The files of the generated package, before they are written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub manifest: String,
    pub lock: Option<String>,
    pub main: String,
    /// The file that runs, with the harness appended.
    pub owner: String,
    /// The owner's `bool` fields set to `true` at the start, so a dialog starts open.
    pub opened: Vec<String>,
}

/// Where the copies stand for the real files, so that a diagnostic in a copy is shown in the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    dir: PathBuf,
    /// `dir` as cargo may report it (canonical).
    canonical: PathBuf,
    owner_real: PathBuf,
}

impl Places {
    /// The source's file name, for messages.
    pub fn owner_name(&self) -> String {
        file_name(&self.owner_real)
    }

    /// The diagnostic with its file replaced by the project's. A message about the generated
    /// `Cargo.toml` or `main.rs` has no place in the project: it goes to the source at line 0.
    pub fn real(&self, mut d: Diagnostic) -> Diagnostic {
        let rel = [&self.dir, &self.canonical]
            .into_iter()
            .find_map(|dir| d.file.strip_prefix(dir).ok().map(Path::to_path_buf));
        let Some(rel) = rel else {
            return d;
        };
        let rel = rel
            .strip_prefix("src")
            .map(Path::to_path_buf)
            .unwrap_or(rel);
        d.file = self.owner_real.clone();
        if rel != Path::new("owner.rs") {
            d.line = 0;
            d.col = 0;
        }
        d
    }
}

/// A prepared run: the request to start, and where its diagnostics go.
pub struct Plan {
    pub request: Request,
    pub places: Places,
    pub form_id: String,
    pub dir: PathBuf,
    /// A line for Output about the copy, when it may not build on its own (see [`crate_note`]).
    pub note: Option<String>,
}

/// A run being prepared on a worker thread. `stopping` is set by Stop: the run is then not started.
pub struct Prep {
    pub rx: Receiver<Result<Plan, Vec<Diagnostic>>>,
    pub stopping: bool,
}

// ---- what the messages say ----------------------------------------------------------------------

fn needs_default(owner: &str) -> String {
    format!(
        "Run Form needs `impl Default for {owner}`: the window starts from `{owner}::default()`. Add `#[derive(Default)]` above `struct {owner}`, or write `impl Default for {owner}` by hand."
    )
}

/// The note under the Output when a run fails: the copy is built on its own.
pub fn own_build_note(source: &str) -> String {
    format!(
        "Run Form builds {source} on its own. If an error is about an item from the rest of the crate (a `mod`, or a `crate::` path), the form can't run this way yet: see Problems.\n"
    )
}

/// Output's line when Run is pressed on a Rust file with no form: the project's program runs instead.
pub const NO_FORM: &str = "no form in this file, running the project";

/// Output's line for a dialog that the run starts open: which of the owner's fields it set. `None`
/// when it set none.
pub fn open_note(opened: &[String]) -> Option<String> {
    (!opened.is_empty()).then(|| {
        let fields: Vec<String> = opened.iter().map(|f| format!("`self.{f}`")).collect();
        format!(
            "the dialog starts open: Run Form sets {} to true in the run (the project's code is not changed).",
            fields.join(", ")
        )
    })
}

/// Output's line for a file whose text uses `crate::` paths: it is built alone, so those paths don't
/// resolve in it. `None` when it has none.
pub fn crate_note(owner_text: &str) -> Option<String> {
    owner_text.contains("crate::").then(|| {
        "this form uses code from elsewhere in the project (crate::…); Run Form copies only this file."
            .to_string()
    })
}

// ---- pure generation ----------------------------------------------------------------------------

/// The form's kind as the designer reads it.
pub fn kind_of(form: &Form) -> Kind {
    if form.context_root && form.owner.is_empty() {
        Kind::FreeDialog
    } else if form.owner.is_empty() && form.legacy_ctx.is_none() {
        Kind::FreeUi
    } else if form.context_root {
        Kind::Context
    } else if form.legacy_ctx.is_some() {
        Kind::Legacy
    } else if form.impl_header.contains(" for ")
        && form.method == "ui"
        && form.sig.contains("Frame")
    {
        Kind::App
    } else {
        Kind::Ui
    }
}

/// The kind and method of the form as the copy has it: Save writes a legacy `update` as the
/// `eframe::App` method `fn ui(&mut self, ui, frame)`; a dialog keeps its name and its `ctx`.
pub fn in_copy(kind: Kind, method: &str) -> (Kind, String) {
    match kind {
        Kind::Legacy => (Kind::App, "ui".to_string()),
        kind => (kind, method.to_string()),
    }
}

/// The index of the form Run Form runs, in `ids` (the forms' ids, in order). `spans[i]` is the byte
/// range of form `i`'s function in `text`, or `None` when it was read from the other file. In code
/// view (`caret_line`, 1-based) the form whose function holds the caret's line; else the designer's
/// form (`designer_id`) when it is one of them; else the first.
pub fn pick_form(
    ids: &[String],
    spans: &[Option<Range<usize>>],
    text: &str,
    designer_id: Option<&str>,
    caret_line: Option<u32>,
) -> Option<usize> {
    let under_caret = caret_line.and_then(|line| {
        spans.iter().position(|span| {
            span.as_ref().is_some_and(|s| {
                line_of(text, s.start) <= line && line_of(text, s.end.saturating_sub(1)) >= line
            })
        })
    });
    under_caret
        .or_else(|| designer_id.and_then(|id| ids.iter().position(|i| i == id)))
        .or((!ids.is_empty()).then_some(0))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// An error with no more place than a file (and a line, 0 for none).
fn problem(file: &Path, line: u32, message: String) -> Diagnostic {
    Diagnostic {
        file: file.to_path_buf(),
        line,
        col: 0,
        severity: Severity::Error,
        message,
    }
}

/// The 1-based line of a byte offset.
fn line_of(text: &str, at: usize) -> u32 {
    text.get(..at)
        .map_or(0, |t| t.matches('\n').count() as u32 + 1)
}

/// A stable 64-bit hash (FNV-1a), so a source always gets the same folder.
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The key of the program for `owner`: a stable hash of its path, which names its folder and package.
fn key_of(owner: &Path) -> String {
    format!("{:016x}", fnv1a(&owner.to_string_lossy()))
}

/// The folder of the program for `owner`, under the target folder.
pub fn dir_for(target: &Path, owner: &Path) -> PathBuf {
    target.join(FOLDER).join(key_of(owner))
}

/// The package name of the program for `owner`. Every copy declares its own `[workspace]`, so cargo
/// keys its build by the name alone: with one shared name, a copy of another form whose own files
/// look unchanged would run that form's binary. A name per source keeps the builds apart.
pub fn package_name(owner: &Path) -> String {
    format!("formust-run-{}", key_of(owner))
}

/// `path` with its `.` and `..` resolved in the text (nothing is read from the disk), so a path
/// dependency reads `/p/mylib`, not `/p/app/../mylib`.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !out.pop() {
                    out.push(part);
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn inherits(table: &toml::Table) -> bool {
    table.get("workspace") == Some(&toml::Value::Boolean(true))
}

fn version_table(version: &str) -> toml::Table {
    let mut t = toml::Table::new();
    t.insert("version".into(), toml::Value::String(version.to_string()));
    t
}

/// One dependency as the generated manifest writes it: a table, with a relative `path` made absolute
/// (against `dir`, or the workspace's folder when inherited). `None` when the workspace has no entry.
fn dependency(
    name: &str,
    value: &toml::Value,
    dir: &Path,
    workspace: Option<(&toml::Table, &Path)>,
) -> Option<toml::Value> {
    let mut table = match value {
        toml::Value::Table(t) => t.clone(),
        toml::Value::String(version) => version_table(version),
        _ => return None,
    };
    let mut dir = dir.to_path_buf();
    if inherits(&table) {
        let (ws, ws_dir) = workspace?;
        let inherited = ws.get("workspace")?.get("dependencies")?.get(name)?;
        let mut merged = match inherited {
            toml::Value::Table(t) => t.clone(),
            toml::Value::String(version) => version_table(version),
            _ => return None,
        };
        if let Some(toml::Value::String(path)) = merged.get("path").cloned() {
            merged.insert(
                "path".into(),
                toml::Value::String(lexical(&ws_dir.join(path)).to_string_lossy().into_owned()),
            );
        }
        for (key, value) in table {
            if key == "workspace" {
                continue;
            }
            // Features add to the inherited ones, as cargo merges them; other keys override.
            let value = match (key.as_str(), merged.get(&key), &value) {
                ("features", Some(toml::Value::Array(old)), toml::Value::Array(new)) => {
                    let mut all = old.clone();
                    all.extend(new.iter().cloned());
                    toml::Value::Array(all)
                }
                _ => value,
            };
            merged.insert(key, value);
        }
        table = merged;
        dir = ws_dir.to_path_buf();
    }
    if let Some(toml::Value::String(path)) = table.get("path").cloned() {
        table.insert(
            "path".into(),
            toml::Value::String(lexical(&dir.join(path)).to_string_lossy().into_owned()),
        );
    }
    Some(toml::Value::Table(table))
}

/// The manifest of the generated package: the project's `[dependencies]` (and `[patch]`) with relative
/// paths made absolute, its edition, and `eframe` (at egui's version) when the project only has egui.
pub fn manifest(project: &ProjectInfo, name: &str) -> Result<String, String> {
    let file = file_name(&project.manifest);
    let table: toml::Table = project
        .manifest_text
        .parse()
        .map_err(|e| format!("Run Form could not read {file}: {e}"))?;
    let dir = project.manifest.parent().unwrap_or(Path::new(""));
    let workspace: Option<toml::Table> = project
        .workspace
        .as_ref()
        .and_then(|(_, text)| text.parse().ok());
    let ws = workspace
        .as_ref()
        .zip(project.workspace.as_ref().map(|(d, _)| d.as_path()));
    let ws_package = |key: &str| {
        workspace
            .as_ref()?
            .get("workspace")?
            .get("package")?
            .get(key)?
            .as_str()
            .map(str::to_string)
    };
    let edition = match table.get("package").and_then(toml::Value::as_table) {
        Some(package) => match package.get("edition") {
            Some(toml::Value::String(edition)) => Some(edition.clone()),
            Some(toml::Value::Table(t)) if inherits(t) => ws_package("edition"),
            _ => None,
        },
        None => None,
    }
    .unwrap_or_else(|| "2024".to_string());

    let mut deps = toml::Table::new();
    if let Some(list) = table.get("dependencies").and_then(toml::Value::as_table) {
        for (name, value) in list {
            let resolved = dependency(name, value, dir, ws).ok_or_else(|| {
                format!(
                    "Run Form could not resolve `{name}` in {file}: it is inherited from the workspace, which has no entry for it"
                )
            })?;
            deps.insert(name.clone(), resolved);
        }
    }
    if !deps.contains_key("eframe") {
        let egui_version = deps
            .get("egui")
            .and_then(|v| v.get("version"))
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        let Some(version) = egui_version else {
            return Err(format!(
                "Run Form needs `eframe` in [dependencies] of {file}: add `eframe = \"0.37\"`, the version of the egui your form uses."
            ));
        };
        deps.insert("eframe".into(), toml::Value::Table(version_table(&version)));
    }

    let mut out = toml::Table::new();
    let mut own = toml::Table::new();
    own.insert("name".into(), toml::Value::String(name.to_string()));
    own.insert("version".into(), toml::Value::String("0.0.0".into()));
    own.insert("edition".into(), toml::Value::String(edition));
    own.insert("publish".into(), toml::Value::Boolean(false));
    out.insert("package".into(), toml::Value::Table(own));
    out.insert("dependencies".into(), toml::Value::Table(deps));
    if let Some(patches) = table.get("patch").and_then(toml::Value::as_table) {
        let mut patch = toml::Table::new();
        for (registry, list) in patches {
            let Some(list) = list.as_table() else {
                continue;
            };
            let mut resolved = toml::Table::new();
            for (name, value) in list {
                if let Some(v) = dependency(name, value, dir, None) {
                    resolved.insert(name.clone(), v);
                }
            }
            patch.insert(registry.clone(), toml::Value::Table(resolved));
        }
        out.insert("patch".into(), toml::Value::Table(patch));
    }
    // An empty workspace: the generated package is not a member of the project's workspace.
    out.insert("workspace".into(), toml::Value::Table(toml::Table::new()));
    toml::to_string(&out).map_err(|e| format!("Run Form could not write the manifest: {e}"))
}

/// The harness appended to the copy of the source. `@...@` marks what depends on the form.
const HARNESS: &str = r#"// Formust's Run Form harness, added to a temporary copy of this file. The project's file is not changed.
#[doc(hidden)]
#[allow(dead_code, unused)]
pub mod formust_run {
    use eframe::egui;

    @STRUCT@

    impl Runner {
        fn new() -> Self {
            @NEW@
        }
    }

@DRAW@
    impl eframe::App for Runner {
        fn ui(&mut self, ui: &mut egui::Ui, @FRAME@: &mut eframe::Frame) {
            @APP_BODY@
        }
    }

    pub fn app() -> Box<dyn eframe::App> {
        Box::new(Runner::new())
    }

    /// The texts one headless frame draws (FORMUST_RUN_FORM_TEST). Two passes: a layout needs one.
    pub fn headless() -> Vec<String> {
@HEADLESS@
    }
}
"#;

/// The headless drawing of a `Ui` or `Context` form: one frame, its texts. Two passes, and the texts
/// of the second are kept, since a layout needs a pass.
const HEADLESS_DRAW: &str = r#"        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut app = Runner::new();
        let mut out = Vec::new();
        for _ in 0..2 {
            let mut full = ctx.run_ui(egui::RawInput::default(), |ui| draw(&mut app, ui));
            // No renderer takes the font texture here.
            full.textures_delta.clear();
            out.clear();
            for clipped in &full.shapes {
                texts(&clipped.shape, &mut out);
            }
        }
        out"#;

/// The harness for `owner_type::method`. A `Ui` form is drawn inside a `CentralPanel`; a dialog is
/// called with the `ctx` every frame; a free dialog is `super::method(ctx)` with no owner; a legacy
/// `update` and an `eframe::App`'s `ui` are called with the frame from the window's
/// `eframe::App::ui`, so they have no headless drawing.
///
/// `opened` are `bool` fields of the owner set to `true` before the first frame, so a dialog that its
/// own flag keeps closed starts open (see [`formust_rust::run::open_flags`]).
pub fn harness(owner_type: &str, method: &str, kind: Kind, opened: &[String]) -> String {
    // `app.owner.m(args)`: the owner is the value `Runner::new` makes with `Default`.
    let call = |recv: &str, args: &str| format!("{recv}.owner.{method}({args})");
    let (structure, new) = if matches!(kind, Kind::FreeDialog | Kind::FreeUi) {
        ("pub struct Runner;".to_string(), "Runner".to_string())
    } else if opened.is_empty() {
        (
            format!("pub struct Runner {{\n        owner: super::{owner_type},\n    }}"),
            format!("Runner {{ owner: super::{owner_type}::default() }}"),
        )
    } else {
        let sets: String = opened
            .iter()
            .map(|field| format!("            owner.{field} = true;\n"))
            .collect();
        (
            format!("pub struct Runner {{\n        owner: super::{owner_type},\n    }}"),
            format!(
                "// The dialog starts open.\n            let mut owner = super::{owner_type}::default();\n{sets}            Runner {{ owner }}"
            ),
        )
    };
    let (draw, app_body, frame, headless) = match kind {
        Kind::FreeDialog => (
            format!(
                "    fn draw(_app: &mut Runner, ui: &mut egui::Ui) {{\n        let _ = super::{method}(ui.ctx());\n    }}\n"
            ),
            "draw(self, ui);".to_string(),
            "_frame",
            HEADLESS_DRAW.to_string(),
        ),
        Kind::FreeUi => (
            format!(
                "    fn draw(_app: &mut Runner, ui: &mut egui::Ui) {{\n        egui::CentralPanel::default().show(ui, |ui| {{\n            let _ = super::{method}(ui);\n        }});\n    }}\n"
            ),
            "draw(self, ui);".to_string(),
            "_frame",
            HEADLESS_DRAW.to_string(),
        ),
        Kind::Ui => (
            format!(
                "    fn draw(app: &mut Runner, ui: &mut egui::Ui) {{\n        egui::CentralPanel::default().show(ui, |ui| {{\n            let _ = {};\n        }});\n    }}\n",
                call("app", "ui")
            ),
            "draw(self, ui);".to_string(),
            "_frame",
            HEADLESS_DRAW.to_string(),
        ),
        Kind::Context => (
            format!(
                "    fn draw(app: &mut Runner, ui: &mut egui::Ui) {{\n        let _ = {};\n    }}\n",
                call("app", "ui.ctx()")
            ),
            "draw(self, ui);".to_string(),
            "_frame",
            HEADLESS_DRAW.to_string(),
        ),
        Kind::Legacy => (
            String::new(),
            format!("let _ = {};", call("self", "ui.ctx(), frame")),
            "frame",
            "        Vec::new()".to_string(),
        ),
        Kind::App => (
            String::new(),
            "eframe::App::ui(&mut self.owner, ui, frame);".to_string(),
            "frame",
            "        Vec::new()".to_string(),
        ),
    };
    HARNESS
        .replace("@STRUCT@", &structure)
        .replace("@NEW@", &new)
        .replace("@DRAW@", &draw)
        .replace("@FRAME@", frame)
        .replace("@APP_BODY@", &app_body)
        .replace("@HEADLESS@", &headless)
}

// ---- the window's theme ---------------------------------------------------------------------------

/// The environment variable that gives the program its start theme: `light`, `dark` or `system`
/// (anything else is `system`). Run Form and Live Run set it from the designer's canvas.
pub const THEME_VAR: &str = "FORMUST_RUN_FORM_THEME";
/// When set, the program prints [`THEME_LINE`] and the theme each time the Light/Dark switch is used.
/// Live Run sets it, reads the line from the program's output and starts the next window in that
/// theme.
pub const THEME_REPORT_VAR: &str = "FORMUST_RUN_FORM_THEME_REPORT";
/// The start of the line the program prints when its theme is switched (see [`THEME_REPORT_VAR`]).
pub const THEME_LINE: &str = "formust-theme: ";

/// The theme a Run Form window starts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunTheme {
    Light,
    Dark,
    System,
}

impl RunTheme {
    /// Its value for [`THEME_VAR`].
    pub fn as_str(self) -> &'static str {
        match self {
            RunTheme::Light => "light",
            RunTheme::Dark => "dark",
            RunTheme::System => "system",
        }
    }

    /// A value of [`THEME_VAR`], as the program reads it: case and spaces don't count; `None` for an
    /// unknown value (the program then follows the system).
    pub fn parse(text: &str) -> Option<RunTheme> {
        match text.trim().to_ascii_lowercase().as_str() {
            "light" => Some(RunTheme::Light),
            "dark" => Some(RunTheme::Dark),
            "system" => Some(RunTheme::System),
            _ => None,
        }
    }

    /// The theme of the designer's canvas: its own choice (`canvas`, `Some(true)` for dark), else
    /// the IDE's (`ide_dark`), which it follows then.
    pub fn of_canvas(canvas: Option<bool>, ide_dark: bool) -> RunTheme {
        if canvas.unwrap_or(ide_dark) {
            RunTheme::Dark
        } else {
            RunTheme::Light
        }
    }

    /// The theme a line of the program's output reports (see [`THEME_LINE`]); `None` for any other
    /// line.
    pub fn from_line(line: &str) -> Option<RunTheme> {
        RunTheme::parse(line.strip_prefix(THEME_LINE)?)
    }
}

/// The generated `main.rs`. `@TITLE@` is the window's title. The native title bar is hidden (its
/// close, minimise and zoom buttons stay), and a strip of the same height drawn by the program takes
/// its place: it shows the title, drags the window, zooms it on a double-click, and holds the
/// Light/Dark switch on the right. The form is drawn below it, in the space it had under the native
/// title bar. The headless run (`FORMUST_RUN_FORM_TEST`) prints the form's texts (`drawn:`), then the
/// strip's (`strip:`), its start theme (`theme:`) and its theme after a click on the switch's other
/// side (`theme after a click:`).
const MAIN: &str = r#"//! Formust's Run Form program: one form in one window, under a title strip with a Light/Dark
//! switch. Generated in a temporary folder.
#[allow(dead_code, unused)]
mod owner;

use eframe::egui;

/// The window's title, drawn in the strip: the native title bar is hidden.
const TITLE: &str = @TITLE@;
/// The strip's height: the native macOS title bar's.
const STRIP: f32 = 28.0;
/// Room on the left for the close, minimise and zoom buttons.
const BUTTONS: f32 = 78.0;

/// The start theme, from FORMUST_RUN_FORM_THEME: light, dark, else the system's.
fn start_theme() -> egui::ThemePreference {
    let value = std::env::var("FORMUST_RUN_FORM_THEME").unwrap_or_default();
    match value.trim().to_ascii_lowercase().as_str() {
        "light" => egui::ThemePreference::Light,
        "dark" => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    }
}

fn theme_name(theme: egui::Theme) -> &'static str {
    match theme {
        egui::Theme::Light => "light",
        egui::Theme::Dark => "dark",
    }
}

/// The title strip at the top of `ui`. Returns the rects of the switch's Light and Dark sides.
fn strip(ui: &mut egui::Ui) -> (egui::Rect, egui::Rect) {
    egui::Panel::top("formust_title_strip")
        .exact_size(STRIP)
        .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 0)))
        .show(ui, |ui| {
            let bar = ui.max_rect();
            // Registered first, so the switch keeps its clicks: the empty strip drags the window
            // and a double-click zooms it, as the native title bar does.
            let drag = ui.interact(bar, egui::Id::new("formust_title_drag"), egui::Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            let title = ui.painter().layout_no_wrap(
                TITLE.to_string(),
                egui::FontId::proportional(13.0),
                ui.visuals().strong_text_color(),
            );
            // Centred, but never under the window's buttons.
            let left = (bar.center().x - title.size().x / 2.0).max(bar.left() + BUTTONS);
            let at = egui::pos2(left, bar.center().y - title.size().y / 2.0);
            ui.painter().galley(at, title, ui.visuals().strong_text_color());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let current = ui.ctx().theme();
                let dark = ui
                    .selectable_label(current == egui::Theme::Dark, "Dark")
                    .on_hover_text("Show the form in dark mode");
                let light = ui
                    .selectable_label(current == egui::Theme::Light, "Light")
                    .on_hover_text("Show the form in light mode");
                for (side, theme) in [(&light, egui::Theme::Light), (&dark, egui::Theme::Dark)] {
                    if side.clicked() && current != theme {
                        ui.ctx().set_theme(theme);
                        // Live Run reads this, so its next window starts in the same theme.
                        if std::env::var_os("FORMUST_RUN_FORM_THEME_REPORT").is_some() {
                            println!("formust-theme: {}", theme_name(theme));
                        }
                    }
                }
                (light.rect, dark.rect)
            })
            .inner
        })
        .inner
}

/// The form below the title strip.
struct Shell {
    form: Box<dyn eframe::App>,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        strip(ui);
        self.form.ui(ui, frame);
    }
}

/// The strip headless: its texts and theme, then a click on the switch's other side.
fn headless_strip() {
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    ctx.set_theme(start_theme());
    let pass = |events: Vec<egui::Event>| {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut sides = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        let mut full = ctx.run_ui(input, |ui| sides = strip(ui));
        // No renderer takes the font texture here.
        full.textures_delta.clear();
        let mut out = Vec::new();
        for clipped in &full.shapes {
            texts(&clipped.shape, &mut out);
        }
        (out, sides)
    };
    pass(Vec::new());
    let (drawn, sides) = pass(Vec::new());
    println!("strip: {}", drawn.join(" | "));
    println!("theme: {}", theme_name(ctx.theme()));
    let other = if ctx.theme() == egui::Theme::Dark { sides.0 } else { sides.1 };
    let pos = other.center();
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    pass(vec![egui::Event::PointerMoved(pos)]);
    pass(vec![button(true)]);
    pass(vec![button(false)]);
    pass(Vec::new());
    println!("theme after a click: {}", theme_name(ctx.theme()));
}

fn main() {
    if std::env::var_os("FORMUST_RUN_FORM_TEST").is_some() {
        println!("drawn: {}", owner::formust_run::headless().join(" | "));
        headless_strip();
        return;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(true),
        ..Default::default()
    };
    let result = eframe::run_native(
        TITLE,
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(start_theme());
            Ok(Box::new(Shell {
                form: owner::formust_run::app(),
            }))
        }),
    );
    if let Err(error) = result {
        eprintln!("The window could not open: {error}");
        std::process::exit(1);
    }
}
"#;

/// The generated `main.rs` (see [`MAIN`]): the headless run when `FORMUST_RUN_FORM_TEST` is set,
/// else the window, under its title strip, in the theme of [`THEME_VAR`].
pub fn main_rs(title: &str) -> String {
    MAIN.replace("@TITLE@", &format!("{title:?}"))
}

/// The generated package for `input`, or the problems that stop it: a free function that is no
/// dialog and takes more than the `ui`, a type with no `Default`, a manifest that can't be made.
pub fn package(input: &Inputs, project: &ProjectInfo) -> Result<Package, Vec<Diagnostic>> {
    let source = input.owner.as_path();
    let free_runs = match input.kind {
        Kind::FreeDialog => true,
        Kind::FreeUi => formust_rust::run::takes_only_ui(&input.owner_text, &input.method),
        _ => false,
    };
    if input.owner_type.is_empty() && !free_runs {
        return Err(vec![problem(
            source,
            0,
            format!(
                "Run Form runs a method of a type, a free dialog (`fn {m}(ctx: &egui::Context)`) or a free function whose only parameter is the ui (`fn {m}(ui: &mut egui::Ui)`), and `{m}` is a free function that is neither. Call it from a method of a type with `impl Default` (such as `ui`) and run that form instead.",
                m = input.method
            ),
        )]);
    }
    if !input.owner_type.is_empty()
        && formust_rust::usage::declares_type(&input.owner_text, &input.owner_type)
        && !formust_rust::usage::has_default(&input.owner_text, &input.owner_type)
    {
        // Placed at the impl that holds the form, when the text imports.
        let line = formust_rust::import(&input.owner_text)
            .ok()
            .and_then(|p| {
                p.forms
                    .iter()
                    .find(|f| f.form.owner == input.owner_type && f.form.method == input.method)
                    .map(|f| line_of(&input.owner_text, f.impl_range.start))
            })
            .unwrap_or(0);
        return Err(vec![problem(
            source,
            line,
            needs_default(&input.owner_type),
        )]);
    }

    let name = package_name(source);
    let manifest = manifest(project, &name).map_err(|m| vec![problem(&project.manifest, 0, m)])?;
    let mut owner = input.owner_text.clone();
    if !owner.ends_with('\n') {
        owner.push('\n');
    }
    owner.push('\n');
    let opened = if input.kind == Kind::Context {
        formust_rust::run::open_flags(&input.owner_text, &input.owner_type, &input.method)
    } else {
        Vec::new()
    };
    owner.push_str(&harness(
        &input.owner_type,
        &input.method,
        input.kind,
        &opened,
    ));
    Ok(Package {
        manifest,
        lock: project.lock.clone(),
        main: main_rs(&input.form_id),
        owner,
        opened,
    })
}

// ---- files and threads --------------------------------------------------------------------------

fn write_if_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(());
    }
    std::fs::write(path, text)
}

/// Writes `package` into `dir`, with `src/` for the copies. Unchanged files are left alone, so cargo's
/// build of an unchanged program stays fresh.
pub fn write(dir: &Path, package: &Package) -> std::io::Result<()> {
    let src = dir.join("src");
    std::fs::create_dir_all(&src)?;
    write_if_changed(&dir.join("Cargo.toml"), &package.manifest)?;
    if let Some(lock) = &package.lock {
        write_if_changed(&dir.join("Cargo.lock"), lock)?;
    }
    write_if_changed(&src.join("main.rs"), &package.main)?;
    write_if_changed(&src.join("owner.rs"), &package.owner)?;
    Ok(())
}

/// Is `name` a program folder's name: a key as [`key_of`] writes it, 16 lowercase hex digits?
fn is_key(name: &str) -> bool {
    name.len() == 16 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Marks `dir` as just run, for [`clean_old`].
fn stamp(dir: &Path) -> std::io::Result<()> {
    std::fs::write(dir.join(STAMP), b"")
}

/// When the program in `dir` last ran: its stamp's time, else the folder's.
fn last_run(dir: &Path) -> Option<std::time::SystemTime> {
    std::fs::symlink_metadata(dir.join(STAMP))
        .or_else(|_| std::fs::symlink_metadata(dir))
        .and_then(|m| m.modified())
        .ok()
}

/// Removes old program folders from `root` (a `<target>/formust-run` folder) and returns them: those
/// not run for `max_age` (before `now`), and beyond the `max_count` most recently run. `keep` (the
/// folder of the run starting now) always stays. Only real folders directly in `root` whose name is
/// a key (see [`key_of`]) are touched: a file, a link or any other name stays, and so does all of
/// `root` when it is no `formust-run` folder or is a link. Errors leave a folder in place.
pub fn clean_old(
    root: &Path,
    keep: &Path,
    now: std::time::SystemTime,
    max_age: std::time::Duration,
    max_count: usize,
) -> Vec<PathBuf> {
    if root.file_name() != Some(std::ffi::OsStr::new(FOLDER)) {
        return Vec::new();
    }
    if !std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir()) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut folders: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_str().is_some_and(is_key))
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()))
        .map(|e| e.path())
        .filter(|path| path.as_path() != keep)
        .map(|path| (last_run(&path).unwrap_or(std::time::UNIX_EPOCH), path))
        .collect();
    // The most recent first; `keep` counts as one of the kept.
    folders.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    let room = max_count.saturating_sub(1);
    let mut removed = Vec::new();
    for (index, (time, path)) in folders.into_iter().enumerate() {
        let old = now.duration_since(time).is_ok_and(|age| age > max_age);
        if (old || index >= room) && std::fs::remove_dir_all(&path).is_ok() {
            removed.push(path);
        }
    }
    removed
}

/// Reads the project of `input`, asks cargo where its target folder is, and writes the program there.
/// `target` replaces the project's target folder (the tests use a shared one). Runs on a worker thread:
/// cargo's `metadata` takes a moment.
pub fn prepare(input: Inputs, target: Option<PathBuf>) -> Result<Plan, Vec<Diagnostic>> {
    prepare_for(input, target, Action::Run, None).map(|(plan, _)| plan)
}

/// What `cargo metadata` said about a project, kept by Live Run so that a rebuild with the same
/// `Cargo.toml` (same path, same text) does not ask cargo again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectCache {
    pub manifest: PathBuf,
    pub manifest_text: String,
    pub workspace_root: PathBuf,
    pub target_directory: PathBuf,
}

/// [`prepare`] with the cargo `action` of the request (Run Form runs, Live Run only builds) and
/// what an earlier call read about the project: `cargo metadata` runs only when `cache` is for
/// another manifest or its text changed. Returns the plan and what to keep for the next call.
pub fn prepare_for(
    input: Inputs,
    target: Option<PathBuf>,
    action: Action,
    cache: Option<ProjectCache>,
) -> Result<(Plan, ProjectCache), Vec<Diagnostic>> {
    let owner = input.owner.clone();
    let io = |message: String| vec![problem(&owner, 0, message)];
    let manifest = formust_build::manifest_for(&owner).map_err(|e| {
        io(format!(
            "Run Form could not find the Cargo.toml of {}: {e}",
            file_name(&owner)
        ))
    })?;
    let manifest_text = std::fs::read_to_string(&manifest).map_err(|e| {
        io(format!(
            "Run Form could not read {}: {e}",
            manifest.display()
        ))
    })?;
    let cache = match cache.filter(|c| c.manifest == manifest && c.manifest_text == manifest_text) {
        Some(cache) => cache,
        None => {
            let meta = formust_build::metadata(None, &manifest)
                .map_err(|e| io(format!("Run Form could not read the project: {e}")))?;
            ProjectCache {
                manifest: manifest.clone(),
                manifest_text: manifest_text.clone(),
                workspace_root: meta.workspace_root,
                target_directory: meta.target_directory,
            }
        }
    };
    let workspace_root = cache.workspace_root.clone();
    let project = ProjectInfo {
        workspace: std::fs::read_to_string(workspace_root.join("Cargo.toml"))
            .ok()
            .map(|t| (workspace_root.clone(), t)),
        lock: std::fs::read_to_string(workspace_root.join("Cargo.lock")).ok(),
        manifest,
        manifest_text,
    };
    let package = package(&input, &project)?;
    let notes: Vec<String> = [crate_note(&input.owner_text), open_note(&package.opened)]
        .into_iter()
        .flatten()
        .collect();
    let note = (!notes.is_empty()).then(|| notes.join("\n"));
    let target_dir = target.unwrap_or_else(|| cache.target_directory.clone());
    let dir = dir_for(&target_dir, &owner);
    write(&dir, &package)
        .map_err(|e| io(format!("Run Form could not write {}: {e}", dir.display())))?;
    // The stamp only dates the folder for cleaning; a failure to write it is no reason not to run.
    let _ = stamp(&dir);
    clean_old(
        &target_dir.join(FOLDER),
        &dir,
        std::time::SystemTime::now(),
        KEEP_FOR,
        KEEP_COUNT,
    );
    let places = Places {
        canonical: dir.canonicalize().unwrap_or_else(|_| dir.clone()),
        dir: dir.clone(),
        owner_real: owner,
    };
    let mut request = Request::new(action, dir.join("Cargo.toml"));
    request.env = vec![("CARGO_TARGET_DIR".into(), target_dir.into_os_string())];
    Ok((
        Plan {
            request,
            places,
            form_id: input.form_id,
            dir,
            note,
        },
        cache,
    ))
}

// ---- the App ------------------------------------------------------------------------------------

impl crate::App {
    /// Run (Cmd+R, the menu, the toolbar): Run Form for the form of the active Rust file, whether the
    /// designer or the code view shows it, or for the free `fn f(ui: &mut egui::Ui)` under the caret
    /// in code view. A Rust file with neither, and any other file, runs the project's own program
    /// (`cargo run`); a Rust file with no form says why in Output.
    pub(crate) fn run_active(&mut self, ctx: &egui::Context) {
        if self.working() {
            return;
        }
        // Live Run shows this tab's form: Run rebuilds it now rather than start a second window.
        if self.live_follows_active() {
            self.live_rebuild(ctx);
            return;
        }
        let rust = self
            .docs
            .get(self.active)
            .is_some_and(|d| designer::is_rust(&d.path));
        if !rust {
            self.start_cargo(ctx, Action::Run);
            return;
        }
        // The designer reads the file now, so a file in code view has its forms too.
        self.sync_designer(true);
        if self.run_is_form() || self.free_ui_at_caret().is_some() {
            self.start_run_form(ctx);
            return;
        }
        self.start_cargo(ctx, Action::Run);
        self.console.insert_str(0, &format!("{NO_FORM}\n"));
    }

    /// Whether Run runs Run Form: the active tab is a Rust file, and the designer has read forms
    /// from it. The toolbar and the menu name the button after this.
    pub(crate) fn run_is_form(&self) -> bool {
        self.docs.get(self.active).is_some_and(|doc| {
            designer::is_rust(&doc.path)
                && self
                    .designer
                    .loaded
                    .as_ref()
                    .is_some_and(|l| l.path == doc.path && !l.forms.is_empty())
        })
    }

    /// Starts Run Form for the form of the active file (see `pick_form`). Nothing is saved: the unsaved text runs.
    pub(crate) fn start_run_form(&mut self, ctx: &egui::Context) {
        if self.working() {
            return;
        }
        self.diagnostics.clear();
        self.marks.clear();
        self.build_progress = None;
        let inputs = match self.run_form_inputs() {
            Ok(inputs) => inputs,
            Err(problem) => {
                self.console = "Run Form did not start.\n".into();
                self.refuse_run(ctx, vec![problem]);
                return;
            }
        };
        let form_id = inputs.form_id.clone();
        self.console = format!("Preparing Run Form for {form_id}...\n");
        self.auto_tab(ctx, bottom::BottomTab::Output);
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(prepare(inputs, None));
            repaint.request_repaint();
        });
        self.form_prep = Some(Prep {
            rx,
            stopping: false,
        });
    }

    /// The theme of the designer's canvas, which a Run Form or Live Run window starts in: the
    /// canvas's own Light/Dark choice, else the IDE's theme, which the canvas follows then.
    pub(crate) fn canvas_theme(&self, ctx: &egui::Context) -> RunTheme {
        RunTheme::of_canvas(self.designer.dark, ctx.global_style().visuals.dark_mode)
    }

    /// The caret's line in code view (1-based); `None` while the designer shows the form.
    fn caret_line(&self) -> Option<u32> {
        (!self.form_showing()).then_some(self.cursor_pos.0 as u32)
    }

    /// In code view, the free `fn f(ui: &mut egui::Ui)` of the active Rust file that holds the caret,
    /// unless a designer form does (a form under the caret wins). The designer does not read free
    /// `ui` functions as forms, so Run Form finds them here.
    pub(crate) fn free_ui_at_caret(&self) -> Option<formust_rust::run::FreeUi> {
        let line = self.caret_line()?;
        let doc = self.docs.get(self.active)?;
        if !designer::is_rust(&doc.path) {
            return None;
        }
        let free = formust_rust::run::free_ui_at(&doc.text, line)?;
        let here = if file_kind(&doc.path) == FileKind::Form {
            Origin::Frm
        } else {
            Origin::Rs
        };
        let form_here = self
            .designer
            .loaded
            .as_ref()
            .filter(|l| l.path == doc.path)
            .is_some_and(|l| {
                l.forms.iter().any(|f| {
                    (f.origin.is_none() || f.origin == Some(here))
                        && f.imported.fn_range.start < free.range.end
                        && free.range.start < f.imported.fn_range.end
                })
            });
        (!form_here).then_some(free)
    }

    /// What the designer shows, for Run Form. A problem names the file it is about.
    fn run_form_inputs(&self) -> Result<Inputs, Diagnostic> {
        let doc = self.docs.get(self.active).ok_or_else(|| {
            problem(
                &self.root,
                0,
                "Run Form needs a form: open a Rust file with a form in the designer.".into(),
            )
        })?;
        let fail = |message: &str| problem(&doc.path, 0, message.to_string());
        // A free `ui` function under the caret runs from the open text, unsaved edits and all.
        if let Some(free) = self.free_ui_at_caret() {
            return Ok(Inputs {
                owner: doc.path.clone(),
                owner_text: doc.text.clone(),
                form_id: format!("::{}", free.name),
                owner_type: String::new(),
                method: free.name,
                kind: Kind::FreeUi,
            });
        }
        let loaded = self
            .designer
            .loaded
            .as_ref()
            .filter(|l| l.path == doc.path)
            .ok_or_else(|| fail("Run Form needs a form: this file has none."))?;
        let ids: Vec<String> = loaded.forms.iter().map(|f| f.form().id()).collect();
        // A form read from this file has its function's range in `doc.text`; one read from the other
        // file of the pair has a range in that file, so it is not under the caret.
        let here = if file_kind(&doc.path) == FileKind::Form {
            Origin::Frm
        } else {
            Origin::Rs
        };
        let spans: Vec<Option<Range<usize>>> = loaded
            .forms
            .iter()
            .map(|f| {
                (f.origin.is_none() || f.origin == Some(here)).then(|| f.imported.fn_range.clone())
            })
            .collect();
        let caret = self.caret_line();
        let shown = pick_form(
            &ids,
            &spans,
            &doc.text,
            self.designer.form_id.as_deref(),
            caret,
        )
        .ok_or_else(|| fail("Run Form needs a form: this file has none."))?;
        let form = loaded.forms[shown].form();
        let (path, text, companion) = self
            .designer_inputs()
            .ok_or_else(|| fail("Run Form needs a Rust file with a form."))?;
        let source = if file_kind(&path) == FileKind::Form {
            companion
        } else {
            Some((path.clone(), text))
        };
        let dirty = crate::designer::export::to_export(loaded).contains(&shown);
        let (owner, owner_text, kind, method) = match source {
            // Unedited and no form file: the source runs as it is.
            Some((source, source_text)) if !dirty && loaded.frm_text.is_none() => {
                (source, source_text, kind_of(form), form.method.clone())
            }
            source => {
                let target = crate::designer::export::target(loaded, &|p| p.exists())
                    .map_err(|e| fail(&e))?;
                let copy = crate::designer::export::preview_text(
                    loaded,
                    shown,
                    source.as_ref().map(|(_, t)| t.as_str()),
                )
                .map_err(|e| fail(&format!("Run Form could not write the form file: {e}")))?;
                let (kind, method) = in_copy(kind_of(form), &form.method);
                (target.frm, copy, kind, method)
            }
        };
        Ok(Inputs {
            owner,
            owner_text,
            form_id: form.id(),
            owner_type: form.owner.clone(),
            method,
            kind,
        })
    }

    /// Takes the preparation when its thread answers: starts the build, or says why it can't.
    pub(crate) fn poll_form_prep(&mut self, ctx: &egui::Context) {
        let Some(prep) = self.form_prep.as_ref() else {
            return;
        };
        let result = match prep.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(vec![problem(
                &self.root,
                0,
                "Run Form stopped: its preparation ended without an answer.".into(),
            )]),
        };
        let Some(prep) = self.form_prep.take() else {
            return;
        };
        if prep.stopping {
            self.console.push_str("Run Form stopped.\n");
            return;
        }
        match result {
            Ok(plan) => self.begin_run_form(ctx, plan),
            Err(problems) => {
                self.console = "Run Form did not start.\n".into();
                self.refuse_run(ctx, problems);
            }
        }
    }

    /// Starts the prepared program: the same build session as Run, with its diagnostics placed in the
    /// project's files.
    fn begin_run_form(&mut self, ctx: &egui::Context, mut plan: Plan) {
        // The window starts in the canvas's theme.
        plan.request
            .env
            .push((THEME_VAR.into(), self.canvas_theme(ctx).as_str().into()));
        self.diagnostics.clear();
        self.marks.clear();
        self.console = format!(
            "Run Form: {} in a window. Only its generated package is built, in {}, not the project's binary. The project's files are not changed.\n",
            plan.form_id,
            plan.dir.display()
        );
        if let Some(note) = &plan.note {
            self.console.push_str(note);
            self.console.push('\n');
        }
        self.auto_tab(ctx, bottom::BottomTab::Output);
        let manifest = plan.request.path.clone();
        let repaint = ctx.clone();
        let build = formust_build::start(plan.request, move || repaint.request_repaint());
        self.building = Some(Session::new(build, Action::Run, manifest).with_places(plan.places));
    }

    /// Says why Run Form did not start: the messages in Output and the problems in Problems.
    fn refuse_run(&mut self, ctx: &egui::Context, problems: Vec<Diagnostic>) {
        for problem in &problems {
            self.console.push_str(&problem.message);
            self.console.push('\n');
            cargo::push_problem(&mut self.diagnostics, problem.clone());
        }
        let marks = self.place_marks(&self.diagnostics);
        self.marks = marks;
        self.auto_tab(ctx, bottom::BottomTab::Problems);
    }

    /// Stop while Run Form is being prepared: the run is not started. Returns whether there was one.
    pub(crate) fn stop_form_prep(&mut self) -> bool {
        let Some(prep) = self.form_prep.as_mut() else {
            return false;
        };
        if !prep.stopping {
            prep.stopping = true;
            self.console.push_str("Stopping...\n");
        }
        true
    }
}

pub mod live;

#[cfg(test)]
mod tests;
