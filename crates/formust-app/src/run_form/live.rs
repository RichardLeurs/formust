//! Live Run: while it is on, each committed change of the form rebuilds the Run Form program and
//! swaps its window.
//!
//! A change is a move of the designer's revision (an edit, an undo or redo, the end of a resize
//! drag; see `Designer::revision`) or of the tab's text (code view). Changes are coalesced: the
//! rebuild starts [`QUIET`] after the last one, or [`MAX_WAIT`] after the first while they keep
//! coming. The program is the same generated package as Run Form's ([`super::prepare_for`]), only
//! built (`cargo build`, not `cargo run`). The window that shows stays until the new build has
//! succeeded; then it is stopped and the new binary starts. A failed build keeps the old window and
//! puts its errors in Problems, at the places of the project's files. The latest change wins: a
//! build in flight for an older text is cancelled, and a preparation in flight is redone when it
//! ends (two preparations never write the program's files at once).
//!
//! The window is a new process each time: its position, size and state start over, except its
//! theme. It starts in the designer canvas's theme; when its Light/Dark switch is used, it prints a
//! line (`formust-theme: dark`, see [`super::THEME_LINE`]) that Live Run reads from its output (and
//! keeps out of Output), and the next window starts in that theme. A change of the canvas's theme
//! after that wins again: the latest choice counts. Nothing is written for it; it lasts while Live
//! Run is on.

use std::ffi::OsString;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use super::{
    Inputs, Plan, ProjectCache, RunTheme, THEME_REPORT_VAR, THEME_VAR, own_build_note, prepare_for,
    problem,
};
use crate::bottom;
use crate::cargo::{self, Session};
use crate::watch::Coalescer;
use eframe::egui;
use formust_build::{Action, Diagnostic, Event};

/// A rebuild starts this long after the last change.
pub const QUIET: Duration = Duration::from_millis(500);
/// ...or this long after the first change, while changes keep coming.
pub const MAX_WAIT: Duration = Duration::from_secs(5);

/// The program Live Run started: the built binary in its own process group, and the lines it
/// prints.
pub struct Window {
    child: Child,
    /// Its exit code once it ended (`None` inside when it was killed).
    exited: Option<Option<i32>>,
    lines: Receiver<String>,
    pub exe: PathBuf,
}

/// Sends each line of `pipe` to `tx`, and asks for a repaint after each and at the end.
fn forward(pipe: impl Read + Send + 'static, tx: Sender<String>, wake: Option<egui::Context>) {
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).split(b'\n') {
            let Ok(line) = line else { break };
            let line = String::from_utf8_lossy(&line);
            if tx
                .send(line.strip_suffix('\r').unwrap_or(&line).to_string())
                .is_err()
            {
                break;
            }
            if let Some(ctx) = &wake {
                ctx.request_repaint();
            }
        }
        if let Some(ctx) = &wake {
            ctx.request_repaint();
        }
    });
}

impl Window {
    /// Starts `exe` in `dir` with `env` added, its input empty and its output read.
    pub fn launch(
        exe: &Path,
        dir: &Path,
        env: &[(OsString, OsString)],
        wake: Option<egui::Context>,
    ) -> std::io::Result<Window> {
        let mut cmd = Command::new(exe);
        cmd.current_dir(dir).envs(env.iter().map(|(k, v)| (k, v)));
        Window::spawn(cmd, exe, wake)
    }

