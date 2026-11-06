//! The design-time preview: a form drawn with real egui widgets in real egui layouts, so it looks as
//! it will at runtime, while nothing in it can react to input. The technique was proven by the
//! phase 1 spike (`spikes/preview`, see "Design surface" in docs/specs.md):
//!
//! - **Own layer:** the preview draws on a layer that is never registered as an Area, so the
//!   hit-test and `rect_contains_pointer` never find its widgets (ScrollArea reads the latter).
//!   Only one such layer: unregistered layers paint in hash order.
//! - **Shield:** one `Sense::CLICK | Sense::DRAG` interact on the parent layer, registered after
//!   the preview pass. Not `click_and_drag()`, whose FOCUSABLE sense would make it a Tab stop.
//! - **Focus guard:** egui hands out focus mid-pass, so focus held by a preview widget is evicted
//!   before the pass and after it, and a Tab press is re-issued so IDE fields keep working.
//! - **AccessKit:** assistive-technology actions (Click, SetValue, Increment, ...) don't go through
//!   the pointer path. They are taken out of the input for the length of the preview pass and put
//!   back after it, so preview widgets never see them and IDE widgets drawn later still do. A
//!   Focus action is handled by egui before the pass; the focus guard evicts it.
//! - **Hit-testing:** every node's rect, clipped to the visible canvas, rebuilt every pass. Act
//!   only on a kept pass: Grid's discarded sizing pass has wrong rects.
//!
//! Every widget and container comes from the `formust-model` registry: the matches on
//! [`WidgetId`] and [`ContainerId`] are exhaustive, so a new registry entry doesn't compile until
//! it has a preview.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Button, CollapsingHeader, Color32, ComboBox, DragValue, Frame, Grid, Id, Label,
    LayerId, Layout, Pos2, ProgressBar, Rect, Response, RichText, ScrollArea, Sense, Separator,
    Shape, Slider, Stroke, StrokeKind, TextEdit, TextWrapMode, Ui, UiBuilder, Vec2,
    containers::scroll_area::ScrollSource, vec2,
};
use formust_model::{
    ContainerId, DefaultValue, Form, Node, NodeId, NodeKind, PropValue, Props, WidgetId,
    container_def, is_fill_size, is_fill_text, number, parse_window_anchor, split_pair, widget_def,
};

/// How to draw one pass.
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    /// Draw the form in dark mode (its colours pick their dark value).
    pub dark: bool,
    pub selected: Option<NodeId>,
    pub hovered: Option<NodeId>,
    /// More selected nodes: the rest of a shift-click range.
    pub also: &'a [NodeId],
    /// The selected node's resize handles: (right edge, bottom edge); the corner when both.
    pub handles: (bool, bool),
    /// The handles are on the left and top edges instead (`formust_model::resize_from_start`).
    pub handles_from_start: (bool, bool),
    /// The IDE's accent colour, for the selection marks.
    pub accent: Color32,
    /// Hold AccessKit actions back from the preview. Always on; off only in a test's control run.
    pub stash_accesskit: bool,
    /// The page each Tabs shows, by Tabs id (the first page when absent).
    pub pages: &'a HashMap<NodeId, NodeId>,
    /// The menus drawn open: the selected menu and the menus around the selection.
    pub open_menus: &'a [NodeId],
    /// The folder of the form's file, where image paths are looked up.
    pub base_dir: Option<&'a std::path::Path>,
}

/// What one pass produced.
#[derive(Clone, Debug, Default)]
pub struct Pass {
    /// Node rects clipped to the visible canvas; entirely clipped nodes are absent. Valid for
    /// hit-testing only when `kept`.
    pub rects: HashMap<NodeId, Rect>,
    /// Nodes whose widget reported any interaction (hover, click, change, focus, drag). It must
    /// stay empty: the tests check it, and it is a cheap alarm if an egui upgrade breaks the shield.
    pub reacted: Vec<NodeId>,
    /// The size of everything the form laid out, unclipped: how big the form wants to be.
    pub content: Vec2,
    /// False for a pass egui will discard (Grid's sizing pass).
    pub kept: bool,
    /// The tab buttons of every Tabs bar drawn, with the page each one shows: a click there shows
    /// that page. They are no nodes of their own.
    pub tab_buttons: Vec<(Rect, NodeId)>,
    /// How much larger a container's rect is than its body (unclipped), for the containers whose
    /// body statements a resize writes around a margin: a Frame, Group, Indent, or an open
    /// CollapsingHeader (its header and indent only; nothing trails the body). A resize takes it
    /// off the dragged size.
    pub insets: HashMap<NodeId, Vec2>,
}

/// The shield's interaction this pass, on the parent layer.
pub struct Drawn {
    pub pass: Pass,
    pub shield: Response,
    /// The preview's own layer.
    pub layer: LayerId,
}

/// Draws `form` with its root laid out in `client` and only `area` visible, then shields `area`.
/// `ui` is the IDE's canvas `Ui`; nothing is allocated in it.
pub fn show(ui: &mut Ui, area: Rect, client: Rect, form: &Form, o: &Options<'_>) -> Drawn {
    let base = ui.scope_id().with("formust_preview");
    let layer = LayerId::new(ui.layer_id().order, base);
    let mut pass = Pass {
        kept: !ui.ctx().will_discard(),
        ..Pass::default()
    };
    let stashed = if o.stash_accesskit {
        stash_accesskit(ui.ctx())
    } else {
        Vec::new()
    };
    // A form of panels fills its window as at runtime; a plain `Ui` form gets the margin of the
    // central panel it usually sits in.
    let margin = if has_panels(form) { 0.0 } else { MARGIN };
    let inner = client.shrink(margin);
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt("formust_preview")
            .layer_id(layer)
            .max_rect(inner),
    );
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    child.set_style(preview_style(o.dark));
    let reissue = begin_focus_guard(&mut child, area, base, layer);
    child
        .painter()
        .rect_filled(client, 0.0, child.visuals().panel_fill);
    let mut cx = Cx {
        pass: &mut pass,
        dark: o.dark,
        pages: o.pages.clone(),
        open_menus: o.open_menus.to_vec(),
        base_dir: o.base_dir.map(std::path::Path::to_path_buf),
        popups: Vec::new(),
        in_popup: false,
    };
    node(&mut child, &form.root, &mut cx);
    draw_popups(&mut child, form, client, &mut cx);
    pass.content = child.min_rect().size() + Vec2::splat(2.0 * margin);
    paint_marks(&child, &pass, o);
    // Registered after the preview, on the parent's layer: the preview layer sorts below every
    // registered layer in the hit-test, so a shield there would receive nothing.
    let shield = ui.interact(area, base.with("shield"), Sense::CLICK | Sense::DRAG);
    evict_focus(ui.ctx(), layer);
    if let Some(direction) = reissue {
        ui.memory_mut(|m| m.move_focus(direction));
    }
    restore_accesskit(ui.ctx(), stashed);
    Drawn {
        pass,
        shield,
        layer,
    }
}

/// The margin around a plain `Ui` form, as a `CentralPanel` gives it.
pub const MARGIN: f32 = 8.0;

/// Does the form consist of panels (`Panel::*`, `CentralPanel`)?
pub fn has_panels(form: &Form) -> bool {
    form.root
        .children()
        .iter()
        .any(|c| matches!(&c.kind, NodeKind::Container { def, .. } if container_def(*def).is_panel))
}

/// The style a user's program gets by default, in the chosen mode: the preview shows the form as
/// it runs, not in the IDE's own style. (Fonts are shared by the whole context.)
fn preview_style(dark: bool) -> Arc<egui::Style> {
    let visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    Arc::new(egui::Style {
        visuals,
        ..egui::Style::default()
    })
}

/// The deepest node whose rect contains `pos`; among equals the later one, which paints on top.
pub fn hit_test(root: &Node, rects: &HashMap<NodeId, Rect>, pos: Pos2) -> Option<NodeId> {
    fn walk(
        n: &Node,
        depth: u32,
        r: &HashMap<NodeId, Rect>,
        p: Pos2,
        best: &mut Option<(u32, NodeId)>,
    ) {
        if r.get(&n.id).is_some_and(|rect| rect.contains(p)) && best.is_none_or(|(d, _)| depth >= d)
        {
            *best = Some((depth, n.id));
        }
        for c in n.children() {
            walk(c, depth + 1, r, p, best);
        }
    }
    let mut best = None;
    walk(root, 0, rects, pos, &mut best);
    best.map(|(_, id)| id)
}

// ---- Input isolation -----------------------------------------------------------------------

fn stash_accesskit(ctx: &egui::Context) -> Vec<egui::Event> {
    ctx.input_mut(|i| {
        let (ak, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut i.events)
            .into_iter()
            .partition(|e| matches!(e, egui::Event::AccessKitActionRequest(_)));
        i.events = rest;
        ak
    })
}

fn restore_accesskit(ctx: &egui::Context, stashed: Vec<egui::Event>) {
    if !stashed.is_empty() {
        ctx.input_mut(|i| i.events.extend(stashed));
    }
}

