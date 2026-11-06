//! The designer's screen: a bar with the form picker, undo and redo and the preview mode; the
//! palette and the outline on the left; the inspector (or "Use in code") on the right; the canvas
//! between them. Drags from the palette, the canvas and the outline drop on the canvas or the
//! outline with an indicator line (see [`edit`]). The conflict prompt and the difference view are
//! dialogs over it.
//!
//! What needs the rest of the IDE (saving, reading other tabs) is queued in [`Designer::actions`]
//! and run by the app after drawing.

use std::path::Path;

use super::align::{ALL_ALIGN, AlignAction};
use super::edit::{self, Item, Payload, Row};
use super::export::{self, Prompt};
use super::preview::{self, Options};
use super::viewport::{self, TITLE_BAR, View};
use super::{DesignForm, Designer, node_title, show_value};
use crate::style::{Palette, hline, pill, segment};
use eframe::egui::{self, Color32, Frame, Margin, Rect, RichText, Sense, Stroke, StrokeKind, vec2};
use formust_model::{
    ALIGN2, ALL_ANCHORS, Anchor, ContainerId, DefaultValue, EventId, Form, Node, NodeId, NodeKind,
    PropDef, PropKind, PropRole, PropValue, WidgetId, container_def, parse_vec2,
    parse_window_anchor, resize_axes, vec2_text, widget_def, window_anchor_text, wrapper_anchor,
};
use formust_rust::{Conflict, Origin};

/// The form's width on the canvas, and the least height it gets.
const FORM_WIDTH: f32 = 640.0;
const FORM_MIN_HEIGHT: f32 = 240.0;
/// A form of panels fills its window, so it gets a window-sized height.
const PANELS_HEIGHT: f32 = 480.0;
/// The narrowest preview the corner grip allows.
const MIN_WIDTH: f32 = 160.0;

/// What the designer asks the app to do after drawing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Export the edited forms (Cmd+S does the same through the tab's Save).
    Save {
        overwrite: bool,
    },
    /// The conflict prompt's answers.
    Keep(String),
    Reimport(String),
    /// Drop the edits and read the form file again.
    Reload,
    /// Turn extended import (`ImportOptions::extended`) on or off and read the file again.
    Extended(bool),
    ShowDiff(Prompt),
}

/// Draws the designer. `read` gives a file's current text (its tab's, else the disk's).
pub fn show(
    d: &mut Designer,
    ui: &mut egui::Ui,
    pal: &Palette,
    read: &dyn Fn(&Path) -> Option<String>,
) {
    top_bar(d, ui, pal);
    hline(ui, pal);
    let Some(loaded) = &d.loaded else { return };
    if let Some(e) = &loaded.error {
        message(ui, pal, e, pal.error);
        return;
    }
    if loaded.forms.is_empty() {
        message(
            ui,
            pal,
            "No forms in this file. A form is a method with a `ui: &mut egui::Ui` parameter, or `fn ui`/`fn update` of an `eframe::App`.",
            pal.text_secondary,
        );
        return;
    }
    d.drop = None;
    keyboard(d, ui.ctx());
    egui::Panel::left("designer-outline")
        .resizable(true)
        .default_size(230.0)
        .min_size(160.0)
        .show_separator_line(true)
        .frame(
            Frame::new()
                .fill(pal.sidebar)
                .inner_margin(Margin::symmetric(8, 8)),
        )
        .show(ui, |ui| {
            palette(d, ui, pal);
            ui.add_space(6.0);
            outline(d, ui, pal);
        });
    egui::Panel::right("designer-properties")
        .resizable(true)
        .default_size(290.0)
        .min_size(220.0)
        .show_separator_line(true)
        .frame(
            Frame::new()
                .fill(pal.sidebar)
                .inner_margin(Margin::symmetric(12, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                pill(ui, pal, |ui| {
                    let (usage, tabs) = (d.show_usage, d.tab_order);
                    if segment(ui, pal, "Inspector", !usage && !tabs).clicked() {
                        d.show_usage = false;
                        d.tab_order = false;
                    }
                    if segment(ui, pal, "Use in code", usage && !tabs).clicked() {
                        d.show_usage = true;
                        d.tab_order = false;
                    }
                    if segment(ui, pal, "Tab order", tabs).clicked() {
                        d.tab_order = true;
                    }
                });
            });
            ui.add_space(4.0);
            if d.tab_order {
                tab_order_panel(d, ui, pal);
            } else if d.show_usage {
                usage(d, ui, pal, read);
            } else if d.selection_len() > 1 {
                multi_inspector(d, ui, pal);
            } else {
                inspector(d, ui, pal);
            }
        });
    canvas_bar(d, ui, pal);
    canvas(d, ui, pal);
    finish_drag(d, ui.ctx());
    prompt_dialog(d, ui.ctx());
    diff_window(d, ui.ctx(), pal);
}

fn message(ui: &mut egui::Ui, pal: &Palette, text: &str, color: Color32) {
    Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
        ui.set_max_width(640.0);
        ui.label(RichText::new(text).color(color));
    });
    let _ = pal;
}

// ---- Keyboard ------------------------------------------------------------------------------

/// The canvas takes no keyboard focus (the shield must not be a Tab stop), so the designer's keys
/// are read here, while no text field has the focus and no dialog is open.
fn keyboard(d: &mut Designer, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    if ctx.text_edit_focused() || d.prompt.is_some() {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) {
        d.redo();
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
        d.undo();
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::D)) {
        d.duplicate_selected();
    }
    // Wrap in row (Cmd+J), Unwrap (Cmd+Shift+J), Fill (Cmd+E). Shift is ignored by a plain Cmd
    // match, so the longer one is asked first.
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::J)) {
        d.unwrap_selected();
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::J)) {
        d.wrap_selected();
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::E)) {
        d.toggle_fill();
    }
    if ctx.input_mut(|i| {
        i.consume_key(Modifiers::NONE, Key::Delete)
            || i.consume_key(Modifiers::NONE, Key::Backspace)
    }) {
        d.delete_selected();
    }
    for (key, delta) in [
        (Key::ArrowUp, -1),
        (Key::ArrowLeft, -1),
        (Key::ArrowDown, 1),
        (Key::ArrowRight, 1),
    ] {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            d.nudge(delta);
        }
    }
    // Copy, Cut and Paste arrive as events (from the Edit menu or the platform), not as keys.
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        match event {
            egui::Event::Copy => {
                if let Some(text) = d.copy_selected() {
                    ctx.copy_text(text);
                }
            }
            egui::Event::Cut => {
                if let Some(text) = d.cut_selected() {
                    ctx.copy_text(text);
                }
            }
            egui::Event::Paste(text) => {
                d.paste(&text);
            }
            _ => {}
        }
    }
}

// ---- Top bar -------------------------------------------------------------------------------

/// The form picker (when the file has several forms), where the form came from, undo and redo,
/// what happened last, and the preview's light or dark mode.
fn top_bar(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    Frame::new()
        .fill(pal.chrome)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let ids: Vec<String> = d.forms().iter().map(|f| f.form().id()).collect();
                let current = d.form_id.clone().unwrap_or_default();
                if ids.len() > 1 {
                    let mut chosen = current.clone();
                    let label = ui.label(RichText::new("Form").color(pal.text_secondary));
                    egui::ComboBox::from_id_salt("designer-form")
                        .selected_text(&chosen)
                        .show_ui(ui, |ui| {
                            for id in &ids {
                                ui.selectable_value(&mut chosen, id.clone(), id);
                            }
                        })
                        .response
                        .labelled_by(label.id);
                    if chosen != current {
                        d.choose_form(chosen);
                    }
                } else if !current.is_empty() {
                    ui.label(RichText::new(&current).strong().color(pal.text));
                }
                let dirty = d.is_dirty();
                if dirty {
                    ui.label(RichText::new("●").small().color(pal.text_secondary))
                        .on_hover_text(
                            "Unsaved designer edits: Save (Cmd+S) exports them to the form file",
                        );
                }
                let (can_undo, can_redo) = d.loaded.as_ref().map_or((false, false), |l| {
                    (l.history.can_undo(), l.history.can_redo())
                });
                if ui
                    .add_enabled(can_undo, egui::Button::new("Undo").small())
                    .on_hover_text("Undo (Cmd+Z)")
                    .clicked()
                {
                    d.undo();
                }
                if ui
                    .add_enabled(can_redo, egui::Button::new("Redo").small())
                    .on_hover_text("Redo (Cmd+Shift+Z)")
                    .clicked()
                {
                    d.redo();
                }
                if ui
                    .add_enabled(dirty, egui::Button::new("Save form").small())
                    .on_hover_text("Export to the .frm.rs (Cmd+S). The .rs is never written.")
                    .clicked()
                {
                    d.actions.push(Action::Save { overwrite: false });
                }
                if let Some(f) = d.current() {
                    if let Some(note) =
                        origin_note(f, d.loaded.as_ref().and_then(|l| l.companion.as_deref()))
                    {
                        ui.label(RichText::new(note).small().color(pal.text_secondary));
                    }
                    let id = f.source_id();
                    if let Some(c) = conflict_note(f) {
                        let changed = matches!(f.conflict, Some(Conflict::Changed { .. }));
                        ui.label(RichText::new(c).small().color(pal.warning));
                        if changed && ui.small_button("Resolve…").clicked() {
                            d.prompt = Some(Prompt::SourceChanged { form: id });
                        }
                    }
                }
                if d.loaded.as_ref().is_some_and(|l| l.stale) {
                    ui.label(
                        RichText::new(
                            "The file changed while you were editing; your edits are kept",
                        )
                        .small()
                        .color(pal.warning),
                    );
                }
                if let Some(s) = &d.status {
                    ui.label(RichText::new(s).small().color(pal.text_secondary));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut extended = d.extended;
                    let hover = if dirty {
                        "Save or undo the designer edits first: switching reads the file again"
                    } else {
                        "Also read if/for/while/loop/match holding widgets, and let x = <widget>;, as nodes instead of locked custom code. Reads the file again."
                    };
                    if ui
                        .add_enabled(!dirty, egui::Checkbox::new(&mut extended, "Extended import"))
                        .on_hover_text(hover)
                        .on_disabled_hover_text(hover)
                        .changed()
                    {
                        d.actions.push(Action::Extended(extended));
                    }
                    if let Some(l) = &d.loaded
                        && !l.notes.is_empty()
                    {
                        ui.label(
                            RichText::new(format!("{} note(s)", l.notes.len()))
                                .small()
                                .color(pal.warning),
                        )
                        .on_hover_text(l.notes.join("\n"));
                    }
                });
            });
        });
}

fn origin_note(f: &DesignForm, companion: Option<&std::path::Path>) -> Option<String> {
    let other = companion?.file_name()?.to_string_lossy().into_owned();
    let other_is_form = other.ends_with(formust_rust::FORM_SUFFIX);
    match f.origin? {
        Origin::Frm if other_is_form => Some(format!("from {other}")),
        Origin::Frm => None,
        Origin::Rs => Some(format!("from the source; {other} doesn't define it")),
    }
}

fn conflict_note(f: &DesignForm) -> Option<&'static str> {
    match f.conflict? {
        Conflict::Unchanged => None,
        Conflict::Changed { .. } => Some("The source has changed since this form was exported"),
        Conflict::MissingInSource => None,
        Conflict::NoHeader => Some("No Formust header: changes to the source can't be detected"),
    }
}

// ---- Palette -------------------------------------------------------------------------------

/// The palette: a filter box and the items by group (each group can be closed), in a scroll area
/// of at most 240 pt, so every item can be reached and the outline keeps its room.
/// A click adds the item into the selected container or after the selected widget; a drag drops it
/// where the indicator line shows.
fn palette(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("PALETTE")
                .size(10.5)
                .strong()
                .color(pal.text_tertiary),
        );
        // Right to left, the clear button first: the field takes exactly the room left, so the
        // row never overflows (a fixed allowance for the button made the resizable panel grow by
        // a few points every frame while a filter was typed).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !d.palette_filter.is_empty()
                && ui
                    .small_button("×")
                    .on_hover_text("Clear the filter")
                    .clicked()
            {
                d.palette_filter.clear();
            }
            let filter = egui::TextEdit::singleline(&mut d.palette_filter)
                .hint_text("Filter")
                .desired_width(ui.available_width());
            ui.add(filter)
                .on_hover_text("Show only the items whose name contains this");
        });
    });
    ui.add_space(4.0);
    let items: Vec<Item> = edit::palette()
        .into_iter()
        .filter(|i| edit::matches_filter(*i, &d.palette_filter))
        .collect();
    let filtering = !d.palette_filter.trim().is_empty();
    // At most 240 pt (or 40% of a short panel): the outline below keeps its room, and the
    // palette scrolls.
    let max_height = (ui.available_height() * 0.4).clamp(120.0, 240.0);
    egui::ScrollArea::vertical()
        .id_salt("designer-palette")
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if items.is_empty() {
                ui.label(
                    RichText::new("Nothing matches")
                        .small()
                        .color(pal.text_tertiary),
                );
            }
            for &group in edit::CATEGORIES {
                let in_group: Vec<Item> = items
                    .iter()
                    .copied()
                    .filter(|i| edit::category(*i) == group)
                    .collect();
                if in_group.is_empty() {
                    continue;
                }
                let closed = !filtering && d.palette_closed.contains(group);
                let arrow = if closed { "▸" } else { "▾" };
                let head = ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!("{arrow} {group}"))
                                .size(11.0)
                                .color(pal.text_secondary),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_text(if closed {
                        "Show this group"
                    } else {
                        "Hide this group"
                    });
                if head.clicked() && !filtering {
                    if closed {
                        d.palette_closed.remove(group);
                    } else {
                        d.palette_closed.insert(group);
                    }
                }
                if closed {
                    continue;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for item in in_group {
                        palette_button(d, ui, pal, item);
                    }
                });
                ui.add_space(4.0);
            }
        });
}