    fn spawn(mut cmd: Command, exe: &Path, wake: Option<egui::Context>) -> std::io::Result<Window> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // A group of its own, so Stop also ends what it started.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn()?;
        let (tx, lines) = channel();
        if let Some(out) = child.stdout.take() {
            forward(out, tx.clone(), wake.clone());
        }
        if let Some(err) = child.stderr.take() {
            forward(err, tx, wake);
        }
        Ok(Window {
            child,
            exited: None,
            lines,
            exe: exe.to_path_buf(),
        })
    }

    /// The lines it printed since the last call.
    pub fn take_lines(&self) -> Vec<String> {
        self.lines.try_iter().collect()
    }

    /// `None` while it runs; once it ended, its exit code (`None` inside when it was killed).
    pub fn ended(&mut self) -> Option<Option<i32>> {
        if self.exited.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.exited = Some(status.code());
        }
        self.exited
    }

    /// Kills it and everything it started, unless it ended already.
    pub fn stop(&mut self) {
        if self.ended().is_some() {
            return;
        }
        // Not reaped yet, so its pid (and group id) is still its own.
        #[cfg(unix)]
        // SAFETY: kill(2) with a negative pid signals that process group; no memory is involved.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.exited = Some(None);
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A preparation in flight. `stale`: a newer change came meanwhile, so its result is dropped and
/// the preparation runs again.
struct Prep {
    rx: Receiver<Result<(Plan, ProjectCache), Vec<Diagnostic>>>,
    key: u64,
    stale: bool,
}

/// A build in flight, for the inputs with key `key`.
struct Build {
    session: Session,
    key: u64,
    form_id: String,
    dir: PathBuf,
    started: Instant,
}

/// Live Run's state, while it is on.
pub struct Live {
    /// The tab it follows.
    pub path: PathBuf,
    /// The target folder for the program; the project's when `None` (the tests use a shared one).
    pub(crate) target: Option<PathBuf>,
    /// More cargo arguments for the build (the tests add `--offline`).
    pub(crate) cargo_args: Vec<OsString>,
    /// More environment for the program (the tests ask for the headless frame).
    pub(crate) child_env: Vec<(OsString, OsString)>,
    /// The last change seen: the designer's revision and the tab's text, hashed.
    seen: Option<u64>,
    pending: Coalescer,
    prep: Option<Prep>,
    build: Option<Build>,
    /// The window that shows, and the key of the inputs it was built from.
    pub window: Option<Window>,
    launched: Option<u64>,
    /// What `cargo metadata` said, for the next preparation.
    cache: Option<ProjectCache>,
    /// The canvas's theme when it was last seen (see [`Live::note_canvas`]).
    canvas: Option<RunTheme>,
    /// The theme last chosen in a window, since the canvas's theme last changed.
    chosen: Option<RunTheme>,
}

impl Live {
    pub fn new(path: PathBuf) -> Live {
        Live {
            path,
            target: None,
            cargo_args: Vec::new(),
            child_env: Vec::new(),
            seen: None,
            pending: Coalescer::new(QUIET, MAX_WAIT),
            prep: None,
            build: None,
            window: None,
            launched: None,
            cache: None,
            canvas: None,
            chosen: None,
        }
    }

    /// Notes the canvas's theme. When it changed since it was last seen, it is the latest choice: the
    /// one made in a window before is dropped.
    pub fn note_canvas(&mut self, canvas: RunTheme) {
        if self.canvas != Some(canvas) {
            self.canvas = Some(canvas);
            self.chosen = None;
        }
    }

    /// Reads a line of the window's output: a theme it reports is kept for the next window, and
    /// `true` says the line was such a report (it stays out of Output).
    pub fn note_line(&mut self, line: &str) -> bool {
        match RunTheme::from_line(line) {
            Some(theme) => {
                self.chosen = Some(theme);
                true
            }
            None => false,
        }
    }

    /// The theme the next window starts in: the one last chosen in a window, else the canvas's.
    pub fn start_theme(&self) -> RunTheme {
        self.chosen.or(self.canvas).unwrap_or(RunTheme::System)
    }

    /// A preparation or a build is in flight.
    pub fn busy(&self) -> bool {
        self.prep.is_some() || self.build.is_some()
    }

    /// Notes the state `key` seen at `now`. A change from the last one starts (or extends) the
    /// wait for the rebuild; the first state seen is no change.
    pub fn observe(&mut self, key: u64, now: Instant) {
        if self.seen.is_some_and(|seen| seen != key) {
            self.pending.push(None, now);
        }
        self.seen = Some(key);
    }

    /// Whether the rebuild is due at `now`; once true, false again until the next change.
    pub fn due(&mut self, now: Instant) -> bool {
        self.pending.take_if_due(now).is_some()
    }

    /// When the waiting rebuild is due, if one waits.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.deadline()
    }

    /// A build of the inputs with key `key` in `dir` ended. When it succeeded and made `exe`, the
    /// old window is stopped and `exe` starts: `Ok(true)`. A failed build (or one with no program)
    /// leaves the window that shows: `Ok(false)`.
    pub fn built(
        &mut self,
        success: bool,
        exe: Option<&Path>,
        dir: &Path,
        key: u64,
        wake: Option<egui::Context>,
    ) -> std::io::Result<bool> {
        let Some(exe) = exe.filter(|_| success) else {
            return Ok(false);
        };
        if let Some(mut old) = self.window.take() {
            old.stop();
        }
        self.launched = None;
        // The theme first, so that `child_env` (the tests') can override it.
        let mut env: Vec<(OsString, OsString)> = vec![
            (THEME_VAR.into(), self.start_theme().as_str().into()),
            (THEME_REPORT_VAR.into(), "1".into()),
        ];
        env.extend(self.child_env.iter().cloned());
        self.window = Some(Window::launch(exe, dir, &env, wake)?);
        self.launched = Some(key);
        Ok(true)
    }

    /// Ends everything: the build in flight and the window. A preparation in flight ends on its own;
    /// its answer is not read.
    pub fn stop_all(&mut self) {
        if let Some(mut build) = self.build.take() {
            build.session.stop();
        }
        self.prep = None;
        if let Some(mut window) = self.window.take() {
            window.stop();
        }
    }
}

