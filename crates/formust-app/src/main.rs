mod about;
mod app_icons;
mod assistant;
mod bookmarks;
mod bottom;
mod cargo;
mod chrome;
mod designer;
mod editor_lines;
mod external;
mod files;
mod find;
mod formatting;
mod highlight;
mod hints;
mod lsp;
mod migrate;
mod prefs;
mod recent;
mod run_form;
mod session;
mod shell;
mod style;
mod system_menus;
mod terminal;
mod terminal_view;
mod theme;
mod tools_editor;
mod watch;

use eframe::egui;
use egui::text::{CCursor, CCursorRange};
use formust_build::{Action, BuildResult, Diagnostic, Severity};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

/// What File > Open offers: Rust sources and form files, Cargo manifests and the other text files the tree lists.
const OPEN_EXTENSIONS: &[&str] = files::TREE_EXTENSIONS;

struct Doc {
    path: PathBuf,
    text: String,
    saved: String,
    editor_id: egui::Id,
}

impl Doc {
    fn dirty(&self) -> bool {
        self.text != self.saved
    }
}

struct App {
    docs: Vec<Doc>,
    active: usize,
    next_editor_id: u32,
    root: PathBuf,
    diagnostics: Vec<Diagnostic>,
    console: String,
    /// The cargo run in flight (Check, Build or Run), if any.
    building: Option<cargo::Session>,
    /// Run Form being prepared (its program is written, then cargo starts), if any.
    form_prep: Option<run_form::Prep>,
    /// A question about unsaved changes that is waiting for an answer.
    confirm: Option<Confirm>,
    /// A change of project waiting for the answer to `confirm`: the new folder, and what to do in
    /// it once the old project's tabs are closed.
    project_switch: Option<(PathBuf, Box<dyn FnOnce(&mut App)>)>,
    /// The user chose to quit despite (or after saving) unsaved changes.
    quit_confirmed: bool,
    /// The Component Packages dialog (File > Import Component Package...) is open.
    /// The latest progress line of the build in flight, and how many units each package compiled last
    /// time (sizes the next bar).
    build_progress: Option<formust_build::Progress>,
    unit_counts: std::collections::HashMap<PathBuf, usize>,
    /// Check, Build and Run options (package, binary, release, program arguments), per open project.
    cargo_options: cargo::Options,
    /// The Cargo Options window, and the workspace's packages it lists.
    cargo_window: cargo::Window,
    /// The external tools (Tools menu) and the Configure Tools window while it is open.
    tools: Vec<formust_build::tools::Tool>,
    tools_editor: Option<tools_editor::Editor>,
    /// A tool that is running: its result comes back here.
    tool_run: Option<Receiver<ToolRun>>,
    /// Initial height of the bottom pane (the user can drag it).
    bottom_height: f32,
    /// The tabbed pane under the editor: selected tab and the terminal.
    bottom: bottom::BottomPane,
    lsp: Option<lsp::Handle>,
    /// The folder the language server was last started for (it follows the project folder).
    lsp_root: Option<PathBuf>,
    lsp_ready: bool,
    menu: Option<NativeMenu>,
    /// Waiting for the server: (file, text at request time, word start, cursor), in chars.
    pending_completion: Option<(PathBuf, String, usize, usize)>,
    completion: Option<Completion>,
    /// The pending completion was started by typing, so an empty answer is not worth a message.
    completion_auto: bool,
    /// Counts typed characters; `typing` waits for a pause after the latest one.
    typed: u64,
    typing: hints::Dwell<u64>,
    hover: hints::Hover,
    /// Diagnostics of the last build placed in the open files; dropped when a file changes.
    marks: Vec<hints::FileMarks>,
    /// Open files' text as the last build started, to place `marks`.
    build_texts: Vec<(PathBuf, String)>,
    /// The form designer for the active Rust file.
    designer: designer::Designer,
    /// Show the form designer instead of the code for the active file.
    form_view: bool,
    /// A Rust file the user just opened (tree, File > Open, Open Recent, the command line): its form
    /// shows once the designer has read it, if it has one. Cleared then, or when the user does
    /// something else (switches tabs, toggles the view).
    auto_form: Option<PathBuf>,
    /// The Rust files the user switched to Code view for in this session: opened again, they stay code.
    code_preferred: std::collections::HashSet<PathBuf>,
    /// Format Code is waiting for the language server (see `formatting`).
    formatting: Option<formatting::Formatting>,
    show_navigator: bool,
    /// The bottom pane (Problems, Output, Terminal) is shown.
    show_bottom: bool,
    /// The path bar (folder > file) above the editor; hidden by default to leave room for code.
    show_jump_bar: bool,
    nav_tab: shell::NavTab,
    expanded: std::collections::HashSet<PathBuf>,
    file_filter: String,
    /// 1-based line and column of the caret in the code editor.
    cursor_pos: (usize, usize),
    /// Where a hover popup for the current target goes (screen position).
    hover_pos: egui::Pos2,
    assistant: assistant::Assistant,
    /// An assistant build is running; its result goes back to the waiting agent.
    assistant_build_reply: Option<std::sync::mpsc::Sender<String>>,
    /// Changes made to open files by other programs, and the watcher that notices them.
    external: external::External,
    /// Project tree: the selected file or folder, a name being typed for a new item, and an
    /// item waiting for the user to confirm its deletion.
    tree_sel: Option<PathBuf>,
    tree_new: Option<shell::TreeNew>,
    tree_delete: Option<PathBuf>,
    tree_rename: Option<shell::TreeRename>,
    outline: shell::Outline,
    /// File > Open Recent: the lists, and the file they are kept in (`None` in tests, so they are
    /// not written to the user's folder).
    recent: recent::Recent,
    recent_file: Option<PathBuf>,
    /// A character range the editor scrolls into view on its next frame (set by go-to).
    reveal: Option<std::ops::Range<usize>>,
    /// The session file the app restores at launch and writes at exit (`None` in tests).
    session_file: Option<PathBuf>,
    /// The signature of the call at the caret (parameter hints while a call is typed).
    signature: Option<shell::SigHint>,
    prefs: prefs::Prefs,
    preferences_open: bool,
    /// Which appearance the Code Colours rows edit (`None`: the current one).
    prefs_dark: Option<bool>,
    /// The Preferences window shows its Colours tab (else its Editor tab).
    prefs_colours_tab: bool,
    about_open: bool,
    find: find::Bar,
    project_bar: find::ProjectBar,
    go_to: find::GoTo,
    /// Numbered bookmarks 0-9 (session only).
    bookmarks: bookmarks::Bookmarks,
    /// The files the last Replace All in Project changed, for Undo Replace.
    project_undo: Vec<find::Change>,
    /// The results of a project search still running on its worker thread.
    project_search_rx: Option<std::sync::mpsc::Receiver<(Vec<find::FileHits>, bool)>>,
    rename: find::Rename,
}

struct Completion {
    /// Text the request was made against; the popup closes when it changes.
    text: String,
    start: usize,
    cursor: usize,
    items: Vec<formust_lsp::CompletionItem>,
    selected: usize,
    /// Enter/Tab accept only an item the user picked (arrows or click), or any item when the
    /// popup was asked for with Ctrl+Space. Auto popups leave Enter and Tab alone.
    chosen: bool,
}

impl App {
    /// The app without a window, menu bar or language server: all state, no side effects.
    /// The tests use it to draw the UI offscreen, with the file's own folder as the project.
    #[cfg(test)]
    fn bare(path: Option<PathBuf>) -> Self {
        let root = path
            .as_ref()
            .and_then(|p| p.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::in_project(root, path)
    }

    /// The app with `root` as its project folder and `path` open. The project folder is walked
    /// for custom components, so it must be the folder the tests (and the user) mean, not a parent
    /// that happens to hold a Cargo.toml: `bare` keeps a file's own folder for that reason.
    fn in_project(root: PathBuf, path: Option<PathBuf>) -> Self {
        // Absolute, so tree rows compare equal to the (absolute) paths of open files.
        let root = root.canonicalize().unwrap_or(root);
        let mut app = Self {
            docs: Vec::new(),
            active: 0,
            next_editor_id: 0,
            root,
            diagnostics: Vec::new(),
            console: String::new(),
            building: None,
            form_prep: None,
            confirm: None,
            project_switch: None,
            quit_confirmed: false,
            build_progress: None,
            unit_counts: Default::default(),
            tools: Vec::new(),
            tools_editor: None,
            tool_run: None,
            bottom_height: 190.0,
            bottom: Default::default(),
            lsp: None,
            lsp_root: None,
            lsp_ready: false,
            menu: None,
            pending_completion: None,
            completion: None,
            completion_auto: false,
            typed: 0,
            typing: Default::default(),
            hover: Default::default(),
            marks: Vec::new(),
            build_texts: Vec::new(),
            designer: Default::default(),
            form_view: false,
            auto_form: None,
            code_preferred: Default::default(),
            formatting: None,
            show_navigator: true,
            show_bottom: true,
            show_jump_bar: false,
            nav_tab: shell::NavTab::default(),
            expanded: Default::default(),
            file_filter: String::new(),
            cursor_pos: (1, 1),
            hover_pos: egui::Pos2::ZERO,
            assistant: Default::default(),
            assistant_build_reply: None,
            external: Default::default(),
            tree_sel: None,
            tree_new: None,
            tree_delete: None,
            tree_rename: None,
            outline: Default::default(),
            recent: Default::default(),
            recent_file: None,
            reveal: None,
            session_file: None,
            signature: None,
            prefs: Default::default(),
            preferences_open: false,
            prefs_dark: None,
            prefs_colours_tab: false,
            about_open: false,
            find: Default::default(),
            project_bar: Default::default(),
            go_to: Default::default(),
            bookmarks: Default::default(),
            project_undo: Vec::new(),
            project_search_rx: None,
            rename: Default::default(),
            cargo_options: Default::default(),
            cargo_window: Default::default(),
        };
        if let Some(p) = path {
            app.open_path(p);
        }
        app
    }

    fn new(path: Option<PathBuf>, ctx: &egui::Context) -> Self {
        let given = path.is_some();
        // A file given on the command line belongs to its Cargo project (see `project_folder`).
        let root = path
            .as_deref()
            .map(project_folder)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."));
        let given_path = path.clone();
        let mut app = Self::in_project(root, path);
        app.assistant.read_older_keychain_names();
        let file = session::Session::file();
        app.session_file = file.clone();
        if let Some(saved) = file.as_deref().and_then(session::Session::load) {
            app.restore(saved, given);
        }
        // The file given on the command line is opened by hand: its form shows, if it has one.
        if let Some(p) = given_path {
            app.open_picked(p);
        }
        app.prefs = prefs::Prefs::load();
        app.prefs.apply();
        app.menu = NativeMenu::install(ctx);
        let file = recent::Recent::file();
        app.recent = file
            .as_deref()
            .map(recent::Recent::load)
            .unwrap_or_default();
        app.recent_file = file;
        app.refresh_recent_menu();
        app.tools = formust_build::tools::load();
        app.refresh_tools_menu();
        app.start_watching(ctx);
        if formust_build::locate_cargo().is_none() {
            app.console = format!("{}\n", formust_build::NOT_FOUND_HINT);
            app.auto_tab(ctx, bottom::BottomTab::Output);
        }
        app
    }

    /// What the session file keeps: the project, the open tabs and the active one.
    fn snapshot(&self) -> session::Session {
        session::Session {
            root: Some(self.root.clone()),
            tabs: self.docs.iter().map(|d| d.path.clone()).collect(),
            active: self.docs.get(self.active).map(|d| d.path.clone()),
            form_view: self.form_view,
        }
    }

    /// A project was opened or created: the problems and build errors of the previous project
    /// (the Problems list, the squiggles and the live check) do not belong to it any more.
    fn clear_build_results(&mut self) {
        self.diagnostics.clear();
        self.marks.clear();
        self.build_texts.clear();
        // The Output pane's text belongs to the old project too.
        self.console.clear();
    }

    /// Reopens the saved session: its project folder (unless a file was given on the command line,
    /// which keeps the folder it is in), its tabs that still exist, and the active tab. A file given on
    /// the command line is the active tab.
    fn restore(&mut self, saved: session::Session, given: bool) {
        if !given && let Some(root) = saved.root.filter(|r| r.is_dir()) {
            self.root = root.clone();
            self.clear_build_results();
        }
        // The file given on the command line is open already, as the first tab. Opening a tab makes
        // it active, so the first tab is made active again below.
        let given_tab = self.active;
        for tab in saved.tabs.iter().filter(|t| t.is_file()) {
            if !self.docs.iter().any(|d| &d.path == tab) {
                self.open_path(tab.clone());
            }
        }
        if given {
            self.active = given_tab;
        }
        if !given
            && let Some(i) = saved
                .active
                .as_ref()
                .and_then(|a| self.docs.iter().position(|d| &d.path == a))
        {
            self.active = i;
        }
        if saved.form_view {
            self.show_form();
        }
    }

    /// Opens `path` in a tab, or switches to it if it is already open.
    fn open_path(&mut self, path: PathBuf) {
        // Absolute, so one file never gets two tabs under different paths.
        let path = path.canonicalize().unwrap_or(path);
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            self.active = i;
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            self.console = format!("could not read {}\n", path.display());
            return;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            self.console = format!("could not read {} as text\n", path.display());
            return;
        };
        self.next_editor_id += 1;
        self.docs.push(Doc {
            path,
            saved: text.clone(),
            text,
            editor_id: egui::Id::unique(("formust-editor", self.next_editor_id)),
        });
        self.active = self.docs.len() - 1;
    }

