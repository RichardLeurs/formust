//! The window layout, modelled on Xcode: a toolbar of grouped buttons on top, the navigator on
//! the left, the editor in the middle with a tab strip and a jump bar, the bottom pane and a
//! status bar below.

use super::*;
use crate::bottom::BottomTab;
use crate::chrome;
use crate::editor_lines::LineMap;
use crate::style::{self, Icon, Palette, hline, icon_button, pill};
use crate::terminal::Terminal;
use crate::terminal_view;
use egui::{Align, Frame, Layout, Margin, Rect, RichText, Sense, Vec2, vec2};
use std::collections::HashSet;

/// Xcode spaces code lines out to about 1.5 times the font size.
const GUTTER_W: f32 = 46.0;
const TOOLBAR_H: f32 = 44.0;
const STATUS_H: f32 = 22.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum NavTab {
    #[default]
    Files,
    Outline,
    /// Project-wide find and replace.
    Search,
}

/// The declarations of the active file, as the language server last read them. `shown` is the file
/// and the text (by key) they were read from; `pending` is the request on its way.
#[derive(Default)]
pub(crate) struct Outline {
    pub shown: Option<(PathBuf, u64)>,
    pub pending: Option<(PathBuf, u64)>,
    pub symbols: Vec<formust_lsp::Symbol>,
    /// When the last request was sent (egui time), to wait for the typing to stop.
    pub asked_at: f64,
}

/// The signature of the call at the caret, as the server gave it for the text with `key`.
pub(crate) struct SigHint {
    pub key: u64,
    pub at: usize,
    pub sig: formust_lsp::Signature,
}

/// How long the text must stay unchanged before the outline asks the server again.
const OUTLINE_QUIET: f64 = 0.6;

/// A key for a text: two equal texts have equal keys.
pub(crate) fn text_key(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// What a declaration is, from its LSP symbol kind.
pub(crate) fn kind_word(kind: u32) -> &'static str {
    match kind {
        2 => "unit",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        22 => "member",
        23 => "record",
        _ => "symbol",
    }
}

/// Changes asked for while drawing, applied once drawing is done (drawing borrows the app).
#[derive(Default)]
struct Pending {
    goto: Option<(PathBuf, u32, u32)>,
    open: Option<PathBuf>,
    close: Option<usize>,
    close_completion: bool,
    complete_clicked: bool,
    accept: Option<usize>,
    /// Typing paused: offer completion if the text before the caret asks for it.
    auto_complete: bool,
    /// File, text and 0-based position to ask the server about.
    hover_request: Option<(PathBuf, String, u32, u32)>,
    /// Cmd-click on an identifier: char index to look up the definition of.
    definition_at: Option<usize>,
    /// A `(` or `,` was typed: file, text, 0-based line and column, caret char index, and the text's key.
    signature_request: Option<(PathBuf, String, u32, u32, usize, u64)>,
    /// The user's answer to an external-change banner.
    external: Option<(external::Key, external::Resolution)>,
}