fn palette_button(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette, item: Item) {
    let is_container = matches!(item, Item::Container(_));
    let text = RichText::new(item.label()).size(11.5).color(pal.text);
    let fill = if is_container {
        pal.accent.gamma_multiply(0.18)
    } else {
        pal.editor
    };
    let button = egui::Button::new(text)
        .fill(fill)
        .stroke(Stroke::new(1.0, pal.separator))
        .corner_radius(4.0)
        .sense(Sense::click_and_drag());
    let r = ui.add(button).on_hover_text(format!(
        "{}: click to add, or drag onto the form or the outline",
        item.label()
    ));
    if r.drag_started() {
        d.drag = Some(Payload::New(item));
    } else if r.clicked() {
        d.add(item, None);
    }
}

// ---- Outline -------------------------------------------------------------------------------

fn outline(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    ui.label(
        RichText::new("OUTLINE")
            .size(10.5)
            .strong()
            .color(pal.text_tertiary),
    );
    ui.add_space(4.0);
    let Some(form) = d.current().map(|f| f.form().clone()) else {
        return;
    };
    let reveal = std::mem::take(&mut d.reveal_row);
    let also = d.also_selected();
    let menu = MenuState::of(d);
    let mut out_rows = RowOut::default();
    let cx = RowCx {
        pal,
        selected: d.selected,
        also: &also,
        reveal,
        menu: &menu,
    };
    let out = egui::ScrollArea::vertical()
        .id_salt("designer-outline-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            row(ui, &cx, &form.root, 0, None, &mut out_rows);
        });
    let RowOut {
        rows,
        clicked,
        dragged,
        action,
    } = out_rows;
    d.rows = rows;
    if let Some((id, shift, secondary, command)) = clicked {
        let in_selection = d.selected == Some(id) || also.contains(&id);
        if command {
            d.toggle_select(id, false);
        } else if shift {
            d.select_range(id, false);
        } else if !(secondary && in_selection) {
            d.select(Some(id), false);
        }
    }
    if let Some(id) = dragged
        && id != form.root.id
    {
        d.drag = Some(Payload::Move(form.anchor_host(id)));
    }
    if let Some(action) = action {
        run_menu_action(d, action);
    }
    // A drop over the outline: before, after or into the row under the pointer.
    if let Some(payload) = d.drag
        && let Some(pos) = ui.ctx().pointer_latest_pos()
        && out.inner_rect.contains(pos)
    {
        let moving = match payload {
            Payload::Move(id) => Some(id),
            Payload::New(_) => None,
        };
        d.drop = edit::outline_drop(&form, &d.rows, pos, moving);
        if let Some(drop) = d.drop {
            ui.painter()
                .line_segment(drop.line, Stroke::new(2.0, pal.accent));
        }
    }
}

/// What the outline rows need to draw.
struct RowCx<'a> {
    pal: &'a Palette,
    selected: Option<NodeId>,
    /// The rest of a shift-click range.
    also: &'a [NodeId],
    reveal: bool,
    menu: &'a MenuState,
}

/// What the outline rows report: their rects, and a click (with shift, or a right-click), a drag
/// or a context menu choice.
#[derive(Default)]
struct RowOut {
    rows: Vec<Row>,
    /// The node, with shift, as a right-click, with Cmd.
    clicked: Option<(NodeId, bool, bool, bool)>,
    dragged: Option<NodeId>,
    action: Option<MenuAction>,
}

/// One row and its children. An anchor wrapper has no row: its node's row says the anchor.
fn row(
    ui: &mut egui::Ui,
    cx: &RowCx<'_>,
    n: &Node,
    depth: usize,
    anchor: Option<Anchor>,
    out: &mut RowOut,
) {
    if let Some(a) = wrapper_anchor(n) {
        row(ui, cx, &n.children()[0], depth, Some(a), out);
        return;
    }
    let pal = cx.pal;
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    out.rows.push(Row {
        id: n.id,
        rect,
        depth,
    });
    let is_sel = cx.selected == Some(n.id);
    let in_range = cx.also.contains(&n.id);
    if is_sel || in_range {
        let fill = if is_sel {
            pal.row_selected
        } else {
            pal.row_selected.gamma_multiply(0.6)
        };
        ui.painter().rect_filled(rect, 4.0, fill);
        if is_sel && cx.reveal {
            resp.scroll_to_me(Some(egui::Align::Center));
        }
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, pal.row_hover);
    }
    let custom = matches!(n.kind, NodeKind::Custom { .. });
    let mut title = node_title(n);
    if let Some(a) = anchor {
        title.push_str(&format!(" · anchor {}", a.label().to_lowercase()));
    }
    let x = rect.left() + 6.0 + depth as f32 * edit::ROW_INDENT;
    let color = if custom { pal.text_secondary } else { pal.text };
    let font = if custom {
        egui::FontId::monospace(11.5)
    } else {
        egui::FontId::proportional(12.5)
    };
    let painter = ui.painter_at(rect);
    let mut x2 = x;
    if custom {
        // A small padlock: custom code is locked.
        let lock = Rect::from_min_size(egui::pos2(x, rect.center().y - 5.0), vec2(8.0, 10.0));
        paint_lock(&painter, lock, pal.text_tertiary);
        x2 += 12.0;
    }
    painter.text(
        egui::pos2(x2, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title.clone(),
        font,
        color,
    );
    let hover = if custom {
        format!(
            "{title}\nLocked custom code: kept exactly as written. It can be moved, copied and deleted."
        )
    } else {
        title.clone()
    };
    let resp = resp.on_hover_text(hover);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, true, is_sel, &title));
    let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
    if resp.drag_started() {
        out.dragged = Some(n.id);
    } else if resp.clicked() {
        out.clicked = Some((n.id, shift, false, command));
    } else if resp.secondary_clicked() {
        out.clicked = Some((n.id, false, true, false));
    }
    resp.context_menu(|ui| node_menu(ui, cx.menu, &mut out.action));
    for c in n.children() {
        row(ui, cx, c, depth + 1, None, out);
    }
}

fn paint_lock(painter: &egui::Painter, r: Rect, color: Color32) {
    let body = Rect::from_min_max(egui::pos2(r.left(), r.top() + r.height() * 0.45), r.max);
    painter.rect_filled(body, 1.5, color);
    let c = egui::pos2(r.center().x, body.top());
    let rad = r.width() * 0.32;
    let pts: Vec<egui::Pos2> = (0..=8)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 8.0);
            c + vec2(a.cos() * rad, a.sin() * rad * 1.3)
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.3, color)));
}

// ---- Layout actions ------------------------------------------------------------------------

/// A layout action from a context menu or the inspector, on the selection.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuAction {
    WrapRow,
    Unwrap,
    Fill(bool),
    Anchor(Option<Anchor>),
    ResetSize,
    Duplicate,
    Delete,
    Align(AlignAction),
}

/// What the layout actions can do for the selection, read before drawing.
#[derive(Clone, Debug, Default)]
struct MenuState {
    any: bool,
    fill: Option<bool>,
    anchor: Option<Anchor>,
    can_anchor: bool,
    can_wrap: bool,
    can_unwrap: bool,
    resizable: bool,
    range: usize,
    /// The align actions that can run, with what each writes.
    align: Vec<(AlignAction, bool, String)>,
}

impl MenuState {
    fn of(d: &Designer) -> MenuState {
        let Some(form) = d.current().map(|f| f.form()) else {
            return MenuState::default();
        };
        let Some(node) = d.selected.and_then(|s| form.find(s)) else {
            return MenuState::default();
        };
        let (anchor, can_anchor) = d.selected_anchor();
        let (w, h) = resize_axes(node);
        MenuState {
            any: node.id != form.root.id,
            fill: d.selected_fill(),
            anchor,
            can_anchor,
            can_wrap: d.can_wrap_selection(),
            can_unwrap: d.unwrap_target().is_some(),
            resizable: w || h,
            range: d.selection_hosts().len(),
            align: ALL_ALIGN
                .iter()
                .map(|a| (*a, d.can_align(*a), a.writes(&form.egui_path)))
                .collect(),
        }
    }
}

/// The context menu of a node (canvas and outline): arrange, Fill, anchor, size, copy and delete.
fn node_menu(ui: &mut egui::Ui, m: &MenuState, out: &mut Option<MenuAction>) {
    if !m.any {
        ui.label("The form itself");
        return;
    }
    fn pick(
        ui: &mut egui::Ui,
        on: bool,
        label: &str,
        shortcut: &str,
        a: MenuAction,
        out: &mut Option<MenuAction>,
    ) {
        let button = egui::Button::new(label).shortcut_text(shortcut);
        if ui.add_enabled(on, button).clicked() {
            *out = Some(a);
            ui.close();
        }
    }
    let wrap = if m.range > 1 {
        format!("Wrap {} in row", m.range)
    } else {
        "Wrap in row".to_string()
    };
    pick(ui, m.can_wrap, &wrap, "⌘J", MenuAction::WrapRow, out);
    pick(ui, m.can_unwrap, "Unwrap", "⇧⌘J", MenuAction::Unwrap, out);
    ui.menu_button("Align", |ui| {
        for (k, (a, on, writes)) in m.align.iter().enumerate() {
            if k == 7 || k == 9 {
                ui.separator();
            }
            let r = ui
                .add_enabled(*on, egui::Button::new(a.label()))
                .on_hover_text(writes)
                .on_disabled_hover_text(writes);
            if r.clicked() {
                *out = Some(MenuAction::Align(*a));
                ui.close();
            }
        }
    });
    ui.separator();
    if let Some(fill) = m.fill {
        let label = if fill { "✔ Fill width" } else { "Fill width" };
        pick(ui, true, label, "⌘E", MenuAction::Fill(!fill), out);
    }
    ui.add_enabled_ui(m.can_anchor, |ui| {
        ui.menu_button("Anchor", |ui| {
            let mut choice = |ui: &mut egui::Ui, a: Option<Anchor>, label: &str| {
                if ui.selectable_label(m.anchor == a, label).clicked() {
                    *out = Some(MenuAction::Anchor(a));
                    ui.close();
                }
            };
            choice(ui, None, "None (normal flow)");
            for &a in ALL_ANCHORS {
                choice(ui, Some(a), a.label());
            }
        });
    });
    pick(
        ui,
        m.resizable,
        "Reset size",
        "",
        MenuAction::ResetSize,
        out,
    );
    ui.separator();
    pick(ui, true, "Duplicate", "⌘D", MenuAction::Duplicate, out);
    pick(ui, true, "Delete", "⌫", MenuAction::Delete, out);
}

fn run_menu_action(d: &mut Designer, a: MenuAction) {
    match a {
        MenuAction::WrapRow => {
            d.wrap_selected();
        }
        MenuAction::Unwrap => {
            d.unwrap_selected();
        }
        MenuAction::Fill(on) => {
            d.set_fill(on);
        }
        MenuAction::Anchor(anchor) => {
            d.set_anchor(anchor);
        }
        MenuAction::ResetSize => {
            d.reset_size();
        }
        MenuAction::Duplicate => {
            d.duplicate_selected();
        }
        MenuAction::Delete => {
            d.delete_selected();
        }
        MenuAction::Align(a) => {
            d.align(a);
        }
    }
}

// ---- Drag and drop -------------------------------------------------------------------------

/// The end of a drag: on release, the drop is applied where the indicator showed; Escape cancels.
/// While dragging, the item's name follows the pointer.
fn finish_drag(d: &mut Designer, ctx: &egui::Context) {
    let Some(payload) = d.drag else { return };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        d.drag = None;
        d.drop = None;
        return;
    }
    if let Some(pos) = ctx.pointer_latest_pos() {
        let label = match payload {
            Payload::New(item) => item.label().to_string(),
            Payload::Move(id) => d
                .current()
                .and_then(|f| f.form().find(id))
                .map_or_else(String::new, node_title),
        };
        egui::Area::new(egui::Id::unique("designer-drag-label"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + vec2(12.0, 8.0))
            .interactable(false)
            .show(ctx, |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(super::one_line(&label, 40));
                });
            });
    }
    if !ctx.input(|i| i.pointer.any_released() || !i.pointer.any_down()) {
        return;
    }
    d.drag = None;
    let Some(drop) = d.drop.take() else { return };
    match payload {
        Payload::New(item) => {
            d.add(item, Some((drop.parent, drop.index)));
        }
        Payload::Move(id) => {
            if d.selection_len() > 1 && d.selection_hosts().contains(&id) {
                d.move_selection_to(drop.parent, drop.index);
            } else {
                d.move_to(id, drop.parent, drop.index);
            }
        }
    }
}

/// A rubber band on the canvas: while the pointer is down, a rectangle from where the drag began;
/// on release, the nodes entirely inside it are selected (the outermost ones, not the form).
fn rubber_band(
    d: &mut Designer,
    ui: &egui::Ui,
    pal: &Palette,
    form: &Form,
    layer: egui::LayerId,
    area: Rect,
) {
    let Some(origin) = d.band else { return };
    let (latest, down) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_down()));
    let Some(now) = latest else { return };
    let band = Rect::from_two_pos(origin, now);
    if down {
        let painter = ui.ctx().layer_painter(layer).with_clip_rect(area);
        painter.rect(
            band,
            0.0,
            pal.accent.gamma_multiply(0.12),
            Stroke::new(1.0, pal.accent),
            StrokeKind::Inside,
        );
        return;
    }
    d.band = None;
    let picked = band_pick(form, &d.rects, band);
    if !picked.is_empty() {
        d.select_all_of(&picked);
    }
}

