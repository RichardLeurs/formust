//! Check, Build and Run through cargo, on top of `formust-build`. Nothing here draws: the App starts
//! a [`Session`], polls its events each frame and places what they say (Output, Problems, marks in
//! the open files). The options a build takes are kept here too, with the rules that turn them into
//! a request.

use crate::{bottom, run_form, style};
use eframe::egui::{self, RichText};
use formust_build::{
    Action, Build, CargoDiagnostic, Diagnostic, Event, Outcome, Package, Request, Selection,
    Severity,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// What the Cargo Options window sets. Kept per open project (the App resets it when the project
/// changes), and not written to disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `--package`, for a workspace with more than one package. `None` is cargo's default.
    pub package: Option<String>,
    /// `--bin` for Run. `None` runs cargo's default binary.
    pub bin: Option<String>,
    /// `--release`.
    pub release: bool,
    /// The program's arguments under Run, split on spaces (no quoting).
    pub program_args: String,
}

/// The cargo action's name for the console and the menus.
pub fn title(action: Action) -> &'static str {
    match action {
        Action::Check => "Check",
        Action::Build => "Build",
        Action::Run => "Run",
    }
}

/// The first line of a run's Output: what is starting.
fn verb(action: Action) -> &'static str {
    match action {
        Action::Check => "Checking",
        Action::Build => "Building",
        Action::Run => "Running",
    }
}

/// The request for `action` on the package at `manifest`, with `options`. `total` is how many units
/// the last build of this package had (sizes the progress bar).
pub fn request(
    action: Action,
    manifest: &Path,
    options: &Options,
    total: Option<usize>,
) -> Request {
    let mut req = Request::new(action, manifest);
    req.selection = match (action, &options.bin) {
        // Check covers tests and examples too, so an error there is not a surprise at Build time.
        (Action::Check, _) => Selection::All,
        (Action::Run, Some(bin)) => Selection::Bin(bin.clone()),
        _ => Selection::Default,
    };
    req.release = options.release;
    if let Some(package) = options.package.as_ref().filter(|p| !p.is_empty()) {
        req.cargo_args.push(OsString::from("--package"));
        req.cargo_args.push(OsString::from(package));
    }
    if action == Action::Run {
        req.program_args = split_args(&options.program_args);
    }
    req.total = total;
    req
}

/// The program arguments typed in the options window: whitespace separates them.
pub fn split_args(text: &str) -> Vec<OsString> {
    text.split_whitespace().map(OsString::from).collect()
}

/// A diagnostic as the app shows it. One with no place in a file (a Cargo.toml problem, a dependency
/// that will not resolve) is placed on its package's `Cargo.toml` at line 0; a line 0 has no place in
/// the text, so it is never drawn as a mark.
pub fn diagnostic_for_app(cargo: &CargoDiagnostic) -> Diagnostic {
    let mut d = cargo.diagnostic.clone();
    if d.file.as_os_str().is_empty() {
        d.file = cargo.manifest_path.clone();
        d.line = 0;
        d.col = 0;
    }
    d
}

/// The Problems entry an event adds, if it is a rustc message.
pub fn problem_for(event: &Event) -> Option<Diagnostic> {
    match event {
        Event::Diagnostic(d) => Some(diagnostic_for_app(d)),
        _ => None,
    }
}

/// Adds a problem unless the list has the same one already (same file, place, severity and
/// message). rust-analyzer reports an error in an included `.frm.rs` from its own checks and again
/// from cargo, and cargo can report one for two targets.
pub fn push_problem(list: &mut Vec<Diagnostic>, problem: Diagnostic) {
    if !list.contains(&problem) {
        list.push(problem);
    }
}

/// The text an event adds to the Output pane: cargo's status lines and the program's output (one
/// per line), and each rustc message as rustc rendered it.
pub fn output_for(event: &Event) -> Option<String> {
    match event {
        Event::Output { line, .. } => Some(format!("{line}\n")),
        Event::Diagnostic(d) if !d.rendered.trim().is_empty() => {
            let mut text = d.rendered.trim_end().to_string();
            text.push('\n');
            Some(text)
        }
        _ => None,
    }
}