impl App {
    pub(crate) fn draw(&mut self, ui: &mut egui::Ui, frame: Option<&eframe::Frame>) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let pal = Palette::of(ctx);
        self.poll_build(ctx);
        self.poll_form_prep(ctx);
        self.poll_tool();
        self.poll_lsp(ctx);
        self.handle_close_request(ctx);
        self.show_confirm(ctx);
        self.show_go_to_line(ctx);
        self.show_rename(ctx);
        self.show_preferences(ctx);
        if self.about_open && crate::about::show(ctx) {
            self.about_open = false;
        }
        self.show_tools_editor(ctx);
        self.show_cargo_window(ctx);
        self.poll_project_search(ctx);
        self.poll_assistant(ctx);
        self.poll_external(ctx);
        self.sync_designer(false);
        self.run_menu_actions(ctx);
        if frame.is_some() {
            chrome::align_window_buttons(TOOLBAR_H.into());
        }
        let mut p = Pending::default();
        self.handle_editor_keys(ctx, &mut p.accept);
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::S,
            ))
        }) {
            self.save_doc(self.active);
        }

        self.toolbar(ui, ctx, &pal);
        self.status_bar(ui, &pal);
        // The assistant is laid out before the bottom pane so it runs the full height, from the
        // toolbar to the status bar, and the bottom pane only spans the space left of it.
        if self.assistant.open {
            self.assistant_panel(ui, &pal);
        }
        if self.show_bottom {
            self.bottom_area(ui, &pal, &mut p);
        }
        if self.show_navigator {
            self.navigator(ui, &pal, &mut p);
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.editor))
            .show(ui, |ui| self.center(ui, &pal, &mut p));

        if p.complete_clicked {
            self.request_completion(ctx, false);
        }
        if p.auto_complete {
            self.request_completion(ctx, true);
        }
        if let Some((path, text, line, col, at, key)) = p.signature_request {
            self.request_signature(path, text, line, col, at, key);
        }
        if let Some((path, text, line, col)) = p.hover_request {
            self.request_hover(path, text, line, col);
        }
        if let Some(index) = p.definition_at {
            self.request_definition_at(index);
        }
        if p.close_completion {
            self.completion = None;
        }
        if let Some(i) = p.accept {
            self.accept_completion(ctx, i);
        }
        if let Some((key, what)) = p.external.take() {
            self.resolve_external(Some(ctx), key, what);
        }
        if let Some(path) = p.open {
            self.remember(|r| r.note_file(&path));
            self.select_in_tree(path);
            // The switch to the form happens after this frame's panels were drawn; without a
            // repaint the window keeps showing the old tab until the next input event.
            ctx.request_repaint();
        }
        if let Some(i) = p.close {
            self.request_close(i);
        }
        if let Some((file, line, col)) = p.goto {
            self.goto(ctx, &file, line, col);
        }
    }

    fn run_menu_actions(&mut self, ctx: &egui::Context) {
        crate::system_menus::hide_edit_items();
        let form = self.run_is_form();
        if let Some(menu) = self.menu.as_mut() {
            menu.show_run_label(form);
        }
        let actions = self.menu.as_ref().map(NativeMenu::poll).unwrap_or_default();
        self.run_menu_list(ctx, actions);
    }

    /// Runs the menu actions the user picked (or a test's).
    pub(crate) fn run_menu_list(&mut self, ctx: &egui::Context, actions: Vec<MenuAction>) {
        for action in actions {
            match action {
                MenuAction::Preferences => self.preferences_open = true,
                MenuAction::About => self.about_open = true,
                MenuAction::SetBookmark => self.set_bookmark_here(ctx),
                MenuAction::UnsetBookmark => self.unset_bookmark_here(ctx),
                MenuAction::PreviousBookmark => self.step_bookmark(ctx, false),
                MenuAction::NextBookmark => self.step_bookmark(ctx, true),
                MenuAction::OpenRecent(i) => self.open_recent(i),
                MenuAction::ConfigureTools => {
                    self.tools_editor = Some(crate::tools_editor::Editor::new(self.tools.clone()))
                }
                MenuAction::Tool(i) => self.run_tool(ctx, i),
                MenuAction::ClearRecent => self.remember(|r| r.clear()),
                MenuAction::NewFile => self.new_file_in_tree(),
                MenuAction::NewForm => self.new_rust_form(None),
                MenuAction::NewFolder => {
                    // The same as the file tree's buttons: the new name is typed in the tree.
                    self.nav_tab = NavTab::Files;
                    let dir = self.tree_target_dir();
                    self.apply_tree_actions(
                        vec![TreeAction::NewIn { dir, is_dir: true }],
                        &mut Pending::default(),
                    );
                }
                MenuAction::RenameItem => {
                    self.nav_tab = NavTab::Files;
                    match self.tree_sel.clone().filter(|s| *s != self.root) {
                        Some(path) => {
                            self.apply_tree_actions(
                                vec![TreeAction::AskRename(path)],
                                &mut Pending::default(),
                            );
                        }
                        None => self
                            .console
                            .push_str("Select a file or folder in the file tree first\n"),
                    }
                }
                MenuAction::ToggleView => {
                    if self.form_view {
                        self.show_code();
                    } else if self.has_form() {
                        self.show_form();
                    }
                }
                MenuAction::Appearance(a) => {
                    a.set(ctx);
                    if let Some(m) = &self.menu {
                        m.show_appearance(a);
                    }
                }
                MenuAction::Edit(cmd) => {
                    let events = cmd.events(crate::clipboard_text);
                    ctx.input_mut(|i| i.events.extend(events));
                }
                MenuAction::ToggleJumpBar => {
                    self.show_jump_bar = !self.show_jump_bar;
                    if let Some(m) = &self.menu {
                        m.jump_bar.set_checked(self.show_jump_bar);
                    }
                }
                MenuAction::ToggleAssistant => self.toggle_assistant(),
                MenuAction::Open => self.pick_file(),
                MenuAction::Save => self.save_active(),
                MenuAction::Format => self.format_active(true),
                MenuAction::Check => self.start_cargo(ctx, Action::Check),
                MenuAction::Build => self.start_cargo(ctx, Action::Build),
                MenuAction::Run => self.run_active(ctx),
                MenuAction::Stop => self.stop_cargo(),
                MenuAction::CargoOptions => self.open_cargo_window(ctx),
                MenuAction::OpenFolder => self.pick_folder(),
                MenuAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }

    // ---- toolbar ----------------------------------------------------------------------

    /// A build (or a Run Form being prepared) is running: the toolbar shows its progress.
    pub(crate) fn working(&self) -> bool {
        self.building.is_some() || self.form_prep.is_some()
    }

    /// One line saying what the IDE is doing, and its colour.
    pub(crate) fn activity(&self, pal: &Palette) -> (String, egui::Color32) {
        if self.working() {
            let label = match (&self.form_prep, &self.build_progress) {
                (Some(_), _) => "Preparing Run Form...",
                (None, Some(p)) => p.label.as_str(),
                (None, None) => "Building...",
            };
            (label.into(), pal.accent)
        } else if let Some(n) = Some(self.errors()).filter(|n| *n > 0) {
            (
                format!("Build failed: {n} error{}", if n == 1 { "" } else { "s" }),
                pal.error,
            )
        } else {
            ("Ready".into(), pal.ok)
        }
    }

    fn errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| matches!(d.severity, Severity::Fatal | Severity::Error))
            .count()
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pal: &Palette) {
        let can_build = !self.working();
        let has_form = self.has_form();
        let form_showing = self.form_showing();
        let run_form = self.run_is_form();
        let (status, status_color) = self.activity(pal);
        let project = self.root.file_name().map_or_else(
            || "Formust".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        egui::Panel::top("toolbar")
            .exact_size(TOOLBAR_H)
            .show_separator_line(false)
            .frame(Frame::new().fill(pal.chrome))
            .show(ui, |ui| {
                let bar = ui.max_rect();
                // The title bar is hidden, so empty space here drags the window. Registered
                // first, so the buttons added after it keep their clicks.
                let drag = ui.interact(bar, egui::Id::unique("titlebar"), Sense::drag());
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.horizontal_centered(|ui| {
                    ui.add_space(78.0); // room for the traffic-light buttons
                    let hover = |r: egui::Response, tip: &str| r.on_hover_text(tip);
                    ui.add_space(8.0);
                    pill(ui, pal, |ui| {
                        if hover(
                            icon_button(ui, pal, Icon::Plus, true, false),
                            "New File (Cmd+N)",
                        )
                        .clicked()
                        {
                            self.new_file_in_tree();
                        }
                        if hover(
                            text_button(ui, pal, "Form"),
                            "New Rust Form: a sample egui form, opened in the designer",
                        )
                        .clicked()
                        {
                            self.new_rust_form(None);
                        }
                        if hover(
                            icon_button(ui, pal, Icon::Folder, true, false),
                            "Open... (Cmd+O). Shift+Cmd+O opens a folder.",
                        )
                        .clicked()
                        {
                            self.pick_file();
                        }
                        if hover(
                            icon_button(ui, pal, Icon::Save, !self.docs.is_empty(), false),
                            "Save (Cmd+S)",
                        )
                        .clicked()
                        {
                            self.save_doc(self.active);
                        }
                    });
                    ui.add_space(8.0);
                    pill(ui, pal, |ui| {
                        if self.working() {
                            if hover(
                                icon_button(ui, pal, Icon::Stop, true, false),
                                "Stop (Cmd+.)",
                            )
                            .clicked()
                            {
                                self.stop_cargo();
                            }
                        } else {
                            if hover(text_button(ui, pal, "Check"), "Check (Cmd+Shift+B)").clicked()
                            {
                                self.start_cargo(ctx, Action::Check);
                            }
                            if hover(text_button(ui, pal, "Build"), "Build (Cmd+B)").clicked() {
                                self.start_cargo(ctx, Action::Build);
                            }
                            let (run_label, run_tip) = if run_form {
                                (
                                    "Run Form",
                                    "Run Form (Cmd+R): runs a form of this file in a window",
                                )
                            } else {
                                ("Run", "Build and Run (Cmd+R)")
                            };
                            if hover(style::run_button(ui, pal, run_label, can_build), run_tip)
                                .clicked()
                                && can_build
                            {
                                self.run_active(ctx);
                            }
                        }
                    });

                    // The activity viewer: project on the left, what is happening on the right.
                    let width = 380.0_f32;
                    let gap = (bar.center().x - width / 2.0 - ui.cursor().min.x).max(14.0);
                    ui.add_space(gap);
                    let (rect, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
                    let fill = if pal.dark {
                        egui::Color32::from_gray(0x2C)
                    } else {
                        egui::Color32::from_rgb(0xF1, 0xF1, 0xF3)
                    };
                    let painter = ui.painter();
                    painter.rect(
                        rect,
                        8.0,
                        fill,
                        egui::Stroke::new(1.0, pal.separator),
                        egui::StrokeKind::Inside,
                    );
                    // The project name, then a chevron. The name is drawn twice, a fraction of a point apart: the
                    // theme ships one weight of SF Pro, so this is how it reads bold.
                    let name_font = egui::FontId::proportional(13.0);
                    let name_pos = rect.left_center() + vec2(12.0, 0.0);
                    let name_width = painter
                        .layout_no_wrap(project.clone(), name_font.clone(), pal.text)
                        .size()
                        .x;
                    painter.text(
                        name_pos,
                        egui::Align2::LEFT_CENTER,
                        &project,
                        name_font.clone(),
                        pal.text,
                    );
                    painter.text(
                        name_pos + vec2(0.35, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &project,
                        name_font,
                        pal.text,
                    );
                    let chevron = Rect::from_center_size(
                        egui::pos2(name_pos.x + name_width + 10.0, rect.center().y),
                        vec2(10.0, 10.0),
                    );
                    crate::app_icons::paint(painter, chevron, "chevron-right", pal.text_tertiary);
                    if self.working() {
                        // A thin bar along the bottom of the pill; it slides when the amount of
                        // work is not known.
                        let track = Rect::from_min_max(
                            egui::pos2(rect.left() + 14.0, rect.bottom() - 5.0),
                            egui::pos2(rect.right() - 14.0, rect.bottom() - 2.5),
                        );
                        painter.rect_filled(track, 1.5, pal.separator);
                        let (a, b) = match self.build_progress.as_ref().and_then(|p| p.fraction) {
                            Some(f) => (0.0, f.clamp(0.02, 1.0)),
                            None => {
                                let t = ui.input(|i| i.time) as f32;
                                let x = (t * 0.8).fract();
                                (x * 1.3 - 0.3, x * 1.3)
                            }
                        };
                        let (a, b) = (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
                        let fill_bar = Rect::from_min_max(
                            egui::pos2(track.left() + track.width() * a, track.top()),
                            egui::pos2(track.left() + track.width() * b, track.bottom()),
                        );
                        painter.rect_filled(fill_bar, 1.5, pal.accent);
                        ui.ctx().request_repaint();
                    }
                    // The status, as a badge at the right: tinted fill, outlined in its colour.
                    let status_font = egui::FontId::proportional(12.0);
                    let status_width = painter
                        .layout_no_wrap(status.clone(), status_font.clone(), status_color)
                        .size()
                        .x;
                    let badge = Rect::from_min_size(
                        egui::pos2(
                            rect.right() - 10.0 - status_width - 16.0,
                            rect.center().y - 9.0,
                        ),
                        vec2(status_width + 16.0, 18.0),
                    );
                    painter.rect(
                        badge,
                        5.0,
                        status_color.gamma_multiply(0.15),
                        egui::Stroke::new(1.0, status_color.gamma_multiply(0.5)),
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        badge.center(),
                        egui::Align2::CENTER_CENTER,
                        &status,
                        status_font,
                        status_color,
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        if hover(
                            icon_button(ui, pal, Icon::Gear, true, self.preferences_open),
                            "Preferences (Cmd+,)",
                        )
                        .clicked()
                        {
                            self.preferences_open = true;
                        }
                        ui.add_space(4.0);
                        // The navigator, bottom pane and assistant toggles: one group. The layout runs right to
                        // left, so the buttons are added assistant first, navigator last (it ends up on the left).
                        pill(ui, pal, |ui| {
                            if hover(
                                style::toggle_button(ui, pal, Icon::Sparkle, self.assistant.open),
                                "Show or hide the assistant (Cmd+Opt+A)",
                            )
                            .clicked()
                            {
                                self.toggle_assistant();
                            }
                            if hover(
                                style::toggle_button(ui, pal, Icon::PanelBottom, self.show_bottom),
                                "Show or hide the bottom pane",
                            )
                            .clicked()
                            {
                                self.show_bottom = !self.show_bottom;
                            }
                            if hover(
                                style::toggle_button(ui, pal, Icon::Sidebar, self.show_navigator),
                                "Show or hide the navigator",
                            )
                            .clicked()
                            {
                                self.show_navigator = !self.show_navigator;
                            }
                        });
                        ui.add_space(4.0);
                        if has_form {
                            pill(ui, pal, |ui| {
                                // Added right to left: Form first, so Code ends up on its left.
                                if style::segment(ui, pal, "Form", form_showing).clicked() {
                                    self.show_form();
                                }
                                if style::segment(ui, pal, "Code", !form_showing).clicked() {
                                    self.show_code();
                                }
                            });
                        }
                    });
                });
            });
    }

    // ---- status bar -------------------------------------------------------------------

    fn status_bar(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let (dot, text) = match (&self.lsp, self.lsp_ready) {
            (None, _) => (pal.text_tertiary, "Language server off"),
            (Some(_), false) => (pal.warning, "Language server starting..."),
            (Some(_), true) => (pal.ok, "Language server ready"),
        };
        egui::Panel::bottom("status")
            .exact_size(STATUS_H)
            .show_separator_line(true)
            .frame(Frame::new().fill(pal.chrome).inner_margin(Margin::ZERO))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, dot);
                    ui.label(RichText::new(text).size(11.5).color(pal.text_secondary));
                    if let Some((note, opacity)) = self.notice() {
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new(note)
                                .size(11.5)
                                .color(pal.text.gamma_multiply(opacity)),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (line, col) = self.cursor_pos;
                        ui.label(
                            RichText::new(format!("Line: {line}  Col: {col}"))
                                .size(11.5)
                                .color(pal.text_secondary),
                        );
                    });
                });
            });
    }

    // ---- navigator --------------------------------------------------------------------

    fn navigator(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) {
        let mut actions = Vec::new();
        if self.nav_tab == NavTab::Outline {
            self.refresh_outline(ui.ctx());
        }
        egui::Panel::left("navigator")
            .resizable(true)
            .default_size(250.0)
            .min_size(170.0)
            .frame(Frame::new().fill(pal.sidebar))
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (tab, icon, tip) in [
                        (NavTab::Files, Icon::Files, "Project"),
                        (NavTab::Outline, Icon::Outline, "Outline"),
                        (NavTab::Search, Icon::Search, "Search the project"),
                    ] {
                        if icon_button(ui, pal, icon, true, self.nav_tab == tab)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            self.nav_tab = tab;
                        }
                    }
                });
                if self.nav_tab == NavTab::Files {
                    // The filter field sits at the top, with a search icon inside it, as in Xcode's navigator. Its
                    // box is 10 pt in from each side of the navigator, and its text is centred in the box.
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        Frame::new()
                            .stroke(egui::Stroke::new(1.0, pal.separator))
                            .corner_radius(6)
                            .inner_margin(Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 17.0);
                                ui.horizontal_centered(|ui| {
                                    let (r, _) =
                                        ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                                    style::paint_icon(
                                        ui.painter(),
                                        r,
                                        Icon::Filter,
                                        pal.text_tertiary,
                                    );
                                    ui.add_space(4.0);
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.file_filter)
                                            .hint_text("Filter files...")
                                            .frame(Frame::NONE)
                                            .margin(Margin::symmetric(0, 0))
                                            .desired_width(f32::INFINITY),
                                    );
                                });
                            });
                    });
                    ui.add_space(4.0);
                    // The file actions get a row of their own: the tabs and the actions together are
                    // wider than the sidebar. Right to left: the first button added is the rightmost.
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let sel = self.tree_sel.clone();
                            let deletable = sel.clone().filter(|s| *s != self.root);
                            if icon_button(ui, pal, Icon::Trash, deletable.is_some(), false)
                                .on_hover_text("Move to Trash")
                                .clicked()
                                && let Some(s) = deletable
                            {
                                actions.push(TreeAction::AskDelete(s));
                            }
                            let target = self.tree_target_dir();
                            if icon_button(ui, pal, Icon::NewFolder, true, false)
                                .on_hover_text("New Folder")
                                .clicked()
                            {
                                actions.push(TreeAction::NewIn {
                                    dir: target.clone(),
                                    is_dir: true,
                                });
                            }
                            if icon_button(ui, pal, Icon::NewFile, true, false)
                                .on_hover_text("New File")
                                .clicked()
                            {
                                actions.push(TreeAction::NewIn {
                                    dir: target.clone(),
                                    is_dir: false,
                                });
                            }
                        });
                    });
                }
                ui.add_space(4.0);
                hline(ui, pal);

                if let Some(path) = self.tree_delete.clone() {
                    self.delete_prompt(ui, pal, &path, &mut actions);
                }

                if self.nav_tab == NavTab::Search {
                    // The search has its own scrolling results list.
                    self.search_tab(ui, pal);
                } else {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            match self.nav_tab {
                                NavTab::Files => actions.extend(self.file_rows(ui, pal, p)),
                                NavTab::Outline => self.outline_rows(ui, pal, p),
                                NavTab::Search => {}
                            }
                        });
                }
            });
        self.apply_tree_actions(actions, p);
    }

    /// Asks the server for the declarations of the active file once its text has been still for a
    /// moment. Nothing is asked again while the answer for this text is shown or on its way.
    pub(crate) fn refresh_outline(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let want = (doc.path.clone(), text_key(&doc.text));
        if self.outline.shown.as_ref() == Some(&want)
            || self.outline.pending.as_ref() == Some(&want)
        {
            return;
        }
        let Some(lsp) = &self.lsp else { return };
        let now = ctx.input(|i| i.time);
        let wait = OUTLINE_QUIET - (now - self.outline.asked_at);
        if wait > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            return;
        }
        let _ = lsp.requests.send(lsp::Req::Symbols {
            path: want.0.clone(),
            text: doc.text.clone(),
            key: want.1,
        });
        self.outline.pending = Some(want);
        self.outline.asked_at = now;
    }

    /// The declarations of the active file, indented by depth. A click goes to the declaration.
    fn outline_rows(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) {
        let Some(doc) = self.docs.get(self.active) else {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Open a file").color(pal.text_tertiary))
            });
            return;
        };
        if self.lsp.is_none() {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("The language server is not running").color(pal.text_tertiary),
                )
            });
            return;
        }
        if self.outline.shown.as_ref().map(|(path, _)| path) != Some(&doc.path) {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Reading the declarations...").color(pal.text_tertiary))
            });
            return;
        }
        if self.outline.symbols.is_empty() {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No declarations").color(pal.text_tertiary))
            });
            return;
        }
        for s in &self.outline.symbols {
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            let painter = ui.painter();
            let inner = rect.shrink2(vec2(6.0, 0.0));
            if resp.hovered() {
                painter.rect_filled(inner, 5.0, pal.row_hover);
            }
            let x = rect.left() + 12.0 + s.depth as f32 * 14.0;
            painter.text(
                egui::pos2(x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &s.name,
                egui::FontId::proportional(13.0),
                pal.text,
            );
            painter.text(
                egui::pos2(rect.right() - 12.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                kind_word(s.kind),
                egui::FontId::proportional(11.0),
                pal.text_tertiary,
            );
            if resp.clicked() {
                p.goto = Some((doc.path.clone(), s.line + 1, s.col + 1));
            }
        }
    }

    /// The "Delete ...?" strip above the file list. Unsaved edits in the item are named.
    fn delete_prompt(
        &mut self,
        ui: &mut egui::Ui,
        pal: &Palette,
        path: &Path,
        actions: &mut Vec<TreeAction>,
    ) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let inside = files::count_items(path).saturating_sub(1);
        let dirty = self
            .docs
            .iter()
            .any(|d| d.path.starts_with(path) && d.dirty());
        let mut text = format!("Move “{name}” to the Trash?");
        if inside > 0 {
            text += &format!(
                " It holds {inside} more {}.",
                if inside == 1 { "item" } else { "items" }
            );
        }
        if dirty {
            text += " Unsaved changes will be lost.";
        }
        Frame::new()
            .fill(pal.row_selected)
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                // As wide as the navigator, not as wide as its text.
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(pal.text)).wrap());
                ui.horizontal(|ui| {
                    // Cancel on the left, the destructive button on the right (as in a macOS dialog).
                    if crate::style::secondary_button(ui, "Cancel").clicked() {
                        self.tree_delete = None;
                    }
                    if crate::style::destructive_button(ui, "Move to Trash").clicked() {
                        actions.push(TreeAction::ConfirmDelete(path.to_path_buf()));
                    }
                });
            });
    }

    fn file_rows(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) -> Vec<TreeAction> {
        if self.expanded.is_empty() {
            self.expanded.insert(self.root.clone());
        }
        let active = self.docs.get(self.active).map(|d| d.path.clone());
        let mut actions = Vec::new();
        if self.file_filter.trim().is_empty() {
            let root = self.root.clone();
            let mut view = TreeView {
                active: active.as_deref(),
                sel: self.tree_sel.as_deref(),
                expanded: &self.expanded,
                new: &mut self.tree_new,
                rename: &mut self.tree_rename,
                toggles: Vec::new(),
                actions: Vec::new(),
                open: None,
            };
            tree_rows(ui, pal, &root, 0, true, &mut view);
            let TreeView {
                toggles,
                open,
                actions: drawn,
                ..
            } = view;
            actions.extend(drawn);
            if open.is_some() {
                p.open = open;
            }
            for t in toggles {
                if !self.expanded.remove(&t) {
                    self.expanded.insert(t);
                }
            }
        } else {
            let needle = self.file_filter.trim().to_lowercase();
            let mut hits = Vec::new();
            collect_files(&self.root, 0, &mut hits);
            for path in hits.into_iter().filter(|h| {
                h.file_name()
                    .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&needle))
            }) {
                if row(
                    ui,
                    pal,
                    0,
                    false,
                    false,
                    &path,
                    false,
                    active.as_deref() == Some(path.as_path()),
                )
                .clicked()
                {
                    actions.push(TreeAction::Select(path.clone()));
                    p.open = Some(path);
                }
            }
        }
        actions
    }

    /// The folder new items go into: the selected folder, the selected file's folder, or the
    /// project folder.
    fn tree_target_dir(&self) -> PathBuf {
        match &self.tree_sel {
            Some(sel) if sel.is_dir() => sel.clone(),
            Some(sel) => sel
                .parent()
                .map_or_else(|| self.root.clone(), Path::to_path_buf),
            None => self.root.clone(),
        }
    }

    /// Applies what the tree asked for this frame.
    fn apply_tree_actions(&mut self, actions: Vec<TreeAction>, p: &mut Pending) {
        for action in actions {
            match action {
                TreeAction::Select(path) => self.tree_sel = Some(path),
                TreeAction::NewIn { dir, is_dir } => {
                    self.expanded.insert(dir.clone());
                    self.tree_new = Some(TreeNew {
                        dir,
                        is_dir,
                        name: String::new(),
                        focus: true,
                    });
                }
                TreeAction::CancelNew => self.tree_new = None,
                TreeAction::NewForm(dir) => self.new_rust_form(Some(dir)),
                TreeAction::Create { dir, is_dir, name } => {
                    self.tree_new = None;
                    let made = if is_dir {
                        files::new_folder(&dir, &name)
                    } else {
                        files::new_file(&dir, &name)
                    };
                    match made {
                        Ok(path) => {
                            self.expanded.insert(dir);
                            if !is_dir {
                                p.open = Some(path.clone());
                            }
                            self.tree_sel = Some(path);
                        }
                        Err(e) => self.note(e),
                    }
                }
                TreeAction::AskDelete(path) => self.tree_delete = Some(path),
                TreeAction::ConfirmDelete(path) => {
                    self.tree_delete = None;
                    self.bookmarks.remove_under(&path);
                    self.delete_in_tree(&path);
                }
                TreeAction::AskRename(path) => {
                    self.tree_new = None;
                    self.tree_delete = None;
                    let name = path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.tree_rename = Some(TreeRename {
                        path,
                        name,
                        focus: true,
                    });
                }
                TreeAction::CancelRename => self.tree_rename = None,
                TreeAction::RenameTo { path, name } => {
                    self.tree_rename = None;
                    self.rename_in_tree(&path, &name);
                }
                TreeAction::MoveInto { path, dir } => self.move_in_tree(&path, &dir),
            }
        }
    }

    /// File > New Rust Form, the toolbar's Form button and the tree's menu: a sample egui form
    /// (`formN.rs`, a struct with a `ui` method) in `dir` (default: the folder of the selection, or
    /// `src/` for the project folder), opened in the designer.
    pub(crate) fn new_rust_form(&mut self, dir: Option<PathBuf>) {
        let mut dir = dir.unwrap_or_else(|| self.tree_target_dir());
        if dir == self.root && self.root.join("src").is_dir() {
            dir = self.root.join("src");
        }
        // Without a manifest naming eframe, egui is taken to be a dependency of its own.
        let eframe = formust_build::manifest_for(&dir)
            .ok()
            .and_then(|m| std::fs::read_to_string(m).ok())
            .is_none_or(|t| t.contains("eframe"));
        match designer::new_form_file(&dir, eframe) {
            Ok(path) => {
                self.expanded.insert(dir);
                self.tree_sel = Some(path.clone());
                self.open_path(path.clone());
                self.show_form();
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.note(format!("Created the form {name}"));
            }
            Err(e) => self.note(format!("Could not create a form: {e}")),
        }
    }

    /// File > New File, and the toolbar's +: a name is typed in the tree, in the folder of the selection.
    pub(crate) fn new_file_in_tree(&mut self) {
        self.nav_tab = NavTab::Files;
        let dir = self.tree_target_dir();
        self.apply_tree_actions(
            vec![TreeAction::NewIn { dir, is_dir: false }],
            &mut Pending::default(),
        );
    }

    /// Renames an item in the tree. Open tabs, forms and the selection follow it.
    fn rename_in_tree(&mut self, path: &Path, name: &str) {
        match files::rename(path, name, &self.root) {
            Ok(moves) => {
                let Some(to) = moves.first().map(|(_, to)| to.clone()) else {
                    return;
                };
                for (from, to) in &moves {
                    self.follow_move(from, to);
                }
                let shown = to
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.tree_sel = Some(to);
                self.note(format!("Renamed to “{shown}”"));
            }
            Err(e) => self.note(format!("Could not rename “{name}”: {e}")),
        }
    }

    /// Moves an item into a folder of the tree (drag and drop). Open tabs, forms and the selection
    /// follow it, as they do for a rename.
    fn move_in_tree(&mut self, path: &Path, dir: &Path) {
        match files::move_into(path, dir, &self.root) {
            Ok(moves) => {
                let Some(to) = moves.first().map(|(_, to)| to.clone()) else {
                    return;
                };
                for (from, to) in &moves {
                    self.follow_move(from, to);
                }
                self.expanded.insert(dir.to_path_buf());
                let shown = to
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let folder = dir.file_name().map_or_else(
                    || "the project".to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                self.tree_sel = Some(to);
                self.note(format!("Moved “{shown}” into “{folder}”"));
            }
            Err(e) => {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.note(format!("Could not move “{name}”: {e}"));
            }
        }
    }

    /// Points everything that names `from` (or a path inside it) at `to` instead.
    fn follow_move(&mut self, from: &Path, to: &Path) {
        self.bookmarks.follow_move(from, to);
        for d in &mut self.docs {
            if let Some(p) = files::moved(&d.path, from, to) {
                d.path = p;
            }
        }
        self.expanded = self
            .expanded
            .iter()
            .map(|d| files::moved(d, from, to).unwrap_or_else(|| d.clone()))
            .collect::<HashSet<PathBuf>>();
        if let Some(sel) = self
            .tree_sel
            .as_deref()
            .and_then(|s| files::moved(s, from, to))
        {
            self.tree_sel = Some(sel);
        }
    }

    /// Deletes an item from disk. Tabs and forms inside it go too; the caller has asked first.
    fn delete_in_tree(&mut self, path: &Path) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        // What goes with the item (its form, or its source) is found before the files are gone.
        let gone: Vec<PathBuf> = if path.is_dir() {
            vec![path.to_path_buf()]
        } else {
            files::removed_with(path)
        };
        let is_gone = |p: &Path| gone.iter().any(|g| p.starts_with(g));
        match files::delete(path, &self.root) {
            Ok(()) => {
                for i in (0..self.docs.len()).rev() {
                    if is_gone(&self.docs[i].path) {
                        self.discard_doc(i);
                    }
                }
                self.expanded.retain(|d| !is_gone(d));
                if self.tree_sel.as_deref().is_some_and(is_gone) {
                    self.tree_sel = None;
                }
                let note = if gone.len() > 1 {
                    let others: Vec<String> = gone[1..]
                        .iter()
                        .map(|p| {
                            p.file_name()
                                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                        })
                        .collect();
                    format!("Moved “{name}” and {} to the Trash", others.join(" and "))
                } else {
                    format!("Moved “{name}” to the Trash")
                };
                self.note(note);
            }
            Err(e) => self.note(format!("Could not move “{name}” to the Trash: {e}")),
        }
    }

    // ---- centre: tabs, jump bar, editor or designer ----------------------------------

    fn center(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) {
        self.tab_strip(ui, pal, p);
        if self.show_jump_bar {
            self.jump_bar(ui, pal);
        }
        for (key, banner, name) in self.active_banners() {
            if let Some(answer) = external::banner_row(ui, pal, &key, &name, &banner) {
                p.external = Some((key, answer));
            }
        }
        // The form designer for the active `.rs` or `.frm.rs` tab.
        if self.form_showing() {
            let docs = &self.docs;
            let read = |p: &Path| App::current_text(docs, p);
            designer::ui::show(&mut self.designer, ui, pal, &read);
            self.run_designer_actions();
            return;
        }
        self.code_editor(ui, pal, p);
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) {
        // Real tabs: the active one has the editor's colour and rounded top corners and is
        // joined to the editor below; the others lie flat on the darker strip.
        let mut active_rect: Option<Rect> = None;
        let strip = Frame::new()
            .fill(pal.sidebar)
            .inner_margin(Margin {
                left: 8,
                right: 8,
                top: 6,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Many files: the row scrolls sideways, like the designer's page tabs.
                style::thin_scroll(ui, pal, |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("editor-tabs")
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let mut switched = None;
                                let designer_dirty: Vec<bool> = self
                                    .docs
                                    .iter()
                                    .map(|d| self.designer.dirty_for(&d.path))
                                    .collect();
                                for (i, d) in self.docs.iter().enumerate() {
                                    let active = i == self.active;
                                    let dirty = d.dirty() || designer_dirty[i];
                                    let name = d.path.file_name().map_or_else(String::new, |n| {
                                        n.to_string_lossy().into_owned()
                                    });
                                    let galley = ui.painter().layout_no_wrap(
                                        name,
                                        egui::FontId::proportional(12.5),
                                        if active { pal.text } else { pal.text_secondary },
                                    );
                                    // The file's icon: blue for a source file, amber for a database. Then the name.
                                    let ext = d
                                        .path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("")
                                        .to_ascii_lowercase();
                                    let (icon_name, icon_color) = match ext.as_str() {
                                        "sqlite" | "sqlite3" | "db" => {
                                            ("database", egui::Color32::from_rgb(0xF5, 0xA5, 0x24))
                                        }
                                        _ if pal.dark => {
                                            ("file-code", egui::Color32::from_rgb(0x60, 0xA5, 0xFA))
                                        }
                                        _ => {
                                            ("file-code", egui::Color32::from_rgb(0x3B, 0x82, 0xF6))
                                        }
                                    };
                                    let width = galley.size().x + 56.0;
                                    let (rect, resp) =
                                        ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
                                    let hovered = resp.hovered();
                                    if active {
                                        active_rect = Some(rect);
                                        let top = egui::CornerRadius {
                                            nw: 8,
                                            ne: 8,
                                            sw: 0,
                                            se: 0,
                                        };
                                        ui.painter().rect_filled(rect, top, pal.editor);
                                        ui.painter().rect_stroke(
                                            rect,
                                            top,
                                            egui::Stroke::new(1.0, pal.separator),
                                            egui::StrokeKind::Inside,
                                        );
                                        // Hide the stroke's bottom edge so the tab runs into the editor.
                                        let seam = Rect::from_min_max(
                                            egui::pos2(rect.left() + 1.0, rect.bottom() - 1.0),
                                            egui::pos2(rect.right() - 1.0, rect.bottom() + 1.0),
                                        );
                                        ui.painter().rect_filled(seam, 0.0, pal.editor);
                                    } else if hovered {
                                        ui.painter().rect_filled(
                                            rect.shrink2(vec2(0.0, 3.0)),
                                            6.0,
                                            pal.row_hover,
                                        );
                                    }
                                    let icon_rect = Rect::from_center_size(
                                        egui::pos2(rect.left() + 18.0, rect.center().y),
                                        vec2(13.0, 13.0),
                                    );
                                    crate::app_icons::paint(
                                        ui.painter(),
                                        icon_rect,
                                        icon_name,
                                        icon_color,
                                    );
                                    let text_pos =
                                        rect.left_center() + vec2(28.0, -galley.size().y / 2.0);
                                    ui.painter().galley(text_pos, galley.clone(), pal.text);
                                    let close = egui::Rect::from_center_size(
                                        rect.right_center() - vec2(13.0, 0.0),
                                        vec2(16.0, 16.0),
                                    );
                                    let over_close = hovered
                                        && ui
                                            .input(|i| i.pointer.hover_pos())
                                            .is_some_and(|h| close.contains(h));
                                    if dirty {
                                        // Unsaved changes: a red dot, which becomes a red x under the pointer.
                                        if over_close {
                                            ui.painter().rect_filled(close, 4.0, pal.row_hover);
                                            style::paint_icon(
                                                ui.painter(),
                                                close.shrink(4.5),
                                                Icon::Close,
                                                pal.error,
                                            );
                                        } else {
                                            ui.painter().circle_filled(
                                                close.center(),
                                                4.0,
                                                pal.error,
                                            );
                                        }
                                    } else if hovered || active {
                                        if over_close {
                                            ui.painter().rect_filled(close, 4.0, pal.row_hover);
                                        }
                                        style::paint_icon(
                                            ui.painter(),
                                            close.shrink(4.5),
                                            Icon::Close,
                                            pal.text_secondary,
                                        );
                                    }
                                    if resp.clicked() {
                                        let clicked_close = ui
                                            .input(|i| i.pointer.interact_pos())
                                            .is_some_and(|h| close.contains(h));
                                        if clicked_close {
                                            p.close = Some(i);
                                        } else {
                                            switched = Some(i);
                                        }
                                    }
                                }
                                if let Some(i) = switched {
                                    self.active = i;
                                    // The designer only shows for a Rust file with forms; anything else is code.
                                    if !self.has_form() {
                                        self.form_view = false;
                                    }
                                }
                            })
                        });
                });
            });
        // The separator under the strip, interrupted where the active tab joins the editor.
        let y = strip.response.rect.bottom() - 0.5;
        let (l, r) = (strip.response.rect.left(), strip.response.rect.right());
        let line = egui::Stroke::new(1.0, pal.separator);
        match active_rect {
            Some(a) => {
                ui.painter()
                    .line_segment([egui::pos2(l, y), egui::pos2(a.left(), y)], line);
                ui.painter()
                    .line_segment([egui::pos2(a.right(), y), egui::pos2(r, y)], line);
            }
            None => {
                ui.painter()
                    .line_segment([egui::pos2(l, y), egui::pos2(r, y)], line);
            }
        }
    }

    fn jump_bar(&self, ui: &mut egui::Ui, pal: &Palette) {
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let mut crumbs: Vec<String> = vec![
            self.root
                .file_name()
                .map_or_else(|| "Project".into(), |n| n.to_string_lossy().into_owned()),
        ];
        if let Ok(rel) = doc.path.strip_prefix(&self.root) {
            crumbs.extend(
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned()),
            );
        } else if let Some(n) = doc.path.file_name() {
            crumbs.push(n.to_string_lossy().into_owned());
        }
        Frame::new()
            .fill(pal.editor)
            .inner_margin(Margin::symmetric(14, 3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    let last = crumbs.len() - 1;
                    for (i, c) in crumbs.iter().enumerate() {
                        if i > 0 {
                            ui.label(
                                RichText::new("\u{203a}")
                                    .size(12.0)
                                    .color(pal.text_tertiary),
                            );
                        }
                        let color = if i == last {
                            pal.text
                        } else {
                            pal.text_secondary
                        };
                        ui.label(RichText::new(c).size(12.0).color(color));
                    }
                });
            });
        hline(ui, pal);
    }

    fn code_editor(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) {
        let ctx = ui.ctx().clone();
        let Some(doc) = self.docs.get_mut(self.active) else {
            ui.add_space(ui.available_height() / 3.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No Editor")
                        .size(22.0)
                        .color(pal.text_tertiary),
                );
                ui.label(
                    RichText::new("Open a file, or choose File > New GUI Application")
                        .color(pal.text_tertiary),
                );
            });
            return;
        };
        let font = editor_font();
        self.marks
            .retain(|m| m.path != doc.path || m.text == doc.text);
        let marks = self
            .marks
            .iter()
            .find(|m| m.path == doc.path)
            .map_or(&[][..], |m| m.items.as_slice());
        let mut flagged = std::collections::BTreeMap::new();
        for m in marks {
            *flagged.entry(m.line + 1).or_insert(false) |= m.is_error();
        }
        // Tab inserts spaces up to the next tab stop, and keeps the focus in the editor. A Tab that
        // completion took (a chosen item) was consumed before this, so what is left is ours.
        if ctx.memory(|m| m.has_focus(doc.editor_id))
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
        {
            let caret = egui::TextEdit::load_state(&ctx, doc.editor_id)
                .and_then(|s| s.cursor.char_range())
                .map(|r| (r.primary.index.0, r.secondary.index.0))
                .unwrap_or((0, 0));
            let (lo, hi) = (caret.0.min(caret.1), caret.0.max(caret.1));
            let (text, at) = prefs::insert_tab(&doc.text, lo, hi, prefs::tab_width());
            doc.text = text;
            let mut state = egui::TextEdit::load_state(&ctx, doc.editor_id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(at))));
            state.store(&ctx, doc.editor_id);
        }
        if self.find.from_selection {
            self.find.from_selection = false;
            let caret = egui::TextEdit::load_state(&ctx, doc.editor_id)
                .and_then(|s| s.cursor.char_range())
                .map(|r| (r.primary.index.0, r.secondary.index.0));
            if let Some((a, b)) = caret.filter(|(a, b)| a != b) {
                let (lo, hi) = (a.min(b), a.max(b));
                let picked: String = doc.text.chars().skip(lo).take(hi - lo).collect();
                if !picked.contains('\n') {
                    self.find.query = picked;
                    self.find.current = None;
                }
            }
        }
        let found = if self.find.open {
            find::matches(&doc.text, &self.find.query, self.find.opts)
        } else {
            Vec::new()
        };
        if self.find.open {
            let asked = find_bar(ui, pal, &mut self.find, found.len())
                .or_else(|| self.find.pending.take().map(FindAction::Go));
            if let Some(action) = asked {
                let caret = egui::TextEdit::load_state(&ctx, doc.editor_id)
                    .and_then(|s| s.cursor.char_range())
                    .map(|r| (r.primary.index.0, r.secondary.index.0));
                match action {
                    FindAction::Go(forward) => {
                        let (lo, hi) = caret.map_or((0, 0), |(a, b)| (a.min(b), a.max(b)));
                        if let Some(i) = find::step(&found, if forward { hi } else { lo }, forward)
                        {
                            self.find.current = Some(i);
                            select_in_editor(&ctx, doc.editor_id, &found[i]);
                            self.find.scroll = Some(found[i].clone());
                        }
                    }
                    FindAction::ReplaceOne => {
                        let lo = caret.map_or(0, |(a, b)| a.min(b));
                        let done = find::replace_one(
                            &doc.text,
                            &self.find.query,
                            self.find.opts,
                            self.find.current,
                            lo,
                            &self.find.with,
                        );
                        if let Some(done) = done {
                            doc.text = done.text;
                            self.find.current = done.index;
                            self.find.note = if done.selected.is_none() {
                                "Replaced the last match".to_string()
                            } else {
                                String::new()
                            };
                            if let Some(r) = done.selected {
                                select_in_editor(&ctx, doc.editor_id, &r);
                                self.find.scroll = Some(r);
                            }
                        }
                    }
                    FindAction::ReplaceAll => {
                        let (text, n) = find::replace_all(
                            &doc.text,
                            &self.find.query,
                            &self.find.with,
                            self.find.opts,
                        );
                        self.find.note = format!("Replaced {n}");
                        if n > 0 {
                            doc.text = text;
                        }
                        self.find.current = None;
                    }
                }
            }
        }
        // Find's match and go-to's line both scroll the editor to them.
        let scroll_to = self.find.scroll.take().or_else(|| self.reveal.take());
        let wrap = crate::prefs::word_wrap();
        // Wrapped text fits the width, so only the height scrolls.
        let scroll = if wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };
        scroll.auto_shrink(false).show(ui, |ui| {
            if let Some([r, g, b]) = crate::prefs::editor_background(pal.dark) {
                ui.painter()
                    .rect_filled(ui.clip_rect(), 0.0, egui::Color32::from_rgb(r, g, b));
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let lines = doc.text.split('\n').count();
                // The width the text wraps to: the row past the gutter (no limit when not wrapping).
                let wrap_width = if wrap {
                    (ui.available_width() - GUTTER_W).max(40.0)
                } else {
                    f32::INFINITY
                };
                let galley = {
                    let mut job = highlight::layout_for(
                        highlight::language_for(&doc.path),
                        doc.text.as_str(),
                        pal,
                        editor_font(),
                        crate::prefs::line_height(),
                    );
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let map = LineMap::new(&galley, lines);
                let caret = egui::TextEdit::load_state(&ctx, doc.editor_id)
                    .and_then(|s| s.cursor.char_range())
                    .map(|r| r.primary.index.0);
                let caret_line =
                    caret.map(|i| doc.text.chars().take(i).filter(|c| *c == '\n').count() + 1);

                // Row backgrounds go down first, then the gutter and the text on top.
                let top = ui.cursor().min.y;
                let left = ui.cursor().min.x;
                let right = ui.clip_rect().right().max(left + 400.0);
                let band = |line: u32| {
                    egui::Rect::from_min_max(
                        egui::pos2(left, top + map.top(line)),
                        egui::pos2(right, top + map.bottom(line)),
                    )
                };
                if let Some(l) = caret_line {
                    let color = crate::prefs::current_line_color(pal.dark)
                        .map_or(pal.current_line, |[r, g, b]| {
                            egui::Color32::from_rgb(r, g, b)
                        });
                    ui.painter().rect_filled(band(l as u32), 0.0, color);
                }

                let bookmark_lines = self.bookmarks.lines_in(&doc.path);
                // Clicking a bookmark's chip unsets it.
                if let Some(line) = gutter(
                    ui,
                    pal,
                    &map,
                    caret_line.map(|l| l as u32),
                    &flagged,
                    &bookmark_lines,
                ) {
                    self.bookmarks.remove_at(&doc.path, line);
                }
                // The same width as the map above, so the galley the text is drawn with is the one it was measured with.
                let lang = highlight::language_for(&doc.path);
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                    let mut job = highlight::layout_for(
                        lang,
                        text.as_str(),
                        pal,
                        editor_font(),
                        crate::prefs::line_height(),
                    );
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let rows = (ui.available_height() / crate::prefs::line_height())
                    .floor()
                    .max(1.0) as usize;
                let len_before = doc.text.len();
                let output = egui::TextEdit::multiline(&mut doc.text)
                    .id(doc.editor_id)
                    // Tab stays in the editor (it inserts spaces, see above) instead of moving focus.
                    .lock_focus(true)
                    .font(font.clone())
                    .frame(Frame::NONE)
                    .margin(Margin::ZERO)
                    .desired_width(wrap_width)
                    .desired_rows(rows)
                    .layouter(&mut layouter)
                    .show(ui);
                // VoiceOver reads this name; without it the editor is announced as unnamed.
                output.response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, "Code editor")
                });
                if let Some(r) = output.cursor_range {
                    let idx = r.primary.index.0;
                    let before: String = doc.text.chars().take(idx).collect();
                    let line = before.matches('\n').count() + 1;
                    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                    self.cursor_pos = (line, col);
                }
                // Language features: squiggles, hover, completion while typing.
                let origin = output.galley_pos.to_vec2();
                // The signature popup under the caret: the label, the active parameter in the accent colour.
                if output.response.has_focus() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.signature = None;
                }
                if let Some(h) = self
                    .signature
                    .as_ref()
                    .filter(|h| h.key == text_key(&doc.text))
                {
                    let anchor = output.galley_pos
                        + output
                            .galley
                            .pos_from_cursor(CCursor::new(h.at))
                            .left_bottom()
                            .to_vec2();
                    egui::Area::new(egui::Id::unique("signature-hint"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(anchor)
                        .show(&ctx, |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                let mut job = egui::text::LayoutJob::default();
                                for (text, marked) in hints::highlight_parameter(
                                    &h.sig.label,
                                    &h.sig.parameters,
                                    h.sig.active,
                                ) {
                                    job.append(
                                        &text,
                                        0.0,
                                        egui::TextFormat {
                                            font_id: egui::FontId::monospace(12.0),
                                            color: if marked { pal.accent } else { pal.text },
                                            ..Default::default()
                                        },
                                    );
                                }
                                ui.label(job);
                            });
                        });
                }
                // Every match of the find bar, the current one stronger; the editor scrolls to it.
                for (i, r) in found.iter().enumerate() {
                    let Some(rect) = hints::range_rect(&output.galley, r) else {
                        continue;
                    };
                    let fill = if self.find.current == Some(i) {
                        pal.warning.gamma_multiply(0.55)
                    } else {
                        pal.warning.gamma_multiply(0.22)
                    };
                    ui.painter().rect_filled(rect.translate(origin), 2.0, fill);
                }
                if let Some(r) = scroll_to.as_ref()
                    && let Some(rect) = hints::range_rect(&output.galley, r)
                {
                    ui.scroll_to_rect(rect.translate(origin), Some(egui::Align::Center));
                }
                for m in marks {
                    let Some(r) = hints::range_rect(&output.galley, &m.range(&doc.text)) else {
                        continue;
                    };
                    let color = if m.is_error() { pal.error } else { pal.warning };
                    let y = r.max.y + origin.y - 1.0;
                    let points = hints::squiggle_points(r.min.x + origin.x, r.max.x + origin.x, y);
                    ui.painter()
                        .add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
                }
                // Cmd-click on a name goes to its definition (marks are not symbols).
                if output.response.clicked() && ui.input(|i| i.modifiers.command) {
                    let pointer = ctx.pointer_interact_pos().unwrap_or_default();
                    let local = pointer - output.galley_pos;
                    if let Some(t) = hints::target_at(&output.galley, &doc.text, local, marks)
                        && t.mark.is_none()
                    {
                        p.definition_at = Some(t.range.start);
                    }
                }
                if output.response.changed() {
                    self.hover.clear();
                    // Any edit hides the signature; typing `(` or `,` asks for the one at the caret.
                    self.signature = None;
                    if let Some(c) = output.cursor_range.map(|r| r.primary.index.0)
                        && c > 0
                        && matches!(doc.text.chars().nth(c - 1), Some('(' | ','))
                    {
                        let (line, col) = line_col(&doc.text, c);
                        p.signature_request = Some((
                            doc.path.clone(),
                            doc.text.clone(),
                            line,
                            col,
                            c,
                            text_key(&doc.text),
                        ));
                    }
                    if doc.text.len() > len_before {
                        self.typed += 1;
                    }
                }
                let now = ui.input(|i| i.time);
                let typed = (self.typed > 0).then_some(self.typed);
                match self.typing.update(typed, now, hints::COMPLETE_DELAY) {
                    hints::Poll::Ready => p.auto_complete = true,
                    hints::Poll::Wait(s) => ctx.request_repaint_after_secs(s as f32),
                    hints::Poll::Idle => {}
                }
                let resting = output.response.hovered()
                    && self.completion.is_none()
                    && ctx.input(|i| !i.pointer.any_down() && i.smooth_scroll_delta == Vec2::ZERO);
                let target = ctx.pointer_hover_pos().filter(|_| resting).and_then(|pt| {
                    hints::target_at(&output.galley, &doc.text, pt - output.galley_pos, marks)
                });
                self.hover.set_target(target.clone());
                let key = target
                    .as_ref()
                    .map(|t| (t.range.start, t.range.end, t.mark));
                match self.hover.dwell.update(key, now, hints::HOVER_DELAY) {
                    hints::Poll::Ready => {
                        let t = target.expect("ready implies a target");
                        let rect =
                            hints::range_rect(&output.galley, &t.range).unwrap_or(Rect::ZERO);
                        let pos = output.galley_pos + rect.left_bottom().to_vec2() + vec2(0.0, 3.0);
                        if let Some(m) = t.mark.map(|i| &marks[i]) {
                            let tag = if m.is_error() { "Error" } else { "Warning" };
                            self.hover.popup = Some(hints::Popup {
                                text: format!("{tag}: {}", m.message),
                                severity: Some(m.severity),
                                pos,
                            });
                        } else {
                            let (line, col) = line_col(&doc.text, t.range.start);
                            self.hover_pos = pos;
                            p.hover_request = Some((doc.path.clone(), doc.text.clone(), line, col));
                        }
                    }
                    hints::Poll::Wait(s) => ctx.request_repaint_after_secs(s as f32),
                    hints::Poll::Idle => {}
                }
                if let Some(popup) = &self.hover.popup {
                    hints::show_popup(&ctx, popup, pal.error, pal.warning);
                }
                if let Some(c) = &self.completion {
                    if c.text != doc.text {
                        p.close_completion = true;
                    } else {
                        let at = output.galley.pos_from_cursor(CCursor::new(c.cursor));
                        let pos = output.galley_pos + at.left_bottom().to_vec2();
                        if let Some(i) = completion_popup(ui.ctx(), pos, c) {
                            p.accept = Some(i);
                        }
                    }
                }
            });
        });
    }

    // ---- bottom pane: Problems, Output, Terminal -------------------------------

    fn bottom_area(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) {
        egui::Panel::bottom("debug-area")
            .resizable(true)
            .default_size(self.bottom_height)
            .min_size(70.0)
            .frame(Frame::new().fill(pal.editor))
            .show(ui, |ui| {
                self.bottom_tabs(ui, pal);
                match self.bottom.tab {
                    BottomTab::Problems => {
                        self.bottom.terminal_focused = false;
                        self.bottom.view_shown = false;
                        self.problems_tab(ui, pal, p);
                    }
                    BottomTab::Output => {
                        self.bottom.terminal_focused = false;
                        self.bottom.view_shown = false;
                        self.output_tab(ui, pal);
                    }
                    BottomTab::Terminal => self.terminal_tab(ui, pal),
                }
            });
    }

    /// Opens the Rename Symbol box (F2) on the name under the caret.
    pub(crate) fn open_rename(&mut self, ctx: &egui::Context) {
        let Some(index) = self.cursor_index(ctx) else {
            return;
        };
        let Some(doc) = self.docs.get(self.active) else {
            return;
        };
        let Some((_, name)) = find::word_at(&doc.text, index) else {
            self.console.push_str("put the caret on a name first\n");
            return;
        };
        let (line, col) = line_col(&doc.text, index);
        self.rename = find::Rename {
            open: true,
            old: name.clone(),
            path: Some(doc.path.clone()),
            line,
            col,
            name,
            focus: true,
            error: String::new(),
            sent: None,
        };
    }

    /// The Configure Tools window: Save writes the tools and rebuilds the Tools menu.
    fn show_tools_editor(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.tools_editor.take() else {
            return;
        };
        match crate::tools_editor::dialog(ctx, &mut editor) {
            crate::tools_editor::Outcome::Open => self.tools_editor = Some(editor),
            crate::tools_editor::Outcome::Saved => {
                self.tools = editor.tools;
                self.refresh_tools_menu();
            }
            crate::tools_editor::Outcome::Cancelled => {}
        }
    }

    /// The Preferences window (Cmd+,): the editor's font size and what the Tab key inserts. Each
    /// change is applied and saved at once.
    fn show_preferences(&mut self, ctx: &egui::Context) {
        if !self.preferences_open {
            return;
        }
        let before = self.prefs;
        let mut done = false;
        let current_dark = Palette::of(ctx).dark;
        // The app's own dialog (no system-grey title bar), with a Done button to close it.
        style::dialog(ctx, "preferences", Some("Preferences"), 520.0, |ui| {
            let pal = Palette::of(ui.ctx());
            // The appearance whose colours the rows below edit: the current one unless the Light/Dark choice says.
            let dark = self.prefs_dark.unwrap_or(current_dark);
            ui.spacing_mut().item_spacing.y = 0.0;

            // Two tabs: the editor's settings, and the code colours.
            let colours = self.prefs_colours_tab;
            ui.horizontal(|ui| {
                style::pill(ui, &pal, |ui| {
                    if style::accent_segment(ui, &pal, "Editor", !colours).clicked() {
                        self.prefs_colours_tab = false;
                    }
                    if style::accent_segment(ui, &pal, "Colours", colours).clicked() {
                        self.prefs_colours_tab = true;
                    }
                });
            });
            ui.add_space(8.0);
            if !colours {
                pref_row(ui, &pal, "Line numbers", |ui| {
                    toggle_caption(ui, &pal, &mut self.prefs.line_numbers, "Show in the gutter");
                });
                pref_row(ui, &pal, "Long lines", |ui| {
                    toggle_caption(
                        ui,
                        &pal,
                        &mut self.prefs.word_wrap,
                        "Wrap to the editor width",
                    );
                });
                pref_row(ui, &pal, "Saving", |ui| {
                    toggle_caption(
                        ui,
                        &pal,
                        &mut self.prefs.format_on_save,
                        "Format Rust files (Cmd+S)",
                    );
                });
                pref_row(ui, &pal, "Font family", |ui| {
                    egui::ComboBox::from_id_salt("prefs-font")
                        .selected_text(theme::EDITOR_FONTS[self.prefs.font].0)
                        .show_ui(ui, |ui| {
                            for (i, (name, _)) in theme::EDITOR_FONTS.iter().enumerate() {
                                ui.selectable_value(&mut self.prefs.font, i, *name);
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Font family")
                        });
                });
                pref_row(ui, &pal, "Font size", |ui| {
                    let (lo, hi) = (*prefs::FONT_SIZES.start(), *prefs::FONT_SIZES.end());
                    // Right to left: the + is the rightmost button.
                    if stepper(ui, &pal, "+", self.prefs.font_size < hi) {
                        self.prefs.font_size = (self.prefs.font_size + 1.0).min(hi);
                    }
                    ui.label(
                        RichText::new(format!("{:.0} pt", self.prefs.font_size))
                            .size(12.5)
                            .color(pal.accent),
                    );
                    if stepper(ui, &pal, "\u{2212}", self.prefs.font_size > lo) {
                        self.prefs.font_size = (self.prefs.font_size - 1.0).max(lo);
                    }
                });
                pref_row(ui, &pal, "Tab key inserts", |ui| {
                    style::pill(ui, &pal, |ui| {
                        // Right to left, so the widths are added from the largest to read 2, 4, 8 from the left.
                        for width in prefs::TAB_WIDTHS.into_iter().rev() {
                            let label = format!("{width} spaces");
                            if style::accent_segment(
                                ui,
                                &pal,
                                &label,
                                self.prefs.tab_width == width,
                            )
                            .clicked()
                            {
                                self.prefs.tab_width = width;
                            }
                        }
                    });
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Code Colours").size(14.0).color(pal.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        style::pill(ui, &pal, |ui| {
                            if style::accent_segment(ui, &pal, "Dark", dark).clicked() {
                                self.prefs_dark = Some(true);
                            }
                            if style::accent_segment(ui, &pal, "Light", !dark).clicked() {
                                self.prefs_dark = Some(false);
                            }
                        });
                    });
                });
                ui.add_space(6.0);
                Frame::new()
                    .stroke(egui::Stroke::new(1.0, pal.separator))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(12, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        // The list scrolls, so the dialog keeps its height on a small screen.
                        egui::ScrollArea::vertical()
                            .id_salt("prefs-colours")
                            .max_height(260.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                let base = if dark { Palette::DARK } else { Palette::LIGHT };
                                let bg = [base.editor.r(), base.editor.g(), base.editor.b()];
                                let line = [
                                    base.current_line.r(),
                                    base.current_line.g(),
                                    base.current_line.b(),
                                ];
                                if let Some(v) = swatch_row(
                                    ui,
                                    &pal,
                                    "Background",
                                    self.prefs.background[dark as usize],
                                    bg,
                                ) {
                                    self.prefs.set_background(dark, v);
                                }
                                if let Some(v) = swatch_row(
                                    ui,
                                    &pal,
                                    "Current line",
                                    self.prefs.current_line[dark as usize],
                                    line,
                                ) {
                                    self.prefs.set_current_line(dark, v);
                                }
                                for (token, (kind, label)) in highlight::TOKENS.iter().enumerate() {
                                    let c = highlight::default_color(*kind, &base);
                                    let fallback = [c.r(), c.g(), c.b()];
                                    if let Some(v) = swatch_row(
                                        ui,
                                        &pal,
                                        label,
                                        self.prefs.colors[dark as usize][token],
                                        fallback,
                                    ) {
                                        self.prefs.set_color(dark, token, v);
                                    }
                                }
                            });
                    });
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Each appearance (View > Appearance) has its own colours and background.",
                )
                .size(12.0)
                .color(pal.text_tertiary),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if crate::style::secondary_button(ui, "Reset to Defaults").clicked() {
                    self.prefs = prefs::Prefs::default();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::style::primary_button(ui, "Done").clicked() {
                        done = true;
                    }
                });
            });
        });
        if done {
            self.preferences_open = false;
            self.prefs_dark = None;
        }
        let after = self.prefs.checked();
        if after != before {
            self.prefs = after;
            after.apply();
            after.save();
        }
    }

    /// The Rename Symbol box: the new name, and Rename to send it to the server.
    fn show_rename(&mut self, ctx: &egui::Context) {
        if !self.rename.open {
            return;
        }
        let mut go = false;
        let mut close = false;
        let title = format!("Rename “{}”", self.rename.old);
        crate::style::dialog(ctx, "rename-symbol", Some(&title), 300.0, |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.rename.name)
                    .id(egui::Id::unique("rename-symbol"))
                    .hint_text("New name")
                    .desired_width(220.0),
            );
            if self.rename.focus {
                field.request_focus();
                self.rename.focus = false;
            }
            if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            crate::style::buttons(ui, |ui| {
                if crate::style::primary_button(ui, "Rename").clicked() {
                    go = true;
                }
                if crate::style::secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
            if !self.rename.error.is_empty() {
                ui.label(
                    RichText::new(&self.rename.error).color(egui::Color32::from_rgb(200, 60, 50)),
                );
            }
        });
        if go {
            let name = self.rename.name.trim().to_string();
            if !formust_lsp::is_identifier(&name) {
                self.rename.error =
                    "A name is letters, digits and _, not starting with a digit".into();
            } else {
                self.rename.name = name;
                self.send_rename();
            }
        } else if close {
            self.rename.open = false;
            self.rename.error.clear();
        }
    }

    /// Shows the places a symbol is used in the Search tab, grouped by file.
    pub(crate) fn show_references(&mut self, locs: Vec<formust_lsp::Location>) {
        let name = self
            .project_bar
            .title
            .strip_prefix("References to ")
            .unwrap_or_default()
            .to_string();
        let name_len = name.chars().count();
        let mut texts: std::collections::HashMap<PathBuf, String> =
            std::collections::HashMap::new();
        let mut files: Vec<find::FileHits> = Vec::new();
        for loc in locs {
            let text = texts.entry(loc.path.clone()).or_insert_with(|| {
                self.docs
                    .iter()
                    .find(|d| d.path == loc.path)
                    .map(|d| d.text.clone())
                    .or_else(|| std::fs::read_to_string(&loc.path).ok())
                    .unwrap_or_default()
            });
            let line_text = text
                .split('\n')
                .nth(loc.line as usize)
                .unwrap_or_default()
                .to_string();
            let line_start: usize = text
                .split('\n')
                .take(loc.line as usize)
                .map(|l| l.chars().count() + 1)
                .sum();
            let start = line_start + loc.col as usize;
            let hit = find::Hit {
                line: loc.line + 1,
                col: loc.col + 1,
                text: line_text,
                range: start..start + name_len,
            };
            match files.iter_mut().find(|f| f.path == loc.path) {
                Some(f) => f.hits.push(hit),
                None => files.push(find::FileHits {
                    path: loc.path.clone(),
                    hits: vec![hit],
                }),
            }
        }
        self.project_bar.results = files;
        self.project_bar.truncated = false;
        self.project_bar.searched = true;
        self.project_bar.note.clear();
    }

    /// Writes the rename's replacements: into open tabs (unsaved, as typing is), and into the files
    /// on disk. Nothing changes when an open file has changed since the rename was asked.
    pub(crate) fn apply_rename(&mut self, files: Vec<formust_lsp::FileEdit>, sent: &find::Sent) {
        let changed_since = sent
            .open
            .iter()
            .any(|(path, text)| self.docs.iter().any(|d| &d.path == path && &d.text != text));
        if changed_since {
            self.note("Nothing renamed: a file changed while the rename was being worked out");
            return;
        }
        let (mut places, mut count) = (0, 0);
        for file in files {
            let edits: Vec<((u32, u32), (u32, u32), String)> = file
                .edits
                .iter()
                .map(|e| (e.start, e.end, e.new_text.clone()))
                .collect();
            if let Some(d) = self.docs.iter_mut().find(|d| d.path == file.path) {
                d.text = find::apply_edits(&d.text, &edits);
            } else {
                let Ok(text) = std::fs::read_to_string(&file.path) else {
                    continue;
                };
                if let Err(e) = std::fs::write(&file.path, find::apply_edits(&text, &edits)) {
                    self.note(format!("Could not write {}: {e}", file.path.display()));
                    continue;
                }
            }
            places += file.edits.len();
            count += 1;
        }
        self.note(format!(
            "Renamed to {} in {count} files ({places} places)",
            sent.new
        ));
    }

    /// The Go to Line box (Cmd+L): a line, or a line and column, in the active file.
    fn show_go_to_line(&mut self, ctx: &egui::Context) {
        if !self.go_to.open {
            return;
        }
        let mut go = false;
        let mut close = false;
        crate::style::dialog(ctx, "go-to-line", Some("Go to Line"), 300.0, |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.go_to.text)
                    .id(egui::Id::unique("go-to-line"))
                    .hint_text("Line or line:column")
                    .desired_width(220.0),
            );
            if self.go_to.focus {
                field.request_focus();
                self.go_to.focus = false;
            }
            if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            crate::style::buttons(ui, |ui| {
                if crate::style::primary_button(ui, "Go").clicked() {
                    go = true;
                }
                if crate::style::secondary_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
            if !self.go_to.error.is_empty() {
                ui.label(
                    RichText::new(&self.go_to.error).color(egui::Color32::from_rgb(200, 60, 50)),
                );
            }
        });
        if go {
            let Some(doc) = self.docs.get(self.active) else {
                self.go_to.error = "Open a file first".into();
                return;
            };
            let lines = doc.text.split('\n').count() as u32;
            match find::parse_position(&self.go_to.text, lines) {
                Ok((line, col)) => {
                    let path = doc.path.clone();
                    self.go_to.open = false;
                    self.go_to.error.clear();
                    self.goto(ctx, &path, line, col);
                }
                Err(e) => self.go_to.error = e,
            }
        } else if close {
            self.go_to.open = false;
            self.go_to.error.clear();
        }
    }

    /// The project search (the navigator's Search tab): find across the project's files, and replace
    /// in them. Rows fill the sidebar's width; the results list takes the rest of its height.
    fn search_tab(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let (mut run, mut replace, mut undo) = (false, false, false);
        let can_undo = !self.project_undo.is_empty();
        let mut open: Option<(PathBuf, u32, u32)> = None;
        ui.add_space(4.0);
        let bar = &mut self.project_bar;
        let field = ui.add(
            egui::TextEdit::singleline(&mut bar.query)
                .id(egui::Id::unique("project-query"))
                .hint_text("Search the project")
                .desired_width(f32::INFINITY),
        );
        if bar.focus {
            field.request_focus();
            bar.focus = false;
        }
        if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            run = true;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui
                .selectable_label(bar.opts.case, "Aa")
                .on_hover_text("Match case")
                .clicked()
            {
                bar.opts.case = !bar.opts.case;
            }
            if ui.selectable_label(bar.opts.word, "Whole word").clicked() {
                bar.opts.word = !bar.opts.word;
            }
            if ui
                .selectable_label(bar.opts.regex, "Regex")
                .on_hover_text("The search is a regular expression")
                .clicked()
            {
                bar.opts.regex = !bar.opts.regex;
            }
            if ui.button("Search").clicked() {
                run = true;
            }
        });
        let status = if bar.searching {
            "Searching…".to_string()
        } else if !bar.title.is_empty() {
            bar.title.clone()
        } else if !bar.searched {
            String::new()
        } else if bar.results.is_empty() {
            "No results".to_string()
        } else {
            let n: usize = bar.results.iter().map(|f| f.hits.len()).sum();
            format!("{n} results in {} files", bar.results.len())
        };
        ui.label(RichText::new(status).size(12.0).color(pal.text_secondary));
        ui.add(
            egui::TextEdit::singleline(&mut bar.with)
                .id(egui::Id::unique("project-replace"))
                .hint_text("Replace with")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            let enabled = !bar.results.is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new("Replace All in Project"))
                .on_hover_text(
                    "Changes the files on disk, and the open tabs (save them to keep the change)",
                )
                .clicked()
            {
                replace = true;
            }
            if can_undo
                && ui
                    .button("Undo Replace")
                    .on_hover_text("Puts back the files the last Replace All changed")
                    .clicked()
            {
                undo = true;
            }
            if !bar.note.is_empty() {
                ui.label(
                    RichText::new(&bar.note)
                        .size(12.0)
                        .color(pal.text_secondary),
                );
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for file in &self.project_bar.results {
                    let shown = file
                        .path
                        .strip_prefix(&self.root)
                        .unwrap_or(&file.path)
                        .to_string_lossy()
                        .into_owned();
                    ui.label(RichText::new(format!("{shown}  ({})", file.hits.len())).strong());
                    for hit in &file.hits {
                        // One line per result: the text is cut to the row's width, with "...".
                        let font = egui::TextStyle::Monospace.resolve(ui.style());
                        let full =
                            format!("{:>5}:{:<4} {}", hit.line, hit.col, one_line(&hit.text));
                        let fitted = fit_one_line(ui, &full, &font, ui.available_width() - 12.0);
                        let row = ui.selectable_label(false, RichText::new(fitted).monospace());
                        if row.clicked() {
                            open = Some((file.path.clone(), hit.line, hit.col));
                        }
                    }
                }
                if self.project_bar.truncated {
                    ui.label(
                        RichText::new(format!(
                            "Only the first {} matches are listed",
                            find::PROJECT_LIMIT
                        ))
                        .color(pal.text_tertiary),
                    );
                }
            });
        if run {
            self.run_project_search();
        }
        if replace {
            self.replace_in_project();
        }
        if undo {
            self.undo_replace_in_project();
        }
        if let Some((path, line, col)) = open {
            let ctx = ui.ctx().clone();
            self.goto(&ctx, &path, line, col);
        }
    }

    /// The text of every project file the tree lists, with the unsaved text of open tabs.
    fn project_texts(&self) -> Vec<(PathBuf, String)> {
        let open: Vec<(PathBuf, String)> = self
            .docs
            .iter()
            .map(|d| (d.path.clone(), d.text.clone()))
            .collect();
        project_texts_of(&self.root, &open)
    }

    /// Searches the project on a worker thread: the files are read there, and the open tabs' text is
    /// used for their files. The results are shown by `poll_project_search` when they are ready.
    pub(crate) fn run_project_search(&mut self) {
        if self.project_search_rx.is_some() {
            return;
        }
        let root = self.root.clone();
        let open: Vec<(PathBuf, String)> = self
            .docs
            .iter()
            .map(|d| (d.path.clone(), d.text.clone()))
            .collect();
        let (query, opts) = (self.project_bar.query.clone(), self.project_bar.opts);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let texts = project_texts_of(&root, &open);
            let hits = find::project_hits(
                texts.iter().map(|(p, t)| (p.clone(), t.as_str())),
                &query,
                opts,
                find::PROJECT_LIMIT,
            );
            let _ = tx.send(hits);
        });
        self.project_search_rx = Some(rx);
        self.project_bar.searching = true;
        self.project_bar.note.clear();
    }

    /// Shows the results of a project search once its worker has sent them.
    pub(crate) fn poll_project_search(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.project_search_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(hits) => {
                self.project_search_rx = None;
                self.show_project_hits(hits);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.project_search_rx = None;
                self.project_bar.searching = false;
            }
        }
    }

    /// Waits for a running project search and shows its results (for tests).
    #[cfg(test)]
    pub(crate) fn finish_project_search(&mut self) {
        if let Some(rx) = self.project_search_rx.take() {
            if let Ok(hits) = rx.recv() {
                self.show_project_hits(hits);
            }
        }
        self.project_bar.searching = false;
    }

    fn show_project_hits(&mut self, (results, truncated): (Vec<find::FileHits>, bool)) {
        let bar = &mut self.project_bar;
        bar.results = results;
        bar.title.clear();
        bar.truncated = truncated;
        bar.searched = true;
        bar.searching = false;
    }

    /// Replaces every match in the project. An open tab takes the change in its text (unsaved, as
    /// typing is); other files are written.
    fn replace_in_project(&mut self) {
        let texts = self.project_texts();
        let (query, with, opts) = (
            self.project_bar.query.clone(),
            self.project_bar.with.clone(),
            self.project_bar.opts,
        );
        let before: std::collections::HashMap<PathBuf, String> = texts.iter().cloned().collect();
        let changed = find::replace_in_files(texts, &query, &with, opts);
        let (mut total, mut files) = (0, 0);
        let mut undo = Vec::new();
        for (path, text, n) in changed {
            let old = before.get(&path).cloned().unwrap_or_default();
            if let Some(d) = self.docs.iter_mut().find(|d| d.path == path) {
                d.text = text.clone();
            } else if let Err(e) = std::fs::write(&path, &text) {
                self.note(format!("Could not write {}: {e}", path.display()));
                continue;
            }
            undo.push(find::Change {
                path,
                before: old,
                after: text,
            });
            total += n;
            files += 1;
        }
        // Only the last replace can be undone.
        self.project_undo = undo;
        self.run_project_search();
        self.project_bar.note = format!("Replaced {total} in {files} files");
    }

    /// Undoes the last Replace All in Project. A file that no longer holds the replaced text (it was
    /// edited since) is left alone, and the bar says how many.
    fn undo_replace_in_project(&mut self) {
        let changes = std::mem::take(&mut self.project_undo);
        let (mut restored, mut kept) = (0, 0);
        for c in changes {
            let now = match self.docs.iter().find(|d| d.path == c.path) {
                Some(d) => Some(d.text.clone()),
                None => std::fs::read_to_string(&c.path).ok(),
            };
            if now.as_deref() != Some(c.after.as_str()) {
                kept += 1;
                continue;
            }
            if let Some(d) = self.docs.iter_mut().find(|d| d.path == c.path) {
                d.text = c.before;
            } else if let Err(e) = std::fs::write(&c.path, &c.before) {
                self.note(format!("Could not write {}: {e}", c.path.display()));
                kept += 1;
                continue;
            }
            restored += 1;
        }
        self.run_project_search();
        self.project_bar.note = if kept == 0 {
            format!("Undid the replace in {restored} files")
        } else {
            format!("Undid the replace in {restored} files; {kept} changed since, left as they are")
        };
    }

    fn bottom_tabs(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let problems = if self.diagnostics.is_empty() {
            "Problems".to_string()
        } else {
            format!("Problems ({})", self.diagnostics.len())
        };
        let names = [problems.as_str(), "Output", "Terminal"];
        let tab = self.bottom.tab;
        let exited = self.bottom.terminal.as_ref().is_some_and(|t| t.exited());
        let (mut clear, mut restart, mut clear_terminal) = (false, false, false);
        let icons = ["alert-triangle", "cpu", "terminal"];
        let clicked =
            style::tab_bar(
                ui,
                pal,
                &names,
                &icons,
                tab.index(),
                pal.editor,
                |ui| match tab {
                    BottomTab::Output => {
                        clear = text_button(ui, pal, "Clear").clicked();
                    }
                    BottomTab::Terminal => {
                        if exited {
                            restart = text_button(ui, pal, "Restart").clicked();
                        } else {
                            clear_terminal = text_button(ui, pal, "Clear terminal").clicked();
                        }
                    }
                    _ => {}
                },
            );
        if let Some(i) = clicked {
            let now = ui.input(|i| i.time);
            self.bottom.choose(BottomTab::ALL[i], now);
        }
        if clear {
            self.console.clear();
        }
        if clear_terminal && let Some(t) = self.bottom.terminal.as_ref() {
            terminal_view::clear(t);
        }
        if restart {
            // Dropping the old terminal hangs up its shell.
            self.bottom.terminal = None;
            self.bottom.terminal_error = None;
            self.bottom.view_shown = false;
        }
    }

    fn problems_tab(&mut self, ui: &mut egui::Ui, pal: &Palette, p: &mut Pending) {
        Frame::new()
            .fill(pal.editor)
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| problem_rows(ui, pal, &self.diagnostics, p));
            });
    }

    fn output_tab(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        Frame::new()
            .fill(pal.editor)
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.console)
                                    .monospace()
                                    .size(12.0)
                                    .color(pal.text),
                            )
                            .selectable(true),
                        );
                    });
            });
    }

    /// The Terminal pane's shell tabs: one per shell (click to show it, its × closes it) and New Shell. A
    /// click takes effect before the shell is drawn, so the new one asks for focus in the same frame.
    fn shell_strip(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let shells = self.bottom.shells();
        let (mut show, mut close, mut new) = (None, None, false);
        Frame::new()
            .fill(pal.chrome)
            .inner_margin(Margin::symmetric(8, 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    for (n, shown) in &shells {
                        if ui.selectable_label(*shown, format!("Shell {n}")).clicked() {
                            show = Some(*n);
                        }
                        if shells.len() > 1
                            && text_button(ui, pal, "\u{d7}")
                                .on_hover_text(format!("Close Shell {n}"))
                                .clicked()
                        {
                            close = Some(*n);
                        }
                        ui.add_space(6.0);
                    }
                    new = text_button(ui, pal, "+ New Shell").clicked();
                });
            });
        if let Some(n) = close {
            self.bottom.close_shell(n);
        } else if let Some(n) = show {
            self.bottom.show_shell(n);
        }
        if new {
            self.bottom.new_shell();
        }
    }

    fn terminal_tab(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        self.shell_strip(ui, pal);
        let just_shown = !self.bottom.view_shown;
        self.bottom.view_shown = true;
        // The project folder changed (another file or project was opened): a shell nobody has
        // typed in yet simply starts again in the new folder; one in use is left alone and
        // offers a button instead (see below), so a running program is never disturbed.
        if self
            .bottom
            .terminal_root
            .as_deref()
            .is_some_and(|r| r != self.root)
            && self
                .bottom
                .terminal
                .as_ref()
                .is_some_and(|t| t.exited() || !t.typed())
        {
            self.bottom.terminal = None;
            self.bottom.terminal_error = None;
        }
        if self.bottom.terminal.is_none() && self.bottom.terminal_error.is_none() {
            // Started the first time the tab is shown, at the size the pane has now.
            let (rows, cols) = terminal_view::fit(ui);
            let ctx = ui.ctx().clone();
            match Terminal::spawn_shell(&self.root, rows, cols, move || ctx.request_repaint()) {
                Ok(t) => {
                    self.bottom.terminal = Some(t);
                    self.bottom.terminal_root = Some(self.root.clone());
                }
                Err(e) => {
                    self.bottom.terminal_error = Some(format!("could not start a shell: {e}"))
                }
            }
        }
        // A shell in use, started in another folder: say so, and let the user move it.
        if let (Some(started_in), Some(term)) = (
            self.bottom.terminal_root.clone(),
            self.bottom.terminal.as_ref(),
        ) && started_in != self.root
            && !term.exited()
        {
            let mut go = false;
            Frame::new()
                .fill(pal.chrome)
                .inner_margin(Margin::symmetric(10, 3))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "The project folder is now {}.",
                                self.root.display()
                            ))
                            .size(12.0)
                            .color(pal.text_secondary),
                        );
                        go = text_button(ui, pal, "cd to project folder").clicked();
                    });
                });
            if go {
                // Only on this click: the shell is idle at a prompt as far as the user is
                // concerned, and the command is visible in the terminal.
                let quoted = self.root.display().to_string().replace('\'', "'\\''");
                term.write(format!("cd '{quoted}'\n").as_bytes());
                self.bottom.terminal_root = Some(self.root.clone());
            }
        }
        if let Some(err) = &self.bottom.terminal_error {
            ui.label(RichText::new(err).size(12.5).color(pal.error));
            self.bottom.terminal_focused = false;
            return;
        }
        let Some(term) = self.bottom.terminal.as_mut() else {
            return;
        };
        self.bottom.terminal_focused =
            terminal_view::show(ui, pal, term, &mut self.bottom.view, just_shown);
    }
}