/// The outermost nodes whose rects lie entirely inside `band`, in tree order (anchor wrappers as
/// their node).
pub fn band_pick(
    form: &Form,
    rects: &std::collections::HashMap<NodeId, Rect>,
    band: Rect,
) -> Vec<NodeId> {
    fn walk(
        n: &Node,
        form: &Form,
        rects: &std::collections::HashMap<NodeId, Rect>,
        band: Rect,
        out: &mut Vec<NodeId>,
    ) {
        if n.id != form.root.id
            && !form.is_locked_branch(n.id)
            && rects.get(&n.id).is_some_and(|r| band.contains_rect(*r))
        {
            out.push(form.anchor_child(n.id));
            return;
        }
        for c in n.children() {
            walk(c, form, rects, band, out);
        }
    }
    let mut out = Vec::new();
    walk(&form.root, form, rects, band, &mut out);
    out.dedup();
    out
}

// ---- Inspector -----------------------------------------------------------------------------

fn section(ui: &mut egui::Ui, pal: &Palette, t: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(t)
            .size(10.5)
            .strong()
            .color(pal.text_tertiary),
    );
    ui.add_space(2.0);
}

/// A change the inspector asks for.
enum Change {
    Set(&'static str, PropValue),
    Clear(&'static str),
    AddEvent(EventId),
    SetEvent(EventId, String),
    RemoveEvent(EventId),
    Act(MenuAction),
    Tab(TabOp),
    Table(TableOp),
}

/// A table's size from the inspector.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TableOp {
    Columns(usize),
    Rows(usize),
}

/// An edit of a Tabs' tabs from the inspector: each changes a tab and its page together.
#[derive(Clone, Debug, PartialEq)]
enum TabOp {
    Add,
    Remove(usize),
    Rename(usize, String),
    Move(usize, usize),
    /// Show this page on the canvas (no edit).
    Show(usize),
}

fn inspector(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    let Some(f) = d.current() else { return };
    let form = f.form().clone();
    let node = d
        .selected
        .and_then(|s| form.find(s))
        .unwrap_or(&form.root)
        .clone();
    let reason = f.custom_reason(node.id).map(str::to_string);
    let form_key = form.id();
    let menu = MenuState::of(d);
    let measured = d.rects.get(&node.id).map(|r| r.size());
    let mut changes: Vec<(Change, bool)> = Vec::new();
    egui::ScrollArea::vertical().id_salt("designer-properties-rows").auto_shrink([false, false]).show(ui, |ui| {
        ui.label(RichText::new(node_title(&node)).strong().color(pal.text));
        match &node.kind {
            NodeKind::Custom { text } => {
                section(ui, pal, "LOCKED CUSTOM CODE");
                if let Some(reason) = &reason {
                    ui.label(RichText::new(format!("Why: {reason}")).color(pal.text_secondary));
                }
                ui.label(RichText::new("Kept exactly as written. It can be moved, copied and deleted; edit its text in the code editor.").small().color(pal.text_tertiary));
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(super::preview::dedent(text)).monospace().color(pal.code_text)).wrap_mode(egui::TextWrapMode::Extend));
            }
            NodeKind::Widget { def, props, events, extra_chain, response_chain, .. } => {
                let wd = widget_def(*def);
                if *def == WidgetId::Image {
                    let shown = super::preview::image_path(props, d.loaded.as_ref().and_then(|l| l.path.parent())).is_some_and(|p| p.is_file());
                    let note = if shown {
                        "The canvas shows the file (a PNG). At runtime egui needs image loaders: see Use in code."
                    } else {
                        "The canvas shows a placeholder: only a local PNG is drawn here. At runtime egui needs image loaders: see Use in code."
                    };
                    ui.label(RichText::new(note).small().color(pal.text_tertiary));
                }
                layout_section(ui, pal, &menu, measured, &mut changes);
                prop_sections(ui, pal, &form.egui_path, wd.props, wd.size_props, props, node.id, &mut changes);
                events_section(ui, pal, &node, *def, events, &mut changes);
                chain_list(ui, pal, "OTHER BUILDER METHODS", extra_chain);
                chain_list(ui, pal, "ON THE RESPONSE", response_chain);
                if !d.rects.contains_key(&node.id) && !matches!(def, WidgetId::AddSpace | WidgetId::EndRow) {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Not visible in the preview (scrolled away, or inside something closed).").small().color(pal.text_tertiary));
                }
            }
            NodeKind::Container { def, props, extra_chain, .. } => {
                if *def == ContainerId::Root {
                    form_section(ui, pal, &form);
                } else {
                    let cd = container_def(*def);
                    layout_section(ui, pal, &menu, measured, &mut changes);
                    if *def == ContainerId::Window {
                        window_anchor_editor(ui, pal, &form.egui_path, props, &mut changes);
                    }
                    if def.is_tabs() {
                        let shown = d.tab_pages.get(&node.id).copied();
                        tabs_editor(ui, pal, &form, &node, shown, &mut changes);
                    }
                    if *def == ContainerId::Table {
                        ui.label(RichText::new("Requires egui_extras: TableBuilder is in the egui_extras crate, which your Cargo.toml must list (Formust doesn't add it).").small().color(pal.warning));
                        table_editor(ui, pal, &form, &node, &mut changes);
                    }
                    if matches!(def, ContainerId::MenuBar | ContainerId::MenuButton) {
                        ui.label(RichText::new("Select a menu to see it open on the canvas. Menu items are buttons; end a handler with ui.close(); to close the menu.").small().color(pal.text_tertiary));
                    }
                    prop_sections(ui, pal, &form.egui_path, cd.props, cd.size_props, props, node.id, &mut changes);
                    chain_list(ui, pal, "OTHER BUILDER METHODS", extra_chain);
                }
            }
        }
    });
    for (change, typing) in changes {
        let key_of = |name: &str| format!("{form_key}:{}:{name}", node.id.0);
        let id = node.id;
        match change {
            Change::Set(name, value) => {
                let key = typing.then(|| key_of(name));
                d.try_edit(key.as_deref(), |f| f.set_prop(id, name, value));
            }
            Change::Clear(name) => {
                d.try_edit(None, |f| f.clear_prop(id, name));
            }
            Change::AddEvent(e) => {
                let name = handler_name(&node);
                d.try_edit(None, |f| {
                    f.set_event(id, e, &Form::default_handler(&name, e))
                });
            }
            Change::SetEvent(e, body) => {
                let key = key_of(e.method());
                d.try_edit(Some(&key), |f| f.set_event(id, e, &body));
            }
            Change::RemoveEvent(e) => {
                d.try_edit(None, |f| f.remove_event(id, e));
            }
            Change::Act(a) => run_menu_action(d, a),
            Change::Tab(op) => run_tab_op(d, id, op, typing),
            Change::Table(TableOp::Columns(n)) => {
                d.try_edit(None, |f| f.set_table_columns(id, n));
            }
            Change::Table(TableOp::Rows(n)) => {
                d.try_edit(None, |f| f.set_table_rows(id, n));
            }
        }
    }
}

/// The tabs of a Tabs: each with its label (and value), up and down, remove, and Add tab.
fn tabs_editor(
    ui: &mut egui::Ui,
    pal: &Palette,
    form: &Form,
    node: &Node,
    shown: Option<NodeId>,
    changes: &mut Vec<(Change, bool)>,
) {
    section(ui, pal, "TABS");
    let tabs = form.tabs(node.id).unwrap_or_default();
    let pages = node.children();
    let count = tabs.len();
    let shown = shown
        .and_then(|s| pages.iter().position(|p| p.id == s))
        .unwrap_or(0);
    egui::Grid::new(("designer-tabs", node.id.0))
        .num_columns(2)
        .spacing(vec2(6.0, 4.0))
        .show(ui, |ui| {
            for (k, tab) in tabs.iter().enumerate() {
                let r = ui.radio(k == shown, "").on_hover_text(format!(
                    "Show this page on the canvas (value {})",
                    tab.value
                ));
                if r.clicked() {
                    changes.push((Change::Tab(TabOp::Show(k)), false));
                }
                ui.horizontal(|ui| {
                    let mut label = match &tab.label {
                        PropValue::Text(t) | PropValue::Expr(t) => t.clone(),
                        other => show_value(other),
                    };
                    let edit = ui
                        .add(egui::TextEdit::singleline(&mut label).desired_width(110.0))
                        .on_hover_text(match &tab.label {
                            PropValue::Expr(_) => "Label (a Rust expression)",
                            _ => "Label",
                        });
                    if edit.changed() {
                        changes.push((Change::Tab(TabOp::Rename(k, label)), true));
                    }
                    if ui
                        .add_enabled(k > 0, egui::Button::new("↑").small())
                        .on_hover_text("Move this tab (and its page) left or up")
                        .clicked()
                    {
                        changes.push((Change::Tab(TabOp::Move(k, k - 1)), false));
                    }
                    if ui
                        .add_enabled(k + 1 < count, egui::Button::new("↓").small())
                        .on_hover_text("Move this tab (and its page) right or down")
                        .clicked()
                    {
                        changes.push((Change::Tab(TabOp::Move(k, k + 1)), false));
                    }
                    if ui
                        .add_enabled(count > 1, egui::Button::new("×").small())
                        .on_hover_text("Remove this tab and its page")
                        .clicked()
                    {
                        changes.push((Change::Tab(TabOp::Remove(k)), false));
                    }
                });
                ui.end_row();
            }
        });
    let can_add = form.can_add_tab(node.id);
    let add = ui
        .add_enabled(can_add, egui::Button::new("Add tab").small())
        .on_hover_text("A new tab with the next number and an empty page")
        .on_disabled_hover_text(
            "These tabs are an enum's variants: add a variant to the enum and an arm by hand",
        );
    if add.clicked() {
        changes.push((Change::Tab(TabOp::Add), false));
    }
}

/// A table's size (columns, body rows) and the kind of each column (`egui_extras::Column::..`).
fn table_editor(
    ui: &mut egui::Ui,
    pal: &Palette,
    form: &Form,
    node: &Node,
    changes: &mut Vec<(Change, bool)>,
) {
    let Some((columns, rows, _)) = form.table_size(node.id) else {
        return;
    };
    section(ui, pal, "TABLE");
    egui::Grid::new(("designer-table", node.id.0))
        .num_columns(2)
        .spacing(vec2(8.0, 4.0))
        .min_col_width(80.0)
        .show(ui, |ui| {
            let mut c = columns;
            let label = ui.label(RichText::new("Columns").color(pal.text));
            if ui
                .add(egui::DragValue::new(&mut c).range(1..=formust_model::MAX_TABLE_COLUMNS))
                .labelled_by(label.id)
                .changed()
            {
                changes.push((Change::Table(TableOp::Columns(c)), false));
            }
            ui.end_row();
            let mut r = rows;
            let label = ui.label(RichText::new("Rows").color(pal.text));
            if ui
                .add(egui::DragValue::new(&mut r).range(1..=formust_model::MAX_TABLE_ROWS))
                .labelled_by(label.id)
                .on_hover_text("Body rows; the header row is the Header height property")
                .changed()
            {
                changes.push((Change::Table(TableOp::Rows(r)), false));
            }
            ui.end_row();
            let kinds = match &node.kind {
                NodeKind::Container { props, .. } => match props.get("columns") {
                    Some(PropValue::Items(items)) => items.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            for (k, kind) in kinds.iter().enumerate() {
                ui.label(RichText::new(format!("Column {}", k + 1)).color(pal.text_secondary));
                let mut text = kind.value.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut text)
                            .code_editor()
                            .desired_width(180.0),
                    )
                    .on_hover_text("An egui_extras::Column: auto(), remainder(), exact(80.0), initial(120.0).resizable(true) ..")
                    .changed()
                {
                    let mut all = kinds.clone();
                    all[k].value = text;
                    changes.push((Change::Set("columns", PropValue::Items(all)), true));
                }
                ui.end_row();
            }
        });
}

/// Applies a tab edit; Show only changes the page the canvas shows.
fn run_tab_op(d: &mut Designer, id: NodeId, op: TabOp, typing: bool) {
    let form_key = d.current().map(|f| f.form().id()).unwrap_or_default();
    match op {
        TabOp::Show(k) => {
            let page = d
                .current()
                .and_then(|f| f.form().find(id))
                .and_then(|n| n.children().get(k))
                .map(|p| p.id);
            if let Some(page) = page {
                d.tab_pages.insert(id, page);
            }
        }
        TabOp::Add => {
            let mut added = None;
            let label = format!(
                "Tab {}",
                d.current()
                    .and_then(|f| f.form().tabs(id))
                    .map_or(1, |t| t.len() + 1)
            );
            d.try_edit(None, |f| {
                added = Some(f.add_tab(id, &label)?);
                Ok(())
            });
            if let Some(page) = added {
                d.tab_pages.insert(id, page);
            }
        }
        TabOp::Remove(k) => {
            d.try_edit(None, |f| f.remove_tab(id, k));
        }
        TabOp::Move(from, to) => {
            d.try_edit(None, |f| f.move_tab(id, from, to));
        }
        TabOp::Rename(k, label) => {
            let key = typing.then(|| format!("{form_key}:{}:tab{k}", id.0));
            let value = match d
                .current()
                .and_then(|f| f.form().tabs(id))
                .and_then(|t| t.get(k).map(|t| t.label.clone()))
            {
                Some(PropValue::Expr(_)) => PropValue::Expr(label),
                _ => PropValue::Text(label),
            };
            d.try_edit(key.as_deref(), |f| f.rename_tab(id, k, value));
        }
    }
}