/// Errors (fatal ones included) among `diagnostics`: the count the status line shows.
pub fn errors(diagnostics: &[Diagnostic]) -> usize {
    diagnostics
        .iter()
        .filter(|d| matches!(d.severity, Severity::Fatal | Severity::Error))
        .count()
}

/// The last line of the Output for a finished action: what happened, in words.
pub fn summary(action: Action, outcome: &Outcome, errors: usize) -> String {
    let name = title(action);
    if outcome.cancelled {
        return format!("{name} stopped.\n");
    }
    if let Some(error) = &outcome.error {
        return format!("{name} could not run: {error}\n");
    }
    let failed = |errors: usize| {
        format!(
            "{name} failed: {errors} error{}.\n",
            if errors == 1 { "" } else { "s" }
        )
    };
    match (action, outcome.result.success) {
        // Under Run the exit code is the program's, and only once the build has succeeded.
        (Action::Run, true) => match outcome.code {
            Some(0) => "The program finished (exit code 0).\n".to_string(),
            Some(code) => format!("The program finished (exit code {code}).\n"),
            None => "The program was stopped.\n".to_string(),
        },
        (_, true) => format!("{name} succeeded.\n"),
        (_, false) => failed(errors.max(1)),
    }
}

/// What the Cargo Options window lists: the workspace's packages, read from `cargo metadata`.
pub fn read_packages(manifest: &Path) -> Result<Vec<Package>, String> {
    formust_build::metadata(None, manifest)
        .map(|m| m.packages)
        .map_err(|e| e.to_string())
}

/// The binary names the window offers: those of `package`, or of every package when none is chosen.
pub fn binaries(packages: &[Package], package: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = packages
        .iter()
        .filter(|p| package.is_none_or(|name| name.is_empty() || p.name == name))
        .flat_map(|p| p.targets.iter())
        .filter(|t| t.kind.iter().any(|k| k == "bin"))
        .map(|t| t.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The file a project opens first: its `src/main.rs`, else its `src/lib.rs`. `None` for a folder
/// with neither (its files are opened from the tree).
pub fn main_source(folder: &Path) -> Option<PathBuf> {
    ["src/main.rs", "src/lib.rs"]
        .iter()
        .map(|rel| folder.join(rel))
        .find(|p| p.is_file())
}

/// A running Check, Build or Run: the cargo session and what the App needs to place its results.
pub struct Session {
    pub build: Build,
    pub action: Action,
    /// The `Cargo.toml` the action runs on.
    pub manifest: PathBuf,
    /// Stop was asked for; the session ends with a `cancelled` outcome.
    pub stopping: bool,
    /// Run Form's copies: their diagnostics are shown at the places of the project's files.
    pub places: Option<crate::run_form::Places>,
}

impl Session {
    pub fn new(build: Build, action: Action, manifest: PathBuf) -> Self {
        Self {
            build,
            action,
            manifest,
            stopping: false,
            places: None,
        }
    }

    pub fn with_places(mut self, places: crate::run_form::Places) -> Self {
        self.places = Some(places);
        self
    }

    /// Stop: cargo and every program it started are killed. The outcome follows on the channel.
    pub fn stop(&mut self) {
        if !self.stopping {
            self.stopping = true;
            self.build.cancel();
        }
    }
}

/// The Cargo Options window's data: the project's packages, read by `cargo metadata` on a worker
/// thread so the window never waits for cargo.
#[derive(Default)]
pub struct Window {
    pub open: bool,
    /// The `Cargo.toml` whose packages are (or were) read.
    manifest: Option<PathBuf>,
    /// The packages read, or why they could not be read. `None` while they are being read.
    packages: Option<Result<Vec<Package>, String>>,
    reading: Option<Receiver<Result<Vec<Package>, String>>>,
}

impl Window {
    /// Reads the packages of `manifest`, unless they are read (or being read) already.
    pub fn read(&mut self, manifest: PathBuf, ctx: &egui::Context) {
        if self.manifest.as_ref() == Some(&manifest)
            && (self.packages.is_some() || self.reading.is_some())
        {
            return;
        }
        self.manifest = Some(manifest.clone());
        self.packages = None;
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(read_packages(&manifest));
            ctx.request_repaint();
        });
        self.reading = Some(rx);
    }

    /// Takes the packages when the worker has them.
    pub fn poll(&mut self) {
        let result = self.reading.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.packages = Some(result);
            self.reading = None;
        }
    }

    /// Drops what was read: the project changed.
    pub fn forget(&mut self) {
        *self = Window {
            open: self.open,
            ..Window::default()
        };
    }
}

