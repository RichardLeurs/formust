//! Layout on top of the tree: Fill, anchors, wrapping nodes in a new container, and resizing
//! through a widget's own size properties. Pure model code; the designer draws and drives it.
//!
//! - **Fill** is not a property of its own. It is a value of a widget's width property (or its
//!   `add_sized` size), read back from what the source has: `f32::INFINITY`, `f32::MAX` and
//!   `<ui>.available_width()` all mean "as wide as there is room". [`fill_way`] says how a widget
//!   is made to fill in egui 0.37.
//! - **Anchors** are a `ui.with_layout(Layout::<dir>(Align::<a>), |ui| { <one node> })` wrapper. The
//!   tree keeps the wrapper as a [`ContainerId::WithLayout`] container, so import and emit are as
//!   before and an unedited wrapper is still written verbatim; the designer shows the wrapped node
//!   with an [`Anchor`] value instead (see [`Form::anchor_of`]). Only the exact layouts of
//!   [`Anchor::layout_text`] are read as anchors; any other `with_layout` stays a container.
//! - **Wrap and unwrap** put siblings into a new container at their place, or move a container's
//!   children out into its parent. Both are one edit, atomic on refusal.
//! - **Resize** maps a width and height to the properties a widget has ([`resize_targets`]).

use crate::form::{EditError, Form, Node, NodeId, NodeKind, PropValue, Props};
use crate::registry::{ContainerId, WidgetId, container_def, widget_def};

// ---- Fill ------------------------------------------------------------------------------------

/// How a widget is made to fill the available width in egui 0.37.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillWay {
    /// `.<prop>(f32::INFINITY)`: the widget clamps the width to what is available (TextEdit).
    Infinity(&'static str),
    /// `.<prop>(ui.available_width())`: an infinite value would size it infinitely (ProgressBar
    /// allocates exactly its desired width, ComboBox uses it as a minimum). The model holds
    /// `f32::INFINITY` there; the emitter writes `ui.available_width()` with the `Ui` in scope.
    AvailableWidth(&'static str),
    /// `ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], widget)`: the widget has
    /// no width method, but stretches in the justified layout `add_sized` gives it (Button,
    /// DragValue). The model holds `f32::INFINITY` as the `add_sized` value.
    AddSized,
}

/// How `def` fills, or `None` when Fill doesn't apply. A Slider's rail is `Spacing::slider_width`
/// whatever room it gets, so it has no Fill.
pub fn fill_way(def: WidgetId) -> Option<FillWay> {
    match def {
        WidgetId::TextEditSingleline | WidgetId::TextEditMultiline => {
            Some(FillWay::Infinity("desired_width"))
        }
        WidgetId::ProgressBar => Some(FillWay::AvailableWidth("desired_width")),
        WidgetId::ComboBox => Some(FillWay::AvailableWidth("width")),
        WidgetId::Button | WidgetId::DragValue => Some(FillWay::AddSized),
        WidgetId::Label
        | WidgetId::Heading
        | WidgetId::Small
        | WidgetId::Strong
        | WidgetId::Weak
        | WidgetId::Monospace
        | WidgetId::Link
        | WidgetId::Hyperlink
        | WidgetId::Checkbox
        | WidgetId::ToggleValue
        | WidgetId::RadioValue
        | WidgetId::SelectableValue
        | WidgetId::Slider
        | WidgetId::Spinner
        | WidgetId::Separator
        | WidgetId::AddSpace
        | WidgetId::EndRow
        | WidgetId::Image
        | WidgetId::ColorEdit
        | WidgetId::ColorEditRgb
        | WidgetId::RangeSlider => None,
    }
}

/// The property of `def` where an infinite value is written as `<ui>.available_width()`.
pub fn available_width_prop(def: WidgetId) -> Option<&'static str> {
    match fill_way(def)? {
        FillWay::AvailableWidth(prop) => Some(prop),
        FillWay::Infinity(_) | FillWay::AddSized => None,
    }
}

/// `f32::INFINITY`, `f32::MAX` (also with `std::`/`core::` or as `f64`), or `<ui>.available_width()`:
/// source text that means "as wide as there is room".
pub fn is_fill_text(text: &str) -> bool {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let t = t
        .strip_prefix("std::")
        .or_else(|| t.strip_prefix("core::"))
        .unwrap_or(&t);
    if matches!(
        t,
        "f32::INFINITY" | "f32::MAX" | "f64::INFINITY" | "f64::MAX"
    ) {
        return true;
    }
    t.strip_suffix(".available_width()").is_some_and(is_ui_text)
}

/// A `Ui` as source text: a name (`ui`), or one column of a `ui.columns(..)` (`cols[0]`).
pub fn is_ui_text(text: &str) -> bool {
    let plain = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || c == '_');
    match text.strip_suffix(']').and_then(|t| t.split_once('[')) {
        Some((name, index)) => {
            plain(name) && !index.is_empty() && index.chars().all(|c| c.is_ascii_digit())
        }
        None => plain(text),
    }
}

/// A `Ui` that is a plain name and can be a closure's parameter (`ui`, not `cols[0]`).
pub fn is_plain_ui(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// A width value meaning Fill.
pub fn is_fill_value(value: &PropValue) -> bool {
    match value {
        PropValue::Float(v) => v.is_infinite() && *v > 0.0,
        PropValue::Expr(t) => is_fill_text(t),
        _ => false,
    }
}

/// The two parts of a size expression: `[a, b]`, `vec2(a, b)`, `egui::vec2(a, b)` or
/// `Vec2::new(a, b)`. `None` for anything else.
pub fn split_pair(text: &str) -> Option<(String, String)> {
    let t = text.trim();
    let inner = if let Some(rest) = t.strip_prefix('[') {
        rest.strip_suffix(']')?
    } else {
        let open = t.find('(')?;
        let head = t[..open].trim();
        let last = head.rsplit("::").next().unwrap_or(head);
        let is_vec2 = last == "vec2" || (last == "new" && head.ends_with("Vec2::new"));
        if !is_vec2 {
            return None;
        }
        t[open + 1..].strip_suffix(')')?
    };
    // Split on the one comma at depth 0.
    let mut depth = 0i32;
    let mut cut = None;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                if cut.is_some() {
                    return None;
                }
                cut = Some(i);
            }
            _ => {}
        }
    }
    let cut = cut?;
    let (a, b) = (inner[..cut].trim(), inner[cut + 1..].trim());
    (!a.is_empty() && !b.is_empty()).then(|| (a.to_string(), b.to_string()))
}

/// An `add_sized` value whose width is Fill: the designer's marker (`f32::INFINITY`) or a pair
/// whose first part is a fill text.
pub fn is_fill_size(value: &PropValue) -> bool {
    match value {
        PropValue::Float(v) => v.is_infinite() && *v > 0.0,
        PropValue::Expr(t) => split_pair(t).is_some_and(|(w, _)| is_fill_text(&w)),
        _ => false,
    }
}

/// Does this widget fill the available width?
pub fn is_fill(def: WidgetId, props: &Props) -> bool {
    let sized = || props.get("add_sized").is_some_and(is_fill_size);
    match fill_way(def) {
        Some(FillWay::Infinity(prop) | FillWay::AvailableWidth(prop)) => {
            props.get(prop).is_some_and(is_fill_value) || sized()
        }
        Some(FillWay::AddSized) => sized(),
        None => false,
    }
}

impl Form {
    /// Turns Fill on or off for a widget. Off removes the width (back to egui's default).
    pub fn set_fill(&mut self, id: NodeId, on: bool) -> Result<(), EditError> {
        let node = self.find(id).ok_or(EditError::NoSuchNode(id))?;
        let (def, props) = match &node.kind {
            NodeKind::Widget { def, props, .. } => (*def, props.clone()),
            NodeKind::Container { .. } => return Err(EditError::NoSuchProp("fill".into())),
            NodeKind::Custom { .. } => return Err(EditError::IsCustom(id)),
        };
        let way = fill_way(def).ok_or_else(|| EditError::NoSuchProp("fill".into()))?;
        let backup = self.clone();
        let result = (|| {
            let sized_fill = props.get("add_sized").is_some_and(is_fill_size);
            match way {
                FillWay::Infinity(prop) | FillWay::AvailableWidth(prop) => {
                    if on {
                        // A fixed `add_sized` would decide the width instead.
                        if props.contains_key("add_sized") {
                            self.clear_prop(id, "add_sized")?;
                        }
                        self.set_prop(id, prop, PropValue::Float(f64::INFINITY))
                    } else {
                        if sized_fill {
                            self.clear_prop(id, "add_sized")?;
                        }
                        if props.get(prop).is_some_and(is_fill_value) {
                            self.clear_prop(id, prop)?;
                        }
                        Ok(())
                    }
                }
                FillWay::AddSized => {
                    if on {
                        self.set_prop(id, "add_sized", PropValue::Float(f64::INFINITY))
                    } else if sized_fill {
                        self.clear_prop(id, "add_sized")
                    } else {
                        Ok(())
                    }
                }
            }
        })();
        if result.is_err() {
            *self = backup;
        }
        result
    }
}

// ---- Anchors ---------------------------------------------------------------------------------

/// Where a node sits in the room its parent gives it, written as a `with_layout` wrapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Anchor {
    Left,
    Right,
    Top,
    Bottom,
    Center,
    /// As wide as the parent: `top_down_justified`.
    Fill,
}

