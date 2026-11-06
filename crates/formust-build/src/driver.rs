//! Runs cargo on a background thread and streams what it does.
//!
//! [`start`] returns at once with a [`Build`]: its channel gets [`Event`]s as cargo works (progress,
//! each diagnostic, each line of other output) and ends with one [`Event::Finished`]. `wake` is called
//! after every event, so a UI can ask for a repaint and poll the channel with `try_recv`, as the app
//! does for builds. [`Build::cancel`] kills cargo and everything it started.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::message::{CargoDiagnostic, Message, parse_message, strip_ansi};
use crate::metadata::{metadata_command, read_output};
use crate::{BuildResult, Diagnostic, Progress};

/// Shown when no cargo can be found.
pub const NOT_FOUND_HINT: &str =
    "cargo was not found. Install Rust with rustup (https://rustup.rs), then restart Formust.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    Check,
    Build,
    /// `cargo run`: the program's output follows [`Event::BuildFinished`] as [`Event::Output`].
    Run,
}

impl Action {
    fn word(self) -> &'static str {
        match self {
            Action::Check => "check",
            Action::Build => "build",
            Action::Run => "run",
        }
    }
}

/// Which targets cargo builds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Selection {
    /// Cargo's default: the library and the binaries (for `run`, the one binary or `default-run`).
    #[default]
    Default,
    /// `--all-targets`: also tests, examples and benches. Not for `run`.
    All,
    Lib,
    Bin(String),
    Example(String),
    Test(String),
}

/// What to run. `path` is a `Cargo.toml`, or a folder or file inside a package (see [`manifest_for`]).
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub action: Action,
    pub path: PathBuf,
    pub selection: Selection,
    pub release: bool,
    /// More cargo arguments, like `--offline` or `--package x`.
    pub cargo_args: Vec<OsString>,
    /// The program's arguments, for `run`.
    pub program_args: Vec<OsString>,
    /// Extra environment for cargo, like `CARGO_TARGET_DIR`.
    pub env: Vec<(OsString, OsString)>,
    /// The cargo executable; [`locate_cargo`]'s when `None`.
    pub cargo: Option<PathBuf>,
    /// How many units the last build of this package had, if known, for [`Progress::fraction`].
    pub total: Option<usize>,
}

impl Request {
    pub fn new(action: Action, path: impl Into<PathBuf>) -> Request {
        Request {
            action,
            path: path.into(),
            ..Request::default()
        }
    }

    /// The cargo command for `manifest`, run in the manifest's folder so the package's own
    /// `.cargo/config.toml` applies.
    pub(crate) fn command(&self, cargo: &Path, manifest: &Path) -> Command {
        let mut cmd = cargo_command(cargo, manifest, &self.env);
        cmd.arg(self.action.word())
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--message-format=json");
        match &self.selection {
            Selection::Default => {}
            Selection::All => {
                cmd.arg("--all-targets");
            }
            Selection::Lib => {
                cmd.arg("--lib");
            }
            Selection::Bin(name) => {
                cmd.args(["--bin", name]);
            }
            Selection::Example(name) => {
                cmd.args(["--example", name]);
            }
            Selection::Test(name) => {
                cmd.args(["--test", name]);
            }
        }
        if self.release {
            cmd.arg("--release");
        }
        cmd.args(&self.cargo_args);
        if self.action == Action::Run && !self.program_args.is_empty() {
            cmd.arg("--").args(&self.program_args);
        }
        cmd
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug)]
pub enum Event {
    Progress(Progress),
    /// A rustc message, once: the same message for two targets (`lib` and `lib test`) comes once.
    Diagnostic(Box<CargoDiagnostic>),
    /// A line that is not a cargo message: cargo's status lines on stderr, and the program's output
    /// under `run`. Invalid UTF-8 is replaced, ANSI codes and a trailing `\r` are removed.
    Output {
        stream: Stream,
        line: String,
    },
    /// Compiling is over. Under `run` the program starts now, if `success`.
    BuildFinished {
        success: bool,
    },
    /// The last event.
    Finished(Outcome),
}