/// Clears focus if a preview widget holds it. Works for widgets with internal ids too (the
/// DragValue inside a Slider), because it asks which layer the focused widget is on.
fn evict_focus(ctx: &egui::Context, layer: LayerId) {
    if let Some(id) = ctx.memory(|m| m.focused())
        && ctx.read_response(id).is_some_and(|r| r.layer_id == layer)
    {
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
}

/// Before the pass: evict, absorb a pending hand-over from a widget earlier in the frame with a
/// zero-size sentinel, evict again (Shift+Tab with nothing focused hands focus to the last
/// focusable widget of the previous pass, a preview widget) and clear the Tab direction. Returns
/// the direction to re-issue afterwards, so Tab carries on to IDE widgets drawn after the canvas.
fn begin_focus_guard(
    ui: &mut Ui,
    canvas: Rect,
    base: Id,
    layer: LayerId,
) -> Option<egui::FocusDirection> {
    evict_focus(ui.ctx(), layer);
    let at = Rect::from_min_size(canvas.min, Vec2::ZERO);
    let _ = ui.interact(
        at,
        base.with("focus_sentinel"),
        Sense::focusable_noninteractive(),
    );
    evict_focus(ui.ctx(), layer);
    let direction = ui.input(|i| {
        if !i.key_pressed(egui::Key::Tab) {
            None
        } else if i.modifiers.shift_only() {
            Some(egui::FocusDirection::Previous)
        } else {
            Some(egui::FocusDirection::Next)
        }
    });
    ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    direction
}

// ---- Marks ---------------------------------------------------------------------------------

/// Where the resize handles of a selected node's rect are: the width's edge, the height's edge and
/// their corner. The right and bottom edges, or the left and top ones where `from_start` says so (a
/// right or bottom panel).
pub fn handle_rects(
    r: Rect,
    (width, height): (bool, bool),
    (left, top): (bool, bool),
) -> [Option<Rect>; 3] {
    let size = vec2(HANDLE, HANDLE);
    let x = if left { r.left() } else { r.right() };
    let y = if top { r.top() } else { r.bottom() };
    [
        width.then(|| Rect::from_center_size(egui::pos2(x, r.center().y), size)),
        height.then(|| Rect::from_center_size(egui::pos2(r.center().x, y), size)),
        (width && height).then(|| Rect::from_center_size(egui::pos2(x, y), size)),
    ]
}

/// The side of a resize handle's square.
pub const HANDLE: f32 = 7.0;

/// Hover and selection outlines, on the preview layer (it draws above the parent layer).
fn paint_marks(ui: &Ui, pass: &Pass, o: &Options<'_>) {
    let painter = ui.painter();
    if let Some(r) = o
        .hovered
        .filter(|h| Some(*h) != o.selected)
        .and_then(|h| pass.rects.get(&h))
    {
        painter.rect_stroke(
            *r,
            2.0,
            Stroke::new(1.0, o.accent.gamma_multiply(0.5)),
            StrokeKind::Outside,
        );
    }
    for r in o.also.iter().filter_map(|s| pass.rects.get(s)) {
        painter.rect_stroke(*r, 2.0, Stroke::new(1.5, o.accent), StrokeKind::Outside);
    }
    if let Some(r) = o.selected.and_then(|s| pass.rects.get(&s)) {
        painter.rect_stroke(*r, 2.0, Stroke::new(2.0, o.accent), StrokeKind::Outside);
        let resizable = o.handles.0 || o.handles.1;
        let squares: Vec<Rect> = if resizable {
            handle_rects(*r, o.handles, o.handles_from_start)
                .into_iter()
                .flatten()
                .collect()
        } else {
            [
                r.left_top(),
                r.right_top(),
                r.left_bottom(),
                r.right_bottom(),
            ]
            .into_iter()
            .map(|p| Rect::from_center_size(p, vec2(6.0, 6.0)))
            .collect()
        };
        for square in squares {
            painter.rect(
                square,
                1.0,
                Color32::WHITE,
                Stroke::new(1.0, o.accent),
                StrokeKind::Inside,
            );
        }
    }
}

// ---- Nodes ---------------------------------------------------------------------------------

struct Cx<'a> {
    pass: &'a mut Pass,
    dark: bool,
    pages: HashMap<NodeId, NodeId>,
    open_menus: Vec<NodeId>,
    base_dir: Option<std::path::PathBuf>,
    /// Open menus and modals, drawn after the form so they lie on top of it.
    popups: Vec<Popup>,
    /// Drawing inside an open menu: a menu here is a submenu.
    in_popup: bool,
}

/// Something drawn over the form after it.
#[derive(Clone, Copy, Debug)]
enum Popup {
    /// An open menu's items, below (or beside, for a submenu) its button.
    Menu { id: NodeId, at: Pos2 },
    /// A modal: a dimmed backdrop over the form and the dialog's frame on it.
    Modal(NodeId),
}

/// Draws the open menus and the modals over the form, in the order they were met (a submenu is met
/// while its menu is drawn, so it lies on top of it). A menu's rect grows by its items' frame, so a
/// drop or click there finds the menu.
fn draw_popups(ui: &mut Ui, form: &Form, client: Rect, cx: &mut Cx) {
    let mut k = 0;
    while k < cx.popups.len() && k < 64 {
        let popup = cx.popups[k];
        k += 1;
        match popup {
            Popup::Menu { id, at } => {
                let Some(n) = form.find(id) else { continue };
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(at, vec2(280.0, 2000.0)))
                        .layout(Layout::top_down(Align::Min)),
                );
                egui::containers::menu::menu_style(child.style_mut());
                let style = child.style().clone();
                let was = std::mem::replace(&mut cx.in_popup, true);
                let r = Frame::menu(&style)
                    .show(&mut child, |ui| {
                        ui.set_min_width(120.0);
                        ui.set_max_width(260.0);
                        kids(ui, n, cx);
                    })
                    .response;
                cx.in_popup = was;
                let visible = r.rect.intersect(ui.clip_rect());
                if visible.is_positive() {
                    let rect = match cx.pass.rects.get(&id) {
                        Some(button) => button.union(visible),
                        None => visible,
                    };
                    cx.pass.rects.insert(id, rect);
                }
            }
            Popup::Modal(id) => {
                let Some(n) = form.find(id) else { continue };
                let backdrop = match n.kind {
                    NodeKind::Container { ref props, .. } => {
                        as_color(props.get("backdrop_color").cloned(), cx.dark)
                    }
                    _ => None,
                }
                .unwrap_or(Color32::from_black_alpha(if cx.dark {
                    160
                } else {
                    100
                }));
                ui.painter()
                    .rect_filled(client.intersect(ui.clip_rect()), 0.0, backdrop);
                // Centred by the width it had in the last pass, as egui centres a modal's area.
                let key = Id::unique(("formust_modal_width", id.0));
                let width = ui.ctx().data(|d| d.get_temp::<f32>(key)).unwrap_or(240.0);
                let room = client.shrink(24.0);
                let left = (client.center().x - width / 2.0).max(room.left());
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_max(
                            egui::pos2(left, room.top() + 40.0),
                            room.max,
                        ))
                        .layout(Layout::top_down(Align::Min)),
                );
                let style = child.style().clone();
                let r = Frame::popup(&style)
                    .inner_margin(12.0)
                    .show(&mut child, |ui| kids(ui, n, cx))
                    .response;
                if (r.rect.width() - width).abs() > 0.5 {
                    ui.ctx().data_mut(|d| d.insert_temp(key, r.rect.width()));
                    ui.ctx().request_repaint();
                }
                cx.rec_rect(ui, id, r.rect);
            }
        }
    }
}

impl Cx<'_> {
    fn rec(&mut self, ui: &Ui, id: NodeId, r: &Response, rect: Rect) {
        if r.clicked()
            || r.hovered()
            || r.contains_pointer()
            || r.changed()
            || r.has_focus()
            || r.dragged()
        {
            self.pass.reacted.push(id);
        }
        let visible = rect.intersect(ui.clip_rect());
        if visible.is_positive() {
            self.pass.rects.insert(id, visible);
        }
    }
}

fn kids(ui: &mut Ui, n: &Node, cx: &mut Cx) {
    if let NodeKind::Container { props, .. } = &n.kind {
        apply_body(ui, props);
    }
    let children = n.children();
    if children.is_empty() {
        empty_placeholder(ui);
    }
    for c in children {
        node(ui, c, cx);
    }
}

/// The children in the container's body `Ui`, and the rect that body took.
fn body_rect(ui: &mut Ui, n: &Node, cx: &mut Cx) -> Rect {
    kids(ui, n, cx);
    ui.min_rect()
}

/// An empty container gets a dashed box at design time, so it can be seen and selected: the one
/// deliberate difference from runtime.
fn empty_placeholder(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(48.0, 18.0), Sense::hover());
    let color = ui.visuals().weak_text_color();
    let r = rect.shrink(1.0);
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    ui.painter().extend(Shape::dashed_line(
        &corners,
        Stroke::new(1.0, color),
        3.0,
        2.0,
    ));
}