impl crate::App {
    /// The `Cargo.toml` of the active file's package, else of the project folder.
    pub(crate) fn cargo_manifest(&self) -> Option<PathBuf> {
        let from = self
            .docs
            .get(self.active)
            .map_or_else(|| self.root.clone(), |d| d.path.clone());
        formust_build::manifest_for(&from)
            .ok()
            .or_else(|| formust_build::manifest_for(&self.root).ok())
    }

    /// Starts Check, Build or Run on the package the active file belongs to, after saving everything.
    /// One cargo run at a time: its output goes to the Output pane, and its diagnostics to Problems.
    pub(crate) fn start_cargo(&mut self, ctx: &egui::Context, action: Action) {
        if self.working() {
            return;
        }
        let Some(manifest) = self.cargo_manifest() else {
            self.console =
                "No Cargo.toml here: open a folder with one, or a Rust file in such a folder\n"
                    .into();
            self.auto_tab(ctx, bottom::BottomTab::Output);
            return;
        };
        self.save_all();
        self.build_texts = self
            .docs
            .iter()
            .map(|d| (d.path.clone(), d.text.clone()))
            .collect();
        self.console = format!("{} {}...\n", verb(action), manifest.display());
        self.diagnostics.clear();
        self.marks.clear();
        self.build_progress = None;
        self.auto_tab(ctx, bottom::BottomTab::Output);
        let total = self.unit_counts.get(&manifest).copied();
        let req = request(action, &manifest, &self.cargo_options, total);
        let repaint = ctx.clone();
        let build = formust_build::start(req, move || repaint.request_repaint());
        self.building = Some(Session::new(build, action, manifest));
    }

    /// Stop: kills the cargo run in flight (and a program it started). Its outcome follows.
    pub(crate) fn stop_cargo(&mut self) {
        if self.stop_form_prep() {
            return;
        }
        if let Some(session) = self.building.as_mut()
            && !session.stopping
        {
            session.stop();
            self.console.push_str("Stopping...\n");
        }
    }

    /// Takes what the cargo run in flight has said: Output lines, Problems as they come, progress, and
    /// at the end the summary, the marks in the open files and the assistant's answer.
    pub(crate) fn poll_build(&mut self, ctx: &egui::Context) {
        let Some(session) = self.building.as_mut() else {
            return;
        };
        let mut finished = None;
        while let Ok(event) = session.build.events().try_recv() {
            if let Some(text) = output_for(&event) {
                self.console.push_str(&text);
            }
            if let Some(problem) = problem_for(&event) {
                let problem = match &session.places {
                    Some(places) => places.real(problem),
                    None => problem,
                };
                push_problem(&mut self.diagnostics, problem);
            }
            match event {
                Event::Progress(p) => self.build_progress = Some(p),
                // Compiling is over and the program has started: the toolbar says so, not "Compiling".
                Event::BuildFinished { success: true } if session.action == Action::Run => {
                    let done = self.build_progress.as_ref().map_or(0, |p| p.done);
                    self.build_progress = Some(formust_build::Progress {
                        label: "Running...".into(),
                        fraction: None,
                        done,
                    });
                }
                Event::Finished(outcome) => {
                    finished = Some(outcome);
                    break;
                }
                _ => {}
            }
        }
        let Some(outcome) = finished else {
            return;
        };
        let Some(session) = self.building.take() else {
            return;
        };
        let failed = errors(&self.diagnostics);
        if let Some(places) = session.places.as_ref().filter(|_| !outcome.result.success) {
            self.console
                .push_str(&run_form::own_build_note(&places.owner_name()));
        }
        self.console
            .push_str(&summary(session.action, &outcome, failed));
        if let Some(p) = self
            .build_progress
            .take()
            .filter(|p| p.done > 0 && outcome.result.success)
        {
            self.unit_counts.insert(session.manifest.clone(), p.done);
        }
        let marks = self.place_marks(&self.diagnostics);
        self.marks = marks;
        self.auto_tab(
            ctx,
            if failed > 0 {
                bottom::BottomTab::Problems
            } else {
                bottom::BottomTab::Output
            },
        );
        self.assistant_build_done(&outcome.result);
    }