#[derive(Debug)]
pub struct Outcome {
    /// `success` is whether the build succeeded (for `run`, the program's own exit code is `code`).
    /// `diagnostics` are the messages with a level other than `failure-note`, without repeats.
    /// `raw_output` is rustc's rendered text and cargo's stderr, up to the end of the build.
    pub result: BuildResult,
    /// cargo's exit code (under `run`, the program's); `None` when it was killed or never ran.
    pub code: Option<i32>,
    pub cancelled: bool,
    /// Why cargo couldn't run at all (no manifest, no cargo); also in `result.raw_output`.
    pub error: Option<String>,
    /// The programs that were linked (binaries, tests, examples), in cargo's order.
    pub executables: Vec<PathBuf>,
}

/// A build in progress. Dropping it doesn't stop cargo; call [`Build::cancel`] for that.
pub struct Build {
    events: Receiver<Event>,
    control: Arc<Control>,
}

impl Build {
    /// Poll with `try_recv`, or block with `recv`/`recv_timeout`. Disconnected after `Finished`.
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Kills cargo and every process it started (they share its process group), now or as soon as it
    /// starts. A `Finished` event with `cancelled` follows.
    pub fn cancel(&self) {
        self.control.cancel();
    }
}

/// Starts `request` on a background thread. Never blocks; every problem arrives as an event.
pub fn start(request: Request, wake: impl Fn() + Send + Sync + 'static) -> Build {
    let (tx, events) = channel();
    let control = Arc::new(Control::default());
    let emitter = Emitter {
        tx,
        wake: Arc::new(wake),
    };
    let worker_control = control.clone();
    std::thread::spawn(move || {
        let outcome =
            run(&request, &emitter, &worker_control).unwrap_or_else(|e| failed(e.to_string()));
        emitter.emit(Event::Finished(outcome));
    });
    Build { events, control }
}

/// The `Cargo.toml` for `path`: `path` itself when it is a `Cargo.toml`, else the nearest one in the
/// folder (or the file's folder) or above it, as cargo finds it. Absolute.
pub fn manifest_for(path: &Path) -> std::io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let start = if path.is_file() {
        if path.file_name().is_some_and(|n| n == "Cargo.toml") {
            return Ok(path);
        }
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else if path.is_dir() {
        path.clone()
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    };
    start
        .ancestors()
        .map(|dir| dir.join("Cargo.toml"))
        .find(|m| m.is_file())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no Cargo.toml in or above {}", path.display()),
            )
        })
}

/// Finds `cargo`: `$CARGO` (set when Formust itself runs under cargo), then `$PATH`, then the usual
/// install folders. An app started from Finder has a minimal `PATH` without `~/.cargo/bin`.
pub fn locate_cargo() -> Option<PathBuf> {
    find_cargo(
        std::env::var_os("CARGO").map(PathBuf::from),
        std::env::var_os("PATH"),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn find_cargo(
    forced: Option<PathBuf>,
    path: Option<OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(p) = forced.filter(|p| p.is_file()) {
        return Some(p);
    }
    let mut dirs: Vec<PathBuf> = path
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = home {
        dirs.push(home.join(".cargo/bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter()
        .map(|d| d.join("cargo"))
        .find(|p| p.is_file())
}

/// `cargo` with its own folder first on `PATH` (so rustup's `rustc` beside it is found), run in the
/// manifest's folder, without colour codes.
pub(crate) fn cargo_command(
    cargo: &Path,
    manifest: &Path,
    env: &[(OsString, OsString)],
) -> Command {
    let mut cmd = Command::new(cargo);
    if let Some(dir) = cargo.parent().filter(|d| !d.as_os_str().is_empty()) {
        let mut dirs = vec![dir.to_path_buf()];
        dirs.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        if let Ok(path) = std::env::join_paths(dirs) {
            cmd.env("PATH", path);
        }
    }
    if let Some(dir) = manifest.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    cmd.env("CARGO_TERM_COLOR", "never")
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null());
    cmd
}

pub(crate) fn missing_cargo() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, NOT_FOUND_HINT)
}

pub(crate) fn spawn_error(cargo: &Path, e: std::io::Error) -> std::io::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        std::io::Error::new(
            e.kind(),
            format!(
                "cargo was not found at {}. {NOT_FOUND_HINT}",
                cargo.display()
            ),
        )
    } else {
        std::io::Error::new(e.kind(), format!("could not run {}: {e}", cargo.display()))
    }
}