fn node(ui: &mut Ui, n: &Node, cx: &mut Cx) {
    match &n.kind {
        NodeKind::Widget {
            def,
            props,
            extra_chain,
            ..
        } => {
            // Disabled as at runtime; hidden is dimmed rather than removed, so it can be selected.
            let disabled = literal_off(props, "enabled");
            let hidden = literal_off(props, "visible");
            let r = if disabled || hidden {
                ui.scope(|ui| {
                    if disabled {
                        ui.disable();
                    }
                    if hidden {
                        ui.multiply_opacity(HIDDEN_OPACITY);
                    }
                    widget(ui, n.id, *def, props, cx.dark, cx.base_dir.as_deref())
                })
                .inner
            } else {
                widget(ui, n.id, *def, props, cx.dark, cx.base_dir.as_deref())
            };
            if let Some(r) = r {
                cx.rec(ui, n.id, &r, r.rect);
                if !extra_chain.is_empty() {
                    badge(ui, r.rect);
                }
                if hidden {
                    hidden_mark(ui, r.rect);
                }
            }
        }
        NodeKind::Container {
            def, props, shell, ..
        } => {
            let head = shell.as_ref().and_then(|s| s.head.as_deref());
            container(ui, n, *def, props, head, cx);
        }
        NodeKind::Custom { text } => {
            let r = custom_block(ui, text);
            cx.rec(ui, n.id, &r, r.rect);
        }
    }
}

/// How a widget set invisible shows at design time.
pub const HIDDEN_OPACITY: f32 = 0.3;

/// A dashed outline around a widget that is invisible at runtime (`visible` is `false`).
fn hidden_mark(ui: &Ui, rect: Rect) {
    let r = rect.expand(1.0);
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    ui.painter().extend(Shape::dashed_line(
        &corners,
        Stroke::new(1.0, ui.visuals().weak_text_color()),
        3.0,
        3.0,
    ));
}

/// A small mark on a widget whose chain has builder methods the designer doesn't know: the preview
/// can't show what they do.
fn badge(ui: &Ui, rect: Rect) {
    let c = rect.right_top() + vec2(-3.0, 3.0);
    ui.painter().circle(
        c,
        3.0,
        Color32::from_rgb(0xE0, 0x8A, 0x00),
        Stroke::new(1.0, ui.visuals().panel_fill),
    );
}

/// Locked custom code: a box with the statement's own text (the first lines of it).
fn custom_block(ui: &mut Ui, text: &str) -> Response {
    const LINES: usize = 6;
    const COLUMNS: usize = 72;
    let body = dedent(text);
    let all: Vec<&str> = body.lines().collect();
    let mut shown: Vec<String> = all
        .iter()
        .take(LINES)
        .map(|l| {
            if l.chars().count() > COLUMNS {
                format!("{}…", l.chars().take(COLUMNS).collect::<String>())
            } else {
                (*l).to_string()
            }
        })
        .collect();
    if all.len() > LINES {
        shown.push(format!("… {} more lines", all.len() - LINES));
    }
    let v = ui.visuals().clone();
    Frame::new()
        .fill(v.faint_bg_color)
        .stroke(Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.add(
                Label::new(
                    RichText::new("Custom code (locked)")
                        .small()
                        .color(v.weak_text_color()),
                )
                .wrap_mode(TextWrapMode::Extend),
            );
            ui.add(
                Label::new(RichText::new(shown.join("\n")).monospace())
                    .wrap_mode(TextWrapMode::Extend),
            );
        })
        .response
}

/// Removes the indentation the source gave the lines after the first (the first starts at the
/// statement, so it has none).
pub fn dedent(text: &str) -> String {
    let text = text.trim_end();
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    let rest: Vec<&str> = lines.collect();
    let indent = rest
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let cut = indent;
    let mut out = first.to_string();
    for l in rest {
        out.push('\n');
        out.push_str(l.get(cut..).unwrap_or(l.trim_start()));
    }
    out
}

// ---- Property values -----------------------------------------------------------------------

fn default_of(def: Option<&formust_model::PropDef>) -> Option<PropValue> {
    def.and_then(|d| match d.default {
        DefaultValue::None => None,
        other => other.to_value(),
    })
}

fn widget_prop(props: &Props, w: WidgetId, name: &str) -> Option<PropValue> {
    props
        .get(name)
        .cloned()
        .or_else(|| default_of(widget_def(w).prop(name)))
}

fn container_prop(props: &Props, c: ContainerId, name: &str) -> Option<PropValue> {
    props
        .get(name)
        .cloned()
        .or_else(|| default_of(container_def(c).prop(name)))
}

/// The contents of the first `"..."` literal in some source text, an escape read as the character
/// after its backslash: a dialog helper call's title, `dialog(ctx, "About", ..` to `About`.
fn first_string_literal(src: &str) -> Option<String> {
    let start = src.find('"')?;
    let mut out = String::new();
    let mut chars = src[start + 1..].chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            _ => out.push(ch),
        }
    }
    None
}

/// Text as the preview shows it: a literal as is, an expression as its source text.
fn as_text(v: Option<PropValue>) -> Option<String> {
    match v? {
        PropValue::Text(s) | PropValue::Expr(s) => Some(s),
        PropValue::Enum(s) => Some(s.to_string()),
        PropValue::Bool(b) => Some(b.to_string()),
        PropValue::Int(i) => Some(i.to_string()),
        PropValue::Float(f) => Some(f.to_string()),
        PropValue::Color { .. } | PropValue::Items(_) | PropValue::Stroke { .. } => None,
    }
}

fn as_bool(v: Option<PropValue>) -> Option<bool> {
    match v? {
        PropValue::Bool(b) => Some(b),
        _ => None,
    }
}

fn as_f64(v: Option<PropValue>) -> Option<f64> {
    match v? {
        PropValue::Float(f) => Some(f),
        PropValue::Int(i) => Some(i as f64),
        PropValue::Expr(e) => e
            .trim()
            .trim_end_matches("_f32")
            .trim_end_matches("_f64")
            .trim_end_matches("f32")
            .trim_end_matches("f64")
            .parse()
            .ok(),
        _ => None,
    }
}

/// One length of a size expression: a literal, the available width (Fill), or the usual widget
/// height `ui.spacing().interact_size.y`.
fn length(ui: &Ui, text: &str) -> Option<f32> {
    if is_fill_text(text) {
        return Some(ui.available_width());
    }
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.ends_with("spacing().interact_size.y") {
        return Some(ui.spacing().interact_size.y);
    }
    if t.ends_with("spacing().interact_size.x") {
        return Some(ui.spacing().interact_size.x);
    }
    number(&t).map(|v| v as f32)
}

/// A size: `[w, h]`, `vec2(w, h)`, or the designer's Fill marker for `add_sized`.
fn size_value(ui: &Ui, v: Option<PropValue>) -> Option<Vec2> {
    let v = v?;
    if is_fill_size(&v) && matches!(v, PropValue::Float(_)) {
        return Some(vec2(ui.available_width(), ui.spacing().interact_size.y));
    }
    match v {
        PropValue::Expr(t) => {
            let (w, h) = split_pair(&t)?;
            Some(vec2(length(ui, &w)?, length(ui, &h)?))
        }
        _ => None,
    }
}

/// A width: a number, or Fill (`f32::INFINITY`, `f32::MAX`, `ui.available_width()`) as the width
/// available here, which is what each of those comes to at runtime.
fn width_value(ui: &Ui, v: Option<PropValue>) -> Option<f32> {
    match v? {
        PropValue::Float(x) if x.is_infinite() && x > 0.0 => Some(ui.available_width()),
        PropValue::Float(x) => Some(x as f32),
        PropValue::Int(i) => Some(i as f32),
        PropValue::Expr(e) => length(ui, &e),
        _ => None,
    }
}

/// `ui.add(w)`, or `ui.add_sized(size, w)` when the widget has a size.
fn put(ui: &mut Ui, sized: Option<Vec2>, w: impl egui::Widget) -> Response {
    match sized {
        Some(size) => ui.add_sized(size, w),
        None => ui.add(w),
    }
}

fn as_enum(v: Option<PropValue>) -> Option<&'static str> {
    match v? {
        PropValue::Enum(s) => Some(s),
        _ => None,
    }
}