    fn pick_file(&mut self) {
        if let Some(chosen) = rfd::FileDialog::new()
            .add_filter("Rust and text files", OPEN_EXTENSIONS)
            .pick_file()
        {
            self.open_in_project(chosen);
        }
    }

    /// File > Open Folder...: the folder becomes the project, and its main source opens.
    fn pick_folder(&mut self) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Open folder")
            .pick_folder()
        else {
            return;
        };
        let main = folder.clone();
        self.switch_project(folder.clone(), move |app| {
            app.remember(|r| r.note_project(&folder));
            if let Some(source) = cargo::main_source(&main) {
                app.open_picked(source);
            }
        });
    }

    /// Opens `chosen` in the project it belongs to: the folder of its Cargo.toml, or of the file itself
    /// when there is none. A Cargo.toml opens the project's main source.
    pub(crate) fn open_in_project(&mut self, chosen: PathBuf) {
        let dir = project_folder(&chosen);
        let is_manifest = chosen.file_name().is_some_and(|n| n == "Cargo.toml");
        let p = if is_manifest {
            cargo::main_source(&dir).unwrap_or_else(|| chosen.clone())
        } else {
            chosen
        };
        let dir = dir.clone();
        self.switch_project(dir.clone(), move |app| {
            app.remember(|r| r.note_project(&dir));
            app.remember(|r| r.note_file(&p));
            app.open_chosen(p);
        });
    }

    /// Opens a file chosen with File > Open. A form file (`.frm.rs`) shows its form in the designer;
    /// any other file shows its code.
    fn open_chosen(&mut self, p: PathBuf) {
        self.select_in_tree(p);
    }

    /// Changes the recent lists with `change`, saves them, and rebuilds the Open Recent menu.
    fn remember(&mut self, change: impl FnOnce(&mut recent::Recent)) {
        change(&mut self.recent);
        if let Some(file) = &self.recent_file {
            self.recent.save(file);
        }
        self.refresh_recent_menu();
    }

    fn refresh_recent_menu(&mut self) {
        let entries = self.recent.menu_entries();
        if let Some(m) = self.menu.as_mut() {
            m.set_recent(entries);
        }
    }

    /// Opens an item of Open Recent: a project's main source (`src/main.rs` or `src/lib.rs`) or a file, with
    /// its folder as the project.
    fn open_recent(&mut self, index: usize) {
        let Some(entry) = self
            .menu
            .as_ref()
            .and_then(|m| m.recent_entries.get(index).cloned())
        else {
            return;
        };
        self.open_recent_entry(entry);
    }

    fn open_recent_entry(&mut self, entry: recent::Entry) {
        if entry.project {
            let folder = entry.path.clone();
            self.switch_project(folder.clone(), move |app| {
                if let Some(main) = cargo::main_source(&folder) {
                    app.open_picked(main);
                }
                app.remember(|r| r.note_project(&folder));
            });
        } else {
            let dir = entry
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let file = entry.path.clone();
            self.switch_project(dir.clone(), move |app| {
                app.open_picked(file.clone());
                app.remember(|r| {
                    r.note_file(&file);
                    r.note_project(&dir);
                });
            });
        }
    }

    /// Saves tab `i`. Designer edits of its file are exported to the form file first (the `.rs`
    /// itself is only written when its text has unsaved edits). False when something was not saved:
    /// the export failed or waits for an answer to the conflict prompt.
    fn save_doc(&mut self, i: usize) -> bool {
        let Some(path) = self.docs.get(i).map(|d| d.path.clone()) else {
            return false;
        };
        if self.designer.dirty_for(&path) {
            if !self.export_designer(&path, false) {
                return false;
            }
            if !self.docs.get(i).is_some_and(|d| d.dirty()) {
                return true;
            }
        }
        let Some(d) = self.docs.get_mut(i) else {
            return false;
        };
        let wrote = std::fs::write(&d.path, &d.text).is_ok();
        if wrote {
            d.saved = d.text.clone();
        }
        let path = d.path.clone();
        if wrote {
            // An answer to "changed on disk" is settled by overwriting it.
            self.forget_external(&path);
        }
        wrote
    }

    /// A file's current text: its tab's, else the disk's.
    fn current_text(docs: &[Doc], path: &Path) -> Option<String> {
        docs.iter()
            .find(|d| d.path == path)
            .map(|d| d.text.clone())
            .or_else(|| std::fs::read_to_string(path).ok())
    }

    /// Exports the designer's edits of the file at `path` (shown or parked) to its form file. An
    /// open tab of the form file takes the new text. False when nothing was written because of an
    /// error or a question for the user (the designer shows it).
    pub(crate) fn export_designer(&mut self, path: &Path, overwrite: bool) -> bool {
        let docs = &self.docs;
        let read = |p: &Path| Self::current_text(docs, p);
        let decisions = designer::export::Decisions {
            overwrite_form_file: overwrite,
        };
        let outcome = self.designer.with_parked(path, |l| {
            designer::export::export(l, &read, &|p| p.exists(), decisions)
        });
        let name = |p: &Path| {
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        };
        match outcome {
            None | Some(designer::export::Outcome::Nothing) => true,
            Some(designer::export::Outcome::Written { path: frm, text }) => {
                if let Some(d) = self.docs.iter_mut().find(|d| d.path == frm) {
                    d.text = text.clone();
                    d.saved = text;
                }
                self.forget_external(&frm);
                self.designer.prompt = None;
                self.designer.status = Some(format!("Saved {}", name(&frm)));
                self.note_designer_loaded();
                true
            }
            Some(designer::export::Outcome::Ask(prompt)) => {
                if self
                    .designer
                    .loaded
                    .as_ref()
                    .is_some_and(|l| l.path == path)
                {
                    self.designer.prompt = Some(prompt);
                } else {
                    self.console.push_str(&format!("{}: the form file or its source changed; open its designer to decide what to keep\n", name(path)));
                }
                false
            }
            Some(designer::export::Outcome::Failed(message)) => {
                self.console
                    .push_str(&format!("Export failed: {message}\n"));
                self.designer.status = Some(format!("Not saved: {message}"));
                false
            }
        }
    }

    /// The designer's file is what the tabs and the disk now say, so the next sync doesn't read
    /// it again (an export wrote the form file the designer already holds).
    fn note_designer_loaded(&mut self) {
        let Some((path, text, companion)) = self.designer_inputs() else {
            return;
        };
        if self.designer.loaded.as_ref().is_none_or(|l| l.path != path) {
            return;
        }
        let key = designer::key_of(
            &path,
            &text,
            companion.as_ref().map(|(p, t)| (p.as_path(), t.as_str())),
        );
        self.designer.text_key = designer::key_of(&path, &text, None);
        if let Some(l) = self.designer.loaded.as_mut() {
            l.key = key;
        }
    }

    /// Runs what the designer asked for while it was drawn.
    pub(crate) fn run_designer_actions(&mut self) {
        let path = self.designer.loaded.as_ref().map(|l| l.path.clone());
        for action in std::mem::take(&mut self.designer.actions) {
            let Some(path) = path.clone() else { continue };
            let docs = &self.docs;
            let read = |p: &Path| Self::current_text(docs, p);
            match action {
                designer::ui::Action::Save { overwrite } => {
                    self.export_designer(&path, overwrite);
                }
                designer::ui::Action::Keep(form) => {
                    self.designer.keep_form(&form, &read);
                    self.export_designer(&path, false);
                }
                designer::ui::Action::Reimport(form) => {
                    if self.designer.reimport_form(&form, &read) {
                        self.designer.status = Some(format!(
                            "{form} re-imported from the source; Save writes it to the form file"
                        ));
                    }
                }
                designer::ui::Action::Reload => {
                    self.designer.reload();
                    self.sync_designer(true);
                }
                designer::ui::Action::ShowDiff(prompt) => {
                    self.designer.diff = self.designer.loaded.as_ref().and_then(|l| {
                        designer::export::diff_for(l, &prompt, &read, &|p| p.exists())
                    });
                }
            }
        }
    }

    /// Saves every tab; false when one of them could not be saved.
    fn save_all(&mut self) -> bool {
        let mut all = true;
        for i in 0..self.docs.len() {
            if self.is_dirty(i) || self.docs[i].path.exists() {
                all &= self.save_doc(i);
            }
        }
        all
    }

    /// Has tab `i` unsaved changes, in its text or in the designer's edits of its forms?
    fn is_dirty(&self, i: usize) -> bool {
        self.docs
            .get(i)
            .is_some_and(|d| d.dirty() || self.designer.dirty_for(&d.path))
    }

    /// The close button of a tab: closes at once, or asks first when there are unsaved changes.
    fn request_close(&mut self, i: usize) {
        match self.docs.get(i).map(|d| d.path.clone()) {
            Some(path) if self.is_dirty(i) => self.confirm = Some(Confirm::Tab(path)),
            Some(_) => self.close_doc(i),
            None => {}
        }
    }

    /// Makes `dir` the project, then runs `then` (what to open or record there). The tabs of the old
    /// project are closed first; when some of them have unsaved changes the user is asked, and the
    /// switch waits for the answer. Staying in the same folder keeps the tabs.
    fn switch_project(&mut self, dir: PathBuf, then: impl FnOnce(&mut App) + 'static) {
        if dir == self.root {
            then(self);
            return;
        }
        let dirty =
            (0..self.docs.len()).any(|i| !self.docs[i].path.starts_with(&dir) && self.is_dirty(i));
        if dirty {
            self.confirm = Some(Confirm::Project(dir.clone()));
            self.project_switch = Some((dir, Box::new(then)));
            return;
        }
        self.close_tabs_outside(&dir, false);
        self.enter_project(dir);
        then(self);
    }

    /// The project folder is now `dir`; the build results belong to the old one.
    fn enter_project(&mut self, dir: PathBuf) {
        self.root = dir;
        self.clear_build_results();
        self.cargo_options = cargo::Options::default();
        self.cargo_window.forget();
    }

    /// Closes the tabs whose files are not in `dir`. With `save`, the unsaved ones are saved first.
    fn close_tabs_outside(&mut self, dir: &Path, save: bool) {
        let mut i = 0;
        while i < self.docs.len() {
            if self.docs[i].path.starts_with(dir) {
                i += 1;
            } else if save {
                if self.is_dirty(i) && !self.save_doc(i) {
                    // Not saved (an export asks a question): the tab stays open.
                    i += 1;
                    continue;
                }
                self.close_doc(i);
            } else {
                self.discard_doc(i);
            }
        }
    }

    /// Closes tab `i` without saving it, the designer's edits of its file included.
    fn discard_doc(&mut self, i: usize) {
        if let Some(path) = self.docs.get(i).map(|d| d.path.clone()) {
            self.designer.discard(&path);
        }
        self.remove_doc(i);
    }

    fn close_doc(&mut self, i: usize) {
        self.remove_doc(i);
    }

    fn remove_doc(&mut self, i: usize) {
        self.docs.remove(i);
        self.active = self.active.min(self.docs.len().saturating_sub(1));
        if !self.has_form() {
            self.form_view = false;
        }
    }

    /// The active tab is a Rust file the designer can show: a form file, or a source that has had
    /// forms this session (so a syntax error while typing doesn't hide the Form button).
    fn has_form(&self) -> bool {
        self.docs.get(self.active).is_some_and(|d| {
            formust_rust::file_kind(&d.path) == formust_rust::FileKind::Form
                || self.designer.had_forms.contains(&d.path)
        })
    }

    /// A file was selected in the project tree: a form file (`.frm.rs`) shows its form; any other
    /// file is opened by hand (see `open_picked`).
    pub(crate) fn select_in_tree(&mut self, path: PathBuf) {
        let is_form_file = formust_rust::file_kind(&path) == formust_rust::FileKind::Form;
        if is_form_file {
            self.open_path(path);
            self.show_form();
        } else {
            self.open_picked(path);
        }
    }

    /// Opens a file the user picked (the tree, File > Open, Open Recent, the command line). A Rust file
    /// with a form shows the form once the designer has read it; any other file shows its code, and so
    /// does a Rust file the user switched to Code for earlier in this session.
    fn open_picked(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        self.form_view = false;
        self.auto_form = None;
        self.open_path(path.clone());
        // A file that could not be read leaves the active tab as it was.
        if self.docs.get(self.active).is_none_or(|d| d.path != path) {
            return;
        }
        if !designer::is_rust(&path) || self.code_preferred.contains(&path) {
            return;
        }
        self.auto_form = Some(path);
        self.sync_designer(true);
    }

    /// The code view of the active tab. Leaving the form for the code is a choice, remembered for the
    /// file so it opens in code next time too.
    pub(crate) fn show_code(&mut self) {
        self.auto_form = None;
        if let Some(doc) = self.docs.get(self.active)
            && designer::is_rust(&doc.path)
        {
            self.code_preferred.insert(doc.path.clone());
        }
        self.form_view = false;
    }

    /// The designer is what the window is showing right now.
    fn form_showing(&self) -> bool {
        self.form_view
            && self
                .docs
                .get(self.active)
                .is_some_and(|d| designer::is_rust(&d.path))
    }

    /// Shows the form designer for the active Rust file (`.rs` or `.frm.rs`).
    fn show_form(&mut self) {
        if !self
            .docs
            .get(self.active)
            .is_some_and(|d| designer::is_rust(&d.path))
        {
            return;
        }
        self.auto_form = None;
        self.sync_designer(true);
        self.form_view = true;
    }

    /// Re-reads the active Rust file for the designer (see `read_designer`), then settles a pending
    /// auto Form view (see `resolve_auto_form`).
    pub(crate) fn sync_designer(&mut self, now: bool) {
        self.read_designer(now);
        self.resolve_auto_form();
    }

    /// The auto Form view of a file the user opened (see `open_picked`): once the designer has read
    /// that file, its form shows if it has one, else the code stays. A change of tab meanwhile cancels
    /// it. It happens once: the flag is cleared either way.
    fn resolve_auto_form(&mut self) {
        let Some(path) = self.auto_form.clone() else {
            return;
        };
        if self.docs.get(self.active).is_none_or(|d| d.path != path) {
            self.auto_form = None;
            return;
        }
        let Some(loaded) = self.designer.loaded.as_ref().filter(|l| l.path == path) else {
            return;
        };
        let has_forms = !loaded.forms.is_empty();
        self.auto_form = None;
        if has_forms {
            self.form_view = true;
        }
    }

    /// Re-reads the active Rust file for the designer when its text, or its companion's, changed.
    /// The companion (a source's form file, a form file's source) is taken from its tab when open,
    /// else from the disk. With `now` it reads at once; otherwise a change to the tab's text is read
    /// at once while the designer shows, and everything else (the disk, a hidden designer's file)
    /// no more than once a second.
    fn read_designer(&mut self, now: bool) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        if !designer::is_rust(&doc.path) {
            return;
        }
        let path = doc.path.clone();
        let text_key = designer::key_of(&path, &doc.text, None);
        let due = now
            || self.designer.loaded.as_ref().is_none_or(|l| l.path != path)
            || (self.form_view && self.designer.text_key != text_key)
            || self
                .designer
                .checked
                .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
        if !due {
            return;
        }
        self.designer.checked = Some(std::time::Instant::now());
        self.designer.text_key = text_key;
        let Some((path, text, companion)) = self.designer_inputs() else {
            return;
        };
        let companion = companion.as_ref().map(|(p, t)| (p.as_path(), t.as_str()));
        let key = designer::key_of(&path, &text, companion);
        if !self.designer.is_loaded(&path, key) {
            let loaded = designer::load(&path, &text, companion);
            self.designer.store(loaded, key);
        }
    }

    /// What the designer reads for the active tab: its path and text, and its companion (a
    /// source's form file, a form file's source) with its text, from its tab when open, else the
    /// disk.
    fn designer_inputs(&self) -> Option<(PathBuf, String, Option<(PathBuf, String)>)> {
        let doc = self.docs.get(self.active)?;
        if !designer::is_rust(&doc.path) {
            return None;
        }
        let path = doc.path.clone();
        let open = |p: &Path| self.docs.iter().any(|d| d.path == p);
        // The companion on disk, else one open in a tab but not saved yet.
        let companion_path = files::companions(&path).into_iter().next().or_else(|| {
            match formust_rust::file_kind(&path) {
                formust_rust::FileKind::Form => {
                    formust_rust::source_path_for(&path, None, open).filter(|p| open(p))
                }
                _ => formust_rust::form_path_for(&path, open)
                    .map(|f| f.path)
                    .filter(|p| open(p)),
            }
        });
        let companion = companion_path.and_then(|p| {
            let text = Self::current_text(&self.docs, &p)?;
            Some((p, text))
        });
        Some((path, doc.text.clone(), companion))
    }

    fn refresh_tools_menu(&mut self) {
        if let Some(m) = self.menu.as_mut() {
            m.set_tools(&self.tools);
        }
    }

    /// Runs a tool from the Tools menu. Its output goes to the Output tab and its compiler-style errors to
    /// Problems. One build or tool at a time, since they share the console.
    fn run_tool(&mut self, ctx: &egui::Context, index: usize) {
        let Some(tool) = self.tools.get(index).cloned() else {
            return;
        };
        if self.working() || self.tool_run.is_some() {
            self.console
                .push_str("Wait for the build or tool to finish before running another tool.\n");
            self.auto_tab(ctx, bottom::BottomTab::Output);
            return;
        }
        self.save_all();
        let file = self.docs.get(self.active).map(|d| d.path.clone());
        let context = self.tool_context(ctx, file.as_deref());
        let plan = formust_build::tools::plan(&tool, &context);
        if !plan.unknown.is_empty() {
            let names: Vec<String> = plan.unknown.iter().map(|m| format!("$({m})")).collect();
            self.console = format!(
                "{} uses an unknown macro: {}. Fix it in Configure Tools.\n",
                tool.name,
                names.join(", ")
            );
            self.auto_tab(ctx, bottom::BottomTab::Output);
            return;
        }
        self.console = format!(
            "Running {}...\n$ {} {}\n",
            tool.name,
            plan.command,
            plan.args.join(" ")
        );
        self.auto_tab(ctx, bottom::BottomTab::Output);
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(run_tool_plan(&plan));
            ctx.request_repaint();
        });
        self.tool_run = Some(rx);
    }

    /// What the tools' macros know now: the active file, the Cargo project, the selected text and the
    /// caret's line.
    fn tool_context(
        &self,
        ctx: &egui::Context,
        file: Option<&Path>,
    ) -> formust_build::tools::Context {
        let mut out = formust_build::tools::Context {
            file: file.map(Path::to_path_buf),
            project_dir: Some(self.root.clone()),
            ..Default::default()
        };
        if let Some(doc) = self.docs.get(self.active)
            && let Some(range) =
                egui::TextEdit::load_state(ctx, doc.editor_id).and_then(|s| s.cursor.char_range())
        {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            let (lo, hi) = (a.min(b), a.max(b));
            out.selection = doc.text.chars().skip(lo).take(hi - lo).collect();
            out.line = doc.text.chars().take(a).filter(|&c| c == '\n').count() as u32 + 1;
        }
        out
    }

    fn poll_tool(&mut self) {
        let Some(rx) = &self.tool_run else {
            return;
        };
        let Ok(done) = rx.try_recv() else {
            return;
        };
        self.tool_run = None;
        self.console.push_str(&done.output);
    }

    /// The window close button and Quit: with unsaved changes the user is asked first.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_confirmed
            && (0..self.docs.len()).any(|i| self.is_dirty(i))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        }
    }

    /// The "save changes?" question: Save, Don't Save or Cancel.
    fn show_confirm(&mut self, ctx: &egui::Context) {
        let Some(what) = self.confirm.clone() else {
            return;
        };
        let (title, detail, save_label, discard_label) = match &what {
            Confirm::Tab(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (
                    format!("Save changes to \u{201C}{name}\u{201D}?"),
                    "Your changes will be lost if you don't save them.".to_string(),
                    "Save",
                    "Don't Save",
                )
            }
            Confirm::Project(dir) => {
                let n = (0..self.docs.len())
                    .filter(|&i| !self.docs[i].path.starts_with(dir) && self.is_dirty(i))
                    .count();
                (
                    format!(
                        "Opening another project closes its tabs. {n} file{} with unsaved changes. Do you want to save {}?",
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "it" } else { "them" }
                    ),
                    "Your changes will be lost if you don't save them.".to_string(),
                    "Save",
                    "Don't Save",
                )
            }
            Confirm::Quit => {
                let n = (0..self.docs.len()).filter(|&i| self.is_dirty(i)).count();
                (
                    format!(
                        "You have {n} file{} with unsaved changes. Do you want to save {}?",
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "it" } else { "them" }
                    ),
                    "Your changes will be lost if you don't save them.".to_string(),
                    "Save All",
                    "Don't Save",
                )
            }
        };
        enum Answer {
            Save,
            Discard,
            Cancel,
        }
        let mut answer = None;
        style::dialog(ctx, "confirm-dialog", None, 380.0, |ui| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(title).strong().size(14.0));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(detail).weak());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if crate::style::secondary_button(ui, discard_label).clicked() {
                    answer = Some(Answer::Discard);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::style::primary_button(ui, save_label).clicked() {
                        answer = Some(Answer::Save);
                    }
                    if crate::style::secondary_button(ui, "Cancel").clicked() {
                        answer = Some(Answer::Cancel);
                    }
                });
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(Answer::Cancel);
        } else if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            answer = Some(Answer::Save);
        }
        let Some(answer) = answer else {
            return;
        };
        self.confirm = None;
        match (what, answer) {
            (_, Answer::Cancel) => self.project_switch = None,
            (Confirm::Project(dir), a) => {
                if let Some((_, then)) = self.project_switch.take() {
                    self.close_tabs_outside(&dir, matches!(a, Answer::Save));
                    self.enter_project(dir);
                    then(self);
                }
            }
            (Confirm::Tab(path), a) => {
                if let Some(i) = self.docs.iter().position(|d| d.path == path) {
                    match a {
                        Answer::Save => {
                            if self.save_doc(i) {
                                self.close_doc(i);
                            }
                        }
                        _ => self.discard_doc(i),
                    }
                }
            }
            (Confirm::Quit, a) => {
                if matches!(a, Answer::Save) && !self.save_all() {
                    // Something wasn't saved (a form export asks first): stay open.
                    return;
                }
                self.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Puts build diagnostics into the open files they belong to (lines and columns are 1-based).
    fn place_marks(&self, diagnostics: &[Diagnostic]) -> Vec<hints::FileMarks> {
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        self.build_texts
            .iter()
            .filter_map(|(path, text)| {
                let items: Vec<_> = diagnostics
                    .iter()
                    .filter(|d| {
                        // Line 0 is a project-level problem (a missing package): it has no place in the text.
                        d.line > 0
                            && matches!(
                                d.severity,
                                Severity::Fatal | Severity::Error | Severity::Warning
                            )
                            && canon(&d.file) == canon(path)
                    })
                    .map(|d| hints::Mark {
                        line: d.line.saturating_sub(1),
                        col: d.col.saturating_sub(1),
                        severity: d.severity,
                        message: d.message.clone(),
                    })
                    .collect();
                (!items.is_empty()).then(|| hints::FileMarks {
                    path: path.clone(),
                    text: text.clone(),
                    items,
                })
            })
            .collect()
    }

    /// Shows `tab` in the bottom pane unless the user is using it (see `BottomPane::auto`).
    fn auto_tab(&mut self, ctx: &egui::Context, tab: bottom::BottomTab) {
        let now = ctx.input(|i| i.time);
        self.bottom.auto(tab, now);
        // A build brings the pane back, so its output is never hidden.
        self.show_bottom = true;
    }

    fn cursor_index(&self, ctx: &egui::Context) -> Option<usize> {
        let doc = self.docs.get(self.active)?;
        let state = egui::TextEdit::load_state(ctx, doc.editor_id)?;
        state.cursor.char_range().map(|r| r.primary.index.0)
    }

    /// `auto` is completion started by typing: it stays silent when it cannot help.
    fn request_completion(&mut self, ctx: &egui::Context, auto: bool) {
        let Some(lsp) = self.lsp.as_ref().filter(|_| !auto || self.lsp_ready) else {
            if !auto {
                self.console
                    .push_str("completion: language server is off\n");
            }
            return;
        };
        let (Some(doc), Some(cursor)) = (self.docs.get(self.active), self.cursor_index(ctx)) else {
            if !auto {
                self.console
                    .push_str("completion: click inside the editor first\n");
            }
            return;
        };
        if auto && !hints::should_complete(&doc.text, cursor) {
            return;
        }
        self.completion_auto = auto;
        let (line, col) = line_col(&doc.text, cursor);
        let start = word_start(&doc.text, cursor);
        let _ = lsp.requests.send(lsp::Req::Complete {
            path: doc.path.clone(),
            text: doc.text.clone(),
            line,
            col,
        });
        self.pending_completion = Some((doc.path.clone(), doc.text.clone(), start, cursor));
    }

    fn request_hover(&mut self, path: PathBuf, text: String, line: u32, col: u32) {
        let Some(lsp) = self.lsp.as_ref().filter(|_| self.lsp_ready) else {
            return;
        };
        self.hover.requested = Some((line, col));
        let _ = lsp.requests.send(lsp::Req::Hover {
            path,
            text,
            line,
            col,
        });
    }

    /// Asks the server for the signature of the call at the caret (after `(` or `,`).
    fn request_signature(
        &mut self,
        path: PathBuf,
        text: String,
        line: u32,
        col: u32,
        at: usize,
        key: u64,
    ) {
        let Some(lsp) = self.lsp.as_ref().filter(|_| self.lsp_ready) else {
            return;
        };
        let _ = lsp.requests.send(lsp::Req::Signature {
            path,
            text,
            line,
            col,
            at,
            key,
        });
    }

    fn request_definition(&mut self, ctx: &egui::Context) {
        if let Some(cursor) = self.cursor_index(ctx) {
            self.request_definition_at(cursor);
        }
    }

    /// Asks the server for every use of the symbol under the caret (Shift+F12).
    fn request_references(&mut self, ctx: &egui::Context) {
        let Some(index) = self.cursor_index(ctx) else {
            return;
        };
        let (Some(lsp), Some(doc)) = (&self.lsp, self.docs.get(self.active)) else {
            self.console.push_str("language server is not running\n");
            return;
        };
        let Some((_, name)) = find::word_at(&doc.text, index) else {
            self.console.push_str("put the caret on a name first\n");
            return;
        };
        let (line, col) = line_col(&doc.text, index);
        self.project_bar.title = format!("References to {name}");
        let _ = lsp.requests.send(lsp::Req::References {
            path: doc.path.clone(),
            text: doc.text.clone(),
            line,
            col,
        });
    }

    /// Asks the server to rename the symbol in the rename box to `self.rename.name`.
    fn send_rename(&mut self) {
        let (Some(lsp), Some(path), Some(doc)) = (
            &self.lsp,
            self.rename.path.clone(),
            self.docs
                .iter()
                .find(|d| Some(&d.path) == self.rename.path.as_ref()),
        ) else {
            self.rename.error = "The language server is not running".into();
            return;
        };
        // The open files as they are now: the server's answer is checked against them.
        let open: Vec<(PathBuf, String)> = self
            .docs
            .iter()
            .map(|d| (d.path.clone(), d.text.clone()))
            .collect();
        let new_name = self.rename.name.clone();
        let _ = lsp.requests.send(lsp::Req::Rename {
            path,
            text: doc.text.clone(),
            line: self.rename.line,
            col: self.rename.col,
            new_name: new_name.clone(),
        });
        self.rename.sent = Some(find::Sent {
            new: new_name,
            open,
        });
        self.rename.open = false;
        self.rename.error.clear();
    }

    /// Asks the server where the symbol at char `index` of the active tab is defined.
    fn request_definition_at(&mut self, index: usize) {
        let (Some(lsp), Some(doc)) = (&self.lsp, self.docs.get(self.active)) else {
            return;
        };
        let (line, col) = line_col(&doc.text, index);
        let _ = lsp.requests.send(lsp::Req::Definition {
            path: doc.path.clone(),
            text: doc.text.clone(),
            line,
            col,
        });
    }

    /// Starts the language server for the project folder, and again when the folder changes.
    /// Not for `/` or the home folder (a Finder launch has no project; rust-analyzer would index it all).
    fn follow_project_with_lsp(&mut self, ctx: &egui::Context) {
        if self.lsp_root.as_deref() == Some(self.root.as_path()) || !lsp_worthy(&self.root) {
            return;
        }
        self.lsp_root = Some(self.root.clone());
        self.lsp_ready = false;
        self.lsp = root_abs(&self.root).and_then(|r| lsp::spawn(&r, ctx.clone()));
        if self.lsp.is_none() {
            self.console
                .push_str("language server off: rust-analyzer was not found. Install it with `rustup component add rust-analyzer`.\n");
        }
    }

    fn poll_lsp(&mut self, ctx: &egui::Context) {
        self.follow_project_with_lsp(ctx);
        let Some(lsp) = &self.lsp else { return };
        let responses: Vec<_> = lsp.responses.try_iter().collect();
        for resp in responses {
            match resp {
                lsp::Resp::Ready => self.lsp_ready = true,
                lsp::Resp::Completion(items) => {
                    let pending = self.pending_completion.take();
                    let auto = self.completion_auto;
                    // An answer for text that has since changed is stale.
                    let current = pending.filter(|(path, text, ..)| {
                        self.docs
                            .get(self.active)
                            .is_some_and(|d| d.path == *path && d.text == *text)
                    });
                    match current {
                        Some((_, text, start, cursor)) if !items.is_empty() => {
                            // Typing `foo` with only `foo` on offer needs no popup.
                            let word: String =
                                text.chars().skip(start).take(cursor - start).collect();
                            if !(auto && items.iter().all(|i| i.label.eq_ignore_ascii_case(&word)))
                            {
                                self.completion = Some(Completion {
                                    text,
                                    start,
                                    cursor,
                                    items,
                                    selected: 0,
                                    chosen: !auto,
                                });
                            }
                        }
                        Some(_) if !auto => self
                            .console
                            .push_str("completion: nothing to suggest here\n"),
                        _ => {}
                    }
                }
                lsp::Resp::Definition(Some(loc)) => {
                    self.goto(ctx, &loc.path, loc.line + 1, loc.col + 1);
                }
                lsp::Resp::Definition(None) => {}
                lsp::Resp::Signature { at, key, sig } => {
                    let current = self
                        .docs
                        .get(self.active)
                        .is_some_and(|d| shell::text_key(&d.text) == key);
                    self.signature = if current {
                        sig.map(|sig| shell::SigHint { key, at, sig })
                    } else {
                        None
                    };
                }
                lsp::Resp::Symbols { path, key, symbols } => {
                    if self.outline.pending.as_ref() == Some(&(path.clone(), key)) {
                        self.outline.pending = None;
                    }
                    self.outline.shown = Some((path, key));
                    self.outline.symbols = symbols;
                }
                lsp::Resp::References(locs) => {
                    self.show_references(locs);
                    self.nav_tab = shell::NavTab::Search;
                    self.show_navigator = true;
                }
                lsp::Resp::Formatted { error } => self.finish_format(ctx, error),
                lsp::Resp::Rename(files) => {
                    if let Some(sent) = self.rename.sent.take() {
                        self.apply_rename(files, &sent);
                    }
                }
                lsp::Resp::Hover(line, col, text) => {
                    let pos = self.hover_pos;
                    self.hover.accept(line, col, text, pos);
                }
                lsp::Resp::Error(e) => {
                    self.console.push_str(&format!(
                        "language server: {e}\nrust-analyzer stopped. Check that the project's Cargo.toml is valid, then reopen the project.\n"
                    ));
                    self.lsp = None;
                    self.lsp_ready = false;
                    return;
                }
            }
        }
    }

    /// Replaces the word being typed with completion `index`.
    fn accept_completion(&mut self, ctx: &egui::Context, index: usize) {
        let Some(c) = self.completion.take() else {
            return;
        };
        let (Some(item), Some(doc)) = (c.items.get(index), self.docs.get_mut(self.active)) else {
            return;
        };
        let chars: Vec<char> = doc.text.chars().collect();
        let (start, cursor) = (c.start.min(chars.len()), c.cursor.min(chars.len()));
        let mut text: String = chars[..start].iter().collect();
        text.push_str(&item.label);
        text.extend(&chars[cursor..]);
        doc.text = text;
        let id = doc.editor_id;
        let at = start + item.label.chars().count();
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(at))));
        state.store(ctx, id);
    }

    fn handle_editor_keys(&mut self, ctx: &egui::Context, accept: &mut Option<usize>) {
        use egui::{Key, Modifiers};
        if self.bottom.tab == bottom::BottomTab::Terminal && self.bottom.terminal_focused {
            // The terminal owns the keyboard (Ctrl+Space, arrows, Esc are the shell's).
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Space)) {
            self.request_completion(ctx, false);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F12)) {
            self.request_definition(ctx);
        }
        // Find (Cmd+F), replace (Cmd+Opt+F), next and previous match (Cmd+G, Shift+Cmd+G).
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::F)) {
            self.find.show(true);
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            self.find.show(false);
        }
        // References (Shift+F12) and Rename Symbol (F2).
        if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F12)) {
            self.request_references(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
            self.open_rename(ctx);
        }
        // Go to Line (Cmd+L) opens its box with the field focused.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::L)) {
            self.go_to.open = true;
            self.go_to.focus = true;
            self.go_to.error.clear();
        }
        // Bookmarks: Ctrl+Shift+0-9 sets one at the caret's line, Ctrl+0-9 jumps to it.
        const DIGITS: [Key; 10] = [
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (n, key) in DIGITS.into_iter().enumerate() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, key)) {
                self.toggle_bookmark(ctx, n);
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, key)) {
                self.go_to_bookmark(ctx, n);
            }
        }
        // Project search (Cmd+Shift+F) opens its tab with the query focused.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)) {
            self.nav_tab = shell::NavTab::Search;
            self.show_navigator = true;
            self.project_bar.focus = true;
        }
        if self.find.open {
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::G)) {
                self.find.pending = Some(false);
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::G)) {
                self.find.pending = Some(true);
            }
        }
        let Some(c) = &mut self.completion else {
            return;
        };
        let (down, up, enter, tab, esc) = ctx.input_mut(|i| {
            // Enter and Tab belong to the editor until an item has been picked.
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                c.chosen && i.consume_key(Modifiers::NONE, Key::Enter),
                c.chosen && i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        let last = c.items.len().saturating_sub(1);
        if down {
            // The first press picks the first item rather than skipping it.
            c.selected = if c.chosen {
                (c.selected + 1).min(last)
            } else {
                0
            };
            c.chosen = true;
        }
        if up {
            c.selected = c.selected.saturating_sub(1);
            c.chosen = true;
        }
        if enter || tab {
            *accept = Some(c.selected);
        }
        if esc {
            self.completion = None;
            self.pending_completion = None;
        }
    }

    /// Sets bookmark `n` at the caret's line of the active file, or clears it when it is already there.
    fn toggle_bookmark(&mut self, ctx: &egui::Context, n: usize) {
        let Some(doc) = self.docs.get(self.active) else {
            self.note("No file open: a bookmark needs one");
            return;
        };
        let line = line_col(&doc.text, self.cursor_index(ctx).unwrap_or(0)).0 + 1;
        let path = doc.path.clone();
        if self.bookmarks.toggle(n, &path, line) {
            self.note(format!("Bookmark {n} set on line {line}"));
        } else {
            self.note(format!("Bookmark {n} cleared"));
        }
    }

    /// Edit > Set Bookmark: the next free number, at the caret's line.
    fn set_bookmark_here(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.docs.get(self.active) else {
            self.note("No file open: a bookmark needs one");
            return;
        };
        let line = line_col(&doc.text, self.cursor_index(ctx).unwrap_or(0)).0 + 1;
        let path = doc.path.clone();
        if let Some(n) = self.bookmarks.at(&path, line) {
            self.note(format!("Line {line} already has bookmark {n}"));
        } else if let Some(n) = self.bookmarks.next_free() {
            self.bookmarks.toggle(n, &path, line);
            self.note(format!("Bookmark {n} set on line {line}"));
        } else {
            self.note("All ten bookmarks (0-9) are set: clear one first (Ctrl+Shift+digit)");
        }
    }

    /// Edit > Unset Bookmark: clears the bookmark on the caret's line, if there is one.
    fn unset_bookmark_here(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.docs.get(self.active) else {
            self.note("No file open: a bookmark needs one");
            return;
        };
        let line = line_col(&doc.text, self.cursor_index(ctx).unwrap_or(0)).0 + 1;
        let path = doc.path.clone();
        match self.bookmarks.remove_at(&path, line) {
            Some(n) => self.note(format!("Bookmark {n} removed from line {line}")),
            None => self.note(format!("Line {line} has no bookmark")),
        }
    }

    /// Edit > Go to Previous / Next Bookmark, in file and line order, wrapping around.
    fn step_bookmark(&mut self, ctx: &egui::Context, forward: bool) {
        let Some(doc) = self.docs.get(self.active) else {
            self.note("No file open: a bookmark needs one");
            return;
        };
        let line = line_col(&doc.text, self.cursor_index(ctx).unwrap_or(0)).0 + 1;
        let path = doc.path.clone();
        let Some((n, to, to_line)) = self
            .bookmarks
            .step(&path, line, forward)
            .map(|(n, p, l)| (n, p.to_path_buf(), l))
        else {
            self.note("No bookmarks are set: Set Bookmark (Edit menu) adds one");
            return;
        };
        self.note(format!("Bookmark {n}"));
        self.goto(ctx, &to, to_line, 1);
    }

    /// Jumps to bookmark `n`, opening its file when it is closed.
    fn go_to_bookmark(&mut self, ctx: &egui::Context, n: usize) {
        let Some((path, line)) = self.bookmarks.get(n).map(|(p, l)| (p.to_path_buf(), l)) else {
            self.note(format!("Bookmark {n} is not set"));
            return;
        };
        self.goto(ctx, &path, line, 1);
    }

    /// Opens the diagnostic's file and moves the cursor to its 1-based line/column.
    fn goto(&mut self, ctx: &egui::Context, file: &Path, line: u32, col: u32) {
        let path = if file.is_absolute() {
            file.to_path_buf()
        } else {
            self.root.join(file)
        };
        // A line target shows the code, whatever view was showing and whatever was about to open.
        self.auto_form = None;
        self.open_path(path);
        self.form_view = false;
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let mut offset = 0;
        for (i, l) in doc.text.split_inclusive('\n').enumerate() {
            if i + 1 == line as usize {
                offset += l.chars().take(col.saturating_sub(1) as usize).count();
                break;
            }
            offset += l.chars().count();
        }
        let id = doc.editor_id;
        // One character is the range the editor scrolls to, so the line is in view.
        let end = (offset + 1).min(doc.text.chars().count());
        self.reveal = Some(offset..end);
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(offset))));
        state.store(ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
    }
}