/// Fill, the anchor, wrapping and the size on the canvas, with Reset size.
fn layout_section(
    ui: &mut egui::Ui,
    pal: &Palette,
    m: &MenuState,
    measured: Option<egui::Vec2>,
    changes: &mut Vec<(Change, bool)>,
) {
    section(ui, pal, "LAYOUT");
    egui::Grid::new("designer-layout")
        .num_columns(2)
        .spacing(vec2(8.0, 4.0))
        .min_col_width(80.0)
        .show(ui, |ui| {
            if let Some(fill) = m.fill {
                let label = ui.label(RichText::new("Fill").color(pal.text));
                let mut on = fill;
                let r = ui
                    .checkbox(&mut on, "")
                    .labelled_by(label.id)
                    .on_hover_text("As wide as there is room (Cmd+E)");
                if r.changed() {
                    changes.push((Change::Act(MenuAction::Fill(on)), false));
                }
                ui.end_row();
            }
            if m.can_anchor {
                ui.label(RichText::new("Anchor").color(pal.text))
                    .on_hover_text("Written as ui.with_layout(..) around this node");
                let mut choice = m.anchor;
                let shown = |a: Option<Anchor>| a.map_or("None", Anchor::label);
                egui::ComboBox::from_id_salt("designer-anchor")
                    .selected_text(shown(choice))
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut choice, None, "None (normal flow)");
                        for &a in ALL_ANCHORS {
                            ui.selectable_value(&mut choice, Some(a), a.label());
                        }
                    });
                if choice != m.anchor {
                    changes.push((Change::Act(MenuAction::Anchor(choice)), false));
                }
                ui.end_row();
            }
            if let Some(size) = measured {
                ui.label(RichText::new("On canvas").color(pal.text_secondary));
                ui.label(
                    RichText::new(format!("{:.0} × {:.0} pt", size.x, size.y)).color(pal.text),
                );
                ui.end_row();
            }
        });
    ui.horizontal_wrapped(|ui| {
        let wrap = if m.range > 1 {
            format!("Wrap {} in row", m.range)
        } else {
            "Wrap in row".to_string()
        };
        if ui
            .add_enabled(m.can_wrap, egui::Button::new(wrap).small())
            .on_hover_text("Put the selection in a new Horizontal here (Cmd+J; shift-click a sibling for a range)")
            .clicked()
        {
            changes.push((Change::Act(MenuAction::WrapRow), false));
        }
        if ui
            .add_enabled(m.can_unwrap, egui::Button::new("Unwrap").small())
            .on_hover_text("Move this container's children out into its place (Cmd+Shift+J)")
            .clicked()
        {
            changes.push((Change::Act(MenuAction::Unwrap), false));
        }
        if ui
            .add_enabled(m.resizable, egui::Button::new("Reset size").small())
            .on_hover_text("Remove the size properties: back to egui's own size")
            .clicked()
        {
            changes.push((Change::Act(MenuAction::ResetSize), false));
        }
    });
}

/// A Window's `.anchor(egui::Align2::X, [dx, dy])`: a corner or side, and an offset.
fn window_anchor_editor(
    ui: &mut egui::Ui,
    pal: &Palette,
    egui_path: &str,
    props: &formust_model::Props,
    changes: &mut Vec<(Change, bool)>,
) {
    let current = match props.get("anchor") {
        Some(PropValue::Expr(t)) => Some(parse_window_anchor(t)),
        _ => None,
    };
    if current == Some(None) {
        // An anchor that isn't literal: edited as text in the properties below.
        return;
    }
    let current = current.flatten();
    let label = |a: &str| a.to_lowercase().replace('_', " ");
    ui.horizontal(|ui| {
        ui.label(RichText::new("Window anchor").color(pal.text));
        let mut choice = current.map(|c| c.0);
        egui::ComboBox::from_id_salt("designer-window-anchor")
            .selected_text(choice.map_or_else(|| "None".to_string(), label))
            .width(120.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut choice, None, "None");
                for &a in ALIGN2 {
                    ui.selectable_value(&mut choice, Some(a), label(a));
                }
            });
        let (mut dx, mut dy) = current.map_or((0.0, 0.0), |c| (c.1, c.2));
        let mut typing = false;
        if choice.is_some() {
            typing |= ui
                .add(egui::DragValue::new(&mut dx).prefix("dx ").speed(1.0))
                .changed();
            typing |= ui
                .add(egui::DragValue::new(&mut dy).prefix("dy ").speed(1.0))
                .changed();
        }
        if choice != current.map(|c| c.0) || typing {
            changes.push(match choice {
                Some(a) => (
                    Change::Set(
                        "anchor",
                        PropValue::Expr(window_anchor_text(egui_path, a, dx, dy)),
                    ),
                    typing,
                ),
                None => (Change::Clear("anchor"), false),
            });
        }
    });
}

/// The `<x>` of a new handler `self.on_<x>_clicked();`: the widget's text, else its kind.
fn handler_name(n: &Node) -> String {
    if let NodeKind::Widget { def, props, .. } = &n.kind {
        for p in ["text", "label", "url"] {
            if let Some(PropValue::Text(t)) = props.get(p)
                && !t.trim().is_empty()
            {
                return t.clone();
            }
        }
        return widget_def(*def).name.to_string();
    }
    "item".to_string()
}

/// The FORM panel's "Dialog" row: for a dialog (a function taking `&egui::Context` that shows a
/// Window, a Modal or a dialog helper call), what it shows; widgets go in there. `None` for a
/// form drawn into a `Ui` and for a legacy `update`.
pub fn dialog_note(form: &Form) -> Option<String> {
    if !form.context_root {
        return None;
    }
    let shown = form.widget_root();
    let free = if form.owner.is_empty() {
        "free fn, "
    } else {
        ""
    };
    Some(if shown == form.root.id {
        format!("{free}takes &egui::Context")
    } else {
        let title = form.find(shown).map(node_title).unwrap_or_default();
        format!("{free}shows {title}")
    })
}

fn form_section(ui: &mut egui::Ui, pal: &Palette, form: &Form) {
    section(ui, pal, "FORM");
    let grid_row = |ui: &mut egui::Ui, k: &str, v: &str| {
        ui.label(RichText::new(k).color(pal.text_secondary));
        ui.label(RichText::new(v).color(pal.text));
        ui.end_row();
    };
    egui::Grid::new("designer-form-props")
        .num_columns(2)
        .spacing(vec2(10.0, 4.0))
        .show(ui, |ui| {
            grid_row(ui, "Owner", &form.owner);
            grid_row(ui, "Method", &form.method);
            grid_row(ui, "State struct", &form.state_struct);
            grid_row(ui, "State field", &form.state_field);
            if form.legacy_ctx.is_some() {
                grid_row(ui, "Legacy", "fn update (exported as fn ui)");
            }
            if let Some(dialog) = dialog_note(form) {
                grid_row(ui, "Dialog", &dialog);
            }
        });
    if !form.state.is_empty() {
        section(ui, pal, "GENERATED STATE");
        for s in &form.state {
            ui.label(
                RichText::new(format!("{}: {}", s.name, s.ty))
                    .monospace()
                    .size(11.0)
                    .color(pal.code_text),
            );
        }
    }
    section(ui, pal, "SIGNATURE");
    ui.add(
        egui::Label::new(
            RichText::new(form.sig.trim())
                .monospace()
                .size(11.0)
                .color(pal.code_text),
        )
        .wrap_mode(egui::TextWrapMode::Wrap),
    );
}

fn chain_list(ui: &mut egui::Ui, pal: &Palette, title: &str, chain: &[String]) {
    if chain.is_empty() {
        return;
    }
    section(ui, pal, title);
    for c in chain {
        ui.add(
            egui::Label::new(
                RichText::new(super::one_line(c.trim(), 60))
                    .monospace()
                    .size(11.0)
                    .color(pal.code_text),
            )
            .wrap_mode(egui::TextWrapMode::Extend),
        );
    }
}

/// Which inspector group a property is in.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Properties,
    TextStyle,
    Behaviour,
    Spacing,
    Size,
}

fn group_of(def: &PropDef, size: &[&str]) -> Group {
    if size.contains(&def.name) {
        return Group::Size;
    }
    match def.role {
        PropRole::Rich(_) => Group::TextStyle,
        PropRole::Response(_)
        | PropRole::AddEnabled
        | PropRole::AddVisible
        | PropRole::InitialFocus => Group::Behaviour,
        PropRole::Spacing(_) => Group::Spacing,
        PropRole::UiSet(_) => Group::Size,
        PropRole::Arg
        | PropRole::Builder(_)
        | PropRole::Flag(_)
        | PropRole::Ctor
        | PropRole::AddSized
        | PropRole::Body => Group::Properties,
    }
}

/// The properties by group (their own, text style, tooltip and enabled/visible, spacing, size),
/// each with its editor.
#[allow(clippy::too_many_arguments)]
fn prop_sections(
    ui: &mut egui::Ui,
    pal: &Palette,
    egui_path: &str,
    defs: &'static [PropDef],
    size: &[&str],
    props: &formust_model::Props,
    id: NodeId,
    changes: &mut Vec<(Change, bool)>,
) {
    for (group, title, salt) in [
        (Group::Properties, "PROPERTIES", "designer-props"),
        (Group::TextStyle, "TEXT STYLE", "designer-text-style"),
        (Group::Behaviour, "BEHAVIOUR", "designer-behaviour"),
        (Group::Spacing, "SPACING AND PADDING", "designer-spacing"),
        (Group::Size, "SIZE", "designer-size"),
    ] {
        // A Tabs' list of tabs has its own editor (`tabs_editor`), which keeps the pages in step.
        let list: Vec<&'static PropDef> = defs
            .iter()
            .filter(|d| group_of(d, size) == group && d.name != "tabs" && d.name != "columns")
            .collect();
        if !list.is_empty() {
            section(ui, pal, title);
            prop_grid(ui, pal, egui_path, (salt, id.0), &list, props, changes);
        }
    }
}

/// Every registry property with its editor. Values equal to a default are not stored
/// (`Form::set_prop`), and an optional property can be cleared back to its default.
fn prop_grid(
    ui: &mut egui::Ui,
    pal: &Palette,
    egui_path: &str,
    salt: (&str, u32),
    defs: &[&'static PropDef],
    props: &formust_model::Props,
    changes: &mut Vec<(Change, bool)>,
) {
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing(vec2(8.0, 4.0))
        .min_col_width(80.0)
        .show(ui, |ui| {
            for def in defs {
                let set = props.get(def.name);
                let label = ui.label(RichText::new(def.label).color(if set.is_some() {
                    pal.text
                } else {
                    pal.text_secondary
                }));
                let label = label.on_hover_text(format!("{} ({})", def.name, role_text(def.role)));
                ui.horizontal(|ui| {
                    let label_id = label.id;
                    if let Some(c) = prop_editor(ui, pal, egui_path, def, set, label_id) {
                        changes.push(c);
                    }
                });
                ui.end_row();
            }
        });
}

fn role_text(role: PropRole) -> String {
    match role {
        PropRole::Arg => "argument".to_string(),
        PropRole::Builder(m) => format!(".{m}(..)"),
        PropRole::Flag(m) => format!(".{m}()"),
        PropRole::Ctor => "constructor".to_string(),
        PropRole::AddSized => "ui.add_sized(..)".to_string(),
        PropRole::AddEnabled => "ui.add_enabled(..)".to_string(),
        PropRole::AddVisible => "ui.add_visible(..)".to_string(),
        PropRole::Body => "in the body".to_string(),
        PropRole::Rich(m) => format!("RichText .{m}(..)"),
        PropRole::Response(m) => format!("response .{m}(..)"),
        PropRole::UiSet(m) => format!("ui.{m}(..) in the body"),
        PropRole::Spacing(f) => format!("ui.spacing_mut().{f} in the body"),
        PropRole::InitialFocus => {
            "response.request_focus() while nothing has the focus".to_string()
        }
    }
}

/// A reset button for a property that is set and can be unset.
fn reset(ui: &mut egui::Ui, def: &'static PropDef) -> Option<(Change, bool)> {
    ui.small_button("×")
        .on_hover_text("Back to the default")
        .clicked()
        .then_some((Change::Clear(def.name), false))
}

fn prop_editor(
    ui: &mut egui::Ui,
    pal: &Palette,
    egui_path: &str,
    def: &'static PropDef,
    value: Option<&PropValue>,
    label: egui::Id,
) -> Option<(Change, bool)> {
    let default = def.default.to_value();
    let optional = def.role != PropRole::Arg;
    // A pair: two numbers while it is literal, else its text below.
    if def.kind == PropKind::Vec2 {
        match value {
            None => {
                let (x, y) = start_pair(def);
                return ui.small_button("Set").clicked().then(|| {
                    (
                        Change::Set(def.name, PropValue::Expr(vec2_text(egui_path, x, y))),
                        false,
                    )
                });
            }
            Some(PropValue::Expr(t)) if let Some((mut x, mut y)) = parse_vec2(t) => {
                let mut typing = false;
                typing |= ui
                    .add(
                        egui::DragValue::new(&mut x)
                            .speed(0.5)
                            .max_decimals(1)
                            .prefix("x "),
                    )
                    .labelled_by(label)
                    .changed();
                typing |= ui
                    .add(
                        egui::DragValue::new(&mut y)
                            .speed(0.5)
                            .max_decimals(1)
                            .prefix("y "),
                    )
                    .changed();
                if typing {
                    return Some((
                        Change::Set(def.name, PropValue::Expr(vec2_text(egui_path, x, y))),
                        true,
                    ));
                }
                return reset(ui, def);
            }
            Some(_) => {}
        }
    }
    // Fill (an infinite width, or `add_sized` at the available width): no number to edit.
    if let Some(PropValue::Float(x)) = value
        && x.is_infinite()
    {
        ui.label(RichText::new("Fill").color(pal.text))
            .on_hover_text("As wide as there is room: the Fill checkbox under Layout");
        return reset(ui, def);
    }
    // A non-literal value of a literal property: its source text, editable as such.
    if let Some(PropValue::Expr(e)) = value
        && def.kind != PropKind::Expr
    {
        let mut text = e.clone();
        let r = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .code_editor()
                    .desired_width(150.0),
            )
            .labelled_by(label);
        let mut out = None;
        if r.changed() {
            out = Some((Change::Set(def.name, PropValue::Expr(text)), true));
        }
        if let Some(c) = reset(ui, def) {
            out = Some(c);
        }
        return out;
    }
    match def.kind {
        PropKind::Bool => {
            let current = match value.or(default.as_ref()) {
                Some(PropValue::Bool(b)) => Some(*b),
                _ => None,
            };
            if matches!(def.default, DefaultValue::None) {
                // No default: unset, true, false, or an expression such as `self.ready`, which
                // the text field above then edits.
                let mut choice = current;
                let mut expression = false;
                let shown = |c: Option<bool>| c.map_or("(not set)".to_string(), |b| b.to_string());
                egui::ComboBox::from_id_salt(("designer-bool", def.name))
                    .selected_text(shown(choice))
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for c in [None, Some(true), Some(false)] {
                            ui.selectable_value(&mut choice, c, shown(c));
                        }
                        if ui
                            .selectable_label(false, "Expression…")
                            .on_hover_text("A bool expression, such as self.ready")
                            .clicked()
                        {
                            expression = true;
                        }
                    });
                if expression {
                    return Some((
                        Change::Set(def.name, PropValue::Expr(EXPRESSION_START.to_string())),
                        false,
                    ));
                }
                return (choice != current).then(|| match choice {
                    Some(b) => (Change::Set(def.name, PropValue::Bool(b)), false),
                    None => (Change::Clear(def.name), false),
                });
            }
            let mut b = current.unwrap_or(false);
            let r = ui.checkbox(&mut b, "").labelled_by(label);
            r.changed()
                .then_some((Change::Set(def.name, PropValue::Bool(b)), false))
        }
        PropKind::Int => match value {
            Some(PropValue::Int(i)) => {
                let mut v = *i;
                let r = ui.add(egui::DragValue::new(&mut v)).labelled_by(label);
                if r.changed() {
                    return Some((Change::Set(def.name, PropValue::Int(v)), true));
                }
                reset(ui, def)
            }
            _ => ui
                .small_button("Set")
                .clicked()
                .then(|| (Change::Set(def.name, PropValue::Int(start_int(def))), false)),
        },
        PropKind::Float => match value.or(if optional { None } else { default.as_ref() }) {
            Some(PropValue::Float(x)) => {
                let mut v = *x;
                let r = ui
                    .add(egui::DragValue::new(&mut v).speed(0.5).max_decimals(2))
                    .labelled_by(label);
                if r.changed() {
                    return Some((Change::Set(def.name, PropValue::Float(v)), true));
                }
                if optional { reset(ui, def) } else { None }
            }
            _ => ui.small_button("Set").clicked().then(|| {
                (
                    Change::Set(def.name, PropValue::Float(start_float(def))),
                    false,
                )
            }),
        },
        PropKind::Text => {
            let mut text = match value.or(default.as_ref()) {
                Some(PropValue::Text(t)) => t.clone(),
                _ => String::new(),
            };
            let r = ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(160.0))
                .labelled_by(label);
            if !r.changed() {
                return None;
            }
            if optional && text.is_empty() {
                Some((Change::Clear(def.name), true))
            } else {
                Some((Change::Set(def.name, PropValue::Text(text)), true))
            }
        }
        PropKind::Expr => {
            let mut text = match value.or(default.as_ref()) {
                Some(PropValue::Expr(t)) => t.clone(),
                Some(other) => show_value(other),
                None => String::new(),
            };
            let r = ui
                .add(
                    egui::TextEdit::singleline(&mut text)
                        .code_editor()
                        .desired_width(170.0)
                        .hint_text("Rust expression"),
                )
                .labelled_by(label);
            if !r.changed() {
                return None;
            }
            if text.trim().is_empty() && (optional || matches!(def.default, DefaultValue::None)) {
                Some((Change::Clear(def.name), true))
            } else {
                Some((Change::Set(def.name, PropValue::Expr(text)), true))
            }
        }
        PropKind::Enum { values, .. } => {
            let current = match value.or(default.as_ref()) {
                Some(PropValue::Enum(e)) => Some(*e),
                _ => None,
            };
            let mut choice = current;
            let unset_allowed = matches!(def.default, DefaultValue::None);
            egui::ComboBox::from_id_salt(("designer-enum", def.name))
                .selected_text(choice.unwrap_or("(default)"))
                .width(130.0)
                .show_ui(ui, |ui| {
                    if unset_allowed {
                        ui.selectable_value(&mut choice, None, "(default)");
                    }
                    for v in values {
                        ui.selectable_value(&mut choice, Some(*v), *v);
                    }
                });
            (choice != current).then(|| match choice {
                Some(v) => (Change::Set(def.name, PropValue::Enum(v)), false),
                None => (Change::Clear(def.name), false),
            })
        }
        PropKind::Color => color_editor(ui, pal, def, value),
        PropKind::Items => items_editor(ui, def, value),
        PropKind::Stroke => stroke_editor(ui, pal, def, value),
        // Handled above, and a non-literal one as an expression.
        PropKind::Vec2 => None,
    }
}

