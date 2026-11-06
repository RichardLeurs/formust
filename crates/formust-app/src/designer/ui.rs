//! The designer's screen: a bar with the form picker, undo and redo and the preview mode; the
//! palette and the outline on the left; the inspector (or "Use in code") on the right; the canvas
//! between them. Drags from the palette, the canvas and the outline drop on the canvas or the
//! outline with an indicator line (see [`edit`]). The conflict prompt and the difference view are
//! dialogs over it.
//!
//! What needs the rest of the IDE (saving, reading other tabs) is queued in [`Designer::actions`]
//! and run by the app after drawing.

use std::path::Path;

use super::edit::{self, Item, Payload, Row};
use super::export::{self, Prompt};
use super::preview::{self, Options};
use super::viewport::{self, TITLE_BAR, View};
use super::{DesignForm, Designer, node_title, show_value};
use crate::style::{Palette, hline, pill, segment};
use eframe::egui::{self, Color32, Frame, Margin, Rect, RichText, Sense, Stroke, StrokeKind, vec2};
use formust_model::{
    ContainerId, DefaultValue, EventId, Form, Node, NodeId, NodeKind, PropDef, PropKind, PropRole,
    PropValue, WidgetId, container_def, widget_def,
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
                    if segment(ui, pal, "Inspector", !d.show_usage).clicked() {
                        d.show_usage = false;
                    }
                    if segment(ui, pal, "Use in code", d.show_usage).clicked() {
                        d.show_usage = true;
                    }
                });
            });
            ui.add_space(4.0);
            if d.show_usage {
                usage(d, ui, pal, read);
            } else {
                inspector(d, ui, pal);
            }
        });
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

/// The first palette: a click adds the item into the selected container or after the selected
/// widget; a drag drops it where the indicator line shows.
fn palette(d: &mut Designer, ui: &mut egui::Ui, pal: &Palette) {
    ui.label(
        RichText::new("PALETTE")
            .size(10.5)
            .strong()
            .color(pal.text_tertiary),
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("designer-palette")
        .max_height(240.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for item in edit::palette() {
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
            });
        });
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
    let mut clicked = None;
    let mut dragged = None;
    let reveal = std::mem::take(&mut d.reveal_row);
    let mut rows = Vec::new();
    let out = egui::ScrollArea::vertical()
        .id_salt("designer-outline-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            row(
                ui,
                pal,
                &form.root,
                0,
                d.selected,
                reveal,
                &mut clicked,
                &mut dragged,
                &mut rows,
            );
        });
    d.rows = rows;
    if let Some(id) = clicked {
        d.select(Some(id), false);
    }
    if let Some(id) = dragged
        && id != form.root.id
    {
        d.drag = Some(Payload::Move(id));
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

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut egui::Ui,
    pal: &Palette,
    n: &Node,
    depth: usize,
    selected: Option<NodeId>,
    reveal: bool,
    clicked: &mut Option<NodeId>,
    dragged: &mut Option<NodeId>,
    rows: &mut Vec<Row>,
) {
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    rows.push(Row {
        id: n.id,
        rect,
        depth,
    });
    let is_sel = selected == Some(n.id);
    if is_sel {
        ui.painter().rect_filled(rect, 4.0, pal.row_selected);
        if reveal {
            resp.scroll_to_me(Some(egui::Align::Center));
        }
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, pal.row_hover);
    }
    let custom = matches!(n.kind, NodeKind::Custom { .. });
    let title = node_title(n);
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
    let resp = resp.on_hover_text(if custom { format!("{title}\nLocked custom code: kept exactly as written. It can be moved, copied and deleted.") } else { title });
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::TreeItem, true, is_sel, node_title(n))
    });
    if resp.drag_started() {
        *dragged = Some(n.id);
    } else if resp.clicked() {
        *clicked = Some(n.id);
    }
    for c in n.children() {
        row(
            ui,
            pal,
            c,
            depth + 1,
            selected,
            reveal,
            clicked,
            dragged,
            rows,
        );
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
            d.move_to(id, drop.parent, drop.index);
        }
    }
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
                prop_sections(ui, pal, wd.props, wd.size_props, props, node.id, &mut changes);
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
                    prop_sections(ui, pal, cd.props, cd.size_props, props, node.id, &mut changes);
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
        }
    }
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