/// What a tool printed. Its output goes to the Output tab; the IDE does not read errors out of it.
struct ToolRun {
    output: String,
}

/// Runs a tool without a shell, so its arguments (a selection, say) stay single arguments.
fn run_tool_plan(plan: &formust_build::tools::Plan) -> ToolRun {
    match Command::new(&plan.command)
        .args(&plan.args)
        .current_dir(&plan.workdir)
        .output()
    {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            let status = out.status.code().map_or_else(
                || "The tool was stopped.".to_string(),
                |code| format!("Exit code {code}."),
            );
            ToolRun {
                output: format!("{text}{status}\n"),
            }
        }
        Err(e) => ToolRun {
            output: format!("Could not run {}: {e}\n", plan.command),
        },
    }
}

/// The project a file belongs to: the folder of its nearest Cargo.toml, else the file's own folder.
fn project_folder(path: &Path) -> PathBuf {
    formust_build::manifest_for(path)
        .ok()
        .and_then(|m| m.parent().map(Path::to_path_buf))
        .or_else(|| path.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// The macOS menu bar. Its key equivalents are handled by AppKit, so they never reach egui.
struct NativeMenu {
    _menu: muda::Menu,
    new_file: muda::MenuId,
    new_form: muda::MenuId,
    new_folder: muda::MenuId,
    rename_item: muda::MenuId,
    preferences: muda::MenuId,
    about: muda::MenuId,
    format: muda::MenuId,
    check: muda::MenuId,
    build: muda::MenuId,
    stop: muda::MenuId,
    cargo_options: muda::MenuId,
    open_folder: muda::MenuId,
    /// File > Open Recent and its items: the menu's list (the entries it shows, by index), the
    /// items' ids and the items themselves (kept alive while they are in the menu).
    open_recent: muda::Submenu,
    recent_entries: Vec<recent::Entry>,
    recent_ids: Vec<(muda::MenuId, usize)>,
    recent_items: Vec<muda::MenuItem>,
    recent_clear: muda::MenuId,
    /// Tools: the submenu, its tools' items (by index in the App's list), Configure Tools... and the items alive.
    tools_menu: muda::Submenu,
    tool_ids: Vec<(muda::MenuId, usize)>,
    /// Bookmarks (Edit menu): the item of each bookmark command, and the items alive (the menu needs them to stay).
    bookmark_ids: Vec<(muda::MenuId, MenuAction)>,
    #[allow(dead_code)]
    bookmark_items: Vec<muda::MenuItem>,
    configure_tools: muda::MenuId,
    tool_items: Vec<muda::MenuItem>,
    toggle_view: muda::MenuId,
    jump_bar: muda::CheckMenuItem,
    assistant: muda::CheckMenuItem,
    open: muda::MenuId,
    save: muda::MenuId,
    run: muda::MenuId,
    /// The Build & Run item, relabelled Run Form while Run runs a form (see `run_is_form`).
    run_item: muda::MenuItem,
    run_form_label: bool,
    quit: muda::MenuId,
    edit: Vec<(muda::MenuItem, EditCmd)>,
    appearance: Vec<(muda::CheckMenuItem, theme::Appearance)>,
    events: Receiver<muda::MenuId>,
}

/// What the unsaved-changes question is about.
#[derive(Clone, PartialEq)]
enum Confirm {
    /// Closing the tab of this file.
    Tab(PathBuf),
    /// Opening another project folder: the tabs of the old one are closed.
    Project(PathBuf),
    /// Quitting Formust.
    Quit,
}

#[derive(Clone, Copy)]
enum MenuAction {
    NewFile,
    /// File > New Rust Form: a sample egui form file, opened in the designer.
    NewForm,
    NewFolder,
    RenameItem,
    ToggleView,
    ToggleJumpBar,
    ToggleAssistant,
    Preferences,
    About,
    /// Edit > Set Bookmark (the next free number at the caret's line), and Previous / Next Bookmark.
    SetBookmark,
    UnsetBookmark,
    PreviousBookmark,
    NextBookmark,
    /// Edit > Format Code.
    Format,
    /// Build > Check, Build, and Stop (cancels the cargo run in flight).
    Check,
    Build,
    Stop,
    /// Build > Cargo Options...
    CargoOptions,
    /// File > Open Folder...
    OpenFolder,
    /// An item of File > Open Recent (its index in the menu's list).
    OpenRecent(usize),
    /// Tools > Configure Tools...
    ConfigureTools,
    /// Tools > a tool, by its index in the App's list.
    Tool(usize),
    ClearRecent,
    Open,
    Save,
    Run,
    Quit,
    Appearance(theme::Appearance),
    Edit(EditCmd),
}

/// Edit-menu commands. AppKit's own Cut/Copy/Paste items would send selectors winit's view does
/// not implement, and their key equivalents would never reach egui, so these are plain items
/// that `edit_events` turns back into the events egui's text fields and the designer expect.
#[derive(Clone, Copy, PartialEq, Debug)]
enum EditCmd {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl EditCmd {
    /// The input events a focused egui widget would have received for this command.
    fn events(self, clipboard: impl FnOnce() -> String) -> Vec<egui::Event> {
        let key = |key, shift| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                mac_cmd: true,
                shift,
                ..Default::default()
            },
        };
        match self {
            Self::Undo => vec![key(egui::Key::Z, false)],
            Self::Redo => vec![key(egui::Key::Z, true)],
            Self::SelectAll => vec![key(egui::Key::A, false)],
            Self::Cut => vec![egui::Event::Cut],
            Self::Copy => vec![egui::Event::Copy],
            Self::Paste => vec![egui::Event::Paste(clipboard())],
        }
    }
}

/// The text on the system clipboard (empty when there is none).
fn clipboard_text() -> String {
    std::process::Command::new("pbpaste")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

impl NativeMenu {
    fn install(ctx: &egui::Context) -> Option<Self> {
        use muda::accelerator::{Accelerator, Code, Modifiers};
        use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
        let cmd = |code| Some(Accelerator::new(Modifiers::META, code));
        let new_file = MenuItem::new("New File", true, cmd(Code::KeyN));
        let new_form = MenuItem::new("New Rust Form", true, None);
        let new_folder = MenuItem::new("New Folder", true, None);
        let rename_item = MenuItem::new("Rename", true, None);
        let toggle_view = MenuItem::new(
            "Switch Between Code and Form",
            true,
            Some(Accelerator::new(
                Modifiers::META | Modifiers::SHIFT,
                Code::KeyF,
            )),
        );
        let jump_bar = CheckMenuItem::new("Show Path Bar", true, false, None);
        let assistant = CheckMenuItem::new(
            "Show Assistant",
            true,
            false,
            Some(Accelerator::new(
                Modifiers::META | Modifiers::ALT,
                Code::KeyA,
            )),
        );
        let open = MenuItem::new("Open...", true, cmd(Code::KeyO));
        let open_folder = MenuItem::new(
            "Open Folder...",
            true,
            Some(Accelerator::new(
                Modifiers::META | Modifiers::SHIFT,
                Code::KeyO,
            )),
        );
        let save = MenuItem::new("Save", true, cmd(Code::KeyS));
        let check = MenuItem::new(
            "Check",
            true,
            Some(Accelerator::new(
                Modifiers::META | Modifiers::SHIFT,
                Code::KeyB,
            )),
        );
        let build = MenuItem::new("Build", true, cmd(Code::KeyB));
        let run = MenuItem::new("Build & Run", true, cmd(Code::KeyR));
        let stop = MenuItem::new("Stop", true, cmd(Code::Period));
        let cargo_options = MenuItem::new("Cargo Options...", true, None);
        let edit: Vec<_> = [
            ("Undo", EditCmd::Undo, Code::KeyZ, Modifiers::META),
            (
                "Redo",
                EditCmd::Redo,
                Code::KeyZ,
                Modifiers::META | Modifiers::SHIFT,
            ),
            ("Cut", EditCmd::Cut, Code::KeyX, Modifiers::META),
            ("Copy", EditCmd::Copy, Code::KeyC, Modifiers::META),
            ("Paste", EditCmd::Paste, Code::KeyV, Modifiers::META),
            (
                "Select All",
                EditCmd::SelectAll,
                Code::KeyA,
                Modifiers::META,
            ),
        ]
        .into_iter()
        .map(|(label, c, code, m)| {
            (
                MenuItem::new(label, true, Some(Accelerator::new(m, code))),
                c,
            )
        })
        .collect();
        let edit_menu = Submenu::new("Edit", true);
        for (i, (item, _)) in edit.iter().enumerate() {
            if i == 2 || i == 5 {
                edit_menu.append(&PredefinedMenuItem::separator()).ok()?;
            }
            edit_menu.append(item).ok()?;
        }
        // Under the Edit commands; Ctrl+Cmd+I (Re-Indent in Xcode) is free of the other shortcuts.
        let format = MenuItem::new(
            "Format Code",
            true,
            Some(Accelerator::new(
                Modifiers::META | Modifiers::CONTROL,
                Code::KeyI,
            )),
        );
        edit_menu.append(&PredefinedMenuItem::separator()).ok()?;
        edit_menu.append(&format).ok()?;
        // Bookmarks, all on Cmd+Ctrl (the editor's Ctrl+digit and Ctrl+Shift+digit keys are separate).
        let bookmark_key =
            |code| Some(Accelerator::new(Modifiers::META | Modifiers::CONTROL, code));
        let set_bookmark = MenuItem::new("Set Bookmark", true, bookmark_key(Code::KeyB));
        let unset_bookmark = MenuItem::new("Unset Bookmark", true, bookmark_key(Code::KeyU));
        let prev_bookmark = MenuItem::new(
            "Go to Previous Bookmark",
            true,
            bookmark_key(Code::BracketLeft),
        );
        let next_bookmark = MenuItem::new(
            "Go to Next Bookmark",
            true,
            bookmark_key(Code::BracketRight),
        );
        edit_menu.append(&PredefinedMenuItem::separator()).ok()?;
        edit_menu.append(&set_bookmark).ok()?;
        edit_menu.append(&unset_bookmark).ok()?;
        edit_menu.append(&prev_bookmark).ok()?;
        edit_menu.append(&next_bookmark).ok()?;
        let bookmark_ids = vec![
            (set_bookmark.id().clone(), MenuAction::SetBookmark),
            (unset_bookmark.id().clone(), MenuAction::UnsetBookmark),
            (prev_bookmark.id().clone(), MenuAction::PreviousBookmark),
            (next_bookmark.id().clone(), MenuAction::NextBookmark),
        ];
        let bookmark_items = vec![set_bookmark, unset_bookmark, prev_bookmark, next_bookmark];
        let menu = Menu::new();
        // Not the system's Quit: that ends the process without asking about unsaved changes.
        let quit = MenuItem::new("Quit Formust", true, cmd(Code::KeyQ));
        let preferences = MenuItem::new("Preferences...", true, cmd(Code::Comma));
        // The About window is the app's own (egui), so it follows the app's light or dark appearance.
        let about = MenuItem::new("About Formust", true, None);
        let app = Submenu::with_items(
            "Formust",
            true,
            &[
                &about,
                &PredefinedMenuItem::separator(),
                &preferences,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::separator(),
                &quit,
            ],
        )
        .ok()?;
        let open_recent = Submenu::new("Open Recent", true);
        let recent_clear = MenuItem::new("Clear Menu", true, None);
        let file = Submenu::with_items(
            "File",
            true,
            &[
                &new_file,
                &new_form,
                &new_folder,
                &rename_item,
                &PredefinedMenuItem::separator(),
                &open_recent,
                &open,
                &open_folder,
                &save,
            ],
        )
        .ok()?;
        let build_menu = Submenu::with_items(
            "Build",
            true,
            &[
                &check,
                &build,
                &run,
                &PredefinedMenuItem::separator(),
                &stop,
                &PredefinedMenuItem::separator(),
                &cargo_options,
            ],
        )
        .ok()?;
        let configure_tools = MenuItem::new("Configure Tools...", true, None);
        let tools_menu = Submenu::with_items("Tools", true, &[&configure_tools]).ok()?;
        let current = theme::Appearance::load();
        let appearance: Vec<_> = theme::Appearance::ALL
            .into_iter()
            .map(|a| (CheckMenuItem::new(a.label(), true, a == current, None), a))
            .collect();
        let appearance_menu = Submenu::new("Appearance", true);
        for (item, _) in &appearance {
            appearance_menu.append(item).ok()?;
        }
        let view = Submenu::with_items(
            "View",
            true,
            &[&toggle_view, &jump_bar, &assistant, &appearance_menu],
        )
        .ok()?;
        menu.append_items(&[&app, &file, &edit_menu, &view, &tools_menu, &build_menu])
            .ok()?;
        menu.init_for_nsapp();

        let (tx, events) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            let _ = tx.send(e.id);
            ctx.request_repaint();
        }));
        Some(Self {
            _menu: menu,
            new_file: new_file.id().clone(),
            new_form: new_form.id().clone(),
            new_folder: new_folder.id().clone(),
            rename_item: rename_item.id().clone(),
            preferences: preferences.id().clone(),
            about: about.id().clone(),
            format: format.id().clone(),
            check: check.id().clone(),
            build: build.id().clone(),
            stop: stop.id().clone(),
            cargo_options: cargo_options.id().clone(),
            open_folder: open_folder.id().clone(),
            open_recent,
            recent_entries: Vec::new(),
            recent_ids: Vec::new(),
            recent_items: Vec::new(),
            tools_menu,
            tool_ids: Vec::new(),
            bookmark_ids,
            bookmark_items,
            configure_tools: configure_tools.id().clone(),
            tool_items: vec![configure_tools],
            recent_clear: recent_clear.id().clone(),
            toggle_view: toggle_view.id().clone(),
            jump_bar,
            assistant,
            open: open.id().clone(),
            save: save.id().clone(),
            run: run.id().clone(),
            run_item: run.clone(),
            run_form_label: false,
            quit: quit.id().clone(),
            edit,
            appearance,
            events,
        })
    }

    /// The Build & Run item says Run Form while Run runs a form (see `run_is_form`).
    fn show_run_label(&mut self, form: bool) {
        if self.run_form_label != form {
            self.run_form_label = form;
            self.run_item
                .set_text(if form { "Run Form" } else { "Build & Run" });
        }
    }

    /// Ticks the chosen appearance, and only that one.
    fn show_appearance(&self, chosen: theme::Appearance) {
        for (item, a) in &self.appearance {
            item.set_checked(*a == chosen);
        }
    }

    /// Rebuilds File > Open Recent for `entries`: the projects, a separator, the files, and Clear
    /// Menu. Nothing recent shows a disabled note.
    fn set_recent(&mut self, entries: Vec<recent::Entry>) {
        while self.open_recent.remove_at(0).is_some() {}
        self.recent_ids.clear();
        self.recent_items.clear();
        if entries.is_empty() {
            let none = muda::MenuItem::new("No Recent Items", false, None);
            let _ = self.open_recent.append(&none);
            self.recent_items.push(none);
            return;
        }
        let title = |e: &recent::Entry| {
            let name = e
                .path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            if e.project {
                format!("{name} (project)")
            } else {
                let folder = e
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                format!("{name} — {folder}")
            }
        };
        let mut previous_project = None;
        for (i, e) in entries.iter().enumerate() {
            if previous_project == Some(true) && !e.project {
                let sep = muda::PredefinedMenuItem::separator();
                let _ = self.open_recent.append(&sep);
            }
            previous_project = Some(e.project);
            let item = muda::MenuItem::new(title(e), true, None);
            let _ = self.open_recent.append(&item);
            self.recent_ids.push((item.id().clone(), i));
            self.recent_items.push(item);
        }
        let sep = muda::PredefinedMenuItem::separator();
        let _ = self.open_recent.append(&sep);
        let clear = muda::MenuItem::new("Clear Menu", true, None);
        let _ = self.open_recent.append(&clear);
        self.recent_clear = clear.id().clone();
        self.recent_items.push(clear);
        self.recent_entries = entries;
    }

    /// Rebuilds the Tools menu: one item per tool, then Configure Tools....
    fn set_tools(&mut self, tools: &[formust_build::tools::Tool]) {
        while self.tools_menu.remove_at(0).is_some() {}
        self.tool_ids.clear();
        self.tool_items.clear();
        for (i, tool) in tools.iter().enumerate() {
            let item = muda::MenuItem::new(&tool.name, true, None);
            let _ = self.tools_menu.append(&item);
            self.tool_ids.push((item.id().clone(), i));
            self.tool_items.push(item);
        }
        if !tools.is_empty() {
            let _ = self
                .tools_menu
                .append(&muda::PredefinedMenuItem::separator());
        }
        let configure = muda::MenuItem::new("Configure Tools...", true, None);
        let _ = self.tools_menu.append(&configure);
        self.configure_tools = configure.id().clone();
        self.tool_items.push(configure);
    }

    fn poll(&self) -> Vec<MenuAction> {
        self.events
            .try_iter()
            .filter_map(|id| {
                [
                    (&self.new_file, MenuAction::NewFile),
                    (&self.new_form, MenuAction::NewForm),
                    (&self.new_folder, MenuAction::NewFolder),
                    (&self.rename_item, MenuAction::RenameItem),
                    (&self.preferences, MenuAction::Preferences),
                    (&self.about, MenuAction::About),
                    (&self.toggle_view, MenuAction::ToggleView),
                    (&self.open, MenuAction::Open),
                    (&self.open_folder, MenuAction::OpenFolder),
                    (&self.save, MenuAction::Save),
                    (&self.check, MenuAction::Check),
                    (&self.build, MenuAction::Build),
                    (&self.run, MenuAction::Run),
                    (&self.stop, MenuAction::Stop),
                    (&self.cargo_options, MenuAction::CargoOptions),
                    (&self.format, MenuAction::Format),
                    (&self.recent_clear, MenuAction::ClearRecent),
                    (&self.configure_tools, MenuAction::ConfigureTools),
                    (&self.quit, MenuAction::Quit),
                ]
                .into_iter()
                .find(|(i, _)| **i == id)
                .map(|(_, a)| a)
                .or_else(|| (*self.jump_bar.id() == id).then_some(MenuAction::ToggleJumpBar))
                .or_else(|| {
                    self.recent_ids
                        .iter()
                        .find(|(item, _)| *item == id)
                        .map(|(_, i)| MenuAction::OpenRecent(*i))
                })
                .or_else(|| {
                    self.tool_ids
                        .iter()
                        .find(|(item, _)| *item == id)
                        .map(|(_, i)| MenuAction::Tool(*i))
                })
                .or_else(|| (*self.assistant.id() == id).then_some(MenuAction::ToggleAssistant))
                .or_else(|| {
                    self.bookmark_ids
                        .iter()
                        .find(|(item, _)| *item == id)
                        .map(|(_, a)| *a)
                })
                .or_else(|| {
                    self.edit
                        .iter()
                        .find(|(item, _)| *item.id() == id)
                        .map(|(_, c)| MenuAction::Edit(*c))
                })
                .or_else(|| {
                    self.appearance
                        .iter()
                        .find(|(item, _)| *item.id() == id)
                        .map(|(_, a)| MenuAction::Appearance(*a))
                })
            })
            .collect()
    }
}