fn failed(error: String) -> Outcome {
    Outcome {
        result: BuildResult {
            success: false,
            diagnostics: Vec::new(),
            raw_output: format!("build failed: {error}\n"),
        },
        code: None,
        cancelled: false,
        error: Some(error),
        executables: Vec::new(),
    }
}

#[derive(Clone)]
struct Emitter {
    tx: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Emitter {
    fn emit(&self, event: Event) {
        // A dropped `Build` is no reason to stop: the build finishes quietly.
        let _ = self.tx.send(event);
        (self.wake)();
    }
}

/// The running child's process group, and whether it was cancelled.
#[derive(Default)]
struct Control {
    state: Mutex<ControlState>,
}

#[derive(Default)]
struct ControlState {
    cancelled: bool,
    group: Option<i32>,
    /// A process group was killed: the build didn't finish on its own.
    killed: bool,
}

impl Control {
    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.cancelled = true;
        if let Some(group) = state.group {
            kill_group(group);
            state.killed = true;
        }
    }

    fn cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancelled
    }

    fn killed(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).killed
    }

    /// Starts `cmd` in a process group of its own and records it; killed at once if already cancelled.
    fn spawn(&self, cmd: &mut Command) -> std::io::Result<std::process::Child> {
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(cmd, 0);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let child = cmd.spawn()?;
        let group = child.id() as i32;
        state.group = Some(group);
        if state.cancelled {
            kill_group(group);
            state.killed = true;
        }
        Ok(child)
    }

    /// The child was reaped: its group id may be reused, so it is forgotten.
    fn finished(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).group = None;
    }
}

fn kill_group(group: i32) {
    #[cfg(unix)]
    // SAFETY: kill(2) with a negative pid signals that process group; no memory is involved.
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
}

/// What both pipe readers add to.
#[derive(Default)]
struct Shared {
    raw: String,
    label: String,
    done: usize,
    build_done: bool,
    seen: HashSet<(PathBuf, u32, u32, String, String)>,
    diagnostics: Vec<Diagnostic>,
}

/// The cargo status words that name what it is doing (`   Compiling egui v0.37.0`).
const STATUS_WORDS: [&str; 9] = [
    "Compiling",
    "Checking",
    "Building",
    "Documenting",
    "Running",
    "Finished",
    "Blocking",
    "Downloading",
    "Locking",
];

fn progress(shared: &Shared, total: Option<usize>) -> Progress {
    Progress {
        label: shared.label.clone(),
        fraction: total.map(|t| (shared.done as f32 / t.max(1) as f32).min(1.0)),
        done: shared.done,
    }
}

fn read_lines(pipe: impl std::io::Read, mut on: impl FnMut(String)) {
    for line in BufReader::new(pipe).split(b'\n') {
        let Ok(line) = line else { break };
        let line = String::from_utf8_lossy(&line);
        on(strip_ansi(line.strip_suffix('\r').unwrap_or(&line)));
    }
}