/// The size properties first, then the rest, each with its editor.
fn prop_sections(
    ui: &mut egui::Ui,
    pal: &Palette,
    defs: &'static [PropDef],
    size: &[&str],
    props: &formust_model::Props,
    id: NodeId,
    changes: &mut Vec<(Change, bool)>,
) {
    let sized: Vec<&'static PropDef> = defs.iter().filter(|d| size.contains(&d.name)).collect();
    let rest: Vec<&'static PropDef> = defs.iter().filter(|d| !size.contains(&d.name)).collect();
    if !rest.is_empty() {
        section(ui, pal, "PROPERTIES");
        prop_grid(ui, pal, ("designer-props", id.0), &rest, props, changes);
    }
    if !sized.is_empty() {
        section(ui, pal, "SIZE");
        prop_grid(ui, pal, ("designer-size", id.0), &sized, props, changes);
    }
}

/// Every registry property with its editor. Values equal to a default are not stored
/// (`Form::set_prop`), and an optional property can be cleared back to its default.
fn prop_grid(
    ui: &mut egui::Ui,
    pal: &Palette,
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
                    if let Some(c) = prop_editor(ui, pal, def, set, label_id) {
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
    def: &'static PropDef,
    value: Option<&PropValue>,
    label: egui::Id,
) -> Option<(Change, bool)> {
    let default = def.default.to_value();
    let optional = def.role != PropRole::Arg;
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
                // No default: unset, true or false.
                let mut choice = current;
                let shown = |c: Option<bool>| c.map_or("(not set)".to_string(), |b| b.to_string());
                egui::ComboBox::from_id_salt(("designer-bool", def.name))
                    .selected_text(shown(choice))
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for c in [None, Some(true), Some(false)] {
                            ui.selectable_value(&mut choice, c, shown(c));
                        }
                    });
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
    }
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
    let mut changed = false;
    changed |= ui
        .color_edit_button_srgba_unmultiplied(&mut l)
        .on_hover_text("Light mode")
        .changed();
    let mut pair = dk.is_some();
    if ui
        .checkbox(
            &mut pair,
            RichText::new("dark").small().color(pal.text_secondary),
        )
        .on_hover_text("A separate colour in dark mode")
        .changed()
    {
        dk = pair.then_some(dk.unwrap_or(l));
        changed = true;
    }
    if let Some(d) = dk.as_mut() {
        changed |= ui
            .color_edit_button_srgba_unmultiplied(d)
            .on_hover_text("Dark mode")
            .changed();
    }
    if changed {
        return Some((
            Change::Set(def.name, PropValue::Color { light: l, dark: dk }),
            true,
        ));
    }
    reset(ui, def)
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
    let opts = Options {
        dark,
        selected: d.selected,
        hovered: d.hovered,
        accent: pal.accent,
        stash_accesskit: !d.open_accesskit,
    };
    let drawn = preview::show(ui, area, view.client(), &form, &opts);
    let pass = drawn.pass;
    d.reacted = pass.reacted.clone();
    d.preview_layer = Some(drawn.layer);
    d.client = Some(view.client());
    if pass.kept {
        d.rects = pass.rects;
        let content = pass.content;
        if (content - d.content).length() > 0.5 {
            d.content = content;
            ui.ctx().request_repaint();
        }
        // Act only on the kept pass: Grid's discarded sizing pass has wrong rects.
        let pointer = drawn.shield.hover_pos();
        let hit = pointer.and_then(|p| preview::hit_test(&form.root, &d.rects, p));
        d.hovered = hit;
        if drawn.shield.clicked() {
            let at = drawn.shield.interact_pointer_pos();
            let hit = at.and_then(|p| preview::hit_test(&form.root, &d.rects, p));
            d.select(hit, true);
        }
        // A drag on a node moves it.
        if drawn.shield.drag_started()
            && let Some(at) = ui.input(|i| i.pointer.press_origin())
            && let Some(hit) = preview::hit_test(&form.root, &d.rects, at)
            && hit != form.root.id
        {
            d.select(Some(hit), true);
            d.drag = Some(Payload::Move(hit));
        }
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
    grip(d, ui, pal, view.client(), area);
    if ui.input(|i| i.key_pressed(egui::Key::Escape))
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