fn root_abs(root: &Path) -> Option<PathBuf> {
    root.canonicalize().ok()
}

/// 0-based (line, column) of the char index `idx` in `text`.
fn line_col(text: &str, idx: usize) -> (u32, u32) {
    let before: Vec<char> = text.chars().take(idx).collect();
    let line = before.iter().filter(|c| **c == '\n').count() as u32;
    let col = before.iter().rev().take_while(|c| **c != '\n').count() as u32;
    (line, col)
}

/// Char index where the identifier ending at `idx` begins.
fn word_start(text: &str, idx: usize) -> usize {
    let chars: Vec<char> = text.chars().take(idx).collect();
    let word = chars
        .iter()
        .rev()
        .take_while(|c| c.is_alphanumeric() || **c == '_')
        .count();
    chars.len() - word
}

/// Draws the completion list at `pos`; returns the item the user clicked.
fn completion_popup(ctx: &egui::Context, pos: egui::Pos2, c: &Completion) -> Option<usize> {
    let mut clicked = None;
    egui::Area::new(egui::Id::unique("completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_height(220.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, item) in c.items.iter().enumerate() {
                        let text = if item.detail.is_empty() {
                            item.label.clone()
                        } else {
                            format!("{}  {}", item.label, item.detail)
                        };
                        let r = ui.selectable_label(
                            i == c.selected,
                            egui::RichText::new(text).monospace(),
                        );
                        if i == c.selected {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            });
        });
    clicked
}