/// The key of what Run Form would run: when it is the same as the window's, nothing is rebuilt.
pub fn inputs_key(inputs: &Inputs) -> u64 {
    let mut h = DefaultHasher::new();
    inputs.owner.hash(&mut h);
    inputs.owner_text.hash(&mut h);
    inputs.form_id.hash(&mut h);
    inputs.owner_type.hash(&mut h);
    inputs.method.hash(&mut h);
    format!("{:?}", inputs.kind).hash(&mut h);
    h.finish()
}

impl crate::App {
    /// Live Run is on.
    pub(crate) fn live_on(&self) -> bool {
        self.live.is_some()
    }

    /// Live Run is preparing or building.
    pub(crate) fn live_busy(&self) -> bool {
        self.live.as_ref().is_some_and(Live::busy)
    }

    /// The toolbar's Live switch: off when on; else on for the active tab's form (as Run Form
    /// picks it), building it at once.
    pub(crate) fn toggle_live(&mut self, ctx: &egui::Context) {
        if self.end_live("Live Run is off.\n") {
            return;
        }
        let rust = self
            .docs
            .get(self.active)
            .is_some_and(|d| crate::designer::is_rust(&d.path));
        if rust {
            self.sync_designer(true);
        }
        if !rust || !(self.run_is_form() || self.free_ui_at_caret().is_some()) {
            self.console = "Live Run needs a form: open a Rust file with a form.\n".into();
            self.auto_tab(ctx, bottom::BottomTab::Output);
            return;
        }
        let path = self.docs[self.active].path.clone();
        self.start_live(ctx, Live::new(path));
    }

    /// Turns Live Run on with `live` and starts its first build.
    pub(crate) fn start_live(&mut self, ctx: &egui::Context, mut live: Live) {
        live.observe(self.live_key(), Instant::now());
        self.live = Some(live);
        self.console =
            "Live Run is on: each change rebuilds the form's program and swaps its window.\n"
                .into();
        self.auto_tab(ctx, bottom::BottomTab::Output);
        self.live_rebuild(ctx);
    }

    /// Turns Live Run off: its build and its window end. Returns whether it was on.
    pub(crate) fn end_live(&mut self, why: &str) -> bool {
        let Some(mut live) = self.live.take() else {
            return false;
        };
        live.stop_all();
        self.console.push_str(why);
        true
    }