pub const ALL_ANCHORS: &[Anchor] = &[
    Anchor::Left,
    Anchor::Right,
    Anchor::Top,
    Anchor::Bottom,
    Anchor::Center,
    Anchor::Fill,
];

impl Anchor {
    pub fn label(self) -> &'static str {
        match self {
            Anchor::Left => "Left",
            Anchor::Right => "Right",
            Anchor::Top => "Top",
            Anchor::Bottom => "Bottom",
            Anchor::Center => "Center",
            Anchor::Fill => "Fill",
        }
    }

    /// The `Layout` constructor and its `Align`.
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Anchor::Left => ("left_to_right", "Min"),
            Anchor::Right => ("right_to_left", "Min"),
            Anchor::Top => ("top_down", "Min"),
            Anchor::Bottom => ("bottom_up", "Min"),
            Anchor::Center => ("top_down", "Center"),
            Anchor::Fill => ("top_down_justified", "Min"),
        }
    }

    /// The layout expression, with the form's egui path: `egui::Layout::right_to_left(egui::Align::Min)`.
    pub fn layout_text(self, egui_path: &str) -> String {
        let (ctor, align) = self.parts();
        format!("{egui_path}Layout::{ctor}({egui_path}Align::{align})")
    }

    /// The anchor a layout expression writes, if it is exactly one of [`Anchor::layout_text`]
    /// (with any path prefix, any spacing).
    pub fn from_layout(text: &str) -> Option<Anchor> {
        let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let (prefix, rest) = split_path(&t, "Layout::")?;
        let open = rest.find('(')?;
        let ctor = &rest[..open];
        let arg = rest[open + 1..].strip_suffix(')')?;
        let (arg_prefix, align) = split_path(arg, "Align::")?;
        if !is_path_prefix(prefix) || !is_path_prefix(arg_prefix) {
            return None;
        }
        ALL_ANCHORS
            .iter()
            .copied()
            .find(|a| a.parts() == (ctor, align))
    }
}

/// `egui::Layout::x` split at `Layout::`: (`egui::`, `x`).
fn split_path<'a>(text: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let at = text.find(name)?;
    Some((&text[..at], &text[at + name.len()..]))
}

/// Empty, or `a::b::`.
fn is_path_prefix(prefix: &str) -> bool {
    prefix.is_empty()
        || prefix.strip_suffix("::").is_some_and(|p| {
            p.split("::")
                .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
        })
}

/// The anchor of a `with_layout` wrapper around exactly one node, with nothing else in it (no
/// comments, no unknown chain), so it can be shown as that node's anchor and removed without loss.
pub fn wrapper_anchor(node: &Node) -> Option<Anchor> {
    let NodeKind::Container {
        def: ContainerId::WithLayout,
        props,
        children,
        closing,
        extra_chain,
        braced: true,
        ..
    } = &node.kind
    else {
        return None;
    };
    // Body statements (spacing, a min width) would be lost with the wrapper: then it is no anchor.
    if children.len() != 1
        || !extra_chain.is_empty()
        || !closing.trim().is_empty()
        || props.keys().any(|k| *k != "layout")
    {
        return None;
    }
    match props.get("layout")? {
        PropValue::Expr(t) | PropValue::Text(t) => Anchor::from_layout(t),
        _ => None,
    }
}

impl Form {
    /// The anchor of `id`: its parent is an anchor wrapper.
    pub fn anchor_of(&self, id: NodeId) -> Option<Anchor> {
        let (parent, _) = self.parent_of(id)?;
        wrapper_anchor(self.find(parent)?)
    }

    /// The node that stands for `id` among its siblings: its anchor wrapper when it has one. Moves,
    /// deletes and copies act on this.
    pub fn anchor_host(&self, id: NodeId) -> NodeId {
        match self.parent_of(id) {
            Some((parent, _)) if self.find(parent).and_then(wrapper_anchor).is_some() => parent,
            _ => id,
        }
    }

    /// The node an anchor wrapper holds, or `id` itself when it isn't one. The designer selects this.
    pub fn anchor_child(&self, id: NodeId) -> NodeId {
        match self.find(id) {
            Some(node) if wrapper_anchor(node).is_some() => node.children()[0].id,
            _ => id,
        }
    }

    /// Is `id` an anchor wrapper?
    pub fn is_anchor_wrapper(&self, id: NodeId) -> bool {
        self.find(id).and_then(wrapper_anchor).is_some()
    }

    /// Can `id` be wrapped (in a row or an anchor) where it is?
    pub fn can_wrap(&self, id: NodeId) -> bool {
        let Some(node) = self.find(id) else {
            return false;
        };
        let Some((parent, _)) = self.parent_of(id) else {
            return false;
        };
        let wrappable = match &node.kind {
            NodeKind::Widget { .. } | NodeKind::Custom { .. } => true,
            NodeKind::Container { def, .. } => {
                !matches!(
                    def,
                    ContainerId::Root
                        | ContainerId::Window
                        | ContainerId::DialogCall
                        | ContainerId::Flow
                        | ContainerId::FlowGroup
                ) && !container_def(*def).is_panel
            }
        };
        wrappable
            && self.takes_widgets(parent)
            && !self.is_locked_branch(id)
            && !(parent == self.root.id && node.bare_tail && self.returns_value())
    }

    /// Sets or removes the anchor of `id`. With none, an anchor wrapper goes and the node takes
    /// its place; a new anchor wraps it; another anchor rewrites the wrapper's layout only.
    pub fn set_anchor(&mut self, id: NodeId, anchor: Option<Anchor>) -> Result<(), EditError> {
        let id = self.anchor_child(id);
        let wrapper = self
            .parent_of(id)
            .map(|(p, _)| p)
            .filter(|p| self.is_anchor_wrapper(*p));
        match (wrapper, anchor) {
            (None, None) => Ok(()),
            (Some(w), None) => self.unwrap(w).map(|_| ()),
            (Some(w), Some(a)) => {
                if self.find(w).and_then(wrapper_anchor) == Some(a) {
                    return Ok(());
                }
                let text = a.layout_text(&self.egui_path);
                self.set_prop(w, "layout", PropValue::Expr(text))
            }
            (None, Some(a)) => {
                if !self.can_wrap(id) {
                    return Err(EditError::Locked(id));
                }
                let backup = self.clone();
                let text = a.layout_text(&self.egui_path);
                let result = self
                    .wrap(&[id], ContainerId::WithLayout)
                    .and_then(|w| self.set_prop(w, "layout", PropValue::Expr(text)));
                if result.is_err() {
                    *self = backup;
                }
                result
            }
        }
    }

    /// Puts the siblings `ids` (a contiguous run, in any order) into a new `def` container at their
    /// place, and returns its id. The new container's closure uses its parent's `Ui` name, so the
    /// moved nodes' text still fits.
    pub fn wrap(&mut self, ids: &[NodeId], def: ContainerId) -> Result<NodeId, EditError> {
        let first = *ids.first().ok_or(EditError::NoSuchNode(NodeId(u32::MAX)))?;
        let (parent, _) = self.parent_of(first).ok_or(EditError::IsRoot)?;
        let mut at: Vec<(usize, NodeId)> = Vec::new();
        for &id in ids {
            match self.parent_of(id) {
                Some((p, i)) if p == parent => at.push((i, id)),
                Some(_) => return Err(EditError::NotAContainer(parent)),
                None if self.find(id).is_some() => return Err(EditError::IsRoot),
                None => return Err(EditError::NoSuchNode(id)),
            }
            if !self.can_wrap(id) {
                return Err(EditError::Locked(id));
            }
        }
        at.sort();
        at.dedup();
        if at.windows(2).any(|w| w[1].0 != w[0].0 + 1) {
            // Not a contiguous run: the nodes between would change places.
            return Err(EditError::Locked(at[0].1));
        }
        let ident = match self.find(parent).map(|n| &n.kind) {
            // A column's `cols[0]` can't name a closure's parameter: the new one is `ui`.
            Some(NodeKind::Container { ident, .. }) if is_plain_ui(ident) => ident.clone(),
            Some(NodeKind::Container { .. }) => "ui".to_string(),
            _ => return Err(EditError::NotAContainer(parent)),
        };
        let backup = self.clone();
        let result = (|| {
            let mut wrapper = self.new_container(def);
            if let NodeKind::Container {
                ident: own,
                closure,
                ..
            } = &mut wrapper.kind
            {
                *own = ident.clone();
                *closure = format!("|{ident}| ");
            }
            let wrapper_id = wrapper.id;
            self.insert(parent, at[0].0, wrapper)?;
            for (k, (_, id)) in at.iter().enumerate() {
                self.move_node(*id, wrapper_id, k)?;
            }
            Ok(wrapper_id)
        })();
        if result.is_err() {
            *self = backup;
        }
        result
    }

    /// Can `id` be unwrapped: a container (not the form, a window, a panel or control flow) whose
    /// own text holds no comment that would be lost.
    pub fn can_unwrap(&self, id: NodeId) -> bool {
        let Some(node) = self.find(id) else {
            return false;
        };
        let NodeKind::Container {
            def,
            closing,
            children,
            ..
        } = &node.kind
        else {
            return false;
        };
        let Some((parent, _)) = self.parent_of(id) else {
            return false;
        };
        let fits = children.iter().all(|c| self.fits(parent, c));
        !matches!(
            def,
            ContainerId::Root
                | ContainerId::Window
                | ContainerId::DialogCall
                | ContainerId::Flow
                | ContainerId::FlowGroup
        ) && !def.is_group()
            && !def.is_part()
            && !container_def(*def).is_panel
            && closing.trim().is_empty()
            && (node.trailing.trim().is_empty()
                || children.last().is_some_and(|c| c.trailing.is_empty()))
            && !self.is_locked_branch(id)
            && fits
    }