fn editor_font() -> egui::FontId {
    let family = theme::editor_family(prefs::font_index());
    egui::FontId::new(prefs::font_size(), family)
}

impl eframe::App for App {
    fn on_exit(&mut self) {
        // Hang up the shell (and its jobs) instead of leaving them behind.
        self.bottom.terminal = None;
        if let Some(file) = &self.session_file {
            self.snapshot().save(file);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.draw(ui, Some(frame));
    }
}

/// A project folder worth indexing: not the file system root and not the home folder.
fn lsp_worthy(root: &Path) -> bool {
    root.parent().is_some() && std::env::var_os("HOME").is_none_or(|h| Path::new(&h) != root)
}

/// Apps started from Finder get a minimal `PATH`; Homebrew's `cargo` and `rust-analyzer` live there.
fn extend_path() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&current).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        if !dirs.iter().any(|d| d == Path::new(extra)) {
            dirs.push(extra.into());
        }
    }
    if let Ok(joined) = std::env::join_paths(dirs) {
        // SAFETY: called first thing in `main`, before any thread is started.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

fn main() -> eframe::Result {
    extend_path();
    // Before any settings are read: copies the old Pascoda folder once (see `migrate`).
    if let Some(support) = migrate::support_folder()
        && let Err(e) = migrate::copy_settings(&support)
    {
        eprintln!("could not copy the settings from the Pascoda folder: {e}");
    }
    let path = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            // navigator 250 + inspector 200 + a usable centre; smaller squeezes the panels over each other
            .with_min_inner_size([860.0, 520.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        "Formust",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            theme::Appearance::restore(&cc.egui_ctx);
            Ok(Box::new(App::new(path, &cc.egui_ctx)))
        }),
    )
}