/// What "Expression…" puts in a bool property for the user to type over (`self.ready`): valid code
/// meanwhile.
pub const EXPRESSION_START: &str = "true";

/// The pair a Set button starts from: egui's own defaults for the spacing.
fn start_pair(def: &PropDef) -> (f64, f64) {
    match def.name {
        "item_spacing" => (8.0, 3.0),
        "button_padding" => (4.0, 1.0),
        _ => (40.0, 20.0),
    }
}

/// A stroke: its width, and a colour for both modes or a light and a dark one.
fn stroke_editor(
    ui: &mut egui::Ui,
    pal: &Palette,
    def: &'static PropDef,
    value: Option<&PropValue>,
) -> Option<(Change, bool)> {
    let Some(&PropValue::Stroke { width, light, dark }) = value else {
        return ui.small_button("Set").clicked().then(|| {
            (
                Change::Set(
                    def.name,
                    PropValue::stroke(
                        1.0,
                        [0xA0, 0xA0, 0xA0, 0xFF],
                        Some([0x60, 0x60, 0x60, 0xFF]),
                    ),
                ),
                false,
            )
        });
    };
    let mut w = width;
    let mut changed = ui
        .add(
            egui::DragValue::new(&mut w)
                .speed(0.1)
                .range(0.0..=64.0)
                .max_decimals(1)
                .suffix(" pt"),
        )
        .on_hover_text("Width")
        .changed();
    let (mut l, mut dk) = (light, dark);
    changed |= color_buttons(ui, pal, &mut l, &mut dk);
    if changed {
        return Some((Change::Set(def.name, PropValue::stroke(w, l, dk)), true));
    }
    reset(ui, def)
}

fn start_int(def: &PropDef) -> i64 {
    match def.name {
        "num_columns" => 2,
        "desired_rows" => 4,
        _ => 0,
    }
}

fn start_float(def: &PropDef) -> f64 {
    match def.name {
        "desired_width" | "width" | "default_size" | "default_width" | "max_width"
        | "min_col_width" => 120.0,
        "desired_height" | "height" | "max_height" | "default_height" | "min_row_height" => 40.0,
        "size" => 16.0,
        _ => 0.0,
    }
}

/// A colour: one value for both modes, or a light and a dark one.
fn color_editor(
    ui: &mut egui::Ui,
    pal: &Palette,
    def: &'static PropDef,
    value: Option<&PropValue>,
) -> Option<(Change, bool)> {
    let Some(PropValue::Color { light, dark }) = value else {
        return ui.small_button("Set").clicked().then(|| {
            (
                Change::Set(def.name, PropValue::color([0x40, 0x80, 0xC0, 0xFF])),
                false,
            )
        });
    };
    let (mut l, mut dk) = (*light, *dark);
    let changed = color_buttons(ui, pal, &mut l, &mut dk);
    if changed {
        return Some((
            Change::Set(def.name, PropValue::Color { light: l, dark: dk }),
            true,
        ));
    }
    reset(ui, def)
}

/// A light colour, a "dark" checkbox and the dark colour when it is on. Returns whether one changed.
fn color_buttons(
    ui: &mut egui::Ui,
    pal: &Palette,
    light: &mut [u8; 4],
    dark: &mut Option<[u8; 4]>,
) -> bool {
    let mut changed = ui
        .color_edit_button_srgba_unmultiplied(light)
        .on_hover_text("Light mode")
        .changed();
    let mut pair = dark.is_some();
    if ui
        .checkbox(
            &mut pair,
            RichText::new("dark").small().color(pal.text_secondary),
        )
        .on_hover_text("A separate colour in dark mode")
        .changed()
    {
        *dark = pair.then_some(dark.unwrap_or(*light));
        changed = true;
    }
    if let Some(d) = dark.as_mut() {
        changed |= ui
            .color_edit_button_srgba_unmultiplied(d)
            .on_hover_text("Dark mode")
            .changed();
    }
    changed
}

/// A combo box's items: each a value expression and a label.
fn items_editor(
    ui: &mut egui::Ui,
    def: &'static PropDef,
    value: Option<&PropValue>,
) -> Option<(Change, bool)> {
    let mut items = match value {
        Some(PropValue::Items(i)) => i.clone(),
        _ => Vec::new(),
    };
    let mut changed = false;
    let mut typing = false;
    let mut remove = None;
    ui.vertical(|ui| {
        for (i, item) in items.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let r = ui
                    .add(
                        egui::TextEdit::singleline(&mut item.value)
                            .code_editor()
                            .desired_width(40.0),
                    )
                    .on_hover_text("Value (a Rust expression)");
                let mut label = match &item.label {
                    PropValue::Text(t) | PropValue::Expr(t) => t.clone(),
                    other => show_value(other),
                };
                let r2 = ui
                    .add(egui::TextEdit::singleline(&mut label).desired_width(90.0))
                    .on_hover_text("Label");
                if r2.changed() {
                    item.label = match &item.label {
                        PropValue::Expr(_) => PropValue::Expr(label),
                        _ => PropValue::Text(label),
                    };
                }
                if r.changed() || r2.changed() {
                    changed = true;
                    typing = true;
                }
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if ui.small_button("Add item").clicked() {
            let value = items.len().to_string();
            items.push(formust_model::ComboItem {
                value,
                label: PropValue::Text(format!("Item {}", items.len() + 1)),
            });
            changed = true;
        }
    });
    if let Some(i) = remove {
        items.remove(i);
        changed = true;
        typing = false;
    }
    changed.then(|| (Change::Set(def.name, PropValue::Items(items)), typing))
}

/// The widget's events: a handler body each, added with `self.on_<x>_<event>();`.
fn events_section(
    ui: &mut egui::Ui,
    pal: &Palette,
    node: &Node,
    def: WidgetId,
    events: &[(EventId, String)],
    changes: &mut Vec<(Change, bool)>,
) {
    let wd = widget_def(def);
    if wd.events.is_empty() && events.is_empty() {
        return;
    }
    section(ui, pal, "EVENTS");
    for (e, body) in events {
        ui.horizontal(|ui| {
            ui.label(RichText::new(e.method()).color(pal.text));
            if ui
                .small_button("×")
                .on_hover_text("Remove this handler")
                .clicked()
            {
                changes.push((Change::RemoveEvent(*e), false));
            }
        });
        let mut text = body.clone();
        let r = ui.add(
            egui::TextEdit::multiline(&mut text)
                .code_editor()
                .desired_rows(1)
                .desired_width(f32::INFINITY)
                .id_salt(("designer-event", node.id.0, e.method())),
        );
        if r.changed() {
            changes.push((Change::SetEvent(*e, text), true));
        }
    }
    let missing: Vec<EventId> = wd
        .events
        .iter()
        .copied()
        .filter(|e| !events.iter().any(|(x, _)| x == e))
        .collect();
    if !missing.is_empty() {
        let mut add = None;
        egui::ComboBox::from_id_salt(("designer-add-event", node.id.0))
            .selected_text("Add handler…")
            .width(140.0)
            .show_ui(ui, |ui| {
                for e in &missing {
                    if ui.selectable_label(false, e.method()).clicked() {
                        add = Some(*e);
                    }
                }
            });
        if let Some(e) = add {
            changes.push((Change::AddEvent(e), false));
        }
    }
}

// ---- Use in code ---------------------------------------------------------------------------

/// What to know about the exported copy: it stands alone, how a program can use it, what Save does
/// for the form's state, and the handler stubs the form calls, with copy buttons.
fn usage(
    d: &mut Designer,
    ui: &mut egui::Ui,
    pal: &Palette,
    read: &dyn Fn(&Path) -> Option<String>,
) {
    let Some(loaded) = &d.loaded else { return };
    let Some(shown) = d.current_index() else {
        return;
    };
    let f = &loaded.forms[shown];
    let target = match export::target(loaded, &|p| p.exists()) {
        Ok(t) => t,
        Err(e) => {
            ui.label(RichText::new(e).color(pal.warning));
            return;
        }
    };
    let source_text = target.source.as_deref().and_then(read);
    // The copy as Save would write it now, so what it already has is left out.
    let copy = export::preview_text(loaded, shown, source_text.as_deref()).ok();
    let source_name = target
        .source
        .as_deref()
        .and_then(|s| s.file_name())
        .map_or_else(
            || "the source".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
    let frm_name = target
        .frm
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let u = formust_rust::use_in_code(
        f.form(),
        &target.frm_from_source(),
        &source_name,
        copy.as_deref(),
    );
    let mut copied = None;
    egui::ScrollArea::vertical().id_salt("designer-usage").auto_shrink([false, false]).show(ui, |ui| {
        ui.label(RichText::new(&u.standalone).color(pal.text_secondary));
        let mut code = |ui: &mut egui::Ui, code: &str| {
            Frame::new().fill(pal.editor).corner_radius(4.0).inner_margin(Margin::same(6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(code).monospace().size(11.0).color(pal.code_text)).wrap_mode(egui::TextWrapMode::Extend));
            });
            if ui.small_button("Copy").on_hover_text(format!("Copy: {}", super::one_line(code, 40))).clicked() {
                copied = Some(code.to_string());
            }
        };
        if let Some(stubs) = &u.stubs {
            section(ui, pal, &format!("Add to {frm_name}: the handlers the form calls"));
            code(ui, stubs);
        }
        if !u.requirements.is_empty() {
            section(ui, pal, "What the program needs");
            for need in &u.requirements {
                ui.label(RichText::new(need).color(pal.text));
            }
        }
        if let Some(state) = &u.state {
            section(ui, pal, "The form's state");
            ui.label(RichText::new(state).color(pal.text));
        }
        if let Some(module) = &u.module {
            section(ui, pal, "Run the copy in a program (optional)");
            ui.label(RichText::new(format!("{frm_name} runs by itself, for example with Run Form. To build it into a program in place of {source_name}, point its module at the copy:")).color(pal.text));
            code(ui, module);
        }
        if u.stubs.is_none() {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Nothing to add: {frm_name} has the handlers the form calls, or the form needs none.")).small().color(pal.text_tertiary));
        }
    });
    if let Some(text) = copied {
        ui.ctx().copy_text(text.clone());
        d.clipboard = Some(text);
    }
}