    /// File > Build > Cargo Options...: the window opens, and the project's packages are read.
    pub(crate) fn open_cargo_window(&mut self, ctx: &egui::Context) {
        self.cargo_window.open = true;
        if let Some(manifest) = self.cargo_manifest() {
            self.cargo_window.read(manifest, ctx);
        }
    }

    /// The Cargo Options window. Its choices apply to the next Check, Build or Run.
    pub(crate) fn show_cargo_window(&mut self, ctx: &egui::Context) {
        if !self.cargo_window.open {
            return;
        }
        self.cargo_window.poll();
        let packages = self.cargo_window.packages.clone();
        let has_manifest = self.cargo_window.manifest.is_some();
        let folder = self.root.display().to_string();
        let mut options = self.cargo_options.clone();
        let (mut done, mut choose) = (false, false);
        style::dialog(ctx, "cargo-options", Some("Cargo Options"), 480.0, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let pal = style::Palette::of(ui.ctx());
            ui.horizontal(|ui| {
                ui.label("Project");
                ui.label(
                    RichText::new(folder)
                        .monospace()
                        .size(11.5)
                        .color(pal.text_secondary),
                );
                if style::secondary_button(ui, "Choose Folder...").clicked() {
                    choose = true;
                }
            });
            match (has_manifest, &packages) {
                (false, _) => {
                    ui.colored_label(pal.error, "No Cargo.toml in or above this folder.");
                }
                (true, None) => {
                    ui.label(RichText::new("Reading the packages...").color(pal.text_secondary));
                }
                (true, Some(Err(e))) => {
                    ui.colored_label(pal.error, e.as_str());
                }
                (true, Some(Ok(list))) => {
                    ui.horizontal(|ui| {
                        ui.label("Package");
                        let current = options.package.clone().unwrap_or_else(|| "Default".into());
                        egui::ComboBox::from_id_salt("cargo-package")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(options.package.is_none(), "Default")
                                    .clicked()
                                {
                                    options.package = None;
                                }
                                for p in list {
                                    let chosen =
                                        options.package.as_deref() == Some(p.name.as_str());
                                    if ui.selectable_label(chosen, &p.name).clicked() {
                                        options.package = Some(p.name.clone());
                                    }
                                }
                            });
                    });
                    let bins = binaries(list, options.package.as_deref());
                    if options.bin.as_ref().is_some_and(|b| !bins.contains(b)) {
                        options.bin = None;
                    }
                    ui.horizontal(|ui| {
                        ui.label("Binary to run");
                        let current = options.bin.clone().unwrap_or_else(|| "Default".into());
                        egui::ComboBox::from_id_salt("cargo-bin")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(options.bin.is_none(), "Default")
                                    .clicked()
                                {
                                    options.bin = None;
                                }
                                for b in &bins {
                                    if ui
                                        .selectable_label(
                                            options.bin.as_deref() == Some(b.as_str()),
                                            b,
                                        )
                                        .clicked()
                                    {
                                        options.bin = Some(b.clone());
                                    }
                                }
                            });
                    });
                }
            }
            ui.checkbox(&mut options.release, "Release build (--release)");
            ui.horizontal(|ui| {
                ui.label("Arguments");
                ui.add(
                    egui::TextEdit::singleline(&mut options.program_args)
                        .hint_text("for Run, separated by spaces")
                        .desired_width(260.0),
                );
            });
            ui.label(
                RichText::new("The choices last while the project is open. Check and Build use the package and the release setting; Run also uses the binary and the arguments.")
                    .size(11.5)
                    .color(pal.text_secondary),
            );
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if style::primary_button(ui, "Done").clicked() {
                        done = true;
                    }
                });
            });
        });
        self.cargo_options = options;
        if choose {
            self.pick_folder();
        }
        if done {
            self.cargo_window.open = false;
        }
    }
}