#[cfg(test)]
mod render_tests;

/// `<target dir>/tmp`, where tests put scratch projects and renders. Found from the test
/// executable (`<target>/debug/deps/x`) because cargo's target dir is configurable.
#[cfg(test)]
pub(crate) fn target_tmp() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let target = exe.ancestors().nth(3).unwrap();
    let dir = target.join("tmp");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_commands_become_the_events_egui_expects() {
        use super::EditCmd::*;
        let ev = |c: super::EditCmd| c.events(|| "clip".into());
        assert!(matches!(ev(Copy)[..], [egui::Event::Copy]));
        assert!(matches!(ev(Cut)[..], [egui::Event::Cut]));
        assert!(matches!(&ev(Paste)[..], [egui::Event::Paste(t)] if t == "clip"));
        assert!(matches!(
            &ev(Redo)[..],
            [egui::Event::Key { key: egui::Key::Z, modifiers, .. }] if modifiers.shift && modifiers.command
        ));
        assert!(matches!(
            &ev(SelectAll)[..],
            [egui::Event::Key {
                key: egui::Key::A,
                ..
            }]
        ));
    }

    use super::*;

    /// A copy of the GUI fixture that a test may write to.
    #[test]
    fn a_new_project_clears_the_output_and_the_problems() {
        let mut app = App::bare(None);
        app.console = "old build output\n".into();
        app.diagnostics = vec![Diagnostic {
            file: PathBuf::from("Cargo.toml"),
            line: 0,
            col: 0,
            severity: Severity::Warning,
            message: "a warning from the last project".into(),
        }];
        app.enter_project(target_tmp().join("another-project"));
        assert!(app.console.is_empty(), "the Output pane is cleared");
        assert!(
            app.diagnostics.is_empty(),
            "the Problems of the old project are cleared"
        );
    }

    #[test]
    fn maps_char_index_to_line_and_word() {
        let text = "begin\n  total := tot";
        assert_eq!(line_col(text, 0), (0, 0));
        assert_eq!(line_col(text, 6), (1, 0));
        assert_eq!(line_col(text, text.chars().count()), (1, 14));
        assert_eq!(word_start(text, text.chars().count()), 17);
        assert_eq!(word_start("a b", 1), 0);
    }

    fn popup(chosen: bool) -> Completion {
        Completion {
            text: String::new(),
            start: 0,
            cursor: 0,
            items: vec![
                formust_lsp::CompletionItem {
                    label: "total".into(),
                    detail: String::new(),
                },
                formust_lsp::CompletionItem {
                    label: "tool".into(),
                    detail: String::new(),
                },
            ],
            selected: 0,
            chosen,
        }
    }

    /// Runs `handle_editor_keys` for one frame in which `key` is pressed.
    fn press(app: &mut App, key: egui::Key) -> Option<usize> {
        let ctx = egui::Context::default();
        let mut accept = None;
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        ctx.run_pass(input, |ctx| app.handle_editor_keys(ctx, &mut accept))
            .drop_without_applying_deltas();
        accept
    }

    #[test]
    fn an_auto_popup_leaves_enter_and_tab_alone_until_an_item_is_picked() {
        let mut app = App::bare(None);
        app.completion = Some(popup(false));
        assert_eq!(press(&mut app, egui::Key::Enter), None);
        assert_eq!(press(&mut app, egui::Key::Tab), None);
        assert!(app.completion.is_some());
        // The first Down picks the first item; Enter then takes it.
        assert_eq!(press(&mut app, egui::Key::ArrowDown), None);
        assert_eq!(app.completion.as_ref().unwrap().selected, 0);
        assert_eq!(press(&mut app, egui::Key::Enter), Some(0));
        assert_eq!(press(&mut app, egui::Key::ArrowDown), None);
        assert_eq!(app.completion.as_ref().unwrap().selected, 1);
        assert_eq!(press(&mut app, egui::Key::Tab), Some(1));
        // Esc dismisses.
        assert_eq!(press(&mut app, egui::Key::Escape), None);
        assert!(app.completion.is_none());
    }

    #[test]
    fn a_ctrl_space_popup_accepts_enter_straight_away() {
        let mut app = App::bare(None);
        app.completion = Some(popup(true));
        assert_eq!(press(&mut app, egui::Key::Enter), Some(0));
    }

    #[test]
    fn closing_a_changed_tab_asks_and_only_saves_when_told_to() {
        let dir = crate::target_tmp().join("close-tab");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("c.rs");
        std::fs::write(&file, "fn c() {}\n").unwrap();
        let ctx = egui::Context::default();
        let open = |text: &str| {
            let mut app = App::bare(Some(file.clone()));
            app.docs[0].text = text.to_string();
            app
        };
        // a clean tab closes at once
        let mut app = open("fn c() {}\n");
        app.request_close(0);
        assert!(app.docs.is_empty() && app.confirm.is_none());
        // a changed tab asks first and stays open
        let mut app = open("fn c() {} // changed\n");
        app.request_close(0);
        assert_eq!(app.docs.len(), 1);
        assert!(matches!(&app.confirm, Some(Confirm::Tab(p)) if *p == app.docs[0].path));
        // Cancel: nothing happens (drive the dialog's answer through the same code path)
        app.confirm = None;
        assert_eq!(app.docs.len(), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn c() {}\n");
        // Don't Save: the tab closes, the file stays as it was
        app.discard_doc(0);
        assert!(app.docs.is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn c() {}\n");
        // Save: the file is written, then the tab closes
        let mut app = open("fn c() {} // changed\n");
        app.save_doc(0);
        app.close_doc(0);
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("// changed")
        );
        let _ = ctx;
    }

    #[test]
    fn quitting_with_unsaved_changes_asks_first() {
        let dir = crate::target_tmp().join("quit-ask");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("q.rs");
        std::fs::write(&file, "fn q() {}\n").unwrap();
        let mut app = App::bare(Some(file));
        assert!(!(0..app.docs.len()).any(|i| app.is_dirty(i)));
        app.docs[0].text.push_str("// typed\n");
        assert!((0..app.docs.len()).any(|i| app.is_dirty(i)));
    }

    /// Cargo's file names are absolute, so a message in an open file is marked there, at its 1-based place.
    #[test]
    fn build_diagnostics_become_marks_in_the_matching_open_file() {
        let dir = crate::target_tmp().join("marks");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("m.rs");
        std::fs::write(&file, "fn main() {\n  nope();\n}\n").unwrap();
        let mut app = App::bare(Some(file.clone()));
        app.build_texts = vec![(app.docs[0].path.clone(), app.docs[0].text.clone())];
        let d = |path: PathBuf, line: u32, severity| Diagnostic {
            file: path,
            line,
            col: 3,
            severity,
            message: "cannot find function `nope`".into(),
        };
        let marks = app.place_marks(&[
            d(file.clone(), 2, Severity::Error),
            d(dir.join("other.rs"), 2, Severity::Error),
            d(file.clone(), 2, Severity::Note),
            // No place in a file (line 0): a project-level problem, never a mark in the text.
            d(dir.join("Cargo.toml"), 0, Severity::Error),
        ]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].items.len(), 1);
        let m = &marks[0].items[0];
        assert_eq!((m.line, m.col), (1, 2));
        assert_eq!(m.range(&marks[0].text), 14..18, "the name `nope`");
    }
}