fn as_color(v: Option<PropValue>, dark: bool) -> Option<Color32> {
    match v? {
        PropValue::Color { light, dark: d } => {
            let c = if dark { d.unwrap_or(light) } else { light };
            Some(Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
        }
        _ => None,
    }
}

/// A literal stroke in the preview's mode.
fn as_stroke(v: Option<PropValue>, dark: bool) -> Option<Stroke> {
    match v? {
        PropValue::Stroke {
            width,
            light,
            dark: d,
        } => {
            let color = as_color(Some(PropValue::Color { light, dark: d }), dark)?;
            Some(Stroke::new(width as f32, color))
        }
        _ => None,
    }
}

/// A literal pair, `egui::vec2(8.0, 3.0)` or `[8.0, 3.0]`.
fn as_vec2(v: Option<PropValue>) -> Option<Vec2> {
    match v? {
        PropValue::Expr(t) => {
            let (x, y) = formust_model::parse_vec2(&t)?;
            Some(vec2(x as f32, y as f32))
        }
        _ => None,
    }
}

/// The widget's text with its styling (font size, colour, bold and so on), as
/// `egui::RichText::new(text).size(..).strong()..` in the exported code.
fn rich(props: &Props, w: WidgetId, text: String, dark: bool) -> RichText {
    let mut t = RichText::new(text);
    if let Some(size) = as_f64(props.get("font_size").cloned()) {
        t = t.size(size as f32);
    }
    if let Some(c) = as_color(props.get("text_color").cloned(), dark) {
        t = t.color(c);
    }
    let on = |name: &str| as_bool(widget_prop(props, w, name)) == Some(true);
    if on("bold") {
        t = t.strong();
    }
    if on("italic") {
        t = t.italics();
    }
    if on("underline") {
        t = t.underline();
    }
    if on("strikethrough") {
        t = t.strikethrough();
    }
    if on("monospace") {
        t = t.monospace();
    }
    t
}

/// A literal `false`, the only enabled or visible value the preview can know is off: an
/// expression such as `self.ready` previews as on.
fn literal_off(props: &Props, name: &str) -> bool {
    props.get(name) == Some(&PropValue::Bool(false))
}

/// The spacing and size statements of a container's body, applied to its `Ui` as at runtime.
fn apply_body(ui: &mut Ui, props: &Props) {
    let f = |name: &str| as_f64(props.get(name).cloned()).map(|v| v as f32);
    if let Some(v) = f("set_min_width") {
        ui.set_min_width(v);
    }
    if let Some(v) = f("set_min_height") {
        ui.set_min_height(v);
    }
    if let Some(v) = f("set_max_width") {
        ui.set_max_width(v);
    }
    if let Some(v) = f("set_max_height") {
        ui.set_max_height(v);
    }
    if let Some(v) = as_vec2(props.get("item_spacing").cloned()) {
        ui.spacing_mut().item_spacing = v;
    }
    if let Some(v) = as_vec2(props.get("button_padding").cloned()) {
        ui.spacing_mut().button_padding = v;
    }
    if let Some(v) = f("indent") {
        ui.spacing_mut().indent = v;
    }
}

/// `0.0..=100.0`, `0..=10`, `-1.0..1.0`: the bounds of a literal range, else `0..=1`.
pub fn parse_range(expr: Option<PropValue>) -> std::ops::RangeInclusive<f64> {
    let parse = |s: &str| {
        s.trim()
            .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '_')
            .trim_end_matches('_')
            .parse::<f64>()
            .ok()
    };
    if let Some(PropValue::Expr(e)) = expr {
        let e = e.trim();
        let (lo, hi) = e
            .split_once("..=")
            .or_else(|| e.split_once(".."))
            .unwrap_or(("", ""));
        if let (Some(lo), Some(hi)) = (parse(lo), parse(hi))
            && lo <= hi
        {
            return lo..=hi;
        }
    }
    0.0..=1.0
}

fn wrap_mode(props: &Props, w: WidgetId) -> Option<TextWrapMode> {
    if as_bool(widget_prop(props, w, "wrap")) == Some(true) {
        return Some(TextWrapMode::Wrap);
    }
    if as_bool(widget_prop(props, w, "truncate")) == Some(true) {
        return Some(TextWrapMode::Truncate);
    }
    if as_bool(widget_prop(props, w, "extend")) == Some(true) {
        return Some(TextWrapMode::Extend);
    }
    match as_enum(widget_prop(props, w, "wrap_mode")) {
        Some("Wrap") => Some(TextWrapMode::Wrap),
        Some("Truncate") => Some(TextWrapMode::Truncate),
        Some("Extend") => Some(TextWrapMode::Extend),
        _ => None,
    }
}

// ---- Widgets -------------------------------------------------------------------------------

/// One widget, on throwaway values: a binding such as `&mut self.name` previews as its type's
/// empty value, and nothing can write back. `None` for widgets with no rect (space, end of row).
fn widget(
    ui: &mut Ui,
    id: NodeId,
    w: WidgetId,
    props: &Props,
    dark: bool,
    base_dir: Option<&std::path::Path>,
) -> Option<Response> {
    let text = |name: &str| as_text(widget_prop(props, w, name)).unwrap_or_default();
    let styled = || rich(props, w, text("text"), dark);
    let salt = ("formust_node", id.0);
    let sized = size_value(ui, props.get("add_sized").cloned());
    let min_size = size_value(ui, props.get("min_size").cloned());
    let r = match w {
        WidgetId::Label => {
            let mut l = Label::new(styled());
            if let Some(m) = wrap_mode(props, w) {
                l = l.wrap_mode(m);
            }
            put(ui, sized, l)
        }
        // `ui.heading(t)` is `Label::new(t.heading())`; sized, as the export writes it.
        WidgetId::Heading => put(ui, sized, Label::new(styled().heading())),
        WidgetId::Small => put(ui, sized, Label::new(styled().small())),
        WidgetId::Strong => ui.strong(styled()),
        WidgetId::Weak => put(ui, sized, Label::new(styled().weak())),
        WidgetId::Monospace => ui.monospace(styled()),
        WidgetId::Button => {
            let mut b = Button::new(styled());
            if as_bool(widget_prop(props, w, "small")) == Some(true) {
                b = b.small();
            }
            if let Some(m) = wrap_mode(props, w) {
                b = b.wrap_mode(m);
            }
            if let Some(c) = as_color(widget_prop(props, w, "fill"), dark) {
                b = b.fill(c);
            }
            if let Some(s) = as_stroke(widget_prop(props, w, "stroke"), dark) {
                b = b.stroke(s);
            }
            if let Some(f) = as_bool(widget_prop(props, w, "frame")) {
                b = b.frame(f);
            }
            if as_bool(widget_prop(props, w, "selected")) == Some(true) {
                b = b.selected(true);
            }
            if let Some(m) = min_size {
                b = b.min_size(m);
            }
            put(ui, sized, b)
        }
        WidgetId::Link => put(ui, sized, egui::Link::new(styled())),
        WidgetId::Hyperlink => {
            let url = text("url");
            match as_text(props.get("text").cloned()) {
                Some(label) => put(ui, sized, egui::Hyperlink::from_label_and_url(label, url)),
                None => put(ui, sized, egui::Hyperlink::new(url)),
            }
        }
        WidgetId::Checkbox => {
            let mut v = false;
            let c = egui::Checkbox::new(&mut v, text("text"))
                .indeterminate(as_bool(widget_prop(props, w, "indeterminate")) == Some(true));
            put(ui, sized, c)
        }
        WidgetId::ToggleValue => ui.toggle_value(&mut false, text("text")),
        WidgetId::RadioValue => ui.radio(false, text("text")),
        WidgetId::SelectableValue => ui.selectable_label(false, text("text")),
        WidgetId::TextEditSingleline | WidgetId::TextEditMultiline => {
            let mut v = String::new();
            let mut t = if w == WidgetId::TextEditSingleline {
                TextEdit::singleline(&mut v)
            } else {
                TextEdit::multiline(&mut v)
            };
            t = t.id_salt(salt);
            if let Some(h) = as_text(props.get("hint_text").cloned()) {
                t = t.hint_text(h);
            }
            if let Some(x) = width_value(ui, widget_prop(props, w, "desired_width")) {
                t = t.desired_width(x);
            }
            if let Some(m) = min_size {
                t = t.min_size(m);
            }
            if let Some(x) = as_f64(widget_prop(props, w, "desired_rows")) {
                t = t.desired_rows(x.max(1.0) as usize);
            }
            if as_bool(widget_prop(props, w, "code_editor")) == Some(true) {
                t = t.code_editor();
            }
            if let Some(c) = as_color(widget_prop(props, w, "text_color"), dark) {
                t = t.text_color(c);
            }
            if let Some(c) = as_color(widget_prop(props, w, "background_color"), dark) {
                t = t.background_color(c);
            }
            put(ui, sized, t)
        }
        WidgetId::Slider => {
            let range = parse_range(widget_prop(props, w, "range"));
            let mut v = *range.start();
            let mut s = Slider::new(&mut v, range);
            if let Some(t) = as_text(props.get("text").cloned()) {
                s = s.text(t);
            }
            if let Some(c) = as_color(widget_prop(props, w, "text_color"), dark) {
                s = s.text_color(c);
            }
            if as_bool(widget_prop(props, w, "integer")) == Some(true) {
                s = s.integer();
            }
            if as_bool(widget_prop(props, w, "vertical")) == Some(true)
                || as_enum(props.get("orientation").cloned()) == Some("Vertical")
            {
                s = s.vertical();
            }
            if let Some(b) = as_bool(widget_prop(props, w, "show_value")) {
                s = s.show_value(b);
            }
            if let Some(p) = as_text(props.get("prefix").cloned()) {
                s = s.prefix(p);
            }
            if let Some(p) = as_text(props.get("suffix").cloned()) {
                s = s.suffix(p);
            }
            put(ui, sized, s)
        }
        WidgetId::DragValue => {
            let mut v = 0.0_f64;
            let mut d = DragValue::new(&mut v);
            if let Some(p) = as_text(props.get("prefix").cloned()) {
                d = d.prefix(p);
            }
            if let Some(p) = as_text(props.get("suffix").cloned()) {
                d = d.suffix(p);
            }
            if let Some(m) = min_size {
                d = d.min_size(m);
            }
            put(ui, sized, d)
        }
        WidgetId::ProgressBar => {
            let progress = as_f64(widget_prop(props, w, "progress")).unwrap_or(0.5) as f32;
            // Never animated: an animation would repaint the IDE forever.
            let mut p = ProgressBar::new(progress.clamp(0.0, 1.0)).animate(false);
            if let Some(t) = as_text(props.get("text").cloned()) {
                p = p.text(t);
            }
            if as_bool(widget_prop(props, w, "show_percentage")) == Some(true) {
                p = p.show_percentage();
            }
            if let Some(x) = width_value(ui, widget_prop(props, w, "desired_width")) {
                p = p.desired_width(x);
            }
            if let Some(x) = as_f64(widget_prop(props, w, "desired_height")) {
                p = p.desired_height(x as f32);
            }
            if let Some(c) = as_color(widget_prop(props, w, "fill"), dark) {
                p = p.fill(c);
            }
            put(ui, sized, p)
        }
        WidgetId::Spinner => {
            let size = as_f64(widget_prop(props, w, "size"))
                .map_or(ui.style().spacing.interact_size.y, |s| s as f32);
            let color = as_color(widget_prop(props, w, "color"), dark);
            match sized {
                Some(s) => {
                    let layout = Layout::centered_and_justified(ui.layout().main_dir());
                    ui.allocate_ui_with_layout(s, layout, |ui| static_spinner(ui, size, color))
                        .inner
                }
                None => static_spinner(ui, size, color),
            }
        }
        WidgetId::Separator => {
            let mut s = Separator::default();
            if let Some(x) = as_f64(widget_prop(props, w, "spacing")) {
                s = s.spacing(x as f32);
            }
            if as_bool(widget_prop(props, w, "horizontal")) == Some(true) {
                s = s.horizontal();
            }
            if as_bool(widget_prop(props, w, "vertical")) == Some(true) {
                s = s.vertical();
            }
            put(ui, sized, s)
        }
        WidgetId::AddSpace => {
            ui.add_space(as_f64(widget_prop(props, w, "amount")).unwrap_or(8.0) as f32);
            return None;
        }
        WidgetId::ComboBox => {
            let items: Vec<String> = match props.get("items") {
                Some(PropValue::Items(items)) => items
                    .iter()
                    .map(|i| as_text(Some(i.label.clone())).unwrap_or_default())
                    .collect(),
                _ => Vec::new(),
            };
            let selected = as_text(props.get("selected_text").cloned())
                .map(|s| unquote_format(&s))
                .or_else(|| items.first().cloned())
                .unwrap_or_default();
            let mut c = match as_text(props.get("label").cloned()) {
                Some(label) => ComboBox::new(salt, label),
                None => ComboBox::from_id_salt(salt),
            };
            c = c.selected_text(selected);
            if let Some(x) = width_value(ui, widget_prop(props, w, "width")) {
                c = c.width(x);
            }
            c.show_ui(ui, |ui| {
                for item in &items {
                    let _ = ui.selectable_label(false, item.as_str());
                }
            })
            .response
        }
        WidgetId::EndRow => {
            ui.end_row();
            return None;
        }
        WidgetId::Image => image_preview(ui, props, dark, base_dir, sized),
        WidgetId::ColorEdit => {
            let mut c = PREVIEW_COLOR;
            ui.color_edit_button_srgba(&mut c)
        }
        WidgetId::ColorEditRgb => {
            let mut rgb = [
                f32::from(PREVIEW_COLOR.r()) / 255.0,
                f32::from(PREVIEW_COLOR.g()) / 255.0,
                f32::from(PREVIEW_COLOR.b()) / 255.0,
            ];
            ui.color_edit_button_rgb(&mut rgb)
        }
        WidgetId::RangeSlider => {
            let range = parse_range(widget_prop(props, w, "range"));
            let span = range.end() - range.start();
            let (mut lo, mut hi) = (range.start() + span * 0.25, range.start() + span * 0.75);
            let mut s = egui::RangeSlider::new(&mut lo, &mut hi, range);
            if let Some(t) = as_text(props.get("text").cloned()) {
                s = s.text(t);
            }
            if let Some(c) = as_color(widget_prop(props, w, "text_color"), dark) {
                s = s.text_color(c);
            }
            if as_bool(widget_prop(props, w, "integer")) == Some(true) {
                s = s.integer();
            }
            if as_bool(widget_prop(props, w, "vertical")) == Some(true)
                || as_enum(props.get("orientation").cloned()) == Some("Vertical")
            {
                s = s.vertical();
            }
            if let Some(b) = as_bool(widget_prop(props, w, "show_value")) {
                s = s.show_value(b);
            }
            if let Some(p) = as_text(props.get("prefix").cloned()) {
                s = s.prefix(p);
            }
            if let Some(p) = as_text(props.get("suffix").cloned()) {
                s = s.suffix(p);
            }
            put(ui, sized, s)
        }
    };
    Some(r)
}