#[cfg(test)]
impl Session {
    /// A session that counts as running, for tests that only need one. Its cargo never gets far: the
    /// manifest it names does not exist.
    pub(crate) fn for_test() -> Self {
        let build = formust_build::start(
            Request::new(Action::Check, "/nonexistent/Cargo.toml"),
            || {},
        );
        Session::new(build, Action::Check, PathBuf::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same problem from rust-analyzer's own check and from cargo is listed once.
    #[test]
    fn identical_problems_are_listed_once() {
        let d = |line, message: &str| Diagnostic {
            file: "src/login.frm.rs".into(),
            line,
            col: 9,
            severity: formust_build::Severity::Error,
            message: message.into(),
        };
        let mut list = Vec::new();
        push_problem(&mut list, d(3, "no method named `labell`"));
        push_problem(&mut list, d(3, "no method named `labell`"));
        push_problem(&mut list, d(4, "no method named `labell`"));
        push_problem(&mut list, d(3, "another"));
        assert_eq!(list.len(), 3);
    }
    use formust_build::{BuildResult, Stream};

    fn cargo_diag(file: &str, line: u32, severity: Severity, rendered: &str) -> CargoDiagnostic {
        CargoDiagnostic {
            diagnostic: Diagnostic {
                file: PathBuf::from(file),
                line,
                col: 3,
                severity,
                message: "mismatched types".into(),
            },
            level: "error".into(),
            code: Some("E0308".into()),
            rendered: rendered.into(),
            spans: Vec::new(),
            target_kind: "bin".into(),
            target_name: "app".into(),
            manifest_path: PathBuf::from("/p/Cargo.toml"),
        }
    }

    fn outcome(success: bool, code: Option<i32>, cancelled: bool, error: Option<&str>) -> Outcome {
        Outcome {
            result: BuildResult {
                success,
                diagnostics: Vec::new(),
                raw_output: String::new(),
            },
            code,
            cancelled,
            error: error.map(str::to_string),
            executables: Vec::new(),
        }
    }

    #[test]
    fn check_covers_all_targets_and_run_uses_the_chosen_binary() {
        let manifest = Path::new("/p/Cargo.toml");
        let mut options = Options {
            bin: Some("tool".into()),
            ..Options::default()
        };
        assert_eq!(
            request(Action::Check, manifest, &options, None).selection,
            Selection::All
        );
        assert_eq!(
            request(Action::Build, manifest, &options, None).selection,
            Selection::Default,
            "build ignores the binary"
        );
        assert_eq!(
            request(Action::Run, manifest, &options, None).selection,
            Selection::Bin("tool".into())
        );
        options.bin = None;
        assert_eq!(
            request(Action::Run, manifest, &options, None).selection,
            Selection::Default
        );
    }

    #[test]
    fn options_become_cargo_arguments() {
        let manifest = Path::new("/p/Cargo.toml");
        let options = Options {
            package: Some("app".into()),
            release: true,
            program_args: "  --port 8080 ".into(),
            ..Options::default()
        };
        let build = request(Action::Build, manifest, &options, Some(7));
        assert!(build.release);
        assert_eq!(
            build.cargo_args,
            vec![OsString::from("--package"), OsString::from("app")]
        );
        assert!(build.program_args.is_empty(), "arguments are for Run only");
        assert_eq!(build.total, Some(7));
        let run = request(Action::Run, manifest, &options, None);
        assert_eq!(
            run.program_args,
            vec![OsString::from("--port"), OsString::from("8080")]
        );
        assert_eq!(
            request(
                Action::Check,
                manifest,
                &Options {
                    package: Some(String::new()),
                    ..Options::default()
                },
                None
            )
            .cargo_args,
            Vec::<OsString>::new()
        );
    }

    #[test]
    fn a_diagnostic_without_a_place_is_placed_on_its_cargo_toml_at_line_zero() {
        let placed = diagnostic_for_app(&cargo_diag("/p/src/main.rs", 4, Severity::Error, "x"));
        assert_eq!(
            (placed.file.as_path(), placed.line),
            (Path::new("/p/src/main.rs"), 4)
        );
        let no_place = diagnostic_for_app(&cargo_diag("", 9, Severity::Error, "x"));
        assert_eq!(no_place.file, PathBuf::from("/p/Cargo.toml"));
        assert_eq!((no_place.line, no_place.col), (0, 0));
        assert_eq!(no_place.message, "mismatched types", "the message is kept");
    }

    #[test]
    fn events_become_problems_and_output() {
        let msg = Event::Diagnostic(Box::new(cargo_diag(
            "/p/src/lib.rs",
            2,
            Severity::Warning,
            "warning: unused\n  --> src/lib.rs:2:3\n",
        )));
        assert_eq!(problem_for(&msg).unwrap().severity, Severity::Warning);
        assert_eq!(
            output_for(&msg).unwrap(),
            "warning: unused\n  --> src/lib.rs:2:3\n"
        );
        let quiet = Event::Diagnostic(Box::new(cargo_diag(
            "/p/src/lib.rs",
            2,
            Severity::Note,
            "  ",
        )));
        assert!(
            output_for(&quiet).is_none(),
            "a message without rendered text adds nothing to Output"
        );
        let line = Event::Output {
            stream: Stream::Stdout,
            line: "hello".into(),
        };
        assert_eq!(output_for(&line).unwrap(), "hello\n");
        assert!(problem_for(&line).is_none());
        assert!(output_for(&Event::BuildFinished { success: true }).is_none());
    }

    #[test]
    fn the_summary_says_what_happened() {
        let errs = [Diagnostic {
            file: PathBuf::new(),
            line: 0,
            col: 0,
            severity: Severity::Error,
            message: String::new(),
        }];
        assert_eq!(
            summary(Action::Check, &outcome(true, Some(0), false, None), 0),
            "Check succeeded.\n"
        );
        assert_eq!(
            summary(
                Action::Build,
                &outcome(false, Some(101), false, None),
                errs.len()
            ),
            "Build failed: 1 error.\n"
        );
        assert_eq!(
            summary(Action::Build, &outcome(false, Some(101), false, None), 3),
            "Build failed: 3 errors.\n"
        );
        assert_eq!(
            summary(Action::Run, &outcome(true, Some(3), false, None), 0),
            "The program finished (exit code 3).\n"
        );
        assert_eq!(
            summary(Action::Run, &outcome(true, Some(0), false, None), 0),
            "The program finished (exit code 0).\n"
        );
        assert_eq!(
            summary(Action::Check, &outcome(false, None, true, None), 0),
            "Check stopped.\n"
        );
        assert_eq!(
            summary(
                Action::Build,
                &outcome(false, None, false, Some("no cargo")),
                0
            ),
            "Build could not run: no cargo\n"
        );
    }

    #[test]
    fn errors_count_fatal_and_error_only() {
        let at = |severity| Diagnostic {
            file: PathBuf::new(),
            line: 0,
            col: 0,
            severity,
            message: String::new(),
        };
        assert_eq!(
            errors(&[
                at(Severity::Error),
                at(Severity::Warning),
                at(Severity::Fatal),
                at(Severity::Note)
            ]),
            2
        );
    }

    #[test]
    fn binaries_follow_the_package_and_main_source_prefers_main_rs() {
        let target = |name: &str, kind: &str| formust_build::Target {
            name: name.into(),
            kind: vec![kind.into()],
            src_path: PathBuf::from("x.rs"),
        };
        let packages = vec![
            Package {
                name: "app".into(),
                manifest_path: PathBuf::from("/w/app/Cargo.toml"),
                targets: vec![target("app", "bin"), target("core", "lib")],
            },
            Package {
                name: "tool".into(),
                manifest_path: PathBuf::from("/w/tool/Cargo.toml"),
                targets: vec![target("tool", "bin"), target("app", "bin")],
            },
        ];
        assert_eq!(
            binaries(&packages, None),
            vec!["app".to_string(), "tool".to_string()]
        );
        assert_eq!(
            binaries(&packages, Some("tool")),
            vec!["app".to_string(), "tool".to_string()]
        );
        assert_eq!(binaries(&packages, Some("core")), Vec::<String>::new());

        let dir = std::env::temp_dir().join(format!("formust-cargo-main-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        assert_eq!(main_source(&dir), None, "no source yet");
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        assert_eq!(main_source(&dir), Some(dir.join("src/lib.rs")));
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        assert_eq!(main_source(&dir), Some(dir.join("src/main.rs")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A throwaway crate with the given `main.rs`, built in its own target folder.
    fn sample_crate(name: &str, main: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("formust-cargo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let manifest = dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/main.rs"), main).unwrap();
        (dir, manifest)
    }

    /// Runs `req` to its end: the problems it reported, the Output it wrote and the outcome.
    fn run_to_end(dir: &Path, mut req: Request) -> (Vec<Diagnostic>, Vec<String>, Outcome) {
        req.env.push((
            OsString::from("CARGO_TARGET_DIR"),
            dir.join("target").into_os_string(),
        ));
        let build = formust_build::start(req, || {});
        let (mut problems, mut output) = (Vec::new(), Vec::new());
        loop {
            let event = build
                .events()
                .recv_timeout(std::time::Duration::from_secs(300))
                .expect("cargo ends");
            if let Some(p) = problem_for(&event) {
                problems.push(p);
            }
            if let Some(text) = output_for(&event) {
                output.push(text);
            }
            if let Event::Finished(outcome) = event {
                return (problems, output, outcome);
            }
        }
    }

    /// Cargo itself: a type error is a Problem at its line, and the Check ends with its summary.
    #[test]
    fn a_real_check_reports_the_error_at_its_line() {
        let (dir, manifest) = sample_crate(
            "check",
            "fn main() {\n    let n: u32 = \"text\";\n    println!(\"{n}\");\n}\n",
        );
        let req = request(Action::Check, &manifest, &Options::default(), None);
        let (problems, _, outcome) = run_to_end(&dir, req);
        assert!(!outcome.result.success);
        let error = problems
            .iter()
            .find(|p| p.severity == Severity::Error)
            .expect("an error");
        assert!(
            error.file.is_absolute() && error.file.ends_with("src/main.rs"),
            "{error:?}"
        );
        assert_eq!(error.line, 2);
        assert_eq!(
            summary(Action::Check, &outcome, errors(&problems)),
            "Check failed: 1 error.\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Run: the program's own output reaches the Output pane line by line, and its exit code ends the run.
    #[test]
    fn a_real_run_streams_the_program_output() {
        let (dir, manifest) = sample_crate(
            "run",
            "fn main() {\n    println!(\"hello from formust\");\n    println!(\"second line\");\n}\n",
        );
        let options = Options {
            program_args: "--flag value".into(),
            ..Options::default()
        };
        let req = request(Action::Run, &manifest, &options, None);
        let (problems, output, outcome) = run_to_end(&dir, req);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(outcome.result.success, "{}", outcome.result.raw_output);
        let text: String = output.concat();
        assert!(
            text.contains("hello from formust\n") && text.contains("second line\n"),
            "{text}"
        );
        assert_eq!(outcome.code, Some(0));
        assert_eq!(
            summary(Action::Run, &outcome, 0),
            "The program finished (exit code 0).\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_title_names_the_action() {
        assert_eq!(
            [
                title(Action::Check),
                title(Action::Build),
                title(Action::Run)
            ],
            ["Check", "Build", "Run"]
        );
    }
}