#[cfg(test)]
mod tree_select_tests {
    use super::*;

    fn pair(name: &str) -> PathBuf {
        let base = target_tmp().join(format!("formust-tree-select-{name}"));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let src = "pub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Name\");\n        ui.text_edit_singleline(&mut self.name);\n    }\n}\n";
        std::fs::write(base.join("login.rs"), src).unwrap();
        let frm = format!(
            "// formust 1 | login.rs | Login::ui | src-hash 00000000\n{}",
            &src[src.find("impl").unwrap()..]
        );
        std::fs::write(base.join("login.frm.rs"), frm).unwrap();
        std::fs::write(base.join("plain.rs"), "fn helper() {}\n").unwrap();
        base.canonicalize().unwrap()
    }

    /// Selecting a form file in the tree shows its form; selecting a source without a form shows its
    /// code, even when the form was showing before. File > Open does the same.
    #[test]
    fn selecting_a_form_file_shows_the_form_and_a_source_shows_its_code() {
        let dir = pair("select");
        let mut app = App::bare(Some(dir.join("plain.rs")));
        app.select_in_tree(dir.join("login.frm.rs"));
        assert!(app.form_showing(), "a .frm.rs shows its form");
        assert_eq!(app.designer.forms().len(), 1);

        app.select_in_tree(dir.join("plain.rs"));
        assert!(
            !app.form_view,
            "a .rs without a form shows its code, not the form"
        );
        assert_eq!(app.docs[app.active].path, dir.join("plain.rs"));

        app.open_chosen(dir.join("login.frm.rs"));
        assert!(app.form_showing(), "a chosen .frm.rs opens in the designer");
        app.open_chosen(dir.join("plain.rs"));
        assert!(!app.form_showing());
    }

    /// The Form | Code toggle: a source with forms offers it and shows them, read with its form file;
    /// a source without forms doesn't.
    #[test]
    fn the_toggle_shows_the_forms_of_a_source_with_its_form_file() {
        let dir = pair("toggle");
        let mut app = App::bare(Some(dir.join("login.rs")));
        app.sync_designer(true);
        assert!(app.has_form(), "a source with a form offers Form");
        app.show_form();
        assert!(app.form_showing());
        let f = app.designer.current().unwrap();
        assert_eq!(f.form().id(), "Login::ui");
        assert_eq!(
            f.origin,
            Some(formust_rust::Origin::Frm),
            "the form file wins for its forms"
        );
        app.show_code();
        assert!(!app.form_showing());

        app.open_path(dir.join("plain.rs"));
        app.sync_designer(true);
        assert!(!app.has_form(), "no forms, no toggle");
        assert!(
            !app.docs.iter().any(|d| d.dirty()),
            "reading for the designer changes nothing"
        );
    }

    /// Typing in the code editor reaches the designer, and a syntax error there doesn't hide the toggle.
    #[test]
    fn edits_in_the_code_reach_the_designer() {
        let dir = pair("edits");
        std::fs::remove_file(dir.join("login.frm.rs")).unwrap();
        let mut app = App::bare(Some(dir.join("login.rs")));
        app.show_form();
        let doc = app.active;
        app.docs[doc].text = app.docs[doc].text.replace("\"Name\"", "\"Your name\"");
        app.sync_designer(true);
        let mut titles = vec![];
        app.designer
            .current()
            .unwrap()
            .form()
            .root
            .walk(&mut |n| titles.push(designer::node_title(n)));
        assert!(
            titles.contains(&"Label \"Your name\"".to_string()),
            "{titles:?}"
        );
        app.docs[doc].text.push_str("impl Broken {");
        app.sync_designer(true);
        assert!(app.designer.loaded.as_ref().unwrap().error.is_some());
        assert!(
            app.has_form(),
            "a syntax error while typing keeps the toggle"
        );
    }

    /// A source with a form that the user opens (the tree, File > Open) shows its form once the designer
    /// has read it; the one-shot is spent then.
    #[test]
    fn opening_a_source_with_a_form_shows_the_designer() {
        let dir = pair("auto");
        let mut app = App::bare(Some(dir.join("plain.rs")));
        app.select_in_tree(dir.join("login.rs"));
        assert!(
            app.form_showing(),
            "a source with a form opens in the designer"
        );
        assert!(app.auto_form.is_none(), "the one-shot is spent");
        assert_eq!(app.designer.current().unwrap().form().id(), "Login::ui");
        app.open_chosen(dir.join("plain.rs"));
        assert!(!app.form_showing(), "another file opens in code");
    }