/// A flat text button for the tab strip's right end.
/// The editor's find bar: the query, the match count, previous/next, case and whole-word options,
/// and (when shown) the replacement row. Returns what the user asked for; `code_editor` acts on it.
fn find_bar(
    ui: &mut egui::Ui,
    pal: &Palette,
    bar: &mut find::Bar,
    found: usize,
) -> Option<FindAction> {
    let mut action = None;
    let mut close = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let field = ui.add(
            egui::TextEdit::singleline(&mut bar.query)
                .id(egui::Id::unique("find-query"))
                .hint_text("Find")
                .desired_width(220.0),
        );
        if bar.focus {
            field.request_focus();
            bar.focus = false;
        }
        if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = Some(FindAction::Go(!ui.input(|i| i.modifiers.shift)));
        }
        if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if field.changed() {
            bar.current = None;
        }
        let status = if bar.query.is_empty() {
            String::new()
        } else if found == 0 {
            "No results".to_string()
        } else {
            match bar.current {
                Some(i) if i < found => format!("{} of {found}", i + 1),
                _ => format!("{found} found"),
            }
        };
        ui.label(RichText::new(status).size(12.0).color(pal.text_secondary));
        if text_button(ui, pal, "‹")
            .on_hover_text("Previous match (Shift+Cmd+G)")
            .clicked()
        {
            action = Some(FindAction::Go(false));
        }
        if text_button(ui, pal, "›")
            .on_hover_text("Next match (Cmd+G)")
            .clicked()
        {
            action = Some(FindAction::Go(true));
        }
        let mut changed = false;
        if ui
            .selectable_label(bar.opts.case, "Aa")
            .on_hover_text("Match case")
            .clicked()
        {
            bar.opts.case = !bar.opts.case;
            changed = true;
        }
        if ui.selectable_label(bar.opts.word, "Whole word").clicked() {
            bar.opts.word = !bar.opts.word;
            changed = true;
        }
        if ui
            .selectable_label(bar.opts.regex, "Regex")
            .on_hover_text("The search is a regular expression")
            .clicked()
        {
            bar.opts.regex = !bar.opts.regex;
            changed = true;
        }
        if changed {
            bar.current = None;
        }
        if ui.selectable_label(bar.replace, "Replace").clicked() {
            bar.replace = !bar.replace;
        }
        if text_button(ui, pal, "×")
            .on_hover_text("Close the find bar (Esc)")
            .clicked()
        {
            close = true;
        }
    });
    if bar.replace {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(
                egui::TextEdit::singleline(&mut bar.with)
                    .id(egui::Id::unique("find-replace"))
                    .hint_text("Replace with")
                    .desired_width(220.0),
            );
            if ui.button("Replace").clicked() {
                action = Some(FindAction::ReplaceOne);
            }
            if ui.button("Replace All").clicked() {
                action = Some(FindAction::ReplaceAll);
            }
            if !bar.note.is_empty() {
                ui.label(
                    RichText::new(&bar.note)
                        .size(12.0)
                        .color(pal.text_secondary),
                );
            }
        });
    }
    if close {
        bar.open = false;
        bar.replace = false;
        bar.current = None;
    }
    action
}