/// The colour a colour picker shows at design time (its binding is not read).
const PREVIEW_COLOR: Color32 = Color32::from_rgb(0x4A, 0x90, 0xD9);

/// The local file an image's source names, if any: `file://path`, a plain path, or the path in
/// `include_image!("path")` (relative to the form's file, as the macro reads it).
pub fn image_path(props: &Props, base_dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    let text = match props.get("uri").or_else(|| props.get("source")) {
        Some(PropValue::Text(t)) => t.clone(),
        Some(PropValue::Expr(e)) if e.contains("include_image!") => first_string_literal(e)?,
        Some(PropValue::Expr(_)) => return None,
        _ => match widget_def(WidgetId::Image)
            .prop("source")
            .map(|p| p.default)
        {
            Some(DefaultValue::Text(t)) => t.to_string(),
            _ => return None,
        },
    };
    if text.contains("://") && !text.starts_with("file://") {
        return None;
    }
    let path = std::path::PathBuf::from(text.trim_start_matches("file://"));
    Some(match base_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    })
}

/// The name an image placeholder shows: the file name, or the source as written.
fn image_name(props: &Props) -> String {
    let text = match props.get("uri").or_else(|| props.get("source")) {
        Some(PropValue::Text(t) | PropValue::Expr(t)) => t.clone(),
        _ => "image".to_string(),
    };
    let inner = first_string_literal(&text).unwrap_or(text);
    inner
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&inner)
        .to_string()
}

/// A local PNG as a texture, loaded once and kept in the context's memory (a failure too).
fn image_texture(ctx: &egui::Context, path: &std::path::Path) -> Option<egui::TextureHandle> {
    let key = Id::unique(("formust_image", path));
    if let Some(cached) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(key)) {
        return cached;
    }
    let loaded = std::fs::read(path)
        .ok()
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .map(|img| {
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            ctx.load_texture(
                format!("formust-image-{}", path.display()),
                color,
                egui::TextureOptions::LINEAR,
            )
        });
    ctx.data_mut(|d| d.insert_temp(key, loaded.clone()));
    loaded
}

/// An image as at runtime when the preview can read the file (a local PNG), else a placeholder box
/// with the file name. The designer installs no image loaders: a URL is always a placeholder.
fn image_preview(
    ui: &mut Ui,
    props: &Props,
    dark: bool,
    base_dir: Option<&std::path::Path>,
    sized: Option<Vec2>,
) -> Response {
    let texture = image_path(props, base_dir).and_then(|p| image_texture(ui.ctx(), &p));
    let natural = texture.as_ref().map_or(vec2(96.0, 64.0), |t| t.size_vec2());
    let float = |name: &str| as_f64(props.get(name).cloned()).map(|v| v as f32);
    let mut size = match as_vec2(props.get("fit_to_exact_size").cloned()) {
        Some(exact) => exact,
        None => {
            let mut s = natural * float("fit_to_original_size").unwrap_or(1.0);
            let mut max = as_vec2(props.get("max_size").cloned()).unwrap_or(Vec2::INFINITY);
            if let Some(w) = float("max_width") {
                max.x = max.x.min(w);
            }
            if let Some(h) = float("max_height") {
                max.y = max.y.min(h);
            }
            max.x = max.x.min(ui.available_width().max(16.0));
            let scale = (max.x / s.x).min(max.y / s.y).min(1.0);
            if scale.is_finite() && scale > 0.0 {
                s *= scale;
            }
            s
        }
    };
    size = size.max(vec2(8.0, 8.0));
    let radius = float("corner_radius").unwrap_or(0.0);
    match texture {
        Some(t) => {
            let mut img = egui::Image::new((t.id(), size)).corner_radius(radius);
            if let Some(c) = as_color(props.get("tint").cloned(), dark) {
                img = img.tint(c);
            }
            if let Some(c) = as_color(props.get("bg_fill").cloned(), dark) {
                img = img.bg_fill(c);
            }
            put(ui, sized, img)
        }
        None => {
            let (rect, response) = ui.allocate_exact_size(sized.unwrap_or(size), Sense::hover());
            if ui.is_rect_visible(rect) {
                let v = ui.visuals();
                let fill =
                    as_color(props.get("bg_fill").cloned(), dark).unwrap_or(v.faint_bg_color);
                ui.painter().rect_filled(rect, radius, fill);
                let r = rect.shrink(0.5);
                let corners = [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                    r.left_top(),
                ];
                ui.painter().extend(Shape::dashed_line(
                    &corners,
                    Stroke::new(1.0, v.weak_text_color()),
                    4.0,
                    3.0,
                ));
                ui.painter().with_clip_rect(rect).text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("Image\n{}", image_name(props)),
                    egui::FontId::proportional(11.0),
                    v.weak_text_color(),
                );
            }
            response
        }
    }
}

/// `format!("{:?}", self.x)` or `"text"`: what a `selected_text` expression most likely shows.
fn unquote_format(expr: &str) -> String {
    let e = expr.trim();
    if let Some(s) = e.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return s.to_string();
    }
    e.to_string()
}