// ---- Canvas --------------------------------------------------------------------------------

fn canvas(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    let Some(form) = d.current().map(|f| f.form().clone()) else {
        return;
    };
    let avail = ui.available_rect_before_wrap();
    ui.painter().rect_filled(avail, 0.0, pal.editor);
    let panels = preview::has_panels(&form);
    let w = d
        .preview_width
        .unwrap_or_else(|| FORM_WIDTH.max(d.content.x));
    let h = if panels {
        PANELS_HEIGHT
    } else {
        FORM_MIN_HEIGHT.max(d.content.y)
    };
    let size = (w.ceil() as i64, h.ceil() as i64);
    let (area, hbar, vbar) = viewport::split_with(avail, size, TITLE_BAR);
    let mut view = View::with_title(area, size, d.scroll, TITLE_BAR);

    // The wheel and the trackpad scroll the canvas (never the form: its scroll areas are inert).
    if ui.rect_contains_pointer(area) {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        if delta != egui::Vec2::ZERO {
            view.scroll_by(-delta);
        }
    }
    if std::mem::take(&mut d.reveal_node)
        && let Some(r) = d.selected.and_then(|s| d.rects.get(&s))
    {
        let o = view.origin();
        view.scroll = view.reveal((
            (r.left() - o.x) as f64,
            (r.top() - o.y) as f64,
            r.width() as f64,
            r.height() as f64,
        ));
    }

    let dark = d.dark.unwrap_or(pal.dark);
    window_chrome(ui, &view, area, &form.id(), dark);
    let also = d.also_selected();
    let handles = d
        .selected
        .and_then(|s| form.find(s))
        .map_or((false, false), resize_axes);
    // The pages around the selection are the ones shown, and the menus around it are open.
    let open_menus = shown_parts(d, &form);
    let base_dir = d
        .loaded
        .as_ref()
        .and_then(|l| l.path.parent().map(std::path::Path::to_path_buf));
    let opts = Options {
        dark,
        selected: d.selected,
        hovered: d.hovered,
        also: &also,
        handles,
        accent: pal.accent,
        stash_accesskit: !d.open_accesskit,
        pages: &d.tab_pages,
        open_menus: &open_menus,
        base_dir: base_dir.as_deref(),
    };
    let drawn = preview::show(ui, area, view.client(), &form, &opts);
    let pass = drawn.pass;
    d.reacted = pass.reacted.clone();
    d.preview_layer = Some(drawn.layer);
    d.client = Some(view.client());
    if pass.kept {
        d.rects = pass.rects;
        d.tab_buttons = pass.tab_buttons;
        let content = pass.content;
        if (content - d.content).length() > 0.5 {
            d.content = content;
            ui.ctx().request_repaint();
        }
        // Act only on the kept pass: Grid's discarded sizing pass has wrong rects.
        let pointer = drawn.shield.hover_pos();
        let hit = pointer.and_then(|p| preview::hit_test(&form.root, &d.rects, p));
        // An anchor wrapper is shown as its node.
        d.hovered = hit.map(|h| form.anchor_child(h));
        let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        if drawn.shield.clicked() {
            let at = drawn.shield.interact_pointer_pos();
            // A tab button shows its page, and selects it so a drop or a paste goes there.
            let tab = at.and_then(|p| {
                d.tab_buttons
                    .iter()
                    .rev()
                    .find(|(r, _)| r.contains(p))
                    .map(|(_, page)| *page)
            });
            let hit = at.and_then(|p| preview::hit_test(&form.root, &d.rects, p));
            // Only where the Tabs itself is hit: an open menu or a modal over the bar wins.
            let tab = tab.filter(|page| form.page_of(*page).map(|(t, _)| t) == hit);
            match (tab, hit) {
                (Some(page), _) if !command && !shift => {
                    if let Some((tabs, _)) = form.page_of(page) {
                        d.tab_pages.insert(tabs, page);
                    }
                    d.select(Some(page), true);
                }
                (_, Some(h)) if command => d.toggle_select(h, true),
                (_, Some(h)) if shift => d.select_range(h, true),
                _ => d.select(hit, true),
            }
        }
        // A right-click selects what it is on (unless that is in the selection) for the menu.
        if drawn.shield.secondary_clicked() {
            let at = drawn.shield.interact_pointer_pos();
            let hit = at
                .and_then(|p| preview::hit_test(&form.root, &d.rects, p))
                .map(|h| form.anchor_child(h));
            let in_selection = hit.is_some_and(|h| d.selected == Some(h) || also.contains(&h));
            if !in_selection {
                d.select(hit, true);
            }
        }
        // A drag on a node moves it (an anchored node with its wrapper), or the whole selection
        // when it is in one; a drag on empty space draws a rubber band.
        if drawn.shield.drag_started()
            && d.resize.is_none()
            && let Some(at) = ui.input(|i| i.pointer.press_origin())
        {
            match preview::hit_test(&form.root, &d.rects, at) {
                Some(hit) if hit != form.root.id => {
                    let shown = form.anchor_child(hit);
                    let in_set = d.selected == Some(shown) || also.contains(&shown);
                    if !in_set {
                        d.select(Some(hit), true);
                    }
                    d.drag = Some(Payload::Move(form.anchor_host(hit)));
                }
                _ => d.band = Some(at),
            }
        }
        rubber_band(d, ui, pal, &form, drawn.layer, area);
    }
    let menu = MenuState::of(d);
    let mut action = None;
    drawn
        .shield
        .context_menu(|ui| node_menu(ui, &menu, &mut action));
    if let Some(a) = action {
        run_menu_action(d, a);
    }
    // A drop over the canvas: the indicator line, on the preview's layer above the form.
    if let Some(payload) = d.drag
        && let Some(pos) = ui.ctx().pointer_latest_pos()
        && area.contains(pos)
    {
        let moving = match payload {
            Payload::Move(id) => Some(id),
            Payload::New(_) => None,
        };
        d.drop = edit::canvas_drop(&form, &d.rects, pos, moving);
        if let Some(drop) = d.drop {
            let painter = ui.ctx().layer_painter(drawn.layer).with_clip_rect(area);
            painter.line_segment(drop.line, Stroke::new(2.5, pal.accent));
            for p in drop.line {
                painter.circle_filled(p, 3.0, pal.accent);
            }
            if let Some(r) = d.rects.get(&drop.parent) {
                painter.rect_stroke(
                    *r,
                    2.0,
                    Stroke::new(1.0, pal.accent.gamma_multiply(0.6)),
                    StrokeKind::Outside,
                );
            }
        }
    }
    paint_overlays(d, ui, pal, &form, drawn.layer, area, view.client());
    let resizing = resize_handles(d, ui, pal, &form, area);
    grip(d, ui, pal, view.client(), area);
    if ui.input(|i| i.key_pressed(egui::Key::Escape))
        && !resizing
        && d.drag.is_none()
        && d.selected.is_some()
        && ui.memory(|m| m.focused().is_none())
    {
        d.select(None, true);
    }
    if let Some(strip) = hbar {
        scroll_bar(ui, pal, &mut view, strip, true);
    }
    if let Some(strip) = vbar {
        scroll_bar(ui, pal, &mut view, strip, false);
    }
    d.scroll = view.scroll;
    ui.allocate_rect(avail, Sense::hover());
}

/// Brings the parts around the selection into view: each Tabs around it shows the page it is on
/// (remembered in `tab_pages` for when the selection moves elsewhere), and returns the menus
/// around it (the selected menu too), which are drawn open.
pub fn shown_parts(d: &mut Designer, form: &Form) -> Vec<NodeId> {
    let mut open = Vec::new();
    let Some(selected) = d.selected.filter(|s| form.find(*s).is_some()) else {
        return open;
    };
    let mut at = Some(selected);
    while let Some(id) = at {
        if let Some(Node {
            kind:
                NodeKind::Container {
                    def: ContainerId::MenuButton,
                    ..
                },
            ..
        }) = form.find(id)
        {
            open.push(id);
        }
        if let Some((tabs, k)) = form.page_of(id)
            && form.parent_of(id).map(|(p, _)| p) == Some(tabs)
            && let Some(page) = form.find(tabs).and_then(|t| t.children().get(k))
        {
            d.tab_pages.insert(tabs, page.id);
        }
        at = form.parent_of(id).map(|(p, _)| p);
    }
    open
}

/// The selected node's resize handles: dragging the right edge, the bottom edge or the corner sets
/// its size properties live; Shift keeps the aspect ratio on the corner, Alt snaps to 4pt, Escape
/// cancels, and one drag is one undo step. Returns whether a resize was going on this frame.
fn resize_handles(
    d: &mut Designer,
    ui: &mut egui::Ui,
    pal: &Palette,
    form: &Form,
    area: Rect,
) -> bool {
    let active = d.resize.is_some();
    if active && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        d.cancel_resize();
        return true;
    }
    let Some(id) = d.selected else {
        d.cancel_resize();
        return active;
    };
    let Some(node) = form.find(id) else {
        d.cancel_resize();
        return active;
    };
    let edges = resize_axes(node);
    let rect = match (&d.resize, d.rects.get(&id)) {
        // The handles stay on the rect the drag started from until it ends.
        (Some(r), _) if r.id == id => r.start,
        (_, Some(r)) => *r,
        _ => return active,
    };
    let kinds = [
        ((true, false), egui::CursorIcon::ResizeHorizontal),
        ((false, true), egui::CursorIcon::ResizeVertical),
        ((true, true), egui::CursorIcon::ResizeNwSe),
    ];
    let (shift, alt, origin, latest, down) = ui.input(|i| {
        (
            i.modifiers.shift,
            i.modifiers.alt,
            i.pointer.press_origin(),
            i.pointer.latest_pos(),
            i.pointer.any_down(),
        )
    });
    for (k, square) in preview::handle_rects(rect, edges).into_iter().enumerate() {
        let Some(square) = square else { continue };
        if !area.intersects(square) {
            continue;
        }
        let (kind, cursor) = kinds[k];
        let r = ui
            .interact(
                square.expand(2.0),
                ui.scope_id().with(("designer-resize", k)),
                Sense::drag(),
            )
            .on_hover_cursor(cursor);
        if r.drag_started() {
            let row_height = ui.ctx().fonts_mut(|f| {
                f.row_height(&egui::TextStyle::Body.resolve(&egui::Style::default()))
            });
            d.begin_resize(id, kind, rect, row_height);
        }
        if r.dragged() && d.resize.as_ref().is_some_and(|z| z.edges == kind) {
            ui.ctx().set_cursor_icon(cursor);
            if let (Some(from), Some(to)) = (origin, latest) {
                d.drag_resize(to - from, shift, alt);
            }
        }
        if r.drag_stopped() {
            d.end_resize();
        }
    }
    if d.resize.is_some() && !down {
        d.end_resize();
    }
    // The size under the pointer while it changes.
    if let (Some(z), Some(pos)) = (&d.resize, latest)
        && let Some(r) = d.rects.get(&z.id)
    {
        egui::Area::new(egui::Id::unique("designer-resize-size"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + vec2(14.0, 10.0))
            .interactable(false)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{:.0} × {:.0}", r.width(), r.height()))
                                .small()
                                .color(pal.text),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
            });
    }
    active || d.resize.is_some()
}

/// The corner grip: dragging it sets the preview's width (not a property of the form; a double
/// click goes back to the default).
fn grip(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette, client: Rect, area: Rect) {
    let rect = Rect::from_center_size(client.right_bottom() - vec2(5.0, 5.0), vec2(14.0, 14.0));
    if !area.intersects(rect) {
        return;
    }
    let r = ui.interact(
        rect,
        ui.scope_id().with("designer-grip"),
        Sense::drag() | Sense::click(),
    );
    let r = r
        .on_hover_text("Drag to try the form at another width (double-click: default)")
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if r.dragged() {
        let w = d.preview_width.unwrap_or(client.width()) + r.drag_delta().x;
        d.preview_width = Some(w.clamp(MIN_WIDTH, 4000.0));
    }
    if r.double_clicked() {
        d.preview_width = None;
    }
    let painter = ui.painter_at(area);
    let c = if r.hovered() || r.dragged() {
        pal.accent
    } else {
        pal.text_tertiary
    };
    for k in [3.0, 7.0, 11.0] {
        painter.line_segment(
            [
                egui::pos2(rect.right() - k, rect.bottom() - 1.0),
                egui::pos2(rect.right() - 1.0, rect.bottom() - k),
            ],
            Stroke::new(1.2, c),
        );
    }
}