/// What the find bar asked for.
enum FindAction {
    /// The next match (true) or the previous one (false).
    Go(bool),
    ReplaceOne,
    ReplaceAll,
}

/// Selects the characters `r` in the editor `id`.
fn select_in_editor(ctx: &egui::Context, id: egui::Id, r: &std::ops::Range<usize>) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(r.start),
        CCursor::new(r.end),
    )));
    state.store(ctx, id);
}

fn text_button(ui: &mut egui::Ui, pal: &Palette, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(pal.text_secondary))
            .frame(false)
            .min_size(vec2(0.0, 22.0)),
    )
}

/// One navigator row: disclosure triangle (folders), icon and name. Returns the response.
fn row(
    ui: &mut egui::Ui,
    pal: &Palette,
    depth: usize,
    is_dir: bool,
    open: bool,
    path: &Path,
    selected: bool,
    active: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
    // Every row can be dragged onto a folder; the payload is the item's path.
    resp.dnd_set_drag_payload(path.to_path_buf());
    let label = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    let painter = ui.painter();
    let inner = rect.shrink2(vec2(6.0, 0.0));
    // While another item is dragged over this row, show where it would land: a folder is outlined (the item
    // goes into it); a file gets an insertion line under its row (the item goes into the file's folder).
    let dragged_over = resp
        .dnd_hover_payload::<PathBuf>()
        .is_some_and(|from| from.as_path() != path);
    if selected || active {
        painter.rect_filled(inner, 5.0, pal.row_selected);
    } else if resp.hovered() || (is_dir && dragged_over) {
        painter.rect_filled(inner, 5.0, pal.row_hover);
    }
    // The open file is outlined in the accent colour and marked "Active", as in the navigator of Xcode.
    if active {
        painter.rect_stroke(
            inner,
            5.0,
            egui::Stroke::new(1.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    if dragged_over && is_dir {
        painter.rect_stroke(
            inner,
            5.0,
            egui::Stroke::new(1.5, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    if dragged_over && !is_dir {
        painter.line_segment(
            [inner.left_bottom(), inner.right_bottom()],
            egui::Stroke::new(2.0, pal.accent),
        );
    }
    let x = rect.left() + 12.0 + depth as f32 * 14.0;
    if is_dir {
        style::paint_disclosure(
            painter,
            egui::pos2(x + 4.0, rect.center().y),
            open,
            pal.text_tertiary,
        );
    }
    let icon_rect =
        egui::Rect::from_center_size(egui::pos2(x + 20.0, rect.center().y), vec2(16.0, 16.0));
    if is_dir {
        style::paint_folder(painter, icon_rect, pal);
    } else {
        style::paint_file(
            painter,
            icon_rect,
            path.extension().and_then(|e| e.to_str()).unwrap_or(""),
            pal,
            active,
        );
    }
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    painter.text(
        egui::pos2(x + 34.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(13.0),
        pal.text,
    );
    if active {
        let badge = egui::Rect::from_min_size(
            egui::pos2(inner.right() - 50.0, rect.center().y - 7.0),
            vec2(42.0, 14.0),
        );
        style::paint_badge(painter, badge, "Active", pal.accent);
    }
    resp
}

const HIDDEN_DIRS: &[&str] = crate::files::HIDDEN_DIRS;

/// What the project tree asked for while it was drawn; applied once drawing is done.
pub(crate) enum TreeAction {
    Select(PathBuf),
    /// A new Rust form in this folder.
    NewForm(PathBuf),
    NewIn {
        dir: PathBuf,
        is_dir: bool,
    },
    AskRename(PathBuf),
    RenameTo {
        path: PathBuf,
        name: String,
    },
    CancelRename,
    Create {
        dir: PathBuf,
        is_dir: bool,
        name: String,
    },
    CancelNew,
    AskDelete(PathBuf),
    ConfirmDelete(PathBuf),
    MoveInto {
        path: PathBuf,
        dir: PathBuf,
    },
}

/// A name being typed for a new file or folder: an editable row at the top of `dir`.
pub(crate) struct TreeNew {
    dir: PathBuf,
    is_dir: bool,
    name: String,
    focus: bool,
}

/// An item being renamed: its name field replaces its row.
pub(crate) struct TreeRename {
    path: PathBuf,
    name: String,
    focus: bool,
}

#[cfg(test)]
impl TreeRename {
    /// An item being renamed, with its current name in the field.
    pub(crate) fn new(path: PathBuf, name: &str) -> Self {
        Self {
            path,
            name: name.to_string(),
            focus: false,
        }
    }
}

#[cfg(test)]
impl TreeNew {
    /// A new item being named in `dir`, with the field not yet focused.
    pub(crate) fn new(dir: PathBuf, is_dir: bool) -> Self {
        Self {
            dir,
            is_dir,
            name: String::new(),
            focus: false,
        }
    }
}

/// The state the project tree reads and writes while it is drawn.
struct TreeView<'a> {
    active: Option<&'a Path>,
    sel: Option<&'a Path>,
    expanded: &'a HashSet<PathBuf>,
    new: &'a mut Option<TreeNew>,
    rename: &'a mut Option<TreeRename>,
    toggles: Vec<PathBuf>,
    actions: Vec<TreeAction>,
    open: Option<PathBuf>,
}

pub(crate) fn listing(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            !name.starts_with('.')
                && if p.is_dir() {
                    !HIDDEN_DIRS.contains(&name.as_str()) && !name.ends_with(".app")
                } else {
                    crate::files::is_listed(p)
                }
        })
        .collect();
    entries.sort_by_key(|p| {
        (
            !p.is_dir(),
            p.file_name().map(|n| n.to_string_lossy().to_lowercase()),
        )
    });
    entries
}

/// A line of source as one line of text: tabs and carriage returns become spaces, the ends are
/// trimmed.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\t' || c == '\r' { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// `text` cut to the longest prefix that, with "...", fits `max_width` points in `font` on one line.
/// Text that fits is returned unchanged.
pub(crate) fn fit_one_line(
    ui: &egui::Ui,
    text: &str,
    font: &egui::FontId,
    max_width: f32,
) -> String {
    let width = |s: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        })
    };
    if width(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "...";
        if width(&candidate) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "..."
}

/// The text of every file under `root` that a search reads: the open tabs' text where a tab is open,
/// else the file on disk (big and non-text files are left out).
fn project_texts_of(root: &Path, open: &[(PathBuf, String)]) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect_files(root, 0, &mut files);
    files
        .into_iter()
        .filter_map(|path| {
            if let Some((_, text)) = open.iter().find(|(p, _)| *p == path) {
                return Some((path, text.clone()));
            }
            // Big or non-text files are not searched.
            if std::fs::metadata(&path).ok()?.len() > 2_000_000 {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            Some((path, text))
        })
        .collect()
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    for p in listing(dir) {
        if p.is_dir() {
            if depth < 6 {
                collect_files(&p, depth + 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

/// The folder `dir` and, when expanded, everything below it.
fn tree_rows(
    ui: &mut egui::Ui,
    pal: &Palette,
    dir: &Path,
    depth: usize,
    is_root: bool,
    view: &mut TreeView,
) {
    let is_open = view.expanded.contains(dir);
    let resp = row(
        ui,
        pal,
        depth,
        true,
        is_open,
        dir,
        view.sel == Some(dir),
        false,
    );
    let renaming = view.rename.as_ref().is_some_and(|r| r.path == dir);
    if renaming {
        rename_overlay(ui, pal, resp.rect, depth, view);
    } else if resp.clicked() {
        view.toggles.push(dir.to_path_buf());
        view.actions.push(TreeAction::Select(dir.to_path_buf()));
    }
    if let Some(from) = resp.dnd_release_payload::<PathBuf>() {
        view.actions.push(TreeAction::MoveInto {
            path: (*from).clone(),
            dir: dir.to_path_buf(),
        });
    }
    context_menu(ui, &resp, |ui| {
        if ui.button("New File").clicked() {
            view.actions.push(TreeAction::NewIn {
                dir: dir.to_path_buf(),
                is_dir: false,
            });
            ui.close();
        }
        if ui.button("New Folder").clicked() {
            view.actions.push(TreeAction::NewIn {
                dir: dir.to_path_buf(),
                is_dir: true,
            });
            ui.close();
        }
        if ui.button("New Rust Form").clicked() {
            view.actions.push(TreeAction::NewForm(dir.to_path_buf()));
            ui.close();
        }
        if !is_root {
            if ui.button("Rename").clicked() {
                view.actions.push(TreeAction::AskRename(dir.to_path_buf()));
                ui.close();
            }
            ui.separator();
            if ui.button("Move to Trash").clicked() {
                view.actions.push(TreeAction::AskDelete(dir.to_path_buf()));
                ui.close();
            }
        }
    });
    if !is_open || depth > 8 {
        return;
    }
    if view.new.as_ref().is_some_and(|n| n.dir == dir) {
        new_name_row(ui, pal, depth + 1, view);
    }
    for p in listing(dir) {
        if p.is_dir() {
            tree_rows(ui, pal, &p, depth + 1, false, view);
        } else {
            let resp = row(
                ui,
                pal,
                depth + 1,
                false,
                false,
                &p,
                view.sel == Some(p.as_path()),
                view.active == Some(p.as_path()),
            );
            // Dropping onto a file moves the dragged item into the file's folder.
            if let Some(from) = resp.dnd_release_payload::<PathBuf>()
                && from.as_path() != p.as_path()
                && let Some(folder) = p.parent()
            {
                view.actions.push(TreeAction::MoveInto {
                    path: (*from).clone(),
                    dir: folder.to_path_buf(),
                });
            }
            let renaming = view.rename.as_ref().is_some_and(|r| r.path == p);
            if renaming {
                rename_overlay(ui, pal, resp.rect, depth + 1, view);
            } else if resp.clicked() {
                view.open = Some(p.clone());
                view.actions.push(TreeAction::Select(p.clone()));
            }
            context_menu(ui, &resp, |ui| {
                if ui.button("Rename").clicked() {
                    view.actions.push(TreeAction::AskRename(p.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Move to Trash").clicked() {
                    view.actions.push(TreeAction::AskDelete(p.clone()));
                    ui.close();
                }
            });
        }
    }
}

/// What an editable name field did this frame.
enum NameEdit {
    Submit(String),
    Cancel,
    Editing,
}

/// The area of a row where its name goes, at `depth`.
fn name_rect(row: Rect, depth: usize) -> Rect {
    let x = row.left() + 12.0 + depth as f32 * 14.0 + 34.0;
    Rect::from_min_max(
        egui::pos2(x, row.top() + 2.0),
        egui::pos2(row.right() - 8.0, row.bottom() - 2.0),
    )
}

/// A name field in `rect`. Enter or clicking away submits it (an empty name cancels); Escape
/// cancels.
fn name_field(
    ui: &mut egui::Ui,
    pal: &Palette,
    rect: Rect,
    name: &mut String,
    focus: &mut bool,
    hint: &str,
) -> NameEdit {
    let edit = ui.put(
        rect,
        egui::TextEdit::singleline(name)
            .hint_text(hint)
            .text_color(pal.text)
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(focus) {
        edit.request_focus();
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        NameEdit::Cancel
    } else if edit.lost_focus() {
        if name.trim().is_empty() {
            NameEdit::Cancel
        } else {
            NameEdit::Submit(name.clone())
        }
    } else {
        NameEdit::Editing
    }
}

/// The name field of a file or folder being created, as a row of its own at the top of `dir`.
fn new_name_row(ui: &mut egui::Ui, pal: &Palette, depth: usize, view: &mut TreeView) {
    let Some(new) = view.new.as_mut() else {
        return;
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let hint = if new.is_dir {
        "Folder name"
    } else {
        "File name"
    };
    match name_field(
        ui,
        pal,
        name_rect(rect, depth),
        &mut new.name,
        &mut new.focus,
        hint,
    ) {
        NameEdit::Submit(name) => view.actions.push(TreeAction::Create {
            dir: new.dir.clone(),
            is_dir: new.is_dir,
            name,
        }),
        NameEdit::Cancel => view.actions.push(TreeAction::CancelNew),
        NameEdit::Editing => {}
    }
}

/// The name field over the row of an item being renamed.
fn rename_overlay(ui: &mut egui::Ui, pal: &Palette, row: Rect, depth: usize, view: &mut TreeView) {
    let Some(rename) = view.rename.as_mut() else {
        return;
    };
    match name_field(
        ui,
        pal,
        name_rect(row, depth),
        &mut rename.name,
        &mut rename.focus,
        "Name",
    ) {
        NameEdit::Submit(name) => view.actions.push(TreeAction::RenameTo {
            path: rename.path.clone(),
            name,
        }),
        NameEdit::Cancel => view.actions.push(TreeAction::CancelRename),
        NameEdit::Editing => {}
    }
}

/// The yellow of warnings in the gutter and the Problems pane. The light palette's warning colour is
/// an amber brown, so light mode uses a clear yellow instead; dark mode already has one.
fn warning_yellow(pal: &Palette) -> egui::Color32 {
    if pal.dark {
        pal.warning
    } else {
        egui::Color32::from_rgb(0xF5, 0xB8, 0x00)
    }
}

fn problem_rows(ui: &mut egui::Ui, pal: &Palette, diagnostics: &[Diagnostic], p: &mut Pending) {
    if diagnostics.is_empty() {
        ui.label(
            RichText::new("No problems")
                .size(12.5)
                .color(pal.text_tertiary),
        );
        return;
    }
    for d in diagnostics {
        let (color, tag) = match d.severity {
            Severity::Fatal | Severity::Error => (pal.error, "Error"),
            Severity::Warning => (warning_yellow(pal), "Warning"),
            _ => (pal.text_tertiary, "Note"),
        };
        let name = d
            .file
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        // One allocated row takes the click, as the navigator's rows do: the marker, the position and
        // the message are painted into it. The message wraps to the panel's width.
        let width = ui.available_width();
        // A project-level problem (line 0, such as a missing package) has no position to show or go to.
        let place = if d.line == 0 {
            name.clone()
        } else {
            format!("{name}:{}:{}", d.line, d.col)
        };
        let position = ui.painter().layout_no_wrap(
            place,
            egui::FontId::proportional(12.5),
            pal.text_secondary,
        );
        let message = ui.painter().layout(
            format!("{tag}: {}", d.message),
            egui::FontId::proportional(12.5),
            pal.text,
            (width - 40.0 - position.size().x).max(80.0),
        );
        let height = message.size().y.max(position.size().y) + 4.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, pal.row_hover);
        }
        // The row is announced by its message (its marker and position are not text to read).
        let label = format!("{tag}: {}", d.message);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
        let mid = rect.top() + height / 2.0;
        ui.painter()
            .circle_filled(egui::pos2(rect.left() + 10.0, mid), 5.0, color);
        let at = egui::pos2(rect.left() + 22.0, mid - position.size().y / 2.0);
        ui.painter()
            .galley(at, position.clone(), pal.text_secondary);
        let at = egui::pos2(at.x + position.size().x + 8.0, mid - message.size().y / 2.0);
        ui.painter().galley(at, message, pal.text);
        if resp.clicked() && d.line > 0 {
            p.goto = Some((d.file.clone(), d.line, d.col));
        }
    }
}

/// Line numbers, problem chips and bookmarks. Returns the line of a bookmark whose chip was clicked.
fn gutter(
    ui: &mut egui::Ui,
    pal: &Palette,
    map: &LineMap,
    caret_line: Option<u32>,
    flagged: &std::collections::BTreeMap<u32, bool>,
    marks: &std::collections::BTreeMap<u32, usize>,
) -> Option<u32> {
    let size = vec2(GUTTER_W, map.height());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Line numbers"));
    let painter = ui.painter_at(rect);
    let clip = ui.clip_rect();
    let lh = crate::prefs::line_height();
    for line in 1..=map.lines() as u32 {
        if rect.top() + map.bottom(line) < clip.top() {
            continue;
        }
        if rect.top() + map.top(line) > clip.bottom() {
            break;
        }
        let y = rect.top() + map.top(line);
        let row = egui::Rect::from_min_size(
            egui::pos2(rect.left(), y),
            vec2(rect.width(), crate::prefs::line_height()),
        );
        let font = egui::FontId::monospace(11.0);
        if crate::prefs::line_numbers() {
            let color = if caret_line == Some(line) {
                pal.line_number_current
            } else {
                pal.line_number
            };
            painter.text(
                egui::pos2(row.right() - 12.0, row.center().y),
                egui::Align2::RIGHT_CENTER,
                line.to_string(),
                font,
                color,
            );
        }
    }
    // Wrapped lines: a small hook on each row after the first, so the row reads as part of the line above.
    for line in 1..=map.lines() as u32 {
        let rows = ((map.bottom(line) - map.top(line)) / lh).round() as u32;
        for k in 1..rows {
            let y = rect.top() + map.top(line) + k as f32 * lh + lh / 2.0;
            let x = rect.right() - 6.0;
            let stroke = egui::Stroke::new(1.0, pal.line_number);
            painter.line_segment([egui::pos2(x, y - 4.0), egui::pos2(x, y)], stroke);
            painter.line_segment([egui::pos2(x, y), egui::pos2(x + 4.0, y)], stroke);
        }
    }
    // Errors and warnings: an arrow-shaped chip, red for an error and yellow for a warning, with the line
    // number in it.
    for (&line, &error) in flagged {
        if line as usize > map.lines() {
            continue;
        }
        let y = rect.top() + map.top(line);
        let row = egui::Rect::from_min_size(egui::pos2(rect.left(), y), vec2(rect.width(), lh));
        let (l, r, t, b) = (
            row.left() + 5.0,
            row.right() - 3.0,
            row.top() + 2.0,
            row.bottom() - 2.0,
        );
        let (fill, text) = if error {
            (pal.error, egui::Color32::WHITE)
        } else {
            (warning_yellow(pal), egui::Color32::BLACK)
        };
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(l, t),
                egui::pos2(r - 7.0, t),
                egui::pos2(r, row.center().y),
                egui::pos2(r - 7.0, b),
                egui::pos2(l, b),
            ],
            fill,
            egui::Stroke::NONE,
        ));
        if crate::prefs::line_numbers() {
            painter.text(
                egui::pos2(r - 11.0, row.center().y),
                egui::Align2::RIGHT_CENTER,
                line.to_string(),
                egui::FontId::monospace(11.0),
                text,
            );
        }
    }
    // Bookmarks: a short green chip with the number, at the gutter's left edge, shaped like the other chips.
    for (&line, &n) in marks {
        if line as usize <= map.lines() {
            let y = rect.top() + map.top(line);
            let row = egui::Rect::from_min_size(egui::pos2(rect.left(), y), vec2(rect.width(), lh));
            let (l, r, t, b) = (
                row.left() + 1.0,
                row.left() + 15.0,
                row.top() + 2.0,
                row.bottom() - 2.0,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(l, t),
                    egui::pos2(r - 4.0, t),
                    egui::pos2(r, row.center().y),
                    egui::pos2(r - 4.0, b),
                    egui::pos2(l, b),
                ],
                egui::Color32::from_rgb(0x2E, 0xA0, 0x43),
                egui::Stroke::NONE,
            ));
            painter.text(
                egui::pos2(l + 5.0, row.center().y),
                egui::Align2::CENTER_CENTER,
                n.to_string(),
                egui::FontId::monospace(9.0),
                egui::Color32::WHITE,
            );
        }
    }
    // A click on a bookmark's chip (the green one at the gutter's left edge) unsets that bookmark.
    let pos = response
        .interact_pointer_pos()
        .filter(|_| response.clicked())?;
    let line = map.line_at(pos.y - rect.top());
    (pos.x - rect.left() < 16.0 && marks.contains_key(&line)).then_some(line)
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    /// A fresh project folder holding `debug.rs`.
    fn project(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("formust-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("debug.rs"), "fn main() {}\n").unwrap();
        d
    }

    #[test]
    fn a_new_file_is_named_then_created_and_opened() {
        let root = project("new");
        let mut app = App::bare(Some(root.join("debug.rs")));
        let mut p = Pending::default();
        app.apply_tree_actions(
            vec![TreeAction::NewIn {
                dir: root.clone(),
                is_dir: false,
            }],
            &mut p,
        );
        assert!(
            app.tree_new
                .as_ref()
                .is_some_and(|n| n.focus && n.dir == root)
        );
        app.apply_tree_actions(
            vec![TreeAction::Create {
                dir: root.clone(),
                is_dir: false,
                name: "unit2".into(),
            }],
            &mut p,
        );
        assert!(app.tree_new.is_none());
        assert!(root.join("unit2.rs").is_file());
        assert_eq!(p.open.as_deref(), Some(root.join("unit2.rs").as_path()));
        assert_eq!(
            app.tree_sel.as_deref(),
            Some(root.join("unit2.rs").as_path())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bad_name_is_reported_and_nothing_is_created() {
        let root = project("bad");
        let mut app = App::bare(Some(root.join("debug.rs")));
        let mut p = Pending::default();
        app.apply_tree_actions(
            vec![TreeAction::Create {
                dir: root.clone(),
                is_dir: false,
                name: "notes.exe".into(),
            }],
            &mut p,
        );
        assert!(!root.join("notes.exe").exists());
        assert!(p.open.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_folder_closes_its_tabs_and_forgets_its_selection() {
        // The app's root is canonical (`/private/var` on macOS): build paths from it.
        let project_dir = project("delete");
        let mut app = App::bare(Some(project_dir.join("debug.rs")));
        let root = app.root.clone();
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("inner.rs"), "mod inner;\n").unwrap();
        app.open_path(sub.join("inner.rs"));
        assert_eq!(app.docs.len(), 2);
        app.tree_sel = Some(sub.join("inner.rs"));

        let mut p = Pending::default();
        app.apply_tree_actions(vec![TreeAction::AskDelete(sub.clone())], &mut p);
        assert_eq!(app.tree_delete.as_deref(), Some(sub.as_path()));
        assert!(sub.exists(), "asking deletes nothing");

        app.apply_tree_actions(vec![TreeAction::ConfirmDelete(sub.clone())], &mut p);
        assert!(!sub.exists());
        assert!(app.tree_delete.is_none());
        assert_eq!(app.docs.len(), 1, "the tab of the deleted file closed");
        assert!(app.tree_sel.is_none());
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn the_project_folder_is_never_deleted() {
        let project_dir = project("root");
        let mut app = App::bare(Some(project_dir.join("debug.rs")));
        let root = app.root.clone();
        let mut p = Pending::default();
        app.apply_tree_actions(vec![TreeAction::ConfirmDelete(root.clone())], &mut p);
        assert!(root.join("debug.rs").exists());
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn renaming_an_open_file_keeps_its_tab_and_selection() {
        let project_dir = project("rename-open");
        let mut app = App::bare(Some(project_dir.join("debug.rs")));
        let root = app.root.clone();
        let mut p = Pending::default();
        app.apply_tree_actions(vec![TreeAction::AskRename(root.join("debug.rs"))], &mut p);
        assert!(
            app.tree_rename
                .as_ref()
                .is_some_and(|r| r.name == "debug.rs" && r.focus)
        );
        app.apply_tree_actions(
            vec![TreeAction::RenameTo {
                path: root.join("debug.rs"),
                name: "main".into(),
            }],
            &mut p,
        );
        assert!(app.tree_rename.is_none());
        assert!(root.join("main.rs").is_file() && !root.join("debug.rs").exists());
        assert_eq!(app.docs[0].path, root.join("main.rs"));
        assert_eq!(
            app.tree_sel.as_deref(),
            Some(root.join("main.rs").as_path())
        );
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn renaming_a_folder_moves_the_open_files_and_the_expansion_under_it() {
        let project_dir = project("rename-dir");
        let mut app = App::bare(Some(project_dir.join("debug.rs")));
        let root = app.root.clone();
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("inner.rs"), "mod inner;\n").unwrap();
        app.open_path(sub.join("inner.rs"));
        app.expanded.insert(sub.clone());
        let mut p = Pending::default();
        app.apply_tree_actions(
            vec![TreeAction::RenameTo {
                path: sub.clone(),
                name: "other".into(),
            }],
            &mut p,
        );
        assert!(root.join("other/inner.rs").is_file());
        assert_eq!(app.docs[1].path, root.join("other/inner.rs"));
        assert!(app.expanded.contains(&root.join("other")));
        assert!(!app.expanded.contains(&sub));
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn a_rename_changes_open_tabs_and_files_on_disk_and_is_refused_after_an_edit() {
        let root = project("rename-apply");
        std::fs::write(root.join("other.rs"), "total = 1;\n").unwrap();
        let mut app = App::bare(Some(root.join("debug.rs")));
        let root = app.root.clone();
        app.docs[0].text =
            "fn debug() {\n    total = 1;\n    println!(\"ok\");\n    total = 2;\n}\n".into();
        let sent = find::Sent {
            new: "sum".into(),
            open: vec![(root.join("debug.rs"), app.docs[0].text.clone())],
        };
        let edits = || {
            vec![
                formust_lsp::FileEdit {
                    path: root.join("debug.rs"),
                    edits: vec![
                        formust_lsp::TextEdit {
                            start: (1, 4),
                            end: (1, 9),
                            new_text: "sum".into(),
                        },
                        formust_lsp::TextEdit {
                            start: (3, 4),
                            end: (3, 9),
                            new_text: "sum".into(),
                        },
                    ],
                },
                formust_lsp::FileEdit {
                    path: root.join("other.rs"),
                    edits: vec![formust_lsp::TextEdit {
                        start: (0, 0),
                        end: (0, 5),
                        new_text: "sum".into(),
                    }],
                },
            ]
        };
        app.apply_rename(edits(), &sent);
        assert_eq!(
            app.docs[0].text,
            "fn debug() {\n    sum = 1;\n    println!(\"ok\");\n    sum = 2;\n}\n",
            "the open tab takes it, unsaved"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("debug.rs")).unwrap(),
            "fn main() {}\n",
            "the disk is left for the save"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("other.rs")).unwrap(),
            "sum = 1;\n"
        );
        assert_eq!(
            app.notice().map(|(m, _)| m.to_string()).as_deref(),
            Some("Renamed to sum in 2 files (3 places)")
        );

        // An open file edited after the rename was asked: nothing is renamed.
        app.docs[0].text.push_str("typed\n");
        let before = app.docs[0].text.clone();
        app.apply_rename(
            edits(),
            &find::Sent {
                new: "again".into(),
                ..sent
            },
        );
        assert_eq!(app.docs[0].text, before);
        assert_eq!(
            std::fs::read_to_string(root.join("other.rs")).unwrap(),
            "sum = 1;\n",
            "not renamed twice"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn references_list_each_use_with_its_line_and_open_the_search_tab() {
        let root = project("references");
        std::fs::write(
            root.join("other.rs"),
            "mod other;\nuse debug;\n  total = total + 1;\n",
        )
        .unwrap();
        let mut app = App::bare(Some(root.join("debug.rs")));
        let root = app.root.clone();
        app.docs[0].text = "let total = 0;\n// reset\n    total = 2;\n".into();
        app.project_bar.title = "References to total".into();
        let locs = vec![
            formust_lsp::Location {
                path: root.join("debug.rs"),
                line: 0,
                col: 4,
            },
            formust_lsp::Location {
                path: root.join("debug.rs"),
                line: 2,
                col: 4,
            },
            formust_lsp::Location {
                path: root.join("other.rs"),
                line: 2,
                col: 2,
            },
        ];
        app.show_references(locs);
        let files: Vec<(String, usize)> = app
            .project_bar
            .results
            .iter()
            .map(|f| {
                (
                    f.path.file_name().unwrap().to_string_lossy().into_owned(),
                    f.hits.len(),
                )
            })
            .collect();
        assert_eq!(
            files,
            [("debug.rs".to_string(), 2), ("other.rs".to_string(), 1)]
        );
        let hit = &app.project_bar.results[0].hits[1];
        assert_eq!((hit.line, hit.col, hit.text.trim()), (3, 5, "total = 2;"));
        assert_eq!(
            app.project_bar.results[1].hits[0].text.trim(),
            "total = total + 1;"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn project_search_sees_unsaved_text_and_replace_all_changes_tabs_and_files() {
        let root = project("project-search");
        std::fs::write(root.join("a.rs"), "foo\nfoo\n").unwrap();
        std::fs::write(root.join("b.txt"), "foo here\n").unwrap();
        let mut app = App::bare(Some(root.join("a.rs")));
        let root = app.root.clone();
        // The open tab has unsaved text: the search must see it, not the disk.
        app.docs[app.active].text = "foo foo foo".to_string();
        app.project_bar.query = "foo".into();
        app.project_bar.with = "bar".into();
        app.run_project_search();
        app.finish_project_search();
        let total: usize = app.project_bar.results.iter().map(|f| f.hits.len()).sum();
        assert_eq!(total, 4, "three in the tab's unsaved text, one in b.txt");
        app.replace_in_project();
        assert_eq!(
            app.docs[app.active].text, "bar bar bar",
            "the tab takes the change, unsaved"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a.rs")).unwrap(),
            "foo\nfoo\n",
            "the disk is not written for an open tab"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("b.txt")).unwrap(),
            "bar here\n"
        );
        assert_eq!(app.project_bar.note, "Replaced 4 in 2 files");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_outline_keys_texts_and_names_declaration_kinds() {
        assert_eq!(text_key("let a = 1;"), text_key("let a = 1;"));
        assert_ne!(text_key("let a = 1;"), text_key("let a = 2;"));
        assert_eq!(kind_word(5), "class");
        assert_eq!(kind_word(8), "field");
        assert_eq!(kind_word(999), "symbol", "an unknown kind is still listed");
    }

    #[test]
    fn a_new_text_file_from_the_tree_is_created_and_opened() {
        let project_dir = project("new-text");
        let mut app = App::bare(Some(project_dir.join("debug.rs")));
        let root = app.root.clone();
        let mut p = Pending::default();
        app.apply_tree_actions(
            vec![TreeAction::Create {
                dir: root.clone(),
                is_dir: false,
                name: "notes.md".into(),
            }],
            &mut p,
        );
        assert!(root.join("notes.md").is_file(), "created");
        assert_eq!(
            p.open.as_deref(),
            Some(root.join("notes.md").as_path()),
            "and opened"
        );
        assert_eq!(
            app.tree_sel.as_deref(),
            Some(root.join("notes.md").as_path())
        );
        app.apply_tree_actions(
            vec![TreeAction::Create {
                dir: root.clone(),
                is_dir: false,
                name: "bin.o".into(),
            }],
            &mut p,
        );
        assert!(
            !root.join("bin.o").exists(),
            "a kind the tree does not create is refused"
        );
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn undo_replace_in_project_restores_the_files_and_leaves_later_edits_alone() {
        let base =
            std::env::temp_dir().join(format!("formust-undo-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.rs"), "fn a() { let foo = 1; }\n").unwrap();
        std::fs::write(base.join("b.txt"), "foo and foo\n").unwrap();
        std::fs::write(base.join("c.txt"), "foo\n").unwrap();
        let mut app = App::bare(Some(base.join("a.rs")));
        let root = app.root.clone();
        app.project_bar.query = "foo".into();
        app.project_bar.with = "bar".into();
        app.replace_in_project();
        assert_eq!(
            std::fs::read_to_string(root.join("b.txt")).unwrap(),
            "bar and bar\n"
        );
        assert_eq!(
            app.project_undo.len(),
            3,
            "every changed file is remembered"
        );

        // c.txt is edited after the replace: undo must not overwrite that edit.
        std::fs::write(root.join("c.txt"), "typed later\n").unwrap();
        app.undo_replace_in_project();
        assert_eq!(
            std::fs::read_to_string(root.join("b.txt")).unwrap(),
            "foo and foo\n",
            "restored"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a.rs")).unwrap(),
            "fn a() { let foo = 1; }\n",
            "restored"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("c.txt")).unwrap(),
            "typed later\n",
            "left alone"
        );
        assert!(
            app.project_bar.note.contains("1 changed since"),
            "{}",
            app.project_bar.note
        );
        assert!(app.project_undo.is_empty(), "one undo only");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn renaming_an_open_file_leaves_no_read_error_behind() {
        let base = std::env::temp_dir().join(format!("formust-rename-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let project = base.join("GuiApp1");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"GuiApp1\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
        let app = App::bare(Some(project.join("src/main.rs")));
        let root = app.root.clone();
        let test3 = root.join("test3.rs");
        std::fs::write(&test3, "mod test3;\n").unwrap();
        let mut app = App::bare(Some(test3.clone()));
        let mut p = Pending::default();
        app.apply_tree_actions(
            vec![TreeAction::RenameTo {
                path: test3.clone(),
                name: "test4.rs".into(),
            }],
            &mut p,
        );
        // What the file system reports for a rename: the old path and the new one.
        let batch = crate::watch::Batch {
            paths: [test3.clone(), root.join("test4.rs")].into_iter().collect(),
            rescan: false,
        };
        app.handle_external(None, &batch);
        let shown: Vec<String> = app
            .external
            .banners
            .values()
            .map(|b| format!("{b:?}"))
            .collect();
        let tabs: Vec<String> = app
            .docs
            .iter()
            .map(|d| d.path.display().to_string())
            .collect();
        assert!(shown.is_empty(), "banners {shown:?}; tabs {tabs:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dragging_an_open_file_into_a_folder_moves_its_tab_and_expands_the_folder() {
        let project_dir = project("move-open");
        let mut app = App::bare(Some(project_dir.join("debug.rs")));
        let root = app.root.clone();
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let mut p = Pending::default();
        app.apply_tree_actions(
            vec![TreeAction::MoveInto {
                path: root.join("debug.rs"),
                dir: sub.clone(),
            }],
            &mut p,
        );
        assert!(sub.join("debug.rs").is_file() && !root.join("debug.rs").exists());
        assert_eq!(app.docs[0].path, sub.join("debug.rs"));
        assert_eq!(
            app.tree_sel.as_deref(),
            Some(sub.join("debug.rs").as_path())
        );
        assert!(app.expanded.contains(&sub));
        // A refused move leaves the tab where it was and says so.
        app.apply_tree_actions(
            vec![TreeAction::MoveInto {
                path: sub.clone(),
                dir: sub.clone(),
            }],
            &mut p,
        );
        assert!(sub.is_dir());
        assert_eq!(app.docs[0].path, sub.join("debug.rs"));
        let _ = std::fs::remove_dir_all(&project_dir);
    }
}

/// A context menu that opens on a secondary click, and on Control-click too: a Mac without a right
/// button (or a trackpad set up that way) sends a Control-click, which egui reads as a plain click.
fn context_menu(ui: &egui::Ui, resp: &egui::Response, add: impl FnOnce(&mut egui::Ui)) {
    let control_click = resp.clicked() && ui.input(|i| i.modifiers.ctrl);
    let open = if control_click || resp.secondary_clicked() {
        Some(egui::SetOpenCommand::Bool(true))
    } else if resp.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let _ = egui::Popup::context_menu(resp).open_memory(open).show(add);
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn project_with_a_unit() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("formust-file-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("debug.rs"), "fn main() {}\n").unwrap();
        dir
    }

    #[test]
    fn the_file_menu_starts_new_files_and_folders_and_renames_the_selection() {
        let dir = project_with_a_unit();
        let mut app = App::bare(Some(dir.join("debug.rs")));
        let root = app.root.clone();
        let ctx = egui::Context::default();

        app.run_menu_list(&ctx, vec![MenuAction::NewFile]);
        assert!(
            app.tree_new
                .as_ref()
                .is_some_and(|n| !n.is_dir && n.dir == root),
            "New File names a file in the target folder"
        );
        app.tree_new = None;

        app.run_menu_list(&ctx, vec![MenuAction::NewFolder]);
        assert!(
            app.tree_new
                .as_ref()
                .is_some_and(|n| n.is_dir && n.dir == root),
            "New Folder names a folder"
        );
        app.tree_new = None;

        app.run_menu_list(&ctx, vec![MenuAction::RenameItem]);
        assert!(
            app.tree_rename.is_none(),
            "nothing is selected: Rename says so"
        );
        assert!(
            app.console.contains("Select a file or folder"),
            "{}",
            app.console
        );

        app.tree_sel = Some(root.join("debug.rs"));
        app.run_menu_list(&ctx, vec![MenuAction::RenameItem]);
        assert!(
            app.tree_rename
                .as_ref()
                .is_some_and(|r| r.path == root.join("debug.rs")),
            "Rename opens the box on the selection"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_project_clears_the_problems_and_build_errors() {
        let old = project_with_a_unit();
        let new = std::env::temp_dir().join(format!("formust-open-clears-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&new);
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("other.rs"), "fn other() {}\n").unwrap();
        let mut app = App::bare(Some(old.join("debug.rs")));
        app.diagnostics = vec![formust_build::Diagnostic {
            file: old.join("debug.rs"),
            line: 2,
            col: 1,
            severity: formust_build::Severity::Error,
            message: "boom".into(),
        }];
        app.restore(
            session::Session {
                root: Some(new.clone()),
                ..Default::default()
            },
            false,
        );
        assert_eq!(app.root, new);
        assert!(
            app.diagnostics.is_empty(),
            "the old project's problems are gone"
        );
        assert!(app.marks.is_empty());
        let _ = std::fs::remove_dir_all(&old);
        let _ = std::fs::remove_dir_all(&new);
    }

    #[test]
    fn a_control_click_opens_a_context_menu_and_a_plain_click_does_not() {
        let mut harness = Harness::new_ui(|ui| {
            let row = ui.button("Row");
            context_menu(ui, &row, |ui| {
                ui.label("Menu item");
            });
        });
        harness.run();
        harness.get_by_label("Row").click();
        harness.run();
        assert!(
            harness.query_by_label("Menu item").is_none(),
            "a plain click does not open the menu"
        );

        harness
            .get_by_label("Row")
            .click_modifiers(egui::Modifiers::CTRL);
        harness.run();
        assert!(
            harness.query_by_label("Menu item").is_some(),
            "Control-click opens the menu"
        );
    }

    #[test]
    fn a_secondary_click_still_opens_the_context_menu() {
        let mut harness = Harness::new_ui(|ui| {
            let row = ui.button("Row");
            context_menu(ui, &row, |ui| {
                ui.label("Menu item");
            });
        });
        harness.run();
        harness.get_by_label("Row").click_secondary();
        harness.run();
        assert!(harness.query_by_label("Menu item").is_some());
    }
}

/// One setting: its name on the left, the control on the right, and a rule under it.
fn pref_row(ui: &mut egui::Ui, pal: &Palette, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(13.0).color(pal.text_secondary));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(5.0);
}

/// A small switch with its caption to the right of it (right to left: the caption is added first).
fn toggle_caption(ui: &mut egui::Ui, pal: &Palette, on: &mut bool, caption: &str) {
    ui.label(RichText::new(caption).size(12.5).color(pal.text));
    if toggle(ui, pal, *on) {
        *on = !*on;
    }
}

/// A switch: a rounded track, lit in the accent colour when on. True when it is clicked.
fn toggle(ui: &mut egui::Ui, pal: &Palette, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 17.0), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 8.5, if on { pal.accent } else { pal.separator });
    let knob_x = if on {
        rect.right() - 8.5
    } else {
        rect.left() + 8.5
    };
    painter.circle_filled(
        egui::pos2(knob_x, rect.center().y),
        6.5,
        egui::Color32::WHITE,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A round step button of the font-size control. Disabled when the size is at its limit.
fn stepper(ui: &mut egui::Ui, pal: &Palette, label: &str, enabled: bool) -> bool {
    let color = if enabled { pal.text } else { pal.text_tertiary };
    let button = egui::Button::new(RichText::new(label).size(13.0).color(color))
        .min_size(vec2(22.0, 20.0))
        .fill(pal.row_hover);
    ui.add_enabled(enabled, button).clicked()
}

/// A colour row: its name, then the swatch, and a Reset when the colour is changed from the default.
/// `Some(v)` when the user picked `v` (`None` = back to the default); `None` when nothing changed.
fn swatch_row(
    ui: &mut egui::Ui,
    pal: &Palette,
    label: &str,
    value: Option<[u8; 3]>,
    fallback: [u8; 3],
) -> Option<Option<[u8; 3]>> {
    let mut change = None;
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.5).color(pal.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut rgb = value.unwrap_or(fallback);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                change = Some(Some(rgb));
            }
            if value.is_some() && ui.small_button("Reset").clicked() {
                change = Some(None);
            }
        });
    });
    ui.add_space(3.0);
    change
}