/// A Spinner frozen at one moment. The real one repaints every frame for as long as it is visible,
/// which would keep the whole IDE repainting while the designer shows it. Same size and stroke.
fn static_spinner(ui: &mut Ui, size: f32, color: Option<Color32>) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if ui.is_rect_visible(rect) {
        let color = color.unwrap_or_else(|| ui.visuals().strong_text_color());
        let radius = (rect.height().min(rect.width()) * 0.5) - 2.0;
        let n_points = (radius.round() as u32).clamp(8, 128);
        let (start, end) = (
            0.25 * std::f64::consts::TAU,
            0.25 * std::f64::consts::TAU + 240f64.to_radians(),
        );
        let points: Vec<Pos2> = (0..n_points)
            .map(|i| {
                let angle = start + (end - start) * f64::from(i) / f64::from(n_points);
                let (sin, cos) = angle.sin_cos();
                rect.center() + radius * vec2(cos as f32, sin as f32)
            })
            .collect();
        ui.painter()
            .add(Shape::line(points, Stroke::new(3.0, color)));
    }
    response
}

// ---- Containers ----------------------------------------------------------------------------

fn container(
    ui: &mut Ui,
    n: &Node,
    c: ContainerId,
    props: &Props,
    head: Option<&str>,
    cx: &mut Cx,
) {
    let id = n.id;
    let salt = ("formust_node", id.0);
    let r: Response = match c {
        ContainerId::Root => {
            // The whole client area is the form, so a click on empty space selects it.
            let start = ui.available_rect_before_wrap();
            for k in n.children() {
                node(ui, k, cx);
            }
            cx.rec_rect(ui, id, start.union(ui.min_rect()));
            return;
        }
        ContainerId::Horizontal => ui.horizontal(|ui| kids(ui, n, cx)).response,
        ContainerId::HorizontalWrapped => ui.horizontal_wrapped(|ui| kids(ui, n, cx)).response,
        ContainerId::HorizontalCentered => ui.horizontal_centered(|ui| kids(ui, n, cx)).response,
        ContainerId::HorizontalTop => ui.horizontal_top(|ui| kids(ui, n, cx)).response,
        ContainerId::Vertical => ui.vertical(|ui| kids(ui, n, cx)).response,
        ContainerId::VerticalCentered => ui.vertical_centered(|ui| kids(ui, n, cx)).response,
        ContainerId::VerticalCenteredJustified => {
            ui.vertical_centered_justified(|ui| kids(ui, n, cx))
                .response
        }
        ContainerId::CenteredAndJustified => {
            ui.centered_and_justified(|ui| kids(ui, n, cx)).response
        }
        ContainerId::Scope => ui.scope(|ui| kids(ui, n, cx)).response,
        ContainerId::Group => {
            let out = ui.group(|ui| body_rect(ui, n, cx));
            cx.pass
                .insets
                .insert(id, out.response.rect.size() - out.inner.size());
            out.response
        }
        ContainerId::Indent => {
            let out = ui.indent(salt, |ui| body_rect(ui, n, cx));
            cx.pass
                .insets
                .insert(id, out.response.rect.size() - out.inner.size());
            out.response
        }
        ContainerId::Grid => {
            let mut g = Grid::new(salt);
            if let Some(x) = as_f64(container_prop(props, c, "num_columns")) {
                g = g.num_columns(x.max(1.0) as usize);
            }
            if as_bool(container_prop(props, c, "striped")) == Some(true) {
                g = g.striped(true);
            }
            if let Some(x) = as_f64(container_prop(props, c, "min_col_width")) {
                g = g.min_col_width(x as f32);
            }
            if let Some(x) = as_f64(container_prop(props, c, "max_col_width")) {
                g = g.max_col_width(x as f32);
            }
            if let Some(x) = as_f64(container_prop(props, c, "min_row_height")) {
                g = g.min_row_height(x as f32);
            }
            g.show(ui, |ui| kids(ui, n, cx)).response
        }
        ContainerId::ScrollArea => {
            let mut area = match as_enum(container_prop(props, c, "direction")) {
                Some("horizontal") => ScrollArea::horizontal(),
                Some("both") => ScrollArea::both(),
                Some("neither") => ScrollArea::neither(),
                _ => ScrollArea::vertical(),
            };
            // Inert: the wheel and drags scroll the canvas, never the form.
            area = area.id_salt(salt).scroll_source(ScrollSource::NONE);
            if let Some(x) = as_f64(container_prop(props, c, "max_height")) {
                area = area.max_height(x as f32);
            }
            if let Some(x) = as_f64(container_prop(props, c, "max_width")) {
                area = area.max_width(x as f32);
            }
            let out = area.show(ui, |ui| kids(ui, n, cx));
            let rect = out.inner_rect;
            cx.rec_rect(ui, id, rect);
            return;
        }
        ContainerId::CollapsingHeader => {
            let title = as_text(container_prop(props, c, "text")).unwrap_or_default();
            let open = as_bool(container_prop(props, c, "default_open")).unwrap_or(false);
            let out = CollapsingHeader::new(title)
                .id_salt(salt)
                .default_open(open)
                .show(ui, |ui| body_rect(ui, n, cx));
            let mut rect = out.header_response.rect;
            if let Some(body) = &out.body_response {
                rect |= body.rect;
            }
            // The header above the body and the indent left of it; the header may be wider than
            // the body, so only the leading offsets count.
            match out.body_returned {
                Some(body) => {
                    cx.pass.insets.insert(id, body.min - rect.min);
                }
                None => {
                    cx.pass.insets.remove(&id);
                }
            }
            cx.rec(ui, id, &out.header_response, rect);
            return;
        }
        ContainerId::PanelTop
        | ContainerId::PanelBottom
        | ContainerId::PanelLeft
        | ContainerId::PanelRight => {
            let mut p = match c {
                ContainerId::PanelTop => egui::Panel::top(salt),
                ContainerId::PanelBottom => egui::Panel::bottom(salt),
                ContainerId::PanelLeft => egui::Panel::left(salt),
                _ => egui::Panel::right(salt),
            };
            // Inert: a resize drag must not move the panel's edge.
            p = p.resizable(false);
            if let Some(b) = as_bool(container_prop(props, c, "show_separator_line")) {
                p = p.show_separator_line(b);
            }
            // The size methods in the order the export writes them, so egui clamps as at runtime.
            let float = |name: &str| as_f64(container_prop(props, c, name)).map(|v| v as f32);
            if let Some(x) = float("default_size") {
                p = p.default_size(x);
            }
            if let Some(x) = float("min_size") {
                p = p.min_size(x);
            }
            if let Some(x) = float("max_size") {
                p = p.max_size(x);
            }
            if let Some(x) = float("exact_size") {
                p = p.exact_size(x);
            }
            if let Some(PropValue::Expr(t)) = props.get("size_range")
                && let Some((lo, hi)) = formust_model::literal_range(t)
            {
                p = p.size_range(lo as f32..=hi as f32);
            }
            // The size the panel shows is its first one: egui would otherwise keep the size it
            // stored on an earlier pass (a panel's size persists), and a new default_size would
            // never show. The panel's id is its parent's scope id with the salt.
            ui.ctx()
                .data_mut(|d| d.remove::<egui::PanelState>(ui.scope_id().with(salt)));
            p.show(ui, |ui| kids(ui, n, cx)).response
        }
        ContainerId::CentralPanel => {
            let p = match as_enum(container_prop(props, c, "variant")) {
                Some("no_frame") => egui::CentralPanel::no_frame(),
                Some("default_margins") => egui::CentralPanel::default_margins(),
                _ => egui::CentralPanel::default(),
            };
            p.show(ui, |ui| kids(ui, n, cx)).response
        }
        ContainerId::WithLayout => {
            let layout =
                layout_of(&as_text(container_prop(props, c, "layout")).unwrap_or_default());
            ui.with_layout(layout, |ui| kids(ui, n, cx)).response
        }
        ContainerId::MenuButton => {
            // A real menu's popup never opens behind the shield. While the menu or something in
            // it is selected, its items are drawn open over the form (see `draw_popups`).
            let title = as_text(container_prop(props, c, "text")).unwrap_or_default();
            let r = if cx.in_popup {
                ui.add(Button::new(title).right_text("⏵"))
            } else {
                ui.menu_button(title, |_| {}).response
            };
            if cx.open_menus.contains(&id) {
                let at = if cx.in_popup {
                    r.rect.right_top()
                } else {
                    r.rect.left_bottom()
                };
                cx.popups.push(Popup::Menu { id, at });
            }
            r
        }
        ContainerId::MenuBar => egui::MenuBar::new().ui(ui, |ui| kids(ui, n, cx)).response,
        ContainerId::Modal => {
            // Drawn over the form, centred on a dimmed backdrop, after everything else.
            cx.popups.push(Popup::Modal(id));
            return;
        }
        ContainerId::Tabs | ContainerId::TabsVertical => {
            let r = tabs_preview(ui, n, c, props, cx);
            cx.rec(ui, id, &r, r.rect);
            return;
        }
        ContainerId::TabPage => ui.vertical(|ui| kids(ui, n, cx)).response,
        ContainerId::Table => {
            let r = table_preview(ui, n, props, cx);
            cx.rec(ui, id, &r, r.rect);
            return;
        }
        // Drawn by their Table.
        ContainerId::TableRow | ContainerId::TableCell => {
            ui.vertical(|ui| kids(ui, n, cx)).response
        }
        ContainerId::AddEnabledUi => {
            let enabled = as_bool(container_prop(props, c, "enabled")).unwrap_or(true);
            ui.add_enabled_ui(enabled, |ui| kids(ui, n, cx)).response
        }
        ContainerId::Frame => {
            let style = ui.style().clone();
            let mut f = match as_enum(container_prop(props, c, "variant")) {
                Some("group") => Frame::group(&style),
                Some("side_top_panel") => Frame::side_top_panel(&style),
                Some("central_panel") => Frame::central_panel(&style),
                Some("window") => Frame::window(&style),
                Some("menu") => Frame::menu(&style),
                Some("popup") => Frame::popup(&style),
                Some("canvas") => Frame::canvas(&style),
                Some("dark_canvas") => Frame::dark_canvas(&style),
                _ => Frame::new(),
            };
            if let Some(col) = as_color(container_prop(props, c, "fill"), cx.dark) {
                f = f.fill(col);
            }
            if let Some(s) = as_stroke(container_prop(props, c, "stroke"), cx.dark) {
                f = f.stroke(s);
            }
            let float = |name: &str| as_f64(container_prop(props, c, name)).map(|v| v as f32);
            if let Some(m) = float("inner_margin") {
                f = f.inner_margin(m);
            }
            if let Some(m) = float("outer_margin") {
                f = f.outer_margin(m);
            }
            if let Some(r) = float("corner_radius") {
                f = f.corner_radius(r);
            }
            if let Some(o) = float("multiply_with_opacity") {
                f = f.multiply_with_opacity(o);
            }
            if as_bool(container_prop(props, c, "invisible")) == Some(true) {
                f = f.invisible();
            }
            let out = f.show(ui, |ui| body_rect(ui, n, cx));
            cx.pass
                .insets
                .insert(id, out.response.rect.size() - out.inner.size());
            out.response
        }
        ContainerId::Window | ContainerId::DialogCall => {
            // A real Window is an Area on a registered layer of its own, which would escape the
            // preview layer and the shield. It is drawn in place, framed like a window. A dialog
            // helper call is drawn the same way, titled by its first string literal.
            let title = match c {
                ContainerId::DialogCall => {
                    let call = as_text(container_prop(props, c, "call")).unwrap_or_default();
                    first_string_literal(&call).unwrap_or_else(|| "Dialog".to_string())
                }
                _ => as_text(container_prop(props, c, "title")).unwrap_or_default(),
            };
            let style = ui.style().clone();
            let frame = Frame::window(&style);
            // A Window is as wide as egui makes it (its title bar spans that width), and as high
            // when a scroll area fills it; the sizes are outer ones, frame included. A dialog
            // helper's size is its own code.
            let size = match c {
                ContainerId::Window => window_size(props),
                _ => None,
            };
            let margin = frame.total_margin().sum();
            let draw = |ui: &mut Ui| {
                frame
                    .show(ui, |ui| {
                        if let Some((w, h)) = size {
                            let w = (w - margin.x).max(0.0);
                            ui.set_max_width(w);
                            ui.set_min_width(w);
                            if let Some(h) = h {
                                ui.set_min_height((h - margin.y).max(0.0));
                            }
                        }
                        ui.strong(title);
                        ui.separator();
                        kids(ui, n, cx);
                    })
                    .response
            };
            // `.anchor(Align2::X, ..)`: placed at that side of the form (the offset isn't shown).
            match window_anchor_layout(props) {
                Some(layout) => ui.with_layout(layout, draw).inner,
                None => draw(ui),
            }
        }
        ContainerId::Flow => {
            // `ImportOptions::extended` only: locked code around the children (`if c {`, `for ..`).
            let v = ui.visuals().clone();
            Frame::new()
                .stroke(Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color))
                .corner_radius(3.0)
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    let head = head.map_or_else(String::new, |h| h.trim().to_string());
                    ui.add(
                        Label::new(
                            RichText::new(head)
                                .monospace()
                                .small()
                                .color(v.weak_text_color()),
                        )
                        .wrap_mode(TextWrapMode::Extend),
                    );
                    ui.vertical(|ui| kids(ui, n, cx));
                })
                .response
        }
        ContainerId::FlowGroup => ui.vertical(|ui| kids(ui, n, cx)).response,
        ContainerId::Columns => {
            // `ui.columns(n, ..)` as at runtime; each column records its own rect, the full
            // column width, so a drop into an empty one finds it.
            let parts = n.children();
            ui.scope(|ui| {
                ui.columns(parts.len().max(1), |cols| {
                    for (col, part) in cols.iter_mut().zip(parts) {
                        let top = col.max_rect().min;
                        kids(col, part, cx);
                        let used = col.min_rect();
                        let rect = Rect::from_min_max(
                            top,
                            egui::pos2(col.max_rect().right(), used.bottom()),
                        );
                        cx.rec_rect(col, part.id, rect);
                    }
                });
            })
            .response
        }
        ContainerId::Sides => {
            let mut sides = egui::containers::Sides::new();
            let float = |name: &str| as_f64(container_prop(props, c, name)).map(|v| v as f32);
            if let Some(h) = float("height") {
                sides = sides.height(h);
            }
            if let Some(s) = float("spacing") {
                sides = sides.spacing(s);
            }
            let on = |name: &str| as_bool(container_prop(props, c, name)) == Some(true);
            if on("shrink_left") {
                sides = sides.shrink_left();
            }
            if on("shrink_right") {
                sides = sides.shrink_right();
            }
            if on("extend") {
                sides = sides.extend();
            }
            if let Some(mode) = match as_enum(container_prop(props, c, "wrap_mode")) {
                Some("Wrap") => Some(TextWrapMode::Wrap),
                Some("Truncate") => Some(TextWrapMode::Truncate),
                Some("Extend") => Some(TextWrapMode::Extend),
                _ => None,
            } {
                sides = sides.wrap_mode(mode);
            }
            if on("truncate") {
                sides = sides.truncate();
            }
            if on("wrap") {
                sides = sides.wrap();
            }
            // Both closures exist at once; egui calls them one after the other.
            let cx = std::cell::RefCell::new(&mut *cx);
            let side = |ui: &mut Ui, k: usize| {
                if let Some(part) = n.children().get(k) {
                    let mut cx = cx.borrow_mut();
                    kids(ui, part, &mut cx);
                    let rect = ui.min_rect();
                    cx.rec_rect(ui, part.id, rect);
                }
            };
            let r = ui
                .scope(|ui| {
                    sides.show(ui, |ui| side(ui, 0), |ui| side(ui, 1));
                })
                .response;
            let cx = cx.into_inner();
            cx.rec(ui, id, &r, r.rect);
            return;
        }
        // Drawn by their Columns or Sides.
        ContainerId::Column | ContainerId::Side => ui.vertical(|ui| kids(ui, n, cx)).response,
    };
    cx.rec(ui, id, &r, r.rect);
}