/// The window around the form: a title bar with the form's name and a frame, in the preview's mode.
fn window_chrome(ui: &egui::Ui, view: &View, area: Rect, title: &str, dark: bool) {
    let painter = ui.painter_at(area);
    let visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let win = view.window();
    let title_rect = Rect::from_min_max(win.min, egui::pos2(win.max.x, win.min.y + view.title));
    painter.rect_filled(win.expand(1.0), 6.0, visuals.window_stroke.color);
    painter.rect_filled(
        title_rect,
        egui::CornerRadius {
            nw: 6,
            ne: 6,
            sw: 0,
            se: 0,
        },
        visuals.window_fill,
    );
    painter.line_segment(
        [title_rect.left_bottom(), title_rect.right_bottom()],
        visuals.window_stroke,
    );
    for (i, c) in [
        Color32::from_rgb(0xFF, 0x5F, 0x57),
        Color32::from_rgb(0xFE, 0xBC, 0x2E),
        Color32::from_rgb(0x28, 0xC8, 0x40),
    ]
    .into_iter()
    .enumerate()
    {
        painter.circle_filled(
            egui::pos2(
                title_rect.left() + 14.0 + i as f32 * 18.0,
                title_rect.center().y,
            ),
            5.5,
            c,
        );
    }
    painter.text(
        title_rect.center(),
        egui::Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(12.5),
        visuals.text_color(),
    );
}

/// One scroll bar in `strip`: a track with a draggable thumb; a click on the track pages.
fn scroll_bar(ui: &mut egui::Ui, pal: &Palette, view: &mut View, strip: Rect, horizontal: bool) {
    let track = strip.shrink(2.0);
    let len = if horizontal {
        track.width()
    } else {
        track.height()
    };
    let id = ui.scope_id().with(if horizontal {
        "designer-hscroll"
    } else {
        "designer-vscroll"
    });
    let (start, thumb_len) = view.thumb(len, horizontal);
    let thumb = if horizontal {
        Rect::from_min_size(
            track.min + vec2(start, 0.0),
            vec2(thumb_len, track.height()),
        )
    } else {
        Rect::from_min_size(track.min + vec2(0.0, start), vec2(track.width(), thumb_len))
    };
    let r = ui.interact(strip, id, Sense::click_and_drag());
    r.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::ScrollBar,
            true,
            if horizontal {
                "Horizontal scroll bar"
            } else {
                "Vertical scroll bar"
            },
        )
    });
    if r.dragged() {
        let d = r.drag_delta();
        let along = if horizontal { d.x } else { d.y };
        let s = view.thumb_drag(len, horizontal, along);
        view.scroll_by(if horizontal {
            vec2(s, 0.0)
        } else {
            vec2(0.0, s)
        });
    } else if r.clicked()
        && let Some(p) = r.interact_pointer_pos()
        && !thumb.contains(p)
    {
        // Page towards the click, like AppKit's "jump to the next page".
        let page = if horizontal {
            view.area.width()
        } else {
            view.area.height()
        } * 0.9;
        let before = if horizontal {
            p.x < thumb.left()
        } else {
            p.y < thumb.top()
        };
        let by = if before { -page } else { page };
        view.scroll_by(if horizontal {
            vec2(by, 0.0)
        } else {
            vec2(0.0, by)
        });
    }
    let painter = ui.painter();
    painter.rect_filled(strip, 0.0, pal.editor);
    let fill = if r.hovered() || r.dragged() {
        pal.text_secondary
    } else {
        pal.text_tertiary
    };
    painter.rect_filled(thumb, 4.0, fill.gamma_multiply(0.7));
}

// ---- Several selected ----------------------------------------------------------------------

/// The inspector for a selection of several nodes: how many, the align actions (each says what
/// it writes), and the properties they all have. A change sets it on every one of them, as one
/// undo step; a value that differs between them shows as unset and is listed as mixed.
fn multi_inspector(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    let Some(f) = d.current() else { return };
    let form = f.form().clone();
    let hosts = d.selection_hosts();
    let ids: Vec<NodeId> = hosts.iter().map(|h| form.anchor_child(*h)).collect();
    let nodes: Vec<&Node> = ids.iter().filter_map(|i| form.find(*i)).collect();
    let menu = MenuState::of(d);
    let mut changes: Vec<(Change, bool)> = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("designer-multi-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{} selected", nodes.len()))
                    .strong()
                    .color(pal.text),
            );
            ui.label(
                RichText::new("Cmd-click adds or removes a node; a drag on empty canvas selects what it encloses.")
                    .small()
                    .color(pal.text_tertiary),
            );
            section(ui, pal, "ALIGN");
            ui.horizontal(|ui| {
                ui.label(RichText::new("Size").color(pal.text_secondary));
                let mut typed = d.size_value.is_some();
                if ui
                    .checkbox(&mut typed, "typed")
                    .on_hover_text("Off: same width/height as the largest selected node. On: the value here.")
                    .changed()
                {
                    d.size_value = typed.then_some(100.0);
                }
                if let Some(v) = d.size_value.as_mut() {
                    ui.add(egui::DragValue::new(v).range(8.0..=4000.0).suffix(" pt"));
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Spacing").color(pal.text_secondary));
                let mut v = d.spacing();
                if ui
                    .add(egui::DragValue::new(&mut v).range(0.0..=200.0).suffix(" pt"))
                    .on_hover_text("For \"Spacing of the parent\"")
                    .changed()
                {
                    d.spacing_value = Some(v);
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (a, on, writes) in &menu.align {
                    if ui
                        .add_enabled(*on, egui::Button::new(a.label()).small())
                        .on_hover_text(writes)
                        .on_disabled_hover_text(writes)
                        .clicked()
                    {
                        changes.push((Change::Act(MenuAction::Align(*a)), false));
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(menu.can_wrap, egui::Button::new("Wrap in row").small())
                    .clicked()
                {
                    changes.push((Change::Act(MenuAction::WrapRow), false));
                }
                if ui.add(egui::Button::new("Duplicate").small()).clicked() {
                    changes.push((Change::Act(MenuAction::Duplicate), false));
                }
                if ui.add(egui::Button::new("Delete").small()).clicked() {
                    changes.push((Change::Act(MenuAction::Delete), false));
                }
            });
            let (common, mixed) = common_props(&nodes);
            if common.is_empty() {
                section(ui, pal, "COMMON PROPERTIES");
                ui.label(
                    RichText::new("They have no property in common.")
                        .small()
                        .color(pal.text_tertiary),
                );
            } else {
                section(ui, pal, "COMMON PROPERTIES");
                let shown: formust_model::Props = common
                    .iter()
                    .filter_map(|(def, v)| v.clone().map(|v| (def.name, v)))
                    .collect();
                let defs: Vec<&'static PropDef> = common.iter().map(|(d, _)| *d).collect();
                prop_grid(
                    ui,
                    pal,
                    &form.egui_path,
                    ("designer-multi-props", 0),
                    &defs,
                    &shown,
                    &mut changes,
                );
                if !mixed.is_empty() {
                    ui.label(
                        RichText::new(format!("Mixed values: {}", mixed.join(", ")))
                            .small()
                            .color(pal.text_tertiary),
                    );
                }
            }
        });
    let key = format!("{}:multi", form.id());
    for (change, typing) in changes {
        match change {
            Change::Set(name, value) => {
                let k = typing.then(|| format!("{key}:{name}"));
                d.try_edit(k.as_deref(), |f| {
                    for id in &ids {
                        f.set_prop(*id, name, value.clone())?;
                    }
                    Ok(())
                });
            }
            Change::Clear(name) => {
                d.try_edit(None, |f| {
                    for id in &ids {
                        f.clear_prop(*id, name)?;
                    }
                    Ok(())
                });
            }
            Change::Act(a) => run_menu_action(d, a),
            Change::AddEvent(_)
            | Change::SetEvent(..)
            | Change::RemoveEvent(_)
            | Change::Tab(_)
            | Change::Table(_) => {}
        }
    }
}

/// The properties every node has (same name and kind), with their value when it is the same in
/// all of them; and the names of those whose values differ. Custom code has none.
pub fn common_props(
    nodes: &[&Node],
) -> (
    Vec<(&'static PropDef, Option<PropValue>)>,
    Vec<&'static str>,
) {
    fn defs_of(n: &Node) -> Option<(&'static [PropDef], &formust_model::Props)> {
        match &n.kind {
            NodeKind::Widget { def, props, .. } => Some((widget_def(*def).props, props)),
            NodeKind::Container { def, props, .. } => Some((container_def(*def).props, props)),
            NodeKind::Custom { .. } => None,
        }
    }
    let Some(all) = nodes.iter().map(|n| defs_of(n)).collect::<Option<Vec<_>>>() else {
        return (Vec::new(), Vec::new());
    };
    let Some((first, _)) = all.first() else {
        return (Vec::new(), Vec::new());
    };
    let mut common = Vec::new();
    let mut mixed = Vec::new();
    for def in first.iter() {
        // Items and bindings belong to one widget; they are not edited for several at once.
        if matches!(def.kind, PropKind::Items) || def.role == PropRole::Body {
            continue;
        }
        let shared = all.iter().all(|(defs, _)| {
            defs.iter()
                .any(|d| d.name == def.name && d.kind == def.kind && d.role == def.role)
        });
        if !shared {
            continue;
        }
        let values: Vec<Option<&PropValue>> = all.iter().map(|(_, p)| p.get(def.name)).collect();
        let same = values.windows(2).all(|w| w[0] == w[1]);
        if !same {
            mixed.push(def.name);
        }
        common.push((def, if same { values[0].cloned() } else { None }));
    }
    (common, mixed)
}

// ---- Canvas toolbar and overlays -------------------------------------------------------------

/// Preset preview widths, in points.
pub const PRESET_WIDTHS: &[f32] = &[320.0, 480.0, 640.0, 800.0, 1024.0];

/// The bar above the canvas: the preview's width (presets; the corner grip drags any width), its
/// light or dark mode (follows the IDE until picked), and the design-time overlays.
fn canvas_bar(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    Frame::new()
        .fill(pal.editor)
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let label = ui.label(RichText::new("Width").color(pal.text_secondary));
                let shown = d
                    .preview_width
                    .map_or_else(|| "Default".to_string(), |w| format!("{w:.0} pt"));
                let mut chosen = d.preview_width;
                egui::ComboBox::from_id_salt("designer-width")
                    .selected_text(shown)
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut chosen, None, "Default");
                        for w in PRESET_WIDTHS {
                            ui.selectable_value(&mut chosen, Some(*w), format!("{w:.0} pt"));
                        }
                    })
                    .response
                    .labelled_by(label.id)
                    .on_hover_text("The preview's width; drag the canvas corner for any other");
                if chosen != d.preview_width {
                    d.preview_width = chosen;
                }
                ui.separator();
                if ui
                    .selectable_label(d.rulers, "Rulers")
                    .on_hover_text("Rulers in points along the top and left (design time only)")
                    .clicked()
                {
                    d.rulers = !d.rulers;
                }
                if ui
                    .selectable_label(d.grid.is_some(), "Grid lines")
                    .on_hover_text("Lines every 8 or 16 points over the form (design time only)")
                    .clicked()
                {
                    d.grid = if d.grid.is_some() { None } else { Some(8.0) };
                }
                if let Some(g) = d.grid {
                    for step in [8.0, 16.0] {
                        if ui
                            .selectable_label(g == step, format!("{step:.0}"))
                            .on_hover_text(format!("A line every {step:.0} pt"))
                            .clicked()
                        {
                            d.grid = Some(step);
                        }
                    }
                }
                if ui
                    .selectable_label(!d.guides_off, "Guides")
                    .on_hover_text("The distance from the selected node to its neighbours and its container's edges")
                    .clicked()
                {
                    d.guides_off = !d.guides_off;
                }
                if ui
                    .selectable_label(d.tab_order, "Tab order")
                    .on_hover_text("Number the widgets in the order Tab visits them, and list them to reorder")
                    .clicked()
                {
                    d.tab_order = !d.tab_order;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dark = d.dark.unwrap_or(pal.dark);
                    pill(ui, pal, |ui| {
                        // Added right to left.
                        if segment(ui, pal, "Dark", dark).clicked() {
                            d.dark = Some(true);
                        }
                        if segment(ui, pal, "Light", !dark).clicked() {
                            d.dark = Some(false);
                        }
                    });
                });
            });
        });
}

/// The width of a ruler strip.
pub const RULER: f32 = 14.0;
/// The colour of the distance guides.
const GUIDE: Color32 = Color32::from_rgb(232, 62, 140);

/// Rulers, grid lines, distance guides and tab numbers on the preview's layer, above the form.
/// Design time only: none of it is in the form or its code.
#[allow(clippy::too_many_arguments)]
fn paint_overlays(
    d: &Designer,
    ui: &egui::Ui,
    pal: &Palette,
    form: &Form,
    layer: egui::LayerId,
    area: Rect,
    client: Rect,
) {
    let painter = ui.ctx().layer_painter(layer).with_clip_rect(area);
    let origin = client.min;
    if let Some(step) = d.grid {
        let color = pal.accent.gamma_multiply(0.16);
        let clip = client.intersect(area);
        let grid = painter.with_clip_rect(clip);
        for x in grid_lines(origin.x, clip.left(), clip.right(), step) {
            grid.line_segment(
                [egui::pos2(x, clip.top()), egui::pos2(x, clip.bottom())],
                Stroke::new(1.0, color),
            );
        }
        for y in grid_lines(origin.y, clip.top(), clip.bottom(), step) {
            grid.line_segment(
                [egui::pos2(clip.left(), y), egui::pos2(clip.right(), y)],
                Stroke::new(1.0, color),
            );
        }
    }
    let single = d.selection_len() == 1;
    if !d.guides_off
        && single
        && let Some(id) = d.selected
        && let Some(r) = d.rects.get(&id)
    {
        for (from, to, label_at) in distance_guides(form, &d.rects, id, *r) {
            painter.line_segment([from, to], Stroke::new(1.0, GUIDE));
            for p in [from, to] {
                let tick = if (from.x - to.x).abs() > (from.y - to.y).abs() {
                    egui::vec2(0.0, 3.0)
                } else {
                    egui::vec2(3.0, 0.0)
                };
                painter.line_segment([p - tick, p + tick], Stroke::new(1.0, GUIDE));
            }
            let text = format!("{:.0}", (to - from).length());
            let galley =
                painter.layout_no_wrap(text, egui::FontId::proportional(10.0), Color32::WHITE);
            let rect = Rect::from_center_size(label_at, galley.size() + egui::vec2(6.0, 2.0));
            painter.rect_filled(rect, 3.0, GUIDE);
            painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
        }
    }
    if d.tab_order {
        for (k, id) in form.tab_order().into_iter().enumerate() {
            let Some(r) = d.rects.get(&id) else { continue };
            let focus = matches!(&form.find(id).map(|n| &n.kind), Some(NodeKind::Widget { props, .. }) if props.get("initial_focus") == Some(&PropValue::Bool(true)));
            let text = if focus {
                format!("{} ★", k + 1)
            } else {
                format!("{}", k + 1)
            };
            let galley =
                painter.layout_no_wrap(text, egui::FontId::proportional(10.5), Color32::WHITE);
            let size = galley.size() + egui::vec2(8.0, 2.0);
            let rect = Rect::from_min_size(r.left_top() - egui::vec2(4.0, 6.0), size);
            painter.rect_filled(rect, 7.0, pal.accent);
            painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
        }
    }
    if d.rulers {
        paint_rulers(
            &painter,
            pal,
            area,
            origin,
            d.selected.and_then(|s| d.rects.get(&s)).copied(),
        );
    }
}