    /// The app's own About window (a free `fn show(ctx: &egui::Context)` that calls the dialog helper)
    /// opens in the designer like any form; a free `fn(ui: &mut Ui)` stays in code.
    #[test]
    fn opening_the_about_source_shows_its_dialog_in_the_designer() {
        let dir = pair("about");
        std::fs::write(dir.join("about.rs"), include_str!("about.rs")).unwrap();
        std::fs::write(
            dir.join("panel.rs"),
            "pub fn panel(ui: &mut egui::Ui) {\n    ui.label(\"x\");\n}\n",
        )
        .unwrap();
        let mut app = App::bare(Some(dir.join("plain.rs")));
        app.select_in_tree(dir.join("about.rs"));
        assert!(app.form_showing(), "the About source opens in the designer");
        assert!(app.auto_form.is_none(), "the one-shot is spent");
        let form = app.designer.current().unwrap().form();
        assert_eq!(form.id(), "::show");
        assert!(form.context_root);
        let dialog = form.widget_root();
        assert!(matches!(
            form.find(dialog).map(|n| &n.kind),
            Some(formust_model::NodeKind::Container {
                def: formust_model::ContainerId::DialogCall,
                ..
            })
        ));
        app.select_in_tree(dir.join("panel.rs"));
        assert!(
            !app.form_showing(),
            "a free Ui function is not a form by default"
        );
    }

    /// A Rust file without a form, a source that does not parse, and a Cargo.toml open in code.
    #[test]
    fn a_file_without_a_form_opens_in_code() {
        let dir = pair("plain");
        std::fs::write(
            dir.join("broken.rs"),
            "pub struct Broken;\nimpl Broken {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n",
        )
        .unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let mut app = App::bare(Some(dir.join("login.frm.rs")));
        app.select_in_tree(dir.join("plain.rs"));
        assert!(!app.form_view, "no form, no designer");
        app.select_in_tree(dir.join("broken.rs"));
        assert!(!app.form_view, "a syntax error with no forms stays in code");
        app.select_in_tree(dir.join("Cargo.toml"));
        assert!(!app.form_view, "a manifest opens in code");
        assert!(app.auto_form.is_none());
    }

    /// Going to a line (Problems, search, bookmarks) shows the code of its file, even a file with a form.
    #[test]
    fn going_to_a_line_of_a_file_with_a_form_shows_its_code() {
        let dir = pair("goto");
        let mut app = App::bare(Some(dir.join("plain.rs")));
        app.select_in_tree(dir.join("login.rs"));
        assert!(app.form_showing());
        let ctx = egui::Context::default();
        app.goto(&ctx, &dir.join("login.rs"), 2, 1);
        assert!(!app.form_view, "a line target shows the code");
        assert_eq!(app.docs[app.active].path, dir.join("login.rs"));
    }

    /// The tabs restored at startup open in code, a source with a form too.
    #[test]
    fn restored_tabs_stay_in_code() {
        let dir = pair("restore");
        let saved = session::Session {
            root: Some(dir.clone()),
            tabs: vec![dir.join("login.rs"), dir.join("plain.rs")],
            active: Some(dir.join("login.rs")),
            form_view: false,
        };
        let mut app = App::bare(None);
        app.restore(saved, false);
        app.sync_designer(true);
        app.sync_designer(false);
        assert!(!app.form_view, "a restored tab shows its code");
        assert!(!app.form_showing());
    }

    /// A file the user switched to Code stays in code when it is opened again, after its tab was closed too.
    #[test]
    fn a_file_switched_to_code_stays_in_code_when_reopened() {
        let dir = pair("prefer-code");
        let mut app = App::bare(Some(dir.join("plain.rs")));
        app.select_in_tree(dir.join("login.rs"));
        assert!(app.form_showing());
        app.show_code();
        app.select_in_tree(dir.join("plain.rs"));
        app.select_in_tree(dir.join("login.rs"));
        assert!(!app.form_view, "switched to Code once, it opens in code");
        let i = app
            .docs
            .iter()
            .position(|d| d.path == dir.join("login.rs"))
            .unwrap();
        app.remove_doc(i);
        app.select_in_tree(dir.join("login.rs"));
        assert!(
            !app.form_view,
            "reopened after its tab was closed, still code"
        );
        app.show_form();
        assert!(app.form_showing(), "the toggle still shows the form");
    }

    /// A dialog (a method showing a Window with the context) is a form, so its file opens in the designer.
    #[test]
    fn a_dialog_file_opens_in_the_designer() {
        let dir = pair("dialog");
        std::fs::write(
            dir.join("about.rs"),
            "pub struct About { name: String }\nimpl About {\n    pub fn show(&mut self, ctx: &egui::Context) {\n        egui::Window::new(\"About\").show(ctx, |ui| {\n            ui.label(\"Hello dialog\");\n        });\n    }\n}\n",
        )
        .unwrap();
        let mut app = App::bare(Some(dir.join("plain.rs")));
        app.select_in_tree(dir.join("about.rs"));
        assert!(app.form_showing(), "a dialog opens in the designer");
        assert_eq!(app.designer.current().unwrap().form().id(), "About::show");
    }

    /// The one-shot does not outlive a change of tab: a file other than the active one never switches.
    #[test]
    fn a_tab_change_cancels_the_auto_form_view() {
        let dir = pair("cancel");
        let mut app = App::bare(Some(dir.join("plain.rs")));
        app.auto_form = Some(dir.join("login.rs"));
        app.sync_designer(false);
        assert!(app.auto_form.is_none());
        assert!(!app.form_view);
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    /// The session of the open tabs is saved at exit and restored at launch: the project, the tabs in
    /// order, the active tab. Tabs whose files are gone are not reopened.
    #[test]
    fn the_tabs_and_the_project_come_back_after_a_restart() {
        let base =
            std::env::temp_dir().join(format!("formust-session-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(base.join("b.rs"), "fn b() {}\n").unwrap();
        let file = base.join("session");

        let mut before = App::bare(Some(base.join("a.rs")));
        before.open_path(base.join("b.rs"));
        before.open_path(base.join("gone.rs"));
        before
            .docs
            .retain(|d| d.path.file_name() != Some(std::ffi::OsStr::new("gone.rs")));
        before.active = 1;
        before.session_file = Some(file.clone());
        eframe::App::on_exit(&mut before);
        let saved = session::Session::load(&file).expect("saved at exit");
        assert_eq!(saved.tabs.len(), 2);
        assert_eq!(
            saved.active.as_deref(),
            Some(base.join("b.rs").canonicalize().unwrap().as_path())
        );

        let mut after = App::bare(None);
        after.restore(saved, false);
        let names: Vec<_> = after
            .docs
            .iter()
            .map(|d| d.path.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a.rs", "b.rs"], "the tabs in their order");
        assert_eq!(
            after.docs[after.active].path.file_name().unwrap(),
            "b.rs",
            "the active tab"
        );
        assert_eq!(
            after.root,
            base.canonicalize().unwrap(),
            "the project folder"
        );

        // A file given on the command line stays the active tab, and the project is its folder.
        let mut given = App::bare(Some(base.join("a.rs")));
        given.restore(session::Session::load(&file).unwrap(), true);
        assert_eq!(given.docs[given.active].path.file_name().unwrap(), "a.rs");
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod project_switch_tests {
    use super::*;

    /// A Cargo project with a binary and a library, in the scratch area.
    fn project(name: &str) -> PathBuf {
        let dir = target_tmp().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "pub fn answer() -> u32 {\n    42\n}\n",
        )
        .unwrap();
        dir.canonicalize().unwrap()
    }

    /// Opening another project closes the tabs of the old one and opens the file in the new one.
    #[test]
    fn opening_another_project_closes_the_old_tabs() {
        let a = project("switch-clean-a");
        let b = project("switch-clean-b");
        let mut app = App::in_project(a.clone(), Some(a.join("src/lib.rs")));
        app.open_path(a.join("src/main.rs"));
        assert_eq!(app.docs.len(), 2);

        let file = b.join("src/lib.rs");
        app.switch_project(b.clone(), move |app| app.open_path(file));
        assert_eq!(app.root, b);
        assert_eq!(app.docs.len(), 1, "only the new project's tab is open");
        assert!(app.docs[0].path.starts_with(&b));
        assert!(app.confirm.is_none());
    }

    /// A tab in the same folder stays open, so opening a file of the project keeps its tabs.
    #[test]
    fn a_file_of_the_same_project_keeps_the_tabs() {
        let a = project("switch-same-a");
        let mut app = App::in_project(a.clone(), Some(a.join("src/lib.rs")));
        app.switch_project(a.clone(), move |app| app.open_path(a.join("src/main.rs")));
        assert_eq!(app.docs.len(), 2);
    }

    /// Unsaved changes in the old project's tabs: the switch waits for the answer, and Save writes
    /// them before their tabs close.
    #[test]
    fn unsaved_tabs_of_the_old_project_are_asked_about_then_saved() {
        let a = project("switch-dirty-a");
        let b = project("switch-dirty-b");
        let mut app = App::in_project(a.clone(), Some(a.join("src/lib.rs")));
        app.docs[0].text.push_str("\n// edited\n");
        let file = b.join("src/lib.rs");
        app.switch_project(b.clone(), move |app| app.open_path(file));
        assert!(matches!(&app.confirm, Some(Confirm::Project(d)) if *d == b));
        assert_eq!(
            app.root, a,
            "the project does not change until the question is answered"
        );
        assert_eq!(app.docs.len(), 1);

        // The answer "Save": what show_confirm does with it.
        app.confirm = None;
        let (_, then) = app.project_switch.take().unwrap();
        app.close_tabs_outside(&b, true);
        app.enter_project(b.clone());
        then(&mut app);
        assert_eq!(app.root, b);
        assert_eq!(app.docs.len(), 1);
        assert!(app.docs[0].path.starts_with(&b));
        let saved = std::fs::read_to_string(a.join("src/lib.rs")).unwrap();
        assert!(
            saved.ends_with("// edited\n"),
            "the edit was saved to the old project"
        );
    }
}

#[cfg(test)]
mod tool_run_tests {
    use super::*;
    use formust_build::tools::Tool;

    fn tool(name: &str, command: &str, args: &str) -> Tool {
        Tool {
            name: name.into(),
            command: command.into(),
            args: args.into(),
            workdir: String::new(),
        }
    }

    /// Runs tool `index` and waits for its result (the worker thread answers in a moment).
    fn run_and_wait(app: &mut App, ctx: &egui::Context, index: usize) {
        app.run_tool(ctx, index);
        for _ in 0..500 {
            if app.tool_run.is_none() {
                return;
            }
            app.poll_tool();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the tool did not finish");
    }

    fn project(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("formust-tools-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The app keeps canonical paths (macOS temp folders are behind a symlink).
        let dir = dir.canonicalize().unwrap();
        let unit = dir.join("main.rs");
        std::fs::write(&unit, "fn main() {}\n").unwrap();
        (dir, unit)
    }

    #[test]
    fn a_tool_runs_with_the_active_file_and_its_output_goes_to_the_console() {
        let (dir, unit) = project("echo");
        let mut app = App::bare(Some(unit.clone()));
        app.tools = vec![tool("Echo", "/bin/echo", "hello $(File)")];
        let ctx = egui::Context::default();
        run_and_wait(&mut app, &ctx, 0);
        assert!(
            app.console.contains(&format!("hello {}", unit.display())),
            "{}",
            app.console
        );
        assert!(app.console.contains("Exit code 0."), "{}", app.console);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tool_error_stays_in_the_output_and_is_not_read_as_a_problem() {
        let (dir, unit) = project("errors");
        let mut app = App::bare(Some(unit));
        app.tools = vec![tool(
            "Check",
            "/bin/sh",
            "-c 'echo \"main.rs:3:5: error: boom\" >&2; exit 1'",
        )];
        let ctx = egui::Context::default();
        run_and_wait(&mut app, &ctx, 0);
        assert!(
            app.console.contains("main.rs:3:5: error: boom"),
            "{}",
            app.console
        );
        assert!(app.console.contains("Exit code 1."), "{}", app.console);
        assert!(app.diagnostics.is_empty(), "tools do not fill Problems");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_command_says_so_and_an_unknown_macro_is_refused_without_running() {
        let (dir, unit) = project("missing");
        let mut app = App::bare(Some(unit));
        let ctx = egui::Context::default();
        app.tools = vec![tool("Nope", "/no/such/tool/here", "")];
        run_and_wait(&mut app, &ctx, 0);
        assert!(
            app.console.contains("Could not run /no/such/tool/here"),
            "{}",
            app.console
        );

        app.tools = vec![tool("Bad", "/bin/echo", "$(Nonsense)")];
        app.run_tool(&ctx, 0);
        assert!(
            app.tool_run.is_none(),
            "an unknown macro does not start the tool"
        );
        assert!(app.console.contains("$(Nonsense)"), "{}", app.console);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