/// Tabs: the bar of buttons (the shown page's selected) and the shown page, laid out as the
/// exported code lays them out. The shown page is the one around the selection, or the last one
/// shown, or the first.
fn tabs_preview(ui: &mut Ui, n: &Node, c: ContainerId, props: &Props, cx: &mut Cx) -> Response {
    let pages = n.children();
    let labels: Vec<String> = match props.get("tabs") {
        Some(PropValue::Items(items)) => items
            .iter()
            .map(|t| as_text(Some(t.label.clone())).unwrap_or_default())
            .collect(),
        _ => Vec::new(),
    };
    let shown = cx
        .pages
        .get(&n.id)
        .and_then(|p| pages.iter().position(|page| page.id == *p))
        .unwrap_or(0);
    let style = as_enum(container_prop(props, c, "style")).unwrap_or("selectable");
    let bar = |ui: &mut Ui, cx: &mut Cx| {
        for (k, page) in pages.iter().enumerate() {
            let label = labels.get(k).cloned().unwrap_or_default();
            let on = k == shown;
            let r = match style {
                "button" => ui.add(Button::new(label).selected(on)),
                "frameless" => ui.add(Button::new(label).frame(false).selected(on)),
                _ => ui.selectable_label(on, label),
            };
            if r.clicked() || r.hovered() || r.has_focus() {
                cx.pass.reacted.push(n.id);
            }
            let visible = r.rect.intersect(ui.clip_rect());
            if visible.is_positive() {
                cx.pass.tab_buttons.push((visible, page.id));
            }
        }
    };
    let page = |ui: &mut Ui, cx: &mut Cx| {
        if let Some(p) = pages.get(shown) {
            let r = ui.scope(|ui| kids(ui, p, cx)).response;
            cx.rec_rect(ui, p.id, r.rect);
        }
    };
    if c == ContainerId::TabsVertical {
        ui.horizontal(|ui| {
            ui.vertical(|ui| bar(ui, cx));
            ui.separator();
            ui.vertical(|ui| page(ui, cx));
        })
        .response
    } else {
        // Two statements in the parent's own layout: the bar's row, then the page's content.
        ui.scope(|ui| {
            ui.horizontal(|ui| bar(ui, cx));
            page(ui, cx);
        })
        .response
    }
}