fn run(request: &Request, emitter: &Emitter, control: &Control) -> std::io::Result<Outcome> {
    let manifest = manifest_for(&request.path)?;
    let cargo = match &request.cargo {
        Some(c) => c.clone(),
        None => locate_cargo().ok_or_else(missing_cargo)?,
    };
    let shared = Arc::new(Mutex::new(Shared {
        label: "Reading the workspace".into(),
        ..Shared::default()
    }));
    emitter.emit(Event::Progress(progress(
        &shared.lock().unwrap(),
        request.total,
    )));
    // The workspace root is where rustc's relative file names start. When cargo can't read the
    // workspace, the build below fails and says why; the package folder stands in.
    let meta = control
        .spawn(
            metadata_command(&cargo, &manifest, &request.env)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .map_err(|e| spawn_error(&cargo, e))?;
    let meta = meta.wait_with_output();
    control.finished();
    let root = meta
        .ok()
        .and_then(|o| read_output(&o).ok())
        .map(|m| m.workspace_root)
        .unwrap_or_else(|| manifest.parent().unwrap_or(Path::new("/")).to_path_buf());
    if control.cancelled() {
        let mut outcome = failed("cancelled".into());
        outcome.error = None;
        outcome.cancelled = true;
        outcome.result.raw_output = "Cancelled.\n".into();
        return Ok(outcome);
    }

    let mut child = control
        .spawn(
            request
                .command(&cargo, &manifest)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .map_err(|e| spawn_error(&cargo, e))?;
    let stderr = child.stderr.take().expect("piped");
    let err_reader = {
        let (shared, emitter, total) = (shared.clone(), emitter.clone(), request.total);
        std::thread::spawn(move || {
            read_lines(stderr, |line| {
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                if !s.build_done {
                    s.raw.push_str(&line);
                    s.raw.push('\n');
                    let status = line.trim_start();
                    if STATUS_WORDS
                        .iter()
                        .any(|w| status.starts_with(&format!("{w} ")))
                    {
                        s.label = status.split(" (").next().unwrap_or(status).to_string();
                        emitter.emit(Event::Progress(progress(&s, total)));
                    }
                }
                drop(s);
                emitter.emit(Event::Output {
                    stream: Stream::Stderr,
                    line,
                });
            })
        })
    };

    let mut build_success = None;
    let mut executables = Vec::new();
    read_lines(child.stdout.take().expect("piped"), |line| {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        // Program output under `run` is never parsed: it comes after `build-finished`.
        let message = if s.build_done {
            None
        } else {
            parse_message(&line, &root)
        };
        match message {
            Some(Message::Diagnostic(d)) => {
                let key = (
                    d.diagnostic.file.clone(),
                    d.diagnostic.line,
                    d.diagnostic.col,
                    d.level.clone(),
                    d.diagnostic.message.clone(),
                );
                if s.seen.insert(key) {
                    s.raw.push_str(&d.rendered);
                    if d.level != "failure-note" {
                        s.diagnostics.push(d.diagnostic.clone());
                    }
                    drop(s);
                    emitter.emit(Event::Diagnostic(d));
                }
            }
            Some(Message::Artifact { executable, .. }) => {
                s.done += 1;
                executables.extend(executable);
                emitter.emit(Event::Progress(progress(&s, request.total)));
            }
            Some(Message::BuildFinished { success }) => {
                s.build_done = true;
                build_success = Some(success);
                drop(s);
                emitter.emit(Event::BuildFinished { success });
            }
            None => {
                if !s.build_done {
                    s.raw.push_str(&line);
                    s.raw.push('\n');
                }
                drop(s);
                emitter.emit(Event::Output {
                    stream: Stream::Stdout,
                    line,
                });
            }
        }
    });
    let status = child.wait();
    control.finished();
    let _ = err_reader.join();
    let status = status?;
    let cancelled = control.killed();
    let mut s = std::mem::take(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()));
    if cancelled {
        s.raw.push_str("Cancelled.\n");
    }
    let success = !cancelled
        && match request.action {
            Action::Run => build_success == Some(true),
            _ => status.success() && build_success != Some(false),
        };
    Ok(Outcome {
        result: BuildResult {
            success,
            diagnostics: s.diagnostics,
            raw_output: s.raw,
        },
        code: status.code(),
        cancelled,
        error: None,
        executables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(request: &Request) -> Vec<String> {
        request
            .command(Path::new("/x/cargo"), Path::new("/p/Cargo.toml"))
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_command_line_follows_the_request() {
        let mut request = Request::new(Action::Check, "/p");
        assert_eq!(
            args(&request),
            [
                "check",
                "--manifest-path",
                "/p/Cargo.toml",
                "--message-format=json"
            ]
        );
        request.selection = Selection::All;
        request.release = true;
        request.cargo_args = vec!["--offline".into()];
        assert_eq!(
            args(&request)[4..],
            ["--all-targets", "--release", "--offline"]
        );
        request = Request::new(Action::Run, "/p");
        request.selection = Selection::Bin("tool".into());
        request.program_args = vec!["--flag".into(), "x y".into()];
        assert_eq!(
            args(&request),
            [
                "run",
                "--manifest-path",
                "/p/Cargo.toml",
                "--message-format=json",
                "--bin",
                "tool",
                "--",
                "--flag",
                "x y"
            ]
        );
        // Program arguments are for `run` only.
        request.action = Action::Build;
        assert!(!args(&request).contains(&"--".to_string()));
    }

    #[test]
    fn cargo_runs_in_the_manifest_folder_with_its_own_folder_first_on_path() {
        let cmd = Request::new(Action::Build, "/p").command(
            Path::new("/opt/rust/bin/cargo"),
            Path::new("/p/sub/Cargo.toml"),
        );
        assert_eq!(cmd.get_current_dir(), Some(Path::new("/p/sub")));
        let env = |key: &str| {
            cmd.get_envs()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| v)
                .map(|v| v.to_os_string())
        };
        let path = env("PATH").expect("PATH is set");
        assert_eq!(
            std::env::split_paths(&path).next().unwrap(),
            Path::new("/opt/rust/bin")
        );
        assert_eq!(env("CARGO_TERM_COLOR"), Some("never".into()));
    }

    #[test]
    fn cargo_is_found_through_the_forced_path_then_path_then_home() {
        let tmp = std::env::temp_dir().join(format!("formust-find-cargo-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("home/.cargo/bin")).unwrap();
        let fake = tmp.join("cargo");
        std::fs::write(&fake, "").unwrap();
        std::fs::write(tmp.join("home/.cargo/bin/cargo"), "").unwrap();
        assert_eq!(
            find_cargo(Some(fake.clone()), None, None),
            Some(fake.clone())
        );
        // A forced path that doesn't exist falls through to the search.
        assert_eq!(
            find_cargo(
                Some(tmp.join("nope")),
                Some(tmp.clone().into_os_string()),
                None
            ),
            Some(fake.clone())
        );
        assert_eq!(
            find_cargo(
                None,
                Some(tmp.join("empty").into_os_string()),
                Some(tmp.join("home"))
            ),
            Some(tmp.join("home/.cargo/bin/cargo"))
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn the_manifest_is_found_from_a_folder_a_file_or_itself() {
        let tmp = std::env::temp_dir().join(format!("formust-manifest-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("pkg/src/bin")).unwrap();
        std::fs::write(tmp.join("pkg/Cargo.toml"), "").unwrap();
        std::fs::write(tmp.join("pkg/src/bin/tool.rs"), "").unwrap();
        let manifest = tmp.join("pkg/Cargo.toml");
        assert_eq!(manifest_for(&manifest).unwrap(), manifest);
        assert_eq!(manifest_for(&tmp.join("pkg")).unwrap(), manifest);
        assert_eq!(manifest_for(&tmp.join("pkg/src/bin")).unwrap(), manifest);
        assert_eq!(
            manifest_for(&tmp.join("pkg/src/bin/tool.rs")).unwrap(),
            manifest
        );
        let missing = manifest_for(&tmp.join("pkg/nope/Cargo.toml")).unwrap_err();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
        assert!(missing.to_string().contains("does not exist"), "{missing}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