/// The positions of grid lines every `step` from `origin`, between `lo` and `hi`.
pub fn grid_lines(origin: f32, lo: f32, hi: f32, step: f32) -> Vec<f32> {
    if step <= 0.0 || hi <= lo {
        return Vec::new();
    }
    let first = ((lo - origin) / step).ceil();
    let mut out = Vec::new();
    let mut k = first;
    while origin + k * step <= hi && out.len() < 2000 {
        out.push(origin + k * step);
        k += 1.0;
    }
    out
}

/// The distance guides of a selected node at `r`: from each side to the nearest sibling that faces
/// it, else to its container's edge. Each is (from, to, where its label goes).
pub fn distance_guides(
    form: &Form,
    rects: &std::collections::HashMap<NodeId, Rect>,
    id: NodeId,
    r: Rect,
) -> Vec<(egui::Pos2, egui::Pos2, egui::Pos2)> {
    let host = form.anchor_host(id);
    let Some((parent, _)) = form.parent_of(host) else {
        return Vec::new();
    };
    let Some(outer) = rects.get(&parent).copied() else {
        return Vec::new();
    };
    let siblings: Vec<Rect> = form
        .find(parent)
        .map(|p| {
            p.children()
                .iter()
                .filter(|c| c.id != host)
                .filter_map(|c| rects.get(&form.anchor_child(c.id)).copied())
                .collect()
        })
        .unwrap_or_default();
    let overlaps_y = |s: &Rect| s.top() < r.bottom() && s.bottom() > r.top();
    let overlaps_x = |s: &Rect| s.left() < r.right() && s.right() > r.left();
    let left = siblings
        .iter()
        .filter(|s| overlaps_y(s) && s.right() <= r.left() + 0.5)
        .map(|s| s.right())
        .fold(outer.left(), f32::max);
    let right = siblings
        .iter()
        .filter(|s| overlaps_y(s) && s.left() >= r.right() - 0.5)
        .map(|s| s.left())
        .fold(outer.right(), f32::min);
    let top = siblings
        .iter()
        .filter(|s| overlaps_x(s) && s.bottom() <= r.top() + 0.5)
        .map(|s| s.bottom())
        .fold(outer.top(), f32::max);
    let bottom = siblings
        .iter()
        .filter(|s| overlaps_x(s) && s.top() >= r.bottom() - 0.5)
        .map(|s| s.top())
        .fold(outer.bottom(), f32::min);
    let (cx, cy) = (r.center().x, r.center().y);
    let mut out = Vec::new();
    let mut push = |a: egui::Pos2, b: egui::Pos2| {
        if (b - a).length() >= 0.5 {
            out.push((a, b, a + (b - a) / 2.0));
        }
    };
    push(egui::pos2(left, cy), egui::pos2(r.left(), cy));
    push(egui::pos2(r.right(), cy), egui::pos2(right, cy));
    push(egui::pos2(cx, top), egui::pos2(cx, r.top()));
    push(egui::pos2(cx, r.bottom()), egui::pos2(cx, bottom));
    out
}

/// Rulers along the top and left of the canvas, in points from the form's top-left corner, with
/// the selected node's extent marked.
fn paint_rulers(
    painter: &egui::Painter,
    pal: &Palette,
    area: Rect,
    origin: egui::Pos2,
    selected: Option<Rect>,
) {
    let top = Rect::from_min_max(area.min, egui::pos2(area.right(), area.top() + RULER));
    let left = Rect::from_min_max(
        egui::pos2(area.left(), area.top() + RULER),
        egui::pos2(area.left() + RULER, area.bottom()),
    );
    for strip in [top, left] {
        painter.rect_filled(strip, 0.0, pal.chrome);
    }
    painter.line_segment(
        [top.left_bottom(), top.right_bottom()],
        Stroke::new(1.0, pal.separator),
    );
    painter.line_segment(
        [left.right_top(), left.right_bottom()],
        Stroke::new(1.0, pal.separator),
    );
    if let Some(s) = selected {
        let mark = pal.accent.gamma_multiply(0.35);
        painter.rect_filled(
            Rect::from_min_max(
                egui::pos2(s.left().max(top.left() + RULER), top.top()),
                egui::pos2(s.right(), top.bottom()),
            ),
            0.0,
            mark,
        );
        painter.rect_filled(
            Rect::from_min_max(
                egui::pos2(left.left(), s.top().max(left.top())),
                egui::pos2(left.right(), s.bottom()),
            ),
            0.0,
            mark,
        );
    }
    let font = egui::FontId::proportional(8.5);
    for x in grid_lines(origin.x, top.left() + RULER, top.right(), 10.0) {
        let units = ((x - origin.x) / 10.0).round() as i64;
        let len = if units % 10 == 0 {
            RULER
        } else if units % 5 == 0 {
            7.0
        } else {
            4.0
        };
        painter.line_segment(
            [
                egui::pos2(x, top.bottom() - len),
                egui::pos2(x, top.bottom()),
            ],
            Stroke::new(1.0, pal.text_tertiary),
        );
        if units % 10 == 0 {
            painter.text(
                egui::pos2(x + 2.0, top.top()),
                egui::Align2::LEFT_TOP,
                format!("{}", units * 10),
                font.clone(),
                pal.text_secondary,
            );
        }
    }
    for y in grid_lines(origin.y, left.top(), left.bottom(), 10.0) {
        let units = ((y - origin.y) / 10.0).round() as i64;
        let len = if units % 10 == 0 {
            RULER
        } else if units % 5 == 0 {
            7.0
        } else {
            4.0
        };
        painter.line_segment(
            [
                egui::pos2(left.right() - len, y),
                egui::pos2(left.right(), y),
            ],
            Stroke::new(1.0, pal.text_tertiary),
        );
        if units % 10 == 0 {
            painter.text(
                egui::pos2(left.left() + 1.0, y + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{}", units * 10),
                font.clone(),
                pal.text_secondary,
            );
        }
    }
}

// ---- Tab order -------------------------------------------------------------------------------

/// The focusable widgets in the order Tab visits them (egui moves the focus in the order widgets
/// are created: tree order). Dragging a row moves that widget in the code, before or after the row
/// it is dropped on; the star sets the widget that takes the focus first.
fn tab_order_panel(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    let Some(form) = d.current().map(|f| f.form().clone()) else {
        return;
    };
    section(ui, pal, "TAB ORDER");
    ui.label(
        RichText::new("Tab visits the widgets in the order they are created. Drag a row to move that widget in the code; ★ gives a widget the focus when nothing else has it (response.request_focus()).")
            .small()
            .color(pal.text_tertiary),
    );
    ui.add_space(4.0);
    let order = form.tab_order();
    if order.is_empty() {
        ui.label(RichText::new("No focusable widgets").color(pal.text_secondary));
        return;
    }
    let mut rows: Vec<(NodeId, Rect)> = Vec::new();
    let mut dragged = None;
    let mut clicked = None;
    let mut star = None;
    egui::ScrollArea::vertical()
        .id_salt("designer-tab-order")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (k, id) in order.iter().enumerate() {
                let Some(node) = form.find(*id) else { continue };
                let (has_focus_prop, on) = match &node.kind {
                    NodeKind::Widget { def, props, .. } => (
                        widget_def(*def).prop("initial_focus").is_some(),
                        props.get("initial_focus") == Some(&PropValue::Bool(true)),
                    ),
                    _ => (false, false),
                };
                ui.horizontal(|ui| {
                    let s = ui
                        .add_enabled(
                            has_focus_prop,
                            egui::Button::new(if on { "★" } else { "☆" })
                                .small()
                                .frame(false),
                        )
                        .on_hover_text("Initial focus: takes the focus while nothing else has it")
                        .on_disabled_hover_text("A combo box has no initial focus here");
                    if s.clicked() {
                        star = Some((*id, !on));
                    }
                    let width = ui.available_width().max(80.0);
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
                    let selected = d.selected == Some(*id);
                    if selected {
                        ui.painter().rect_filled(rect, 4.0, pal.row_selected);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 4.0, pal.row_hover);
                    }
                    let title = format!("{}  {}", k + 1, node_title(node));
                    ui.painter().text(
                        egui::pos2(rect.left() + 6.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &title,
                        egui::FontId::proportional(12.5),
                        pal.text,
                    );
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::ListItem, true, selected, &title)
                    });
                    if resp.drag_started() {
                        dragged = Some(*id);
                    } else if resp.clicked() {
                        clicked = Some(*id);
                    }
                    rows.push((*id, rect));
                });
            }
        });
    if let Some((id, on)) = star {
        d.set_initial_focus(id, on);
    }
    if let Some(id) = clicked {
        d.select(Some(id), false);
    }
    if let Some(id) = dragged {
        d.tab_drag = Some(id);
    }
    let Some(moving) = d.tab_drag else { return };
    let (pos, released) = ui.input(|i| (i.pointer.latest_pos(), !i.pointer.any_down()));
    let target = pos.and_then(|p| {
        rows.iter()
            .find(|(_, r)| r.y_range().contains(p.y))
            .map(|(id, r)| (*id, p.y > r.center().y, *r))
    });
    if let Some((_, after, r)) = target {
        let y = if after {
            r.bottom() + 1.0
        } else {
            r.top() - 1.0
        };
        ui.painter().line_segment(
            [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
            Stroke::new(2.0, pal.accent),
        );
    }
    if released {
        d.tab_drag = None;
        if let Some((to, after, _)) = target
            && to != moving
        {
            d.move_in_tab_order(moving, to, after);
        }
    }
}

// ---- Conflict prompt and difference --------------------------------------------------------

fn prompt_dialog(d: &mut Designer, ctx: &egui::Context) {
    let Some(prompt) = d.prompt.clone() else {
        return;
    };
    let (title, detail) = match &prompt {
        Prompt::SourceChanged { form } => (
            format!("The source of {form} has changed"),
            "The .rs form body is not what this form file was exported from. Keep the form file's version (it is saved with the new source hash), re-import the form from the source (the designer's version is replaced), or look at the difference first.".to_string(),
        ),
        Prompt::FormFileChanged { path } => (
            format!("{} changed outside the designer", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())),
            "The form file is not what the designer read. Overwrite it with the designer's edits, reload it (the designer's edits are dropped), or look at the difference first.".to_string(),
        ),
    };
    let mut answer: Option<Option<Action>> = None;
    crate::style::dialog(ctx, "designer-conflict", None, 460.0, |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(&title).strong().size(14.0));
        ui.add_space(4.0);
        ui.label(RichText::new(&detail).weak());
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if crate::style::secondary_button(ui, "Show diff").clicked() {
                answer = Some(Some(Action::ShowDiff(prompt.clone())));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match &prompt {
                    Prompt::SourceChanged { form } => {
                        if crate::style::primary_button(ui, "Keep .frm.rs").clicked() {
                            answer = Some(Some(Action::Keep(form.clone())));
                        }
                        if crate::style::secondary_button(ui, "Re-import").clicked() {
                            answer = Some(Some(Action::Reimport(form.clone())));
                        }
                    }
                    Prompt::FormFileChanged { .. } => {
                        if crate::style::primary_button(ui, "Overwrite").clicked() {
                            answer = Some(Some(Action::Save { overwrite: true }));
                        }
                        if crate::style::secondary_button(ui, "Reload").clicked() {
                            answer = Some(Some(Action::Reload));
                        }
                    }
                }
                if crate::style::secondary_button(ui, "Cancel").clicked() {
                    answer = Some(None);
                }
            });
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(None);
    }
    match answer {
        Some(Some(Action::ShowDiff(p))) => d.actions.push(Action::ShowDiff(p)),
        Some(Some(action)) => {
            d.prompt = None;
            d.actions.push(action);
        }
        Some(None) => d.prompt = None,
        None => {}
    }
}

fn diff_window(d: &mut Designer, ctx: &egui::Context, pal: &Palette) {
    let Some(diff) = d.diff.clone() else { return };
    let mut open = true;
    egui::Window::new(&diff.title)
        .id(egui::Id::unique("designer-diff"))
        .open(&mut open)
        .default_size(vec2(720.0, 480.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!("− {}    + {}", diff.old_label, diff.new_label))
                    .small()
                    .color(pal.text_secondary),
            );
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in crate::assistant::diff::lines(&diff.old, &diff.new) {
                        let (mark, color) = match line.kind {
                            crate::assistant::diff::Kind::Same => (" ", pal.code_text),
                            crate::assistant::diff::Kind::Add => ("+", pal.ok),
                            crate::assistant::diff::Kind::Del => ("−", pal.error),
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{mark} {}", line.text))
                                    .monospace()
                                    .size(11.0)
                                    .color(color),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    }
                });
        });
    if !open {
        d.diff = None;
    }
}