    /// The state Live Run follows: the designer's revision and the active tab's text.
    fn live_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.designer.revision.hash(&mut h);
        if let Some(doc) = self.docs.get(self.active) {
            doc.path.hash(&mut h);
            doc.text.hash(&mut h);
        }
        h.finish()
    }

    /// Run while Live Run is on for the active tab: rebuild now, instead of a second program.
    pub(crate) fn live_follows_active(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| self.docs.get(self.active).is_some_and(|d| d.path == l.path))
    }

    /// Starts a rebuild of the active tab's form, unless the window shows it already. The latest
    /// change wins: a build for older inputs is cancelled; a preparation in flight is marked to be
    /// redone, so two never write the program at once.
    pub(crate) fn live_rebuild(&mut self, ctx: &egui::Context) {
        if !self.live_follows_active() {
            return;
        }
        if let Some(prep) = self.live.as_mut().and_then(|l| l.prep.as_mut()) {
            prep.stale = true;
            return;
        }
        self.sync_designer(true);
        let inputs = match self.run_form_inputs() {
            Ok(inputs) => inputs,
            Err(p) => {
                // Typing can leave the file unreadable for a moment: the window stays, and the
                // next change tries again.
                self.console
                    .push_str(&format!("Live Run is waiting: {}\n", p.message));
                return;
            }
        };
        let key = inputs_key(&inputs);
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.build.as_ref().is_some_and(|b| b.key == key) {
            return;
        }
        if let Some(mut old) = live.build.take() {
            old.session.stop();
            self.console
                .push_str("Live Run: a newer change replaces the build in flight.\n");
        }
        let showing =
            live.launched == Some(key) && live.window.as_mut().is_some_and(|w| w.ended().is_none());
        if showing {
            return;
        }
        let (tx, rx) = channel();
        let (target, cache) = (live.target.clone(), live.cache.clone());
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(prepare_for(inputs, target, Action::Build, cache));
            repaint.request_repaint();
        });
        live.prep = Some(Prep {
            rx,
            key,
            stale: false,
        });
    }

    /// Each frame while Live Run is on: the window's output, the preparation and build in flight,
    /// and the changes that call for a rebuild.
    pub(crate) fn poll_live(&mut self, ctx: &egui::Context) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let path = live.path.clone();
        if !self.docs.iter().any(|d| d.path == path) {
            self.end_live("Live Run ended: its tab was closed.\n");
            return;
        }
        self.poll_live_window();
        self.poll_live_prep(ctx);
        self.poll_live_build(ctx);
        let now = Instant::now();
        if self.live_follows_active() {
            let key = self.live_key();
            if let Some(live) = self.live.as_mut() {
                live.observe(key, now);
            }
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.due(now) {
            self.live_rebuild(ctx);
        } else if let Some(deadline) = live.deadline() {
            ctx.request_repaint_after(deadline.saturating_duration_since(now));
        }
    }

    /// The window's lines go to Output, but for the theme it reports (kept for the next window);
    /// its end is said there.
    fn poll_live_window(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let Some(window) = live.window.as_mut() else {
            return;
        };
        let mut lines = window.take_lines();
        let ended = window.ended();
        if ended.is_some() {
            // What it printed last, before it ended.
            lines.extend(window.take_lines());
        }
        let mut text = String::new();
        for line in lines {
            if !live.note_line(&line) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        if let Some(code) = ended {
            text.push_str(&match code {
                Some(code) => format!(
                    "The Live Run window closed (exit code {code}). The next change opens it again.\n"
                ),
                None => "The Live Run window was stopped.\n".to_string(),
            });
            if let Some(live) = self.live.as_mut() {
                live.window = None;
                live.launched = None;
            }
        }
        self.console.push_str(&text);
    }

    fn poll_live_prep(&mut self, ctx: &egui::Context) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let result = match live.prep.as_ref().map(|p| p.rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => return,
            Some(Ok(result)) => result,
            Some(Err(TryRecvError::Disconnected)) => Err(vec![problem(
                &live.path,
                0,
                "Live Run stopped: its preparation ended without an answer.".into(),
            )]),
        };
        let Some(prep) = live.prep.take() else {
            return;
        };
        if let Ok((_, cache)) = &result {
            live.cache = Some(cache.clone());
        }
        if prep.stale {
            self.live_rebuild(ctx);
            return;
        }
        match result {
            Ok((plan, _)) => self.live_build(ctx, plan, prep.key),
            Err(problems) => {
                self.console
                    .push_str("Live Run could not build; the window keeps the last build.\n");
                self.diagnostics.clear();
                for p in &problems {
                    self.console.push_str(&p.message);
                    self.console.push('\n');
                    cargo::push_problem(&mut self.diagnostics, p.clone());
                }
                let marks = self.place_marks(&self.diagnostics);
                self.marks = marks;
                self.auto_tab(ctx, bottom::BottomTab::Problems);
            }
        }
    }

    /// Starts the build of a prepared program.
    fn live_build(&mut self, ctx: &egui::Context, plan: Plan, key: u64) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let mut request = plan.request;
        request.cargo_args.extend(live.cargo_args.iter().cloned());
        let manifest = request.path.clone();
        self.diagnostics.clear();
        self.marks.clear();
        self.console = format!(
            "Live Run: building {} in {}...\n",
            plan.form_id,
            plan.dir.display()
        );
        if let Some(note) = &plan.note {
            self.console.push_str(note);
            self.console.push('\n');
        }
        let repaint = ctx.clone();
        let build = formust_build::start(request, move || repaint.request_repaint());
        live.build = Some(Build {
            session: Session::new(build, Action::Build, manifest).with_places(plan.places),
            key,
            form_id: plan.form_id,
            dir: plan.dir,
            started: Instant::now(),
        });
    }

    /// The build's Output and Problems; at its end, the new window, or why the old one stays.
    fn poll_live_build(&mut self, ctx: &egui::Context) {
        let Some(build) = self.live.as_mut().and_then(|l| l.build.as_mut()) else {
            return;
        };
        let mut finished = None;
        while let Ok(event) = build.session.build.events().try_recv() {
            if let Some(text) = cargo::output_for(&event) {
                self.console.push_str(&text);
            }
            if let Some(p) = cargo::problem_for(&event) {
                let p = match &build.session.places {
                    Some(places) => places.real(p),
                    None => p,
                };
                cargo::push_problem(&mut self.diagnostics, p);
            }
            if let Event::Finished(outcome) = event {
                finished = Some(outcome);
                break;
            }
        }
        let Some(outcome) = finished else {
            return;
        };
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let Some(build) = live.build.take() else {
            return;
        };
        let failed = cargo::errors(&self.diagnostics);
        if outcome.cancelled {
            self.console.push_str("Live Run: the build was stopped.\n");
        } else if outcome.result.success {
            let exe = outcome.executables.last().cloned();
            live.note_canvas(RunTheme::of_canvas(
                self.designer.dark,
                ctx.global_style().visuals.dark_mode,
            ));
            match live.built(
                true,
                exe.as_deref(),
                &build.dir,
                build.key,
                Some(ctx.clone()),
            ) {
                Ok(true) => {
                    let exe = live
                        .window
                        .as_ref()
                        .map_or_else(String::new, |w| w.exe.display().to_string());
                    self.console.push_str(&format!(
                        "Live Run: {} rebuilt in {:.1} s; the window shows the change ({exe}).\n",
                        build.form_id,
                        build.started.elapsed().as_secs_f32()
                    ));
                }
                Ok(false) => self
                    .console
                    .push_str("Live Run: cargo built no program; the window stays.\n"),
                Err(e) => self
                    .console
                    .push_str(&format!("Live Run could not start the program: {e}\n")),
            }
        } else {
            if let Some(places) = build.session.places.as_ref() {
                self.console.push_str(&own_build_note(&places.owner_name()));
            }
            self.console
                .push_str(&cargo::summary(Action::Build, &outcome, failed));
            self.console
                .push_str("Live Run: the window keeps the last build that worked.\n");
        }
        let marks = self.place_marks(&self.diagnostics);
        self.marks = marks;
        if failed > 0 {
            self.auto_tab(ctx, bottom::BottomTab::Problems);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A burst of changes is one rebuild, [`QUIET`] after the last; the same state again is no
    /// change; a long burst rebuilds after [`MAX_WAIT`].
    #[test]
    fn a_burst_of_changes_is_one_rebuild() {
        let mut live = Live::new(PathBuf::from("/p/login.rs"));
        let t0 = Instant::now();
        live.observe(1, t0);
        assert_eq!(live.deadline(), None, "the first state is no change");
        live.observe(1, t0 + Duration::from_millis(50));
        assert_eq!(live.deadline(), None, "the same state is no change");
        for (i, ms) in [100u64, 200, 300].into_iter().enumerate() {
            live.observe(2 + i as u64, t0 + Duration::from_millis(ms));
        }
        assert!(
            !live.due(t0 + Duration::from_millis(700)),
            "still in the burst's wait"
        );
        assert!(
            live.due(t0 + Duration::from_millis(800)),
            "500 ms after the last"
        );
        assert!(
            !live.due(t0 + Duration::from_millis(900)),
            "one rebuild for the burst"
        );
        // Changes every 200 ms for longer than MAX_WAIT: due at MAX_WAIT after the first.
        let t1 = t0 + Duration::from_secs(10);
        let mut due = Vec::new();
        for step in 0..40u64 {
            let now = t1 + Duration::from_millis(step * 200);
            live.observe(100 + step, now);
            if live.due(now) {
                due.push(now - t1);
            }
        }
        assert_eq!(due.first(), Some(&MAX_WAIT), "{due:?}");
    }

    /// The next window starts in the theme last chosen in a window (its reported line, which stays
    /// out of Output), else the canvas's; a change of the canvas's theme after that wins again.
    #[test]
    fn the_next_window_keeps_the_theme_chosen_in_the_last() {
        let mut live = Live::new(PathBuf::from("/p/login.rs"));
        assert_eq!(live.start_theme(), RunTheme::System, "nothing seen yet");
        live.note_canvas(RunTheme::Light);
        assert_eq!(live.start_theme(), RunTheme::Light);
        assert!(!live.note_line("drawn: Name | Go"));
        assert!(live.note_line("formust-theme: dark"));
        assert_eq!(live.start_theme(), RunTheme::Dark, "chosen in the window");
        live.note_canvas(RunTheme::Light);
        assert_eq!(
            live.start_theme(),
            RunTheme::Dark,
            "the canvas did not change"
        );
        live.note_canvas(RunTheme::Dark);
        live.note_canvas(RunTheme::Light);
        assert_eq!(
            live.start_theme(),
            RunTheme::Light,
            "the canvas changed since: the latest choice"
        );
    }

    /// The program gets its start theme and the request to report a switch; the tests' environment
    /// comes after, so it can override them.
    #[test]
    fn the_window_is_launched_with_its_theme() {
        let mut live = Live::new(PathBuf::from("/p/login.rs"));
        live.note_canvas(RunTheme::Dark);
        let out = crate::target_tmp().join("formust-live-theme-env.txt");
        let _ = std::fs::remove_file(&out);
        let script = crate::target_tmp().join("formust-live-theme-env.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"${THEME_VAR}|${THEME_REPORT_VAR}\" > '{}'\n",
                out.display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        assert!(
            live.built(true, Some(&script), &std::env::temp_dir(), 1, None)
                .unwrap()
        );
        let started = Instant::now();
        while live.window.as_mut().unwrap().ended().is_none()
            && started.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "dark|1\n");
    }

    fn sleeper() -> Window {
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30");
        Window::spawn(cmd, Path::new("/bin/sleep"), None).unwrap()
    }

    /// A failed build (or one without a program) leaves the window that shows; a good one stops it
    /// and starts the new program.
    #[test]
    fn a_failed_build_keeps_the_old_window() {
        let mut live = Live::new(PathBuf::from("/p/login.rs"));
        live.window = Some(sleeper());
        live.launched = Some(1);
        let dir = std::env::temp_dir();
        assert!(
            !live
                .built(false, Some(Path::new("/usr/bin/true")), &dir, 2, None)
                .unwrap()
        );
        assert!(!live.built(true, None, &dir, 2, None).unwrap());
        let window = live.window.as_mut().expect("the old window stays");
        assert_eq!(window.exe, Path::new("/bin/sleep"));
        assert_eq!(window.ended(), None, "and still runs");
        assert_eq!(live.launched, Some(1));

        let pid = live.window.as_ref().unwrap().child.id();
        assert!(
            live.built(true, Some(Path::new("/bin/echo")), &dir, 3, None)
                .unwrap()
        );
        assert_eq!(live.window.as_ref().unwrap().exe, Path::new("/bin/echo"));
        assert_eq!(live.launched, Some(3));
        // SAFETY: signal 0 only asks whether the process exists.
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        assert!(!alive, "the old window was stopped");

        // Dropping Live Run (the app quits) stops its window too.
        live.window = Some(sleeper());
        let pid = live.window.as_ref().unwrap().child.id();
        drop(live);
        // SAFETY: as above.
        assert_ne!(
            unsafe { libc::kill(pid as i32, 0) },
            0,
            "dropped, so stopped"
        );
    }

    /// Stop ends the build and the window.
    #[test]
    fn stopping_ends_the_window() {
        let mut live = Live::new(PathBuf::from("/p/login.rs"));
        live.window = Some(sleeper());
        let pid = live.window.as_ref().unwrap().child.id();
        live.stop_all();
        assert!(live.window.is_none() && !live.busy());
        // SAFETY: signal 0 only asks whether the process exists.
        assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0);
    }
}