    /// Moves a container's children out into its parent, in its place, and removes it. Comments
    /// above it go to its first child and a same-line comment after it to its last. Returns the
    /// children's ids.
    pub fn unwrap(&mut self, id: NodeId) -> Result<Vec<NodeId>, EditError> {
        if !self.can_unwrap(id) {
            return Err(match self.find(id).map(|n| &n.kind) {
                None => EditError::NoSuchNode(id),
                Some(NodeKind::Container { .. }) => EditError::Locked(id),
                Some(_) => EditError::NotAContainer(id),
            });
        }
        let (parent, at) = self.parent_of(id).ok_or(EditError::IsRoot)?;
        let backup = self.clone();
        let result = (|| {
            let container = self.remove(id)?;
            let NodeKind::Container { mut children, .. } = container.kind else {
                return Err(EditError::NotAContainer(id));
            };
            if let Some(first) = children.first_mut()
                && !container.leading.trim().is_empty()
            {
                let above = match container.leading.rfind('\n') {
                    Some(i) => &container.leading[..=i],
                    None => "",
                };
                let own = first
                    .leading
                    .strip_prefix("\r\n")
                    .or_else(|| first.leading.strip_prefix('\n'))
                    .unwrap_or(&first.leading);
                first.leading = format!("{above}{own}");
            }
            if let Some(last) = children.last_mut()
                && !container.trailing.trim().is_empty()
            {
                last.trailing = container.trailing.clone();
            }
            let ids: Vec<NodeId> = children.iter().map(|c| c.id).collect();
            for (k, child) in children.into_iter().enumerate() {
                self.insert(parent, at + k, child)?;
            }
            Ok(ids)
        })();
        if result.is_err() {
            *self = backup;
        }
        result
    }
}

// ---- Window anchor ---------------------------------------------------------------------------

/// The nine `egui::Align2` constants.
pub const ALIGN2: &[&str] = &[
    "LEFT_TOP",
    "CENTER_TOP",
    "RIGHT_TOP",
    "LEFT_CENTER",
    "CENTER_CENTER",
    "RIGHT_CENTER",
    "LEFT_BOTTOM",
    "CENTER_BOTTOM",
    "RIGHT_BOTTOM",
];

/// A Window's `.anchor(egui::Align2::<align>, [dx, dy])`, as the arguments' text.
pub fn window_anchor_text(egui_path: &str, align: &str, dx: f64, dy: f64) -> String {
    format!("{egui_path}Align2::{align}, [{}, {}]", num(dx), num(dy))
}

/// The parts of a Window's anchor arguments, when they are literal: `(align, dx, dy)`.
pub fn parse_window_anchor(text: &str) -> Option<(&'static str, f64, f64)> {
    let comma = text.find(',')?;
    let (align, offset) = (text[..comma].trim(), text[comma + 1..].trim());
    let name = align.rsplit("::").next()?;
    let (prefix, _) = split_path(align, "Align2::")?;
    if !is_path_prefix(prefix) {
        return None;
    }
    let align = ALIGN2.iter().copied().find(|a| *a == name)?;
    let (dx, dy) = split_pair(offset)?;
    Some((align, number(&dx)?, number(&dy)?))
}