/// A table as a grid of its cells: no `egui_extras::TableBuilder` here (its scroll area and
/// column resizing would take input), but the same rows and columns, striped like it, with the
/// row heights as minimum heights. A row's rect is its cells' together.
fn table_preview(ui: &mut Ui, n: &Node, props: &Props, cx: &mut Cx) -> Response {
    let rows = n.children();
    let columns = rows.first().map_or(1, |r| r.children().len().max(1));
    let float =
        |name: &str| as_f64(container_prop(props, ContainerId::Table, name)).map(|v| v as f32);
    let row_height = float("row_height").unwrap_or(18.0);
    let striped = as_bool(container_prop(props, ContainerId::Table, "striped")) == Some(true);
    let header = formust_model::table_has_header(props);
    let salt = ("formust_node", n.id.0);
    egui::Frame::new()
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .show(ui, |ui| {
            Grid::new(salt)
                .num_columns(columns)
                .striped(striped)
                .min_col_width(48.0)
                .min_row_height(row_height)
                .show(ui, |ui| {
                    for (k, row) in rows.iter().enumerate() {
                        let mut rect: Option<Rect> = None;
                        for cell in row.children() {
                            let r = ui
                                .vertical(|ui| {
                                    if header && k == 0 {
                                        ui.set_min_height(float("header").unwrap_or(row_height));
                                    }
                                    kids(ui, cell, cx);
                                })
                                .response;
                            cx.rec_rect(ui, cell.id, r.rect);
                            rect = Some(rect.map_or(r.rect, |a| a.union(r.rect)));
                        }
                        if let Some(rect) = rect {
                            cx.rec_rect(ui, row.id, rect);
                        }
                        ui.end_row();
                    }
                });
        })
        .response
}

impl Cx<'_> {
    /// A rect with no response of its own (the root, a scroll area's inner rect).
    fn rec_rect(&mut self, ui: &Ui, id: NodeId, rect: Rect) {
        let visible = rect.intersect(ui.clip_rect());
        if visible.is_positive() {
            self.pass.rects.insert(id, visible);
        }
    }
}

/// egui 0.37's own outer size of a Window, and its least one (`Window::new`).
const WINDOW_DEFAULT: Vec2 = vec2(340.0, 420.0);
const WINDOW_MIN: Vec2 = vec2(96.0, 32.0);

/// A Window's outer width at runtime, and its height when a scroll area fills it (`vscroll` or
/// `hscroll`; otherwise egui fits the height to the content). `None` for an `auto_sized` window,
/// which fits its content. The chain's order decides: `default_width`, then `default_size`, then
/// `fixed_size`; the least and largest sizes clamp it.
fn window_size(props: &Props) -> Option<(f32, Option<f32>)> {
    if props.get("auto_sized") == Some(&PropValue::Bool(true)) {
        return None;
    }
    let float = |name: &str| as_f64(props.get(name).cloned()).map(|v| v as f32);
    let pair = |name: &str| as_vec2(props.get(name).cloned());
    let mut size = WINDOW_DEFAULT;
    let (mut min, mut max) = (WINDOW_MIN, Vec2::INFINITY);
    if let Some(w) = float("default_width") {
        size.x = w;
    }
    if let Some(h) = float("default_height") {
        size.y = h;
    }
    if let Some(v) = float("min_width") {
        min.x = v;
    }
    if let Some(v) = float("min_height") {
        min.y = v;
    }
    if let Some(v) = float("max_width") {
        max.x = v;
    }
    if let Some(v) = float("max_height") {
        max.y = v;
    }
    if let Some(v) = pair("default_size") {
        size = v;
    }
    if let Some(v) = pair("min_size") {
        min = v;
    }
    if let Some(v) = pair("max_size") {
        max = v;
    }
    if let Some(v) = pair("fixed_size") {
        (size, min, max) = (v, v, v);
    }
    let size = size.max(min).min(max);
    let scrolls = ["vscroll", "hscroll"]
        .iter()
        .any(|n| props.get(*n) == Some(&PropValue::Bool(true)));
    Some((size.x, scrolls.then_some(size.y)))
}

/// The layout that puts a Window where its `.anchor(egui::Align2::X, ..)` says.
fn window_anchor_layout(props: &Props) -> Option<Layout> {
    let Some(PropValue::Expr(text)) = props.get("anchor") else {
        return None;
    };
    let (align, _, _) = parse_window_anchor(text)?;
    let (h, v) = align.split_once('_')?;
    let h = match h {
        "LEFT" => Align::Min,
        "CENTER" => Align::Center,
        _ => Align::Max,
    };
    Some(if v == "BOTTOM" {
        Layout::bottom_up(h)
    } else {
        Layout::top_down(h)
    })
}

/// The few `Layout` expressions a `with_layout` call usually has; anything else lays out top-down.
fn layout_of(expr: &str) -> Layout {
    let align = if expr.contains("Center") {
        Align::Center
    } else if expr.contains("Max") {
        Align::Max
    } else {
        Align::Min
    };
    if expr.contains("right_to_left") {
        Layout::right_to_left(align)
    } else if expr.contains("left_to_right") {
        Layout::left_to_right(align)
    } else if expr.contains("bottom_up") {
        Layout::bottom_up(align)
    } else if expr.contains("top_down_justified") {
        Layout::top_down_justified(align)
    } else if expr.contains("centered_and_justified") {
        Layout::centered_and_justified(egui::Direction::TopDown)
    } else {
        Layout::top_down(align)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use formust_model::{ContainerShell, Props};

    fn widget_node(id: u32, def: WidgetId) -> Node {
        Node {
            id: NodeId(id),
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: None,
            kind: NodeKind::Widget {
                def,
                props: Props::new(),
                events: vec![],
                extra_chain: vec![],
                response_chain: vec![],
                recv: "ui".into(),
                let_prefix: None,
            },
        }
    }

    fn container_node(id: u32, def: ContainerId, children: Vec<Node>) -> Node {
        Node {
            id: NodeId(id),
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: None,
            kind: NodeKind::Container {
                def,
                props: Props::new(),
                children,
                closing: String::new(),
                extra_chain: vec![],
                ident: "ui".into(),
                recv: "ui".into(),
                closure: "|ui| ".into(),
                braced: true,
                shell: Some(ContainerShell {
                    head: None,
                    tail: String::new(),
                }),
            },
        }
    }

    fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
    }

    /// root(0) > horizontal(1) > [button(2), group(3) > label(4)]
    fn tree() -> (Node, HashMap<NodeId, Rect>) {
        let group = container_node(3, ContainerId::Group, vec![widget_node(4, WidgetId::Label)]);
        let row = container_node(
            1,
            ContainerId::Horizontal,
            vec![widget_node(2, WidgetId::Button), group],
        );
        let root = container_node(0, ContainerId::Root, vec![row]);
        let rects = HashMap::from([
            (NodeId(0), r(0.0, 0.0, 400.0, 300.0)),
            (NodeId(1), r(10.0, 10.0, 300.0, 60.0)),
            (NodeId(2), r(12.0, 12.0, 80.0, 40.0)),
            (NodeId(3), r(90.0, 12.0, 200.0, 58.0)),
            (NodeId(4), r(96.0, 18.0, 150.0, 40.0)),
        ]);
        (root, rects)
    }

    #[test]
    fn the_deepest_node_under_the_pointer_is_hit() {
        let (root, rects) = tree();
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(100.0, 20.0)),
            Some(NodeId(4))
        );
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(190.0, 50.0)),
            Some(NodeId(3))
        );
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(20.0, 20.0)),
            Some(NodeId(2))
        );
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(250.0, 20.0)),
            Some(NodeId(1))
        );
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(350.0, 200.0)),
            Some(NodeId(0))
        );
        assert_eq!(hit_test(&root, &rects, egui::pos2(500.0, 500.0)), None);
    }

    /// A clipped-away node has no rect, so the pointer reaches what is visible there.
    #[test]
    fn a_node_without_a_rect_is_never_hit() {
        let (root, mut rects) = tree();
        rects.remove(&NodeId(4));
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(100.0, 20.0)),
            Some(NodeId(3))
        );
        rects.remove(&NodeId(3));
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(100.0, 20.0)),
            Some(NodeId(1))
        );
    }

    /// Siblings that overlap (a negative item spacing): the later one paints on top and wins.
    #[test]
    fn of_overlapping_siblings_the_later_one_wins() {
        let (root, mut rects) = tree();
        rects.insert(NodeId(3), r(60.0, 12.0, 200.0, 58.0));
        assert_eq!(
            hit_test(&root, &rects, egui::pos2(70.0, 50.0)),
            Some(NodeId(3))
        );
    }

    #[test]
    fn a_dialog_call_is_titled_by_its_first_string_literal() {
        let call = r#"crate::style::dialog(ctx, "Say \"hi\"", None, "x", "#;
        assert_eq!(first_string_literal(call).as_deref(), Some(r#"Say "hi""#));
        assert_eq!(first_string_literal("dialog(ctx, &self.title, "), None);
        assert_eq!(first_string_literal("dialog(ctx, \"open"), None);
    }

    #[test]
    fn ranges_are_read_from_literal_expressions() {
        let e = |s: &str| Some(PropValue::Expr(s.into()));
        assert_eq!(parse_range(e("0.0..=100.0")), 0.0..=100.0);
        assert_eq!(parse_range(e("1..=10")), 1.0..=10.0);
        assert_eq!(parse_range(e("-5.0..5.0")), -5.0..=5.0);
        assert_eq!(parse_range(e("0.0_f32..=1.0f32")), 0.0..=1.0);
        assert_eq!(parse_range(e("self.min..=self.max")), 0.0..=1.0);
        assert_eq!(parse_range(None), 0.0..=1.0);
    }

    #[test]
    fn custom_text_loses_only_the_source_indentation() {
        let src = "if self.busy {\n            ui.spinner();\n        }";
        assert_eq!(dedent(src), "if self.busy {\n    ui.spinner();\n}");
        assert_eq!(dedent("let x = 1;"), "let x = 1;");
    }
}