/// A literal number of a size expression: `12`, `12.5`, `-3.0_f32`, `4f32`.
pub fn number(text: &str) -> Option<f64> {
    let t = text.trim();
    let t = t
        .strip_suffix("_f32")
        .or_else(|| t.strip_suffix("_f64"))
        .or_else(|| t.strip_suffix("f32"))
        .or_else(|| t.strip_suffix("f64"))
        .unwrap_or(t);
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A number as Rust source, always with a decimal point: `12.0`, `12.5`.
pub fn num(v: f64) -> String {
    format!("{v:?}")
}

/// A literal pair, `egui::vec2(8.0, 3.0)` or `[8.0, 3.0]`, as its two numbers.
pub fn parse_vec2(text: &str) -> Option<(f64, f64)> {
    let (a, b) = split_pair(text)?;
    Some((number(&a)?, number(&b)?))
}

/// `egui::vec2(x, y)` with the form's egui path.
pub fn vec2_text(egui_path: &str, x: f64, y: f64) -> String {
    format!("{egui_path}vec2({}, {})", num(x), num(y))
}

// ---- Resize ----------------------------------------------------------------------------------

/// Where one axis of a resize is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeTarget {
    /// A float property in points: `.desired_width(120.0)`.
    Float(&'static str),
    /// A row count: `.desired_rows(4)`.
    Rows(&'static str),
    /// `.min_size(egui::vec2(w, h))`, both axes at once.
    MinSize,
    /// One float for both axes: a Spinner's `.size(16.0)`.
    Square(&'static str),
    /// `.fit_to_exact_size(egui::vec2(w, h))`, both axes at once (an Image).
    ExactSize,
    /// A pair property, `.default_size(egui::vec2(w, h))` of a Window: only the dragged axes
    /// change, the other keeps its written number.
    Pair(&'static str),
}

/// The properties a resize of `node` writes, per axis; `None` on an axis that falls back to
/// `ui.add_sized([w, h], ..)` when the widget has it, or can't be resized at all.
///
/// The sizes are those of the node's rect on the canvas, except for the body statements of a
/// container with a margin (Frame, Group, Indent, CollapsingHeader): those size the body, and the
/// designer takes the margin it measured off the dragged size first.
pub fn resize_targets(node: &Node) -> (Option<SizeTarget>, Option<SizeTarget>) {
    use SizeTarget::*;
    match &node.kind {
        NodeKind::Widget { def, .. } => match def {
            WidgetId::TextEditSingleline => (Some(Float("desired_width")), None),
            WidgetId::TextEditMultiline => {
                (Some(Float("desired_width")), Some(Rows("desired_rows")))
            }
            WidgetId::ProgressBar => (Some(Float("desired_width")), Some(Float("desired_height"))),
            WidgetId::ComboBox => (Some(Float("width")), None),
            WidgetId::Spinner => (Some(Square("size")), Some(Square("size"))),
            WidgetId::Button | WidgetId::DragValue => (Some(MinSize), Some(MinSize)),
            WidgetId::Label
            | WidgetId::Heading
            | WidgetId::Small
            | WidgetId::Strong
            | WidgetId::Weak
            | WidgetId::Monospace
            | WidgetId::Link
            | WidgetId::Hyperlink
            | WidgetId::Checkbox
            | WidgetId::ToggleValue
            | WidgetId::RadioValue
            | WidgetId::SelectableValue
            | WidgetId::Slider
            | WidgetId::Separator
            | WidgetId::AddSpace
            | WidgetId::EndRow
            | WidgetId::ColorEdit
            | WidgetId::ColorEditRgb
            | WidgetId::RangeSlider => (None, None),
            // An image keeps its aspect ratio inside `.fit_to_exact_size(..)`, written as a pair.
            WidgetId::Image => (Some(ExactSize), Some(ExactSize)),
        },
        NodeKind::Container { def, props, .. } => {
            let body = (Some(Float("set_min_width")), Some(Float("set_min_height")));
            match def {
                ContainerId::ScrollArea => (Some(Float("max_width")), Some(Float("max_height"))),
                // A layout with no margin of its own: its rect is its `Ui`'s, which the body's
                // `ui.set_min_width(..)` and `ui.set_min_height(..)` grow.
                ContainerId::Horizontal
                | ContainerId::HorizontalWrapped
                | ContainerId::Vertical
                | ContainerId::VerticalCentered
                | ContainerId::HorizontalCentered
                | ContainerId::HorizontalTop
                | ContainerId::VerticalCenteredJustified
                | ContainerId::CenteredAndJustified
                | ContainerId::Scope
                | ContainerId::WithLayout
                | ContainerId::AddEnabledUi => body,
                // Their body sits inside a margin (an indent's is to its left, outside its rect).
                ContainerId::Group | ContainerId::Indent | ContainerId::CollapsingHeader => body,
                // Only margins and a stroke the preview draws as at runtime: an expression margin
                // (`egui::Margin::symmetric(8, 4)`) would put the drag off by what it adds.
                ContainerId::Frame if frame_margins_known(props) => body,
                ContainerId::Frame => (None, None),
                // The panel's outer size, margins included, on the axis it grows along.
                ContainerId::PanelLeft | ContainerId::PanelRight => match panel_size_prop(props) {
                    Some(name) => (Some(Float(name)), None),
                    None => (None, None),
                },
                ContainerId::PanelTop | ContainerId::PanelBottom => match panel_size_prop(props) {
                    Some(name) => (None, Some(Float(name))),
                    None => (None, None),
                },
                // `.height(h)`: the row's least height; it always takes the full width.
                ContainerId::Sides => (None, Some(Float("height"))),
                ContainerId::Window => window_targets(props),
                // The central panel is what the other panels leave; the root is the preview's
                // width (the canvas grip); a column, side, tab page, row or cell is sized by its
                // holder; a grid by its cells (its column widths are per column); a menu button
                // by its text (its body statements size the open menu); a menu bar takes the
                // full width; a modal's preview frame has another margin than at runtime; a
                // dialog helper's frame and size are the helper's own code; tabs and tables are
                // written by hand with no overall size; control flow is locked code.
                ContainerId::CentralPanel
                | ContainerId::Root
                | ContainerId::Grid
                | ContainerId::Columns
                | ContainerId::Column
                | ContainerId::Side
                | ContainerId::MenuButton
                | ContainerId::MenuBar
                | ContainerId::Modal
                | ContainerId::DialogCall
                | ContainerId::Tabs
                | ContainerId::TabsVertical
                | ContainerId::TabPage
                | ContainerId::Table
                | ContainerId::TableRow
                | ContainerId::TableCell
                | ContainerId::Flow
                | ContainerId::FlowGroup => (None, None),
            }
        }
        NodeKind::Custom { .. } => (None, None),
    }
}

/// Which edges carry the handles: `(left, top)` instead of `(right, bottom)`. A right panel grows
/// leftwards and a bottom panel upwards, so their handle is on the edge towards the form.
pub fn resize_from_start(node: &Node) -> (bool, bool) {
    match &node.kind {
        NodeKind::Container {
            def: ContainerId::PanelRight,
            ..
        } => (true, false),
        NodeKind::Container {
            def: ContainerId::PanelBottom,
            ..
        } => (false, true),
        _ => (false, false),
    }
}

/// Does the designer take a measured margin off the dragged size before writing it (the body
/// statements of a container whose rect is larger than its body)?
pub fn resize_body_inset(node: &Node) -> bool {
    matches!(
        &node.kind,
        NodeKind::Container {
            def: ContainerId::Group
                | ContainerId::Indent
                | ContainerId::CollapsingHeader
                | ContainerId::Frame,
            ..
        }
    )
}

/// A Frame whose margins and stroke are plain numbers (or egui's own for its variant).
fn frame_margins_known(props: &Props) -> bool {
    let number = |name: &str| matches!(props.get(name), None | Some(PropValue::Float(_)));
    number("inner_margin")
        && number("outer_margin")
        && matches!(props.get("stroke"), None | Some(PropValue::Stroke { .. }))
}

/// The panel property a drag writes: `exact_size` when the panel has a literal one, else
/// `default_size` (its first size; `resizable` stays as written). `None` when an `exact_size`
/// expression decides the size.
fn panel_size_prop(props: &Props) -> Option<&'static str> {
    match props.get("exact_size") {
        None => Some("default_size"),
        Some(PropValue::Float(_)) => Some("exact_size"),
        Some(_) => None,
    }
}

/// The range a panel's size is clamped to, from its literal `min_size`, `max_size` and
/// `size_range` in the order egui applies them; open ends are infinite.
pub fn panel_size_range(props: &Props) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    if let Some(PropValue::Float(v)) = props.get("min_size") {
        lo = *v;
        hi = hi.max(*v);
    }
    if let Some(PropValue::Float(v)) = props.get("max_size") {
        hi = *v;
        lo = lo.min(*v);
    }
    if let Some(PropValue::Expr(text)) = props.get("size_range")
        && let Some((a, b)) = literal_range(text)
    {
        (lo, hi) = (a, b);
    }
    (lo, hi)
}

/// `10.0..=300.0` or `10.0..300.0` as its two numbers.
pub fn literal_range(text: &str) -> Option<(f64, f64)> {
    let (a, b) = text.split_once("..=").or_else(|| text.split_once(".."))?;
    Some((number(a)?, number(b)?))
}

/// A Window's outer size, which egui 0.37 counts with its frame and title bar. The width is the
/// window's at runtime: its title bar spans the width it is given. The height is only when a
/// scroll area fills the window (`vscroll` or `hscroll`); otherwise egui sizes it to its content.
/// `auto_sized` windows have no size; a pair that is no literal can't be rewritten one axis at a
/// time.
fn window_targets(props: &Props) -> (Option<SizeTarget>, Option<SizeTarget>) {
    use SizeTarget::*;
    if props.get("auto_sized") == Some(&PropValue::Bool(true)) {
        return (None, None);
    }
    let scrolls = ["vscroll", "hscroll"]
        .iter()
        .any(|n| props.get(n) == Some(&PropValue::Bool(true)));
    // `fixed_size` comes last in the chain, so it wins over `default_size`, which wins over
    // `default_width` and `default_height`.
    match ["fixed_size", "default_size"]
        .into_iter()
        .find(|n| props.contains_key(n))
    {
        Some(name) => match props.get(name) {
            Some(PropValue::Expr(t)) if parse_vec2(t).is_some() => {
                (Some(Pair(name)), scrolls.then_some(Pair(name)))
            }
            _ => (None, None),
        },
        None => (
            Some(Float("default_width")),
            scrolls.then_some(Float("default_height")),
        ),
    }
}

/// Does `node` take `ui.add_sized(..)`?
fn has_add_sized(node: &Node) -> bool {
    matches!(&node.kind, NodeKind::Widget { def, .. } if widget_def(*def).prop("add_sized").is_some())
}

/// Which axes of `node` can be dragged: (width, height). The handles are on the right and bottom
/// edges, or the left and top ones ([`resize_from_start`]).
pub fn resize_axes(node: &Node) -> (bool, bool) {
    let (w, h) = resize_targets(node);
    let sized = has_add_sized(node);
    (w.is_some() || sized, h.is_some() || sized)
}

/// A size asked for with the mouse: the new width and height in points (the unchanged axis at its
/// current value), the row count for a row-based height, and which axes the drag changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeRequest {
    pub width: f64,
    pub height: f64,
    pub rows: Option<i64>,
    pub change_width: bool,
    pub change_height: bool,
}

impl Form {
    /// Writes a size into `id`'s size properties ([`resize_targets`]). An axis with no property of
    /// its own, or a widget already sized with `add_sized`, gets `ui.add_sized([w, h], ..)`, which
    /// then replaces the width and height properties it overrules.
    pub fn resize(&mut self, id: NodeId, req: SizeRequest) -> Result<(), EditError> {
        let node = self.find(id).ok_or(EditError::NoSuchNode(id))?;
        if let NodeKind::Custom { .. } = node.kind {
            return Err(EditError::IsCustom(id));
        }
        let (wt, ht) = resize_targets(node);
        let sized = has_add_sized(node);
        let sized_set =
            matches!(&node.kind, NodeKind::Widget { props, .. } if props.contains_key("add_sized"));
        let use_sized = sized
            && (sized_set
                || (req.change_width && wt.is_none())
                || (req.change_height && ht.is_none()));
        // A panel's first size is clamped by egui to its size range: written clamped, the number
        // says what the panel shows.
        let panel_range = match &node.kind {
            NodeKind::Container { def, props, .. } if container_def(*def).is_panel => {
                Some(panel_size_range(props))
            }
            _ => None,
        };
        // The number a pair keeps on the axis the drag leaves alone.
        let pair_now = |name: &str| match &node.kind {
            NodeKind::Container { props, .. } => match props.get(name) {
                Some(PropValue::Expr(t)) => parse_vec2(t),
                _ => None,
            },
            _ => None,
        };
        let pairs: Vec<(&'static str, Option<(f64, f64)>)> = [wt, ht]
            .into_iter()
            .flatten()
            .filter_map(|t| match t {
                SizeTarget::Pair(n) => Some((n, pair_now(n))),
                _ => None,
            })
            .collect();
        let backup = self.clone();
        let path = self.egui_path.clone();
        let result = (|| {
            if use_sized {
                for target in [wt, ht].into_iter().flatten() {
                    let name = match target {
                        SizeTarget::Float(n)
                        | SizeTarget::Rows(n)
                        | SizeTarget::Square(n)
                        | SizeTarget::Pair(n) => n,
                        SizeTarget::MinSize => "min_size",
                        SizeTarget::ExactSize => "fit_to_exact_size",
                    };
                    self.clear_prop(id, name)?;
                }
                let text = format!("[{}, {}]", num(req.width), num(req.height));
                return self.set_prop(id, "add_sized", PropValue::Expr(text));
            }
            let mut min_size_done = false;
            for (change, target, value) in [
                (req.change_width, wt, req.width),
                (req.change_height, ht, req.height),
            ] {
                let Some(target) = target.filter(|_| change) else {
                    continue;
                };
                match target {
                    SizeTarget::Float(name) => {
                        let value = match panel_range {
                            Some((lo, hi)) if name == "default_size" => value.max(lo).min(hi),
                            _ => value,
                        };
                        self.set_prop(id, name, PropValue::Float(value))?;
                    }
                    SizeTarget::Pair(name) if !min_size_done => {
                        min_size_done = true;
                        let now = pairs.iter().find(|(n, _)| *n == name).and_then(|(_, v)| *v);
                        let (w0, h0) = now.unwrap_or((req.width, req.height));
                        let w = if req.change_width { req.width } else { w0 };
                        let h = if req.change_height { req.height } else { h0 };
                        self.set_prop(id, name, PropValue::Expr(vec2_text(&path, w, h)))?;
                    }
                    SizeTarget::Pair(_) => {}
                    SizeTarget::Rows(name) => {
                        let rows = req.rows.unwrap_or(1).max(1);
                        self.set_prop(id, name, PropValue::Int(rows))?;
                    }
                    SizeTarget::MinSize if !min_size_done => {
                        min_size_done = true;
                        let text = format!("{path}vec2({}, {})", num(req.width), num(req.height));
                        self.set_prop(id, "min_size", PropValue::Expr(text))?;
                    }
                    SizeTarget::MinSize => {}
                    SizeTarget::ExactSize if !min_size_done => {
                        min_size_done = true;
                        let text = format!("{path}vec2({}, {})", num(req.width), num(req.height));
                        self.set_prop(id, "fit_to_exact_size", PropValue::Expr(text))?;
                    }
                    SizeTarget::ExactSize => {}
                    SizeTarget::Square(name) => {
                        let side = match (req.change_width, req.change_height) {
                            (true, true) => req.width.max(req.height),
                            (true, false) => req.width,
                            _ => req.height,
                        };
                        self.set_prop(id, name, PropValue::Float(side))?;
                        break;
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            *self = backup;
        }
        result
    }

    /// Removes every size property of `id` (Fill included): back to egui's own size.
    pub fn reset_size(&mut self, id: NodeId) -> Result<(), EditError> {
        let node = self.find(id).ok_or(EditError::NoSuchNode(id))?;
        let names: Vec<&'static str> = match &node.kind {
            NodeKind::Widget { def, props, .. } => widget_def(*def)
                .size_props
                .iter()
                .copied()
                .filter(|n| props.contains_key(n))
                .collect(),
            NodeKind::Container { def, props, .. } => container_def(*def)
                .size_props
                .iter()
                .copied()
                .filter(|n| props.contains_key(n))
                .collect(),
            NodeKind::Custom { .. } => return Err(EditError::IsCustom(id)),
        };
        for name in names {
            self.clear_prop(id, name)?;
        }
        Ok(())
    }
}

// ---- Columns ---------------------------------------------------------------------------------

impl Form {
    /// Sets the number of columns of a Columns (1 to [`MAX_COLUMNS`]). New columns are empty; the
    /// children of removed ones go to the end of the new last column, so nothing is lost. One edit.
    pub fn set_columns(&mut self, id: NodeId, count: usize) -> Result<(), EditError> {
        let (cols, now) = match self.find(id).map(|n| &n.kind) {
            Some(NodeKind::Container {
                def: ContainerId::Columns,
                ident,
                children,
                ..
            }) => (ident.clone(), children.len()),
            Some(_) => return Err(EditError::NoSuchProp("count".into())),
            None => return Err(EditError::NoSuchNode(id)),
        };
        if !(1..=MAX_COLUMNS).contains(&count) {
            return Err(EditError::NoSuchProp("count".into()));
        }
        let backup = self.clone();
        let result = (|| {
            if count < now {
                let kept = self.find(id).expect("checked").children()[count - 1].id;
                let moving: Vec<NodeId> = self.find(id).expect("checked").children()[count..]
                    .iter()
                    .flat_map(|c| c.children().iter().map(|k| k.id))
                    .collect();
                for k in moving {
                    self.move_node(k, kept, usize::MAX)?;
                }
            }
            let added: Vec<Node> = (now..count).map(|i| self.new_column(&cols, i)).collect();
            self.edit(id, |node| {
                if let NodeKind::Container {
                    children, props, ..
                } = &mut node.kind
                {
                    let leading = children.first().map(|c| c.leading.clone());
                    children.truncate(count);
                    for mut column in added {
                        column.leading = leading.clone().unwrap_or_default();
                        children.push(column);
                    }
                    props.insert("count", PropValue::Int(count as i64));
                }
            })
        })();
        if result.is_err() {
            *self = backup;
        }
        result
    }
}

/// The most columns the designer makes.
pub const MAX_COLUMNS: usize = 12;

// ---- Multi-selection -------------------------------------------------------------------------

impl Form {
    /// The nodes of `ids` that aren't inside another of them, in tree order (parents first,
    /// siblings in their order). Unknown ids and the root are left out.
    pub fn top_level(&self, ids: &[NodeId]) -> Vec<NodeId> {
        let mut out = Vec::new();
        fn walk(n: &Node, ids: &[NodeId], out: &mut Vec<NodeId>) {
            if ids.contains(&n.id) {
                out.push(n.id);
                return;
            }
            for c in n.children() {
                walk(c, ids, out);
            }
        }
        for c in self.root.children() {
            walk(c, ids, &mut out);
        }
        out
    }

    /// Puts `ids` (anywhere in the form) next to each other after the first of them in tree order,
    /// in tree order, then wraps them in a new `def` container there. One edit, atomic on refusal.
    pub fn wrap_any(&mut self, ids: &[NodeId], def: ContainerId) -> Result<NodeId, EditError> {
        let ids = self.top_level(ids);
        let first = *ids.first().ok_or(EditError::NoSuchNode(NodeId(u32::MAX)))?;
        let backup = self.clone();
        let result = (|| {
            for &id in &ids {
                if !self.can_wrap(id) {
                    return Err(EditError::Locked(id));
                }
            }
            let (parent, _) = self.parent_of(first).ok_or(EditError::IsRoot)?;
            for (k, &id) in ids.iter().enumerate().skip(1) {
                let (_, at) = self.parent_of(first).ok_or(EditError::IsRoot)?;
                if self.parent_of(id) == Some((parent, at + k)) {
                    continue;
                }
                let node = self.remove(id)?;
                let (_, at) = self.parent_of(first).ok_or(EditError::IsRoot)?;
                self.insert(parent, at + k, node)?;
            }
            self.wrap(&ids, def)
        })();
        if result.is_err() {
            *self = backup;
        }
        result
    }

    /// Removes several nodes (the top-level ones of `ids`) as one edit, atomic on refusal.
    pub fn remove_all(&mut self, ids: &[NodeId]) -> Result<(), EditError> {
        let ids = self.top_level(ids);
        let backup = self.clone();
        for id in ids {
            if let Err(e) = self.remove(id) {
                *self = backup;
                return Err(e);
            }
        }
        Ok(())
    }

    /// Sets `ui.add_space(amount)` right after `after` (an existing space there is updated, and an
    /// amount of zero or less removes it). For "distribute evenly".
    pub fn set_space_after(&mut self, after: NodeId, amount: f64) -> Result<(), EditError> {
        let (parent, at) = self.parent_of(after).ok_or(EditError::IsRoot)?;
        let next = self
            .find(parent)
            .and_then(|p| p.children().get(at + 1))
            .filter(|n| {
                matches!(
                    n.kind,
                    NodeKind::Widget {
                        def: WidgetId::AddSpace,
                        ..
                    }
                )
            })
            .map(|n| n.id);
        let amount = (amount * 2.0).round() / 2.0;
        match next {
            Some(space) if amount <= 0.0 => self.remove(space).map(|_| ()),
            Some(space) => self.set_prop(space, "amount", PropValue::Float(amount)),
            None if amount <= 0.0 => Ok(()),
            None => {
                let mut node = self.new_widget(WidgetId::AddSpace);
                if let NodeKind::Widget { props, .. } = &mut node.kind {
                    props.insert("amount", PropValue::Float(amount));
                }
                self.insert(parent, at + 1, node)
            }
        }
    }

    /// The `ui.add_space(..)` amount right after `id`, if there is one with a literal amount.
    pub fn space_after(&self, id: NodeId) -> Option<f64> {
        let (parent, at) = self.parent_of(id)?;
        match &self.find(parent)?.children().get(at + 1)?.kind {
            NodeKind::Widget {
                def: WidgetId::AddSpace,
                props,
                ..
            } => match props.get("amount") {
                Some(PropValue::Float(v)) => Some(*v),
                Some(PropValue::Int(v)) => Some(*v as f64),
                _ => None,
            },
            _ => None,
        }
    }
}

// ---- Tab order -------------------------------------------------------------------------------

/// Does the widget take the keyboard focus with Tab? egui moves the focus in the order widgets are
/// created, so these, in tree order, are the form's tab order.
pub fn is_focusable(def: WidgetId) -> bool {
    match def {
        WidgetId::Button
        | WidgetId::Link
        | WidgetId::Hyperlink
        | WidgetId::Checkbox
        | WidgetId::ToggleValue
        | WidgetId::RadioValue
        | WidgetId::SelectableValue
        | WidgetId::TextEditSingleline
        | WidgetId::TextEditMultiline
        | WidgetId::Slider
        | WidgetId::DragValue
        | WidgetId::ComboBox
        | WidgetId::ColorEdit
        | WidgetId::ColorEditRgb
        | WidgetId::RangeSlider => true,
        WidgetId::Image
        | WidgetId::Label
        | WidgetId::Heading
        | WidgetId::Small
        | WidgetId::Strong
        | WidgetId::Weak
        | WidgetId::Monospace
        | WidgetId::ProgressBar
        | WidgetId::Spinner
        | WidgetId::Separator
        | WidgetId::AddSpace
        | WidgetId::EndRow => false,
    }
}

impl Form {
    /// The focusable widgets in the order Tab visits them: tree order.
    pub fn tab_order(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.root.walk(&mut |n| {
            if let NodeKind::Widget { def, .. } = &n.kind
                && is_focusable(*def)
            {
                out.push(n.id);
            }
        });
        out
    }

    /// Moves the widget `id` so it comes just before (or after) `target` in the tab order: its host
    /// (an anchor wrapper, if it has one) goes next to `target`'s host, in `target`'s container.
    pub fn move_in_tab_order(
        &mut self,
        id: NodeId,
        target: NodeId,
        after: bool,
    ) -> Result<(), EditError> {
        let host = self.anchor_host(id);
        let target_host = self.anchor_host(target);
        if host == target_host {
            return Ok(());
        }
        let (parent, at) = self.parent_of(target_host).ok_or(EditError::IsRoot)?;
        let mut index = at + usize::from(after);
        if let Some((old_parent, old)) = self.parent_of(host)
            && old_parent == parent
            && old < index
        {
            index -= 1;
        }
        self.move_node(host, parent, index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props_of(form: &Form, id: NodeId) -> Props {
        match &form.find(id).unwrap().kind {
            NodeKind::Widget { props, .. } | NodeKind::Container { props, .. } => props.clone(),
            NodeKind::Custom { .. } => Props::new(),
        }
    }

    fn add(form: &mut Form, parent: NodeId, def: WidgetId) -> NodeId {
        let node = form.new_widget(def);
        let id = node.id;
        form.insert(parent, usize::MAX, node).unwrap();
        id
    }

    #[test]
    fn fill_texts_and_pairs_are_read() {
        for t in [
            "f32::INFINITY",
            "f32 :: MAX",
            "std::f32::INFINITY",
            "ui.available_width()",
            "inner . available_width ()",
        ] {
            assert!(is_fill_text(t), "{t}");
        }
        for t in [
            "120.0",
            "self.w",
            "available_width()",
            "a.b.available_width()",
        ] {
            assert!(!is_fill_text(t), "{t}");
        }
        assert_eq!(
            split_pair("[ui.available_width(), 20.0]"),
            Some(("ui.available_width()".into(), "20.0".into()))
        );
        assert_eq!(
            split_pair("egui::vec2(f(1, 2), 3.0)"),
            Some(("f(1, 2)".into(), "3.0".into()))
        );
        assert_eq!(
            split_pair("egui::Vec2::new(1.0, 2.0)"),
            Some(("1.0".into(), "2.0".into()))
        );
        assert_eq!(split_pair("size"), None);
        assert_eq!(split_pair("[1.0, 2.0, 3.0]"), None);
        assert_eq!(split_pair("foo(1.0, 2.0)"), None);
    }

    #[test]
    fn fill_is_written_the_way_each_widget_takes_it() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let text = add(&mut form, root, WidgetId::TextEditSingleline);
        let bar = add(&mut form, root, WidgetId::ProgressBar);
        let button = add(&mut form, root, WidgetId::Button);
        let label = add(&mut form, root, WidgetId::Label);
        for id in [text, bar, button] {
            form.set_fill(id, true).unwrap();
            let NodeKind::Widget { def, props, .. } = &form.find(id).unwrap().kind else {
                panic!()
            };
            assert!(is_fill(*def, props), "{def:?}");
        }
        assert_eq!(
            props_of(&form, text).get("desired_width"),
            Some(&PropValue::Float(f64::INFINITY))
        );
        assert_eq!(
            props_of(&form, button).get("add_sized"),
            Some(&PropValue::Float(f64::INFINITY))
        );
        assert!(form.set_fill(label, true).is_err());
        for id in [text, bar, button] {
            form.set_fill(id, false).unwrap();
            assert!(
                props_of(&form, id).is_empty() || !props_of(&form, id).contains_key("add_sized")
            );
            let NodeKind::Widget { def, props, .. } = &form.find(id).unwrap().kind else {
                panic!()
            };
            assert!(!is_fill(*def, props));
        }
        // A button with `enabled` set fills too: the two wrappers nest.
        form.set_prop(button, "enabled", PropValue::Bool(false))
            .unwrap();
        form.set_fill(button, true).unwrap();
        let NodeKind::Widget { def, props, .. } = &form.find(button).unwrap().kind else {
            panic!()
        };
        assert!(is_fill(*def, props));
        assert_eq!(props.get("enabled"), Some(&PropValue::Bool(false)));
    }

    #[test]
    fn imported_fill_values_read_as_fill() {
        let mut props = Props::new();
        props.insert("desired_width", PropValue::Expr("f32::MAX".into()));
        assert!(is_fill(WidgetId::TextEditMultiline, &props));
        let mut props = Props::new();
        props.insert(
            "add_sized",
            PropValue::Expr("[ui.available_width(), 24.0]".into()),
        );
        assert!(is_fill(WidgetId::Button, &props));
        assert!(!is_fill(WidgetId::Label, &props), "a label has no Fill");
    }

    #[test]
    fn anchor_layouts_round_trip_and_nothing_else_is_an_anchor() {
        for &a in ALL_ANCHORS {
            for path in ["egui::", "eframe::egui::", ""] {
                assert_eq!(Anchor::from_layout(&a.layout_text(path)), Some(a));
            }
        }
        assert_eq!(
            Anchor::from_layout("egui::Layout::right_to_left( egui::Align::Min )"),
            Some(Anchor::Right)
        );
        for t in [
            "egui::Layout::right_to_left(egui::Align::Center)",
            "egui::Layout::top_down(egui::Align::Max)",
            "my_layout",
            "egui::Layout::right_to_left(egui::Align::Min).with_main_wrap(true)",
            "x + egui::Layout::left_to_right(egui::Align::Min)",
        ] {
            assert_eq!(Anchor::from_layout(t), None, "{t}");
        }
    }

    #[test]
    fn an_anchor_wraps_rewrites_and_unwraps_as_one_edit_each() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let a = add(&mut form, root, WidgetId::Label);
        let b = add(&mut form, root, WidgetId::Button);
        form.set_anchor(b, Some(Anchor::Right)).unwrap();
        assert_eq!(form.anchor_of(b), Some(Anchor::Right));
        let host = form.anchor_host(b);
        assert_ne!(host, b);
        assert_eq!(form.parent_of(host), Some((root, 1)));
        assert_eq!(form.anchor_child(host), b);
        form.set_anchor(b, Some(Anchor::Bottom)).unwrap();
        assert_eq!(
            form.anchor_host(b),
            host,
            "the same wrapper, another layout"
        );
        assert_eq!(form.anchor_of(b), Some(Anchor::Bottom));
        form.set_anchor(b, None).unwrap();
        assert_eq!(form.anchor_of(b), None);
        assert_eq!(form.parent_of(b), Some((root, 1)));
        assert!(form.find(host).is_none());
        assert_eq!(form.anchor_of(a), None);
        // A Window's root in a dialog can't be wrapped.
        let mut dialog = Form::new("D", "show");
        dialog.context_root = true;
        let window = dialog.new_container(ContainerId::Window);
        let window_id = window.id;
        dialog.insert(dialog.root.id, 0, window).unwrap();
        assert!(!dialog.can_wrap(window_id));
        assert_eq!(
            dialog.set_anchor(window_id, Some(Anchor::Right)),
            Err(EditError::Locked(window_id))
        );
    }

    #[test]
    fn wrapping_takes_a_contiguous_run_and_unwrapping_puts_it_back() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let ids: Vec<NodeId> = (0..4)
            .map(|_| add(&mut form, root, WidgetId::Label))
            .collect();
        let before = form.clone();
        assert!(
            form.wrap(&[ids[0], ids[2]], ContainerId::Horizontal)
                .is_err()
        );
        assert_eq!(form, before, "a refused wrap changes nothing");
        let row = form
            .wrap(&[ids[2], ids[1]], ContainerId::Horizontal)
            .unwrap();
        assert_eq!(form.parent_of(row), Some((root, 1)));
        assert_eq!(form.parent_of(ids[1]), Some((row, 0)));
        assert_eq!(form.parent_of(ids[2]), Some((row, 1)));
        assert_eq!(form.parent_of(ids[3]), Some((root, 2)));
        let out = form.unwrap(row).unwrap();
        assert_eq!(out, [ids[1], ids[2]]);
        for (k, id) in ids.iter().enumerate() {
            assert_eq!(form.parent_of(*id), Some((root, k)));
        }
        assert!(form.unwrap(ids[0]).is_err(), "a widget isn't unwrapped");
        assert!(form.unwrap(root).is_err());
    }

    #[test]
    fn unwrapping_keeps_the_containers_comments() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let a = add(&mut form, root, WidgetId::Label);
        let row = form.wrap(&[a], ContainerId::Horizontal).unwrap();
        form.edit(row, |n| {
            n.leading = "\n        // the row\n        ".into();
            n.trailing = " // after".into();
        })
        .unwrap();
        form.unwrap(row).unwrap();
        let node = form.find(a).unwrap();
        assert!(node.leading.contains("// the row"), "{:?}", node.leading);
        assert_eq!(node.trailing, " // after");
        // A comment inside the body itself can't go anywhere: refused.
        let row = form.wrap(&[a], ContainerId::Horizontal).unwrap();
        form.edit(row, |n| {
            if let NodeKind::Container { closing, .. } = &mut n.kind {
                *closing = "\n        // inside\n        ".into();
            }
        })
        .unwrap();
        assert_eq!(form.unwrap(row), Err(EditError::Locked(row)));
    }

    #[test]
    fn a_resize_writes_the_widgets_own_size_properties() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let text = add(&mut form, root, WidgetId::TextEditMultiline);
        let label = add(&mut form, root, WidgetId::Label);
        let button = add(&mut form, root, WidgetId::Button);
        let strong = add(&mut form, root, WidgetId::Strong);
        let req = |w: f64, h: f64, cw: bool, ch: bool| SizeRequest {
            width: w,
            height: h,
            rows: Some(6),
            change_width: cw,
            change_height: ch,
        };
        form.resize(text, req(200.0, 90.0, true, true)).unwrap();
        let p = props_of(&form, text);
        assert_eq!(p.get("desired_width"), Some(&PropValue::Float(200.0)));
        assert_eq!(p.get("desired_rows"), Some(&PropValue::Int(6)));
        form.resize(label, req(150.0, 30.0, true, false)).unwrap();
        assert_eq!(
            props_of(&form, label).get("add_sized"),
            Some(&PropValue::Expr("[150.0, 30.0]".into()))
        );
        form.resize(button, req(80.0, 30.0, false, true)).unwrap();
        assert_eq!(
            props_of(&form, button).get("min_size"),
            Some(&PropValue::Expr("egui::vec2(80.0, 30.0)".into()))
        );
        // Strong text has no sized form (it would read back as a bold Label).
        assert_eq!(resize_axes(form.find(strong).unwrap()), (false, false));
        // A filled button sized with the mouse gets a fixed add_sized, and its min_size goes.
        form.set_fill(button, true).unwrap();
        form.resize(button, req(90.0, 30.0, true, false)).unwrap();
        let p = props_of(&form, button);
        assert_eq!(
            p.get("add_sized"),
            Some(&PropValue::Expr("[90.0, 30.0]".into()))
        );
        assert!(!p.contains_key("min_size"));
        form.reset_size(button).unwrap();
        assert!(
            props_of(&form, button).is_empty()
                || !props_of(&form, button).contains_key("add_sized")
        );
        form.reset_size(text).unwrap();
        assert!(!props_of(&form, text).contains_key("desired_rows"));
    }

    fn req(w: f64, h: f64, cw: bool, ch: bool) -> SizeRequest {
        SizeRequest {
            width: w,
            height: h,
            rows: None,
            change_width: cw,
            change_height: ch,
        }
    }

    fn add_container(form: &mut Form, parent: NodeId, def: ContainerId) -> NodeId {
        let node = form.new_container(def);
        let id = node.id;
        form.insert(parent, usize::MAX, node).unwrap();
        id
    }

    /// Every widget and container has handles exactly where a size can be written: the audit as a
    /// table, so a change to it is a deliberate one.
    #[test]
    fn the_resize_axes_of_every_registry_entry() {
        use crate::registry::{ALL_CONTAINERS, ALL_WIDGETS};
        let mut form = Form::new("F", "ui");
        let none = [
            WidgetId::Strong,
            WidgetId::Monospace,
            WidgetId::ToggleValue,
            WidgetId::RadioValue,
            WidgetId::SelectableValue,
            WidgetId::AddSpace,
            WidgetId::EndRow,
            WidgetId::ColorEdit,
            WidgetId::ColorEditRgb,
        ];
        for &w in ALL_WIDGETS {
            let node = form.new_widget(w);
            let want = if none.contains(&w) {
                (false, false)
            } else if w == WidgetId::ComboBox {
                // `.width(..)`; its height is egui's.
                (true, false)
            } else {
                (true, true)
            };
            assert_eq!(resize_axes(&node), want, "{w:?}");
        }
        for &c in ALL_CONTAINERS {
            let node = form.new_container(c);
            let want = match c {
                ContainerId::PanelLeft | ContainerId::PanelRight => (true, false),
                ContainerId::PanelTop | ContainerId::PanelBottom => (false, true),
                ContainerId::Sides => (false, true),
                // Only the width, until a scroll area fills its height.
                ContainerId::Window => (true, false),
                ContainerId::Horizontal
                | ContainerId::HorizontalWrapped
                | ContainerId::Vertical
                | ContainerId::VerticalCentered
                | ContainerId::HorizontalCentered
                | ContainerId::HorizontalTop
                | ContainerId::VerticalCenteredJustified
                | ContainerId::CenteredAndJustified
                | ContainerId::Scope
                | ContainerId::WithLayout
                | ContainerId::AddEnabledUi
                | ContainerId::ScrollArea
                | ContainerId::Group
                | ContainerId::Indent
                | ContainerId::CollapsingHeader
                | ContainerId::Frame => (true, true),
                _ => (false, false),
            };
            assert_eq!(resize_axes(&node), want, "{c:?}");
            assert_eq!(
                resize_from_start(&node),
                match c {
                    ContainerId::PanelRight => (true, false),
                    ContainerId::PanelBottom => (false, true),
                    _ => (false, false),
                },
                "{c:?}"
            );
        }
    }

    /// A panel's drag writes its first size (`default_size`), or its `exact_size` when it has one,
    /// clamped to its literal size range; `resizable` is left as written.
    #[test]
    fn a_panel_resize_writes_its_default_or_exact_size() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let left = add_container(&mut form, root, ContainerId::PanelLeft);
        let bottom = add_container(&mut form, root, ContainerId::PanelBottom);
        form.set_prop(left, "resizable", PropValue::Bool(false))
            .unwrap();
        form.resize(left, req(220.0, 500.0, true, false)).unwrap();
        let p = props_of(&form, left);
        assert_eq!(p.get("default_size"), Some(&PropValue::Float(220.0)));
        assert_eq!(p.get("resizable"), Some(&PropValue::Bool(false)));
        assert_eq!(p.len(), 3, "the id, resizable and the size: {p:?}");
        // Clamped to the size range egui would clamp it to.
        form.set_prop(left, "max_size", PropValue::Float(300.0))
            .unwrap();
        form.resize(left, req(400.0, 500.0, true, false)).unwrap();
        assert_eq!(
            props_of(&form, left).get("default_size"),
            Some(&PropValue::Float(300.0))
        );
        form.set_prop(left, "size_range", PropValue::Expr("120.0..=260.0".into()))
            .unwrap();
        form.resize(left, req(40.0, 500.0, true, false)).unwrap();
        assert_eq!(
            props_of(&form, left).get("default_size"),
            Some(&PropValue::Float(120.0))
        );
        // A bottom panel: its height; an exact size stays exact.
        form.set_prop(bottom, "exact_size", PropValue::Float(40.0))
            .unwrap();
        form.resize(bottom, req(600.0, 64.0, false, true)).unwrap();
        let p = props_of(&form, bottom);
        assert_eq!(p.get("exact_size"), Some(&PropValue::Float(64.0)));
        assert!(!p.contains_key("default_size"));
        // An exact size that is an expression decides the size: no handles.
        form.set_prop(bottom, "exact_size", PropValue::Expr("self.h".into()))
            .unwrap();
        assert_eq!(resize_axes(form.find(bottom).unwrap()), (false, false));
        form.reset_size(left).unwrap();
        assert_eq!(
            props_of(&form, left).len(),
            2,
            "the id and resizable are left"
        );
        assert_eq!(literal_range("10..20.5"), Some((10.0, 20.5)));
        assert_eq!(literal_range("self.r"), None);
    }

    /// A Window's width is `default_width`, or the dragged axis of its `default_size` or
    /// `fixed_size` pair; its height only when a scroll area fills it.
    #[test]
    fn a_window_resize_writes_its_outer_size() {
        let mut form = Form::new("D", "show");
        form.context_root = true;
        let root = form.root.id;
        let window = add_container(&mut form, root, ContainerId::Window);
        form.resize(window, req(380.0, 200.0, true, false)).unwrap();
        assert_eq!(
            props_of(&form, window).get("default_width"),
            Some(&PropValue::Float(380.0))
        );
        form.set_prop(window, "vscroll", PropValue::Bool(true))
            .unwrap();
        assert_eq!(resize_axes(form.find(window).unwrap()), (true, true));
        form.resize(window, req(380.0, 260.0, true, true)).unwrap();
        assert_eq!(
            props_of(&form, window).get("default_height"),
            Some(&PropValue::Float(260.0))
        );
        // A pair keeps the number on the axis the drag leaves alone.
        form.set_prop(
            window,
            "default_size",
            PropValue::Expr("[300.0, 150.0]".into()),
        )
        .unwrap();
        form.resize(window, req(410.0, 999.0, true, false)).unwrap();
        assert_eq!(
            props_of(&form, window).get("default_size"),
            Some(&PropValue::Expr("egui::vec2(410.0, 150.0)".into()))
        );
        form.set_prop(
            window,
            "fixed_size",
            PropValue::Expr("egui::vec2(200.0, 100.0)".into()),
        )
        .unwrap();
        form.resize(window, req(999.0, 120.0, false, true)).unwrap();
        let p = props_of(&form, window);
        assert_eq!(
            p.get("fixed_size"),
            Some(&PropValue::Expr("egui::vec2(200.0, 120.0)".into()))
        );
        assert_eq!(
            p.get("default_size"),
            Some(&PropValue::Expr("egui::vec2(410.0, 150.0)".into())),
            "the pair that loses to fixed_size is left alone"
        );
        form.set_prop(window, "fixed_size", PropValue::Expr("self.size".into()))
            .unwrap();
        assert_eq!(resize_axes(form.find(window).unwrap()), (false, false));
        form.clear_prop(window, "fixed_size").unwrap();
        form.set_prop(window, "auto_sized", PropValue::Bool(true))
            .unwrap();
        assert_eq!(resize_axes(form.find(window).unwrap()), (false, false));
        form.reset_size(window).unwrap();
        let p = props_of(&form, window);
        for name in ["default_width", "default_height", "default_size"] {
            assert!(!p.contains_key(name), "{name}");
        }
    }

    /// A Frame, Group, Indent and CollapsingHeader write their body's least size (the designer
    /// takes the margin off first); a Frame whose margin is an expression has no handles. Sides
    /// write their height.
    #[test]
    fn framed_containers_and_sides_resize_their_body() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        for def in [
            ContainerId::Frame,
            ContainerId::Group,
            ContainerId::Indent,
            ContainerId::CollapsingHeader,
        ] {
            let c = add_container(&mut form, root, def);
            assert!(resize_body_inset(form.find(c).unwrap()), "{def:?}");
            form.resize(c, req(150.0, 60.0, true, true)).unwrap();
            let p = props_of(&form, c);
            assert_eq!(
                p.get("set_min_width"),
                Some(&PropValue::Float(150.0)),
                "{def:?}"
            );
            assert_eq!(
                p.get("set_min_height"),
                Some(&PropValue::Float(60.0)),
                "{def:?}"
            );
        }
        let frame = add_container(&mut form, root, ContainerId::Frame);
        form.set_prop(frame, "inner_margin", PropValue::Float(8.0))
            .unwrap();
        assert_eq!(resize_axes(form.find(frame).unwrap()), (true, true));
        form.set_prop(
            frame,
            "inner_margin",
            PropValue::Expr("egui::Margin::symmetric(8, 4)".into()),
        )
        .unwrap();
        assert_eq!(resize_axes(form.find(frame).unwrap()), (false, false));
        let sides = add_container(&mut form, root, ContainerId::Sides);
        assert!(!resize_body_inset(form.find(sides).unwrap()));
        form.resize(sides, req(500.0, 36.0, false, true)).unwrap();
        assert_eq!(
            props_of(&form, sides).get("height"),
            Some(&PropValue::Float(36.0))
        );
    }

    /// Heading, Small and Weak are sized with `add_sized`, as a Label is.
    #[test]
    fn the_heading_family_resizes_with_add_sized() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        for def in [WidgetId::Heading, WidgetId::Small, WidgetId::Weak] {
            let id = add(&mut form, root, def);
            form.resize(id, req(160.0, 32.0, true, false)).unwrap();
            assert_eq!(
                props_of(&form, id).get("add_sized"),
                Some(&PropValue::Expr("[160.0, 32.0]".into())),
                "{def:?}"
            );
            form.reset_size(id).unwrap();
            assert!(!props_of(&form, id).contains_key("add_sized"));
        }
    }

    /// A `with_layout` with spacing or size statements is no anchor: removing it as one would lose
    /// them.
    #[test]
    fn a_wrapper_with_body_statements_is_no_anchor() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let a = add(&mut form, root, WidgetId::Label);
        form.set_anchor(a, Some(Anchor::Right)).unwrap();
        let wrapper = form.anchor_host(a);
        assert_ne!(wrapper, a);
        form.set_prop(wrapper, "set_min_width", PropValue::Float(90.0))
            .unwrap();
        assert_eq!(form.anchor_of(a), None);
        assert_eq!(form.anchor_host(a), a);
        assert_eq!(resize_axes(form.find(wrapper).unwrap()), (true, true));
    }

    #[test]
    fn literal_pairs_read_back() {
        assert_eq!(parse_vec2("egui::vec2(8.0, 3.0)"), Some((8.0, 3.0)));
        assert_eq!(parse_vec2("[4, 1.5]"), Some((4.0, 1.5)));
        assert_eq!(parse_vec2("egui::vec2(self.x, 3.0)"), None);
        assert_eq!(vec2_text("egui::", 8.0, 3.0), "egui::vec2(8.0, 3.0)");
        assert_eq!(parse_vec2(&vec2_text("", 1.0, 2.5)), Some((1.0, 2.5)));
    }

    #[test]
    fn window_anchor_arguments_read_back() {
        let text = window_anchor_text("egui::", "RIGHT_TOP", -8.0, 8.0);
        assert_eq!(text, "egui::Align2::RIGHT_TOP, [-8.0, 8.0]");
        assert_eq!(parse_window_anchor(&text), Some(("RIGHT_TOP", -8.0, 8.0)));
        assert_eq!(
            parse_window_anchor("Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)"),
            Some(("CENTER_CENTER", 0.0, 0.0))
        );
        assert_eq!(parse_window_anchor("self.align, [0.0, 0.0]"), None);
    }

    #[test]
    fn a_scattered_selection_is_gathered_and_wrapped_as_one_edit() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let a = add(&mut form, root, WidgetId::Label);
        let b = add(&mut form, root, WidgetId::Button);
        let row = form.new_container(ContainerId::Horizontal);
        let row_id = row.id;
        form.insert(root, usize::MAX, row).unwrap();
        let c = add(&mut form, row_id, WidgetId::Checkbox);
        let d = add(&mut form, root, WidgetId::Label);
        // In any order; `c` is inside another container.
        let column = form.wrap_any(&[d, c, a], ContainerId::Vertical).unwrap();
        assert_eq!(form.parent_of(column), Some((root, 0)));
        let kids: Vec<NodeId> = form
            .find(column)
            .unwrap()
            .children()
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(kids, [a, c, d], "tree order");
        assert_eq!(form.parent_of(b), Some((root, 1)));
        assert!(form.find(row_id).unwrap().children().is_empty());
        // A refused gather changes nothing.
        let before = form.clone();
        assert!(form.wrap_any(&[root], ContainerId::Vertical).is_err());
        assert_eq!(form, before);
        assert_eq!(form.top_level(&[column, a, b]), [column, b]);
    }

    #[test]
    fn several_nodes_are_removed_together_and_spaces_set_between() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let a = add(&mut form, root, WidgetId::Label);
        let b = add(&mut form, root, WidgetId::Label);
        let c = add(&mut form, root, WidgetId::Label);
        form.set_space_after(a, 12.0).unwrap();
        assert_eq!(form.space_after(a), Some(12.0));
        form.set_space_after(a, 4.0).unwrap();
        assert_eq!(form.space_after(a), Some(4.0), "updated, not doubled");
        assert_eq!(form.root.children().len(), 4);
        form.set_space_after(a, 0.0).unwrap();
        assert_eq!(form.space_after(a), None);
        assert_eq!(form.root.children().len(), 3);
        form.remove_all(&[c, a]).unwrap();
        assert_eq!(form.root.children().len(), 1);
        assert_eq!(form.root.children()[0].id, b);
    }

    #[test]
    fn the_tab_order_is_tree_order_and_moving_in_it_moves_the_node() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let label = add(&mut form, root, WidgetId::Label);
        let name = add(&mut form, root, WidgetId::TextEditSingleline);
        let row = form.new_container(ContainerId::Horizontal);
        let row_id = row.id;
        form.insert(root, usize::MAX, row).unwrap();
        let ok = add(&mut form, row_id, WidgetId::Button);
        let check = add(&mut form, root, WidgetId::Checkbox);
        assert_eq!(form.tab_order(), [name, ok, check]);
        assert!(!form.tab_order().contains(&label));
        // The checkbox before the button: into the row, first.
        form.move_in_tab_order(check, ok, false).unwrap();
        assert_eq!(form.tab_order(), [name, check, ok]);
        assert_eq!(form.parent_of(check), Some((row_id, 0)));
        // The text field after the button.
        form.move_in_tab_order(name, ok, true).unwrap();
        assert_eq!(form.tab_order(), [check, ok, name]);
        // An anchored widget moves with its wrapper.
        form.set_anchor(ok, Some(Anchor::Right)).unwrap();
        let host = form.anchor_host(ok);
        form.move_in_tab_order(ok, check, false).unwrap();
        assert_eq!(form.parent_of(host), Some((row_id, 0)));
        assert_eq!(form.tab_order(), [ok, check, name]);
    }

    #[test]
    fn columns_come_with_their_parts_and_the_count_changes_them() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let cols = form.new_container(ContainerId::Columns);
        let cols_id = cols.id;
        form.insert(root, 0, cols).unwrap();
        let parts: Vec<NodeId> = form
            .find(cols_id)
            .unwrap()
            .children()
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(parts.len(), 2);
        let a = add(&mut form, parts[1], WidgetId::Label);
        form.set_columns(cols_id, 4).unwrap();
        assert_eq!(form.find(cols_id).unwrap().children().len(), 4);
        form.set_columns(cols_id, 1).unwrap();
        assert_eq!(form.parent_of(a), Some((parts[0], 0)), "moved, not lost");
        assert!(form.set_columns(cols_id, MAX_COLUMNS + 1).is_err());
        // A new Columns inside a column names its slice afresh.
        let inner = form.new_container(ContainerId::Columns);
        let inner_id = inner.id;
        form.insert(parts[0], 0, inner).unwrap();
        let NodeKind::Container { ident, .. } = &form.find(inner_id).unwrap().kind else {
            panic!()
        };
        assert_eq!(ident, "cols2");
        let sides = form.new_container(ContainerId::Sides);
        assert_eq!(sides.children().len(), 2);
        assert!(is_ui_text("cols[0]") && is_ui_text("ui") && !is_ui_text("cols[x]"));
        assert!(is_fill_text("cols[1].available_width()"));
        assert!(is_plain_ui("ui") && !is_plain_ui("cols[0]"));
    }
}
