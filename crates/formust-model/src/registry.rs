//! The widget and container registry: one table entry per palette item, holding the label, typed
//! properties (defaults and enum values), events and size properties.
//!
//! The table describes *data*. The shapes of the calls (`ui.label(..)` or `ui.add(Label::new(..))`)
//! live with the importer and emitter in `formust-rust`, which `match` exhaustively on [`WidgetId`]
//! and [`ContainerId`], so a new widget doesn't compile until every part exists.

use crate::form::PropValue;

/// Every palette widget. Keep [`ALL_WIDGETS`] and [`widget_def`] in step; the `match` enforces one
/// of them and a test enforces the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WidgetId {
    Label,
    Heading,
    Small,
    Strong,
    Weak,
    Monospace,
    Button,
    Link,
    Hyperlink,
    Checkbox,
    ToggleValue,
    RadioValue,
    SelectableValue,
    TextEditSingleline,
    TextEditMultiline,
    Slider,
    DragValue,
    ProgressBar,
    Spinner,
    Separator,
    AddSpace,
    ComboBox,
    EndRow,
    /// `ui.image(source)` or `ui.add(egui::Image::new(source).chain())`.
    Image,
    /// `ui.color_edit_button_srgba(&mut colour)`, bound to an `egui::Color32`.
    ColorEdit,
    /// `ui.color_edit_button_rgb(&mut colour)`, bound to a `[f32; 3]`.
    ColorEditRgb,
    /// `ui.add(egui::RangeSlider::new(&mut low, &mut high, range))`: two bound values.
    RangeSlider,
}

pub const ALL_WIDGETS: &[WidgetId] = &[
    WidgetId::Label,
    WidgetId::Heading,
    WidgetId::Small,
    WidgetId::Strong,
    WidgetId::Weak,
    WidgetId::Monospace,
    WidgetId::Button,
    WidgetId::Link,
    WidgetId::Hyperlink,
    WidgetId::Checkbox,
    WidgetId::ToggleValue,
    WidgetId::RadioValue,
    WidgetId::SelectableValue,
    WidgetId::TextEditSingleline,
    WidgetId::TextEditMultiline,
    WidgetId::Slider,
    WidgetId::DragValue,
    WidgetId::ProgressBar,
    WidgetId::Spinner,
    WidgetId::Separator,
    WidgetId::AddSpace,
    WidgetId::ComboBox,
    WidgetId::EndRow,
    WidgetId::Image,
    WidgetId::ColorEdit,
    WidgetId::ColorEditRgb,
    WidgetId::RangeSlider,
];

/// Every container and root. `Root` is the function body itself: a plain `Ui`, or (when its children
/// are panels) the holder of `Panel::*` and `CentralPanel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContainerId {
    Root,
    Horizontal,
    HorizontalWrapped,
    Vertical,
    VerticalCentered,
    Grid,
    ScrollArea,
    CollapsingHeader,
    Group,
    Indent,
    PanelTop,
    PanelBottom,
    PanelLeft,
    PanelRight,
    CentralPanel,
    // Recognised on import but not offered by the first palette: the measurement on real code
    // showed they carry whole forms (see "Later" in docs/specs.md).
    HorizontalCentered,
    HorizontalTop,
    VerticalCenteredJustified,
    CenteredAndJustified,
    Scope,
    WithLayout,
    MenuButton,
    AddEnabledUi,
    Frame,
    /// `ui.columns(n, |cols| { cols[0].label(..); .. })`: its children are one [`ContainerId::Column`]
    /// per column, which can't be moved or removed on their own (the count does that).
    Columns,
    /// One column of a [`ContainerId::Columns`]. It has no text of its own: its children are the
    /// statements on `cols[i]`, and its `ident` is `cols[i]`.
    Column,
    /// `egui::Sides::new().show(ui, |ui| { .. }, |ui| { .. })`: its children are the two
    /// [`ContainerId::Side`] closures, left then right.
    Sides,
    /// One closure of a [`ContainerId::Sides`].
    Side,
    Window,
    /// A dialog shown by a helper call, `dialog(ctx, .., |ui| { .. })`, at the top of a dialog
    /// form. Its call up to the closure is kept verbatim in the `call` property.
    DialogCall,
    // Only with `ImportOptions::extended`: code around children, its head locked verbatim.
    /// One body of control flow: `if c {`, `else {`, `for x in y {`, `while c {`, `loop {`, `A => {`.
    Flow,
    /// Several bodies that belong together: an `if`/`else` chain, or a `match` and its arms.
    FlowGroup,
    /// `egui::MenuBar::new().ui(ui, |ui| { ui.menu_button(..); .. })`: a row of menus.
    MenuBar,
    /// `egui::Modal::new(id).show(ctx, |ui| { .. })`: a dialog over a dimmed backdrop. Like a
    /// `Window` it takes the `&Context`, so it sits at the root of a dialog or legacy form.
    Modal,
    /// Tabs built from buttons: a horizontal bar of `ui.selectable_value(&mut field, value, label)`
    /// followed by `match field { value => { page }, .. }`. Its children are one
    /// [`ContainerId::TabPage`] per tab; the tabs (value and label) are its `tabs` property.
    Tabs,
    /// The same with the bar as a column to the left of the pages:
    /// `ui.horizontal(|ui| { ui.vertical(|ui| { bar }); ui.separator(); ui.vertical(|ui| { match }) })`.
    TabsVertical,
    /// One arm of a Tabs' `match`: the page of one tab. Fixed in place like a column.
    TabPage,
    /// `egui_extras::TableBuilder::new(ui).column(..)..header(h, |mut header| { header.col(|ui| ..); })
    /// .body(|mut body| { body.row(h, |mut row| { row.col(|ui| ..); }); })`. Needs the program to
    /// depend on `egui_extras`. Its children are its rows (the header first, when it has one).
    Table,
    /// One row of a Table (the header row too): one [`ContainerId::TableCell`] per column.
    TableRow,
    /// One cell of a table row: `row.col(|ui| { .. })`.
    TableCell,
}

pub const ALL_CONTAINERS: &[ContainerId] = &[
    ContainerId::Root,
    ContainerId::Horizontal,
    ContainerId::HorizontalWrapped,
    ContainerId::Vertical,
    ContainerId::VerticalCentered,
    ContainerId::Grid,
    ContainerId::ScrollArea,
    ContainerId::CollapsingHeader,
    ContainerId::Group,
    ContainerId::Indent,
    ContainerId::PanelTop,
    ContainerId::PanelBottom,
    ContainerId::PanelLeft,
    ContainerId::PanelRight,
    ContainerId::CentralPanel,
    ContainerId::HorizontalCentered,
    ContainerId::HorizontalTop,
    ContainerId::VerticalCenteredJustified,
    ContainerId::CenteredAndJustified,
    ContainerId::Scope,
    ContainerId::WithLayout,
    ContainerId::MenuButton,
    ContainerId::AddEnabledUi,
    ContainerId::Frame,
    ContainerId::Columns,
    ContainerId::Column,
    ContainerId::Sides,
    ContainerId::Side,
    ContainerId::Window,
    ContainerId::DialogCall,
    ContainerId::Flow,
    ContainerId::FlowGroup,
    ContainerId::MenuBar,
    ContainerId::Modal,
    ContainerId::Tabs,
    ContainerId::TabsVertical,
    ContainerId::TabPage,
    ContainerId::Table,
    ContainerId::TableRow,
    ContainerId::TableCell,
];

/// A `Response` method that can head `if <widget>.<event>() { .. }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventId {
    Clicked,
    SecondaryClicked,
    MiddleClicked,
    DoubleClicked,
    TripleClicked,
    Changed,
    Hovered,
    GainedFocus,
    LostFocus,
    HasFocus,
    DragStarted,
    Dragged,
    DragStopped,
}

pub const ALL_EVENTS: &[EventId] = &[
    EventId::Clicked,
    EventId::SecondaryClicked,
    EventId::MiddleClicked,
    EventId::DoubleClicked,
    EventId::TripleClicked,
    EventId::Changed,
    EventId::Hovered,
    EventId::GainedFocus,
    EventId::LostFocus,
    EventId::HasFocus,
    EventId::DragStarted,
    EventId::Dragged,
    EventId::DragStopped,
];

impl EventId {
    /// The `egui::Response` method name.
    pub fn method(self) -> &'static str {
        match self {
            EventId::Clicked => "clicked",
            EventId::SecondaryClicked => "secondary_clicked",
            EventId::MiddleClicked => "middle_clicked",
            EventId::DoubleClicked => "double_clicked",
            EventId::TripleClicked => "triple_clicked",
            EventId::Changed => "changed",
            EventId::Hovered => "hovered",
            EventId::GainedFocus => "gained_focus",
            EventId::LostFocus => "lost_focus",
            EventId::HasFocus => "has_focus",
            EventId::DragStarted => "drag_started",
            EventId::Dragged => "dragged",
            EventId::DragStopped => "drag_stopped",
        }
    }

    pub fn from_method(name: &str) -> Option<EventId> {
        ALL_EVENTS.iter().copied().find(|e| e.method() == name)
    }
}

/// What a property's value is. Any property may also hold [`PropValue::Expr`] when the source has a
/// non-literal there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropKind {
    Bool,
    Int,
    Float,
    Text,
    Color,
    /// A Rust expression, such as `&mut self.name` or `0.0..=1.0`.
    Expr,
    /// One of `values`, written `<egui path><path>::<value>`.
    Enum {
        path: &'static str,
        values: &'static [&'static str],
    },
    /// The static items of a ComboBox: [`PropValue::Items`].
    Items,
    /// A size or spacing pair, held as [`PropValue::Expr`] (`egui::vec2(8.0, 3.0)`) and edited as two
    /// numbers when it is literal (see `layout::parse_vec2`).
    Vec2,
    /// A line: [`PropValue::Stroke`], written `egui::Stroke::new(width, colour)`.
    Stroke,
}

/// How a property appears in code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropRole {
    /// A positional argument of the call or constructor.
    Arg,
    /// `.method(value)`.
    Builder(&'static str),
    /// `.method()`, on when the value is `true`.
    Flag(&'static str),
    /// Picks the constructor: `ScrollArea::<value>()`.
    Ctor,
    /// `ui.add_sized(value, widget)`.
    AddSized,
    /// `ui.add_enabled(value, widget)`.
    AddEnabled,
    /// `ui.add_visible(value, widget)`.
    AddVisible,
    /// Filled from the body of the closure rather than the chain (ComboBox items).
    Body,
    /// A method of the `egui::RichText` the `text` argument becomes when one of these is set:
    /// `.size(v)`, `.color(v)`, or `.strong()` for a Bool, on when `true`.
    Rich(&'static str),
    /// `.method(value)` on the widget's `Response`: `.on_hover_text("..")`.
    Response(&'static str),
    /// `<ui>.method(value);`, a statement at the start of a container's body: `ui.set_min_width(120.0);`.
    UiSet(&'static str),
    /// `<ui>.spacing_mut().field = value;`, a statement at the start of a container's body.
    Spacing(&'static str),
    /// Initial focus, on when `true`: the widget's response is bound and
    /// `if <ui>.memory(|m| m.focused().is_none()) { response.request_focus(); }` follows it, so the
    /// widget takes the keyboard focus whenever nothing else has it (on the first frame too).
    InitialFocus,
}

impl PropRole {
    /// A statement at the start of a container's body rather than part of its call.
    pub fn is_body_statement(self) -> bool {
        matches!(self, PropRole::UiSet(_) | PropRole::Spacing(_))
    }

    /// `ui.add_sized`, `ui.add_enabled` or `ui.add_visible` around a widget.
    pub fn is_wrapper(self) -> bool {
        matches!(
            self,
            PropRole::AddSized | PropRole::AddEnabled | PropRole::AddVisible
        )
    }
}

/// A registry default. [`PropValue`] owns its text, so the tables use this `const` form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DefaultValue {
    /// Absent unless set. The call has no such argument or chain method by default.
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(&'static str),
    Enum(&'static str),
    Expr(&'static str),
}

impl DefaultValue {
    pub fn to_value(self) -> Option<PropValue> {
        match self {
            DefaultValue::None => None,
            DefaultValue::Bool(v) => Some(PropValue::Bool(v)),
            DefaultValue::Int(v) => Some(PropValue::Int(v)),
            DefaultValue::Float(v) => Some(PropValue::Float(v)),
            DefaultValue::Text(v) => Some(PropValue::Text(v.to_string())),
            DefaultValue::Enum(v) => Some(PropValue::Enum(v)),
            DefaultValue::Expr(v) => Some(PropValue::Expr(v.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PropDef {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: PropKind,
    pub default: DefaultValue,
    pub role: PropRole,
}

/// The struct field a new stateful widget binds to: `self.form.<field>` of type `ty`.
#[derive(Clone, Copy, Debug)]
pub struct StateDef {
    /// The property holding the `&mut self.form.<field>` expression.
    pub prop: &'static str,
    /// The field's type. A leading `egui::` is written with the form's own egui path.
    pub ty: &'static str,
    /// A second bound property of the same type (a RangeSlider's `high`), named after its default.
    pub also: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct WidgetDef {
    pub id: WidgetId,
    pub name: &'static str,
    pub label: &'static str,
    pub props: &'static [PropDef],
    pub events: &'static [EventId],
    /// The properties the inspector groups as size. The canvas resize handles write them (see
    /// `layout::resize_targets`), and Reset size clears them.
    pub size_props: &'static [&'static str],
    pub state: Option<StateDef>,
}

#[derive(Clone, Copy, Debug)]
pub struct ContainerDef {
    pub id: ContainerId,
    pub name: &'static str,
    pub label: &'static str,
    pub props: &'static [PropDef],
    pub size_props: &'static [&'static str],
    /// A root panel, recognised directly under the form's body as well as nested in other panels.
    pub is_panel: bool,
    /// Offered by the palette. The others are only ever imported.
    pub in_palette: bool,
}

impl WidgetDef {
    pub fn prop(&self, name: &str) -> Option<&'static PropDef> {
        self.props.iter().find(|p| p.name == name)
    }
}

impl ContainerDef {
    pub fn prop(&self, name: &str) -> Option<&'static PropDef> {
        self.props.iter().find(|p| p.name == name)
    }
}

const fn p(
    name: &'static str,
    label: &'static str,
    kind: PropKind,
    default: DefaultValue,
    role: PropRole,
) -> PropDef {
    PropDef {
        name,
        label,
        kind,
        default,
        role,
    }
}
const fn arg_text(name: &'static str, label: &'static str, default: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Text,
        DefaultValue::Text(default),
        PropRole::Arg,
    )
}
const fn arg_expr(name: &'static str, label: &'static str, default: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Expr,
        DefaultValue::Expr(default),
        PropRole::Arg,
    )
}
const fn b_bool(name: &'static str, label: &'static str, default: bool) -> PropDef {
    p(
        name,
        label,
        PropKind::Bool,
        DefaultValue::Bool(default),
        PropRole::Builder(name),
    )
}
const fn b_float(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Float,
        DefaultValue::None,
        PropRole::Builder(name),
    )
}
const fn b_int(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Int,
        DefaultValue::None,
        PropRole::Builder(name),
    )
}
const fn b_text(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Text,
        DefaultValue::None,
        PropRole::Builder(name),
    )
}
const fn b_expr(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Expr,
        DefaultValue::None,
        PropRole::Builder(name),
    )
}
const fn b_color(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Color,
        DefaultValue::None,
        PropRole::Builder(name),
    )
}
const fn b_vec2(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Vec2,
        DefaultValue::None,
        PropRole::Builder(name),
    )
}
const fn b_stroke(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Stroke,
        DefaultValue::None,
        PropRole::Builder(name),
    )
}
const fn flag(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Bool,
        DefaultValue::Bool(false),
        PropRole::Flag(name),
    )
}

const ADD_SIZED: PropDef = p(
    "add_sized",
    "Size",
    PropKind::Expr,
    DefaultValue::None,
    PropRole::AddSized,
);
const ENABLED: PropDef = p(
    "enabled",
    "Enabled",
    PropKind::Bool,
    DefaultValue::None,
    PropRole::AddEnabled,
);
const VISIBLE: PropDef = p(
    "visible",
    "Visible",
    PropKind::Bool,
    DefaultValue::None,
    PropRole::AddVisible,
);

/// `response.request_focus()` while nothing has the focus (see [`PropRole::InitialFocus`]).
const INITIAL_FOCUS: PropDef = p(
    "initial_focus",
    "Initial focus",
    PropKind::Bool,
    DefaultValue::Bool(false),
    PropRole::InitialFocus,
);

/// `.on_hover_text("..")` on the response.
const TOOLTIP: PropDef = p(
    "tooltip",
    "Tooltip",
    PropKind::Text,
    DefaultValue::None,
    PropRole::Response("on_hover_text"),
);

// Text styling: the `text` argument becomes `egui::RichText::new(text)` with these methods.
const FONT_SIZE: PropDef = p(
    "font_size",
    "Font size",
    PropKind::Float,
    DefaultValue::None,
    PropRole::Rich("size"),
);
const TEXT_COLOR: PropDef = p(
    "text_color",
    "Text colour",
    PropKind::Color,
    DefaultValue::None,
    PropRole::Rich("color"),
);
const fn rich_flag(name: &'static str, label: &'static str, method: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Bool,
        DefaultValue::Bool(false),
        PropRole::Rich(method),
    )
}
const BOLD: PropDef = rich_flag("bold", "Bold", "strong");
const ITALIC: PropDef = rich_flag("italic", "Italic", "italics");
const UNDERLINE: PropDef = rich_flag("underline", "Underline", "underline");
const STRIKETHROUGH: PropDef = rich_flag("strikethrough", "Strikethrough", "strikethrough");
const MONOSPACE: PropDef = rich_flag("monospace", "Monospace", "monospace");

// Statements at the start of a container's body, on its own `Ui`.
const fn ui_set(name: &'static str, label: &'static str) -> PropDef {
    p(
        name,
        label,
        PropKind::Float,
        DefaultValue::None,
        PropRole::UiSet(name),
    )
}
const SET_MIN_WIDTH: PropDef = ui_set("set_min_width", "Min width");
const SET_MIN_HEIGHT: PropDef = ui_set("set_min_height", "Min height");
const SET_MAX_WIDTH: PropDef = ui_set("set_max_width", "Max width");
const SET_MAX_HEIGHT: PropDef = ui_set("set_max_height", "Max height");
const ITEM_SPACING: PropDef = p(
    "item_spacing",
    "Item spacing",
    PropKind::Vec2,
    DefaultValue::None,
    PropRole::Spacing("item_spacing"),
);
const BUTTON_PADDING: PropDef = p(
    "button_padding",
    "Button padding",
    PropKind::Vec2,
    DefaultValue::None,
    PropRole::Spacing("button_padding"),
);
const INDENT: PropDef = p(
    "indent",
    "Indent",
    PropKind::Float,
    DefaultValue::None,
    PropRole::Spacing("indent"),
);
/// The size statements, for the inspector's size group and Reset size.
const BODY_SIZES: &[&str] = &[
    "set_min_width",
    "set_min_height",
    "set_max_width",
    "set_max_height",
];

const WRAP_MODE: PropKind = PropKind::Enum {
    path: "TextWrapMode",
    values: &["Extend", "Wrap", "Truncate"],
};
const ALIGN: PropKind = PropKind::Enum {
    path: "Align",
    values: &["Min", "Center", "Max"],
};

const CLICKS: &[EventId] = &[
    EventId::Clicked,
    EventId::SecondaryClicked,
    EventId::MiddleClicked,
    EventId::DoubleClicked,
    EventId::TripleClicked,
    EventId::Hovered,
];
const HOVER: &[EventId] = &[EventId::Hovered];
const VALUE_EVENTS: &[EventId] = &[
    EventId::Changed,
    EventId::Clicked,
    EventId::DoubleClicked,
    EventId::SecondaryClicked,
    EventId::Hovered,
    EventId::GainedFocus,
    EventId::LostFocus,
];
const DRAG_EVENTS: &[EventId] = &[
    EventId::Changed,
    EventId::DragStarted,
    EventId::Dragged,
    EventId::DragStopped,
    EventId::Hovered,
    EventId::DoubleClicked,
    EventId::SecondaryClicked,
    EventId::GainedFocus,
    EventId::LostFocus,
];
const TEXT_EVENTS: &[EventId] = &[
    EventId::Changed,
    EventId::GainedFocus,
    EventId::LostFocus,
    EventId::HasFocus,
    EventId::Hovered,
    EventId::SecondaryClicked,
];
const NO_EVENTS: &[EventId] = &[];

const TEXT_ONLY: &[PropDef] = &[
    arg_text("text", "Text", "Text"),
    FONT_SIZE,
    TEXT_COLOR,
    BOLD,
    ITALIC,
    UNDERLINE,
    STRIKETHROUGH,
    MONOSPACE,
    TOOLTIP,
    ENABLED,
    VISIBLE,
];
/// Heading, Small and Weak: sized as `ui.add_sized(s, egui::Label::new(RichText::new(t).heading()))`
/// (see `formust_rust::forms::text_style`). Strong and Monospace can't be: their `RichText` method
/// is the Label's own Bold and Monospace, so the sized form would read back as a Label.
const SIZED_TEXT: &[PropDef] = &[
    arg_text("text", "Text", "Text"),
    FONT_SIZE,
    TEXT_COLOR,
    BOLD,
    ITALIC,
    UNDERLINE,
    STRIKETHROUGH,
    MONOSPACE,
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
];

const LABEL_PROPS: &[PropDef] = &[
    arg_text("text", "Text", "Label"),
    flag("wrap", "Wrap"),
    flag("truncate", "Truncate"),
    flag("extend", "Extend"),
    p(
        "wrap_mode",
        "Wrap mode",
        WRAP_MODE,
        DefaultValue::None,
        PropRole::Builder("wrap_mode"),
    ),
    p(
        "halign",
        "Horizontal align",
        ALIGN,
        DefaultValue::None,
        PropRole::Builder("halign"),
    ),
    p(
        "selectable",
        "Selectable",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("selectable"),
    ),
    b_expr("sense", "Sense"),
    b_bool("show_tooltip_when_elided", "Tooltip when elided", true),
    FONT_SIZE,
    TEXT_COLOR,
    BOLD,
    ITALIC,
    UNDERLINE,
    STRIKETHROUGH,
    MONOSPACE,
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
];
const BUTTON_PROPS: &[PropDef] = &[
    arg_text("text", "Text", "Button"),
    flag("wrap", "Wrap"),
    flag("truncate", "Truncate"),
    flag("small", "Small"),
    p(
        "wrap_mode",
        "Wrap mode",
        WRAP_MODE,
        DefaultValue::None,
        PropRole::Builder("wrap_mode"),
    ),
    b_color("fill", "Fill"),
    b_stroke("stroke", "Stroke"),
    p(
        "frame",
        "Frame",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("frame"),
    ),
    p(
        "frame_when_inactive",
        "Frame when inactive",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("frame_when_inactive"),
    ),
    b_expr("sense", "Sense"),
    b_vec2("min_size", "Min size"),
    b_expr("corner_radius", "Corner radius"),
    b_text("shortcut_text", "Shortcut text"),
    b_text("left_text", "Left text"),
    b_text("right_text", "Right text"),
    b_bool("selected", "Selected", false),
    b_float("gap", "Gap"),
    FONT_SIZE,
    TEXT_COLOR,
    BOLD,
    ITALIC,
    UNDERLINE,
    STRIKETHROUGH,
    MONOSPACE,
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const LINK_PROPS: &[PropDef] = &[
    arg_text("text", "Text", "Link"),
    FONT_SIZE,
    TEXT_COLOR,
    BOLD,
    ITALIC,
    UNDERLINE,
    STRIKETHROUGH,
    MONOSPACE,
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const HYPERLINK_PROPS: &[PropDef] = &[
    arg_text("url", "URL", "https://www.egui.rs/"),
    p(
        "text",
        "Text",
        PropKind::Text,
        DefaultValue::None,
        PropRole::Arg,
    ),
    b_bool("open_in_new_tab", "Open in a new tab", false),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const CHECKBOX_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.checked"),
    arg_text("text", "Text", "Check"),
    b_bool("indeterminate", "Indeterminate", false),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const TOGGLE_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.toggled"),
    arg_text("text", "Text", "Toggle"),
    TOOLTIP,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const CHOICE_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.choice"),
    arg_expr("alternative", "Alternative", "0"),
    arg_text("text", "Text", "Choice"),
    TOOLTIP,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const TEXT_EDIT_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.text"),
    b_text("hint_text", "Hint text"),
    b_float("desired_width", "Desired width"),
    b_int("desired_rows", "Desired rows"),
    b_vec2("min_size", "Min size"),
    b_bool("password", "Password", false),
    b_int("char_limit", "Character limit"),
    b_bool("interactive", "Interactive", true),
    b_bool("clip_text", "Clip text", true),
    b_bool("cursor_at_end", "Cursor at end", true),
    b_bool("lock_focus", "Tab indents", false),
    flag("code_editor", "Code editor"),
    b_expr("id_salt", "Id salt"),
    b_expr("font", "Font"),
    b_expr("frame", "Frame"),
    b_expr("margin", "Margin"),
    b_color("text_color", "Text colour"),
    b_color("background_color", "Background colour"),
    b_text("prefix", "Prefix"),
    b_text("suffix", "Suffix"),
    b_expr("return_key", "Return key"),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const SLIDER_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.value"),
    arg_expr("range", "Range", "0.0..=1.0"),
    b_text("text", "Text"),
    b_color("text_color", "Text colour"),
    flag("integer", "Integer"),
    flag("vertical", "Vertical"),
    p(
        "orientation",
        "Orientation",
        PropKind::Enum {
            path: "SliderOrientation",
            values: &["Horizontal", "Vertical"],
        },
        DefaultValue::None,
        PropRole::Builder("orientation"),
    ),
    p(
        "clamping",
        "Clamping",
        PropKind::Enum {
            path: "SliderClamping",
            values: &["Never", "Edits", "Always"],
        },
        DefaultValue::None,
        PropRole::Builder("clamping"),
    ),
    b_bool("logarithmic", "Logarithmic", false),
    b_bool("show_value", "Show value", true),
    b_bool("smart_aim", "Smart aim", true),
    p(
        "trailing_fill",
        "Trailing fill",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("trailing_fill"),
    ),
    b_float("step_by", "Step"),
    b_float("drag_value_speed", "Drag speed"),
    b_text("prefix", "Prefix"),
    b_text("suffix", "Suffix"),
    b_int("fixed_decimals", "Fixed decimals"),
    b_int("min_decimals", "Min decimals"),
    b_int("max_decimals", "Max decimals"),
    b_expr("handle_shape", "Handle shape"),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const DRAG_VALUE_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.value"),
    b_float("speed", "Speed"),
    b_expr("range", "Range"),
    b_text("prefix", "Prefix"),
    b_text("suffix", "Suffix"),
    b_int("fixed_decimals", "Fixed decimals"),
    b_int("min_decimals", "Min decimals"),
    b_int("max_decimals", "Max decimals"),
    b_vec2("min_size", "Min size"),
    b_bool("clamp_existing_to_range", "Clamp existing", true),
    b_bool("update_while_editing", "Update while editing", true),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
    INITIAL_FOCUS,
];
const PROGRESS_PROPS: &[PropDef] = &[
    p(
        "progress",
        "Progress",
        PropKind::Float,
        DefaultValue::Float(0.5),
        PropRole::Arg,
    ),
    b_text("text", "Text"),
    flag("show_percentage", "Show percentage"),
    b_bool("animate", "Animate", false),
    b_float("desired_width", "Desired width"),
    b_float("desired_height", "Desired height"),
    b_color("fill", "Fill"),
    b_expr("corner_radius", "Corner radius"),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
];
const SPINNER_PROPS: &[PropDef] = &[
    b_float("size", "Size"),
    b_color("color", "Colour"),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
];
const SEPARATOR_PROPS: &[PropDef] = &[
    b_float("spacing", "Spacing"),
    flag("horizontal", "Horizontal"),
    flag("vertical", "Vertical"),
    b_float("grow", "Grow"),
    b_float("shrink", "Shrink"),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
];
const ADD_SPACE_PROPS: &[PropDef] = &[p(
    "amount",
    "Amount",
    PropKind::Float,
    DefaultValue::Float(8.0),
    PropRole::Arg,
)];
const COMBO_PROPS: &[PropDef] = &[
    p(
        "id_salt",
        "Id salt",
        PropKind::Text,
        DefaultValue::None,
        PropRole::Arg,
    ),
    p(
        "label",
        "Label",
        PropKind::Text,
        DefaultValue::None,
        PropRole::Arg,
    ),
    b_expr("selected_text", "Selected text"),
    b_float("width", "Width"),
    b_float("height", "Height"),
    flag("wrap", "Wrap"),
    flag("truncate", "Truncate"),
    p(
        "wrap_mode",
        "Wrap mode",
        WRAP_MODE,
        DefaultValue::None,
        PropRole::Builder("wrap_mode"),
    ),
    p(
        "value",
        "Value",
        PropKind::Expr,
        DefaultValue::Expr("&mut self.form.choice"),
        PropRole::Body,
    ),
    p(
        "items",
        "Items",
        PropKind::Items,
        DefaultValue::None,
        PropRole::Body,
    ),
    // No tooltip: `show_ui` returns the popup's result, not the box's response.
    ENABLED,
    VISIBLE,
];

/// `ui.image(source)` / `egui::Image::new(source)`: a `"file://.."` or `"https://.."` text, or an
/// expression such as `egui::include_image!("../assets/logo.png")`. Loading needs image loaders in
/// the program (`egui_extras::install_image_loaders`), which Formust never adds.
const IMAGE_PROPS: &[PropDef] = &[
    arg_text("source", "Source", "file://assets/image.png"),
    p(
        "uri",
        "URI",
        PropKind::Text,
        DefaultValue::None,
        PropRole::Arg,
    ),
    b_vec2("fit_to_exact_size", "Exact size"),
    b_float("fit_to_original_size", "Original size scale"),
    b_float("max_width", "Max width"),
    b_float("max_height", "Max height"),
    b_vec2("max_size", "Max size"),
    b_bool("maintain_aspect_ratio", "Keep aspect ratio", true),
    flag("shrink_to_fit", "Shrink to fit"),
    b_float("corner_radius", "Corner radius"),
    b_color("tint", "Tint"),
    b_color("bg_fill", "Background"),
    b_bool("show_loading_spinner", "Loading spinner", true),
    b_text("alt_text", "Alt text"),
    b_expr("sense", "Sense"),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
];
const COLOR_EDIT_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.color"),
    TOOLTIP,
    ENABLED,
    VISIBLE,
];
const COLOR_EDIT_RGB_PROPS: &[PropDef] = &[
    arg_expr("value", "Value", "&mut self.form.rgb"),
    TOOLTIP,
    ENABLED,
    VISIBLE,
];
/// `egui::RangeSlider::new(&mut low, &mut high, range)` (egui 0.37), with the Slider's chain.
const RANGE_SLIDER_PROPS: &[PropDef] = &[
    arg_expr("low", "Low", "&mut self.form.low"),
    arg_expr("high", "High", "&mut self.form.high"),
    arg_expr("range", "Range", "0.0..=1.0"),
    b_text("text", "Text"),
    b_color("text_color", "Text colour"),
    b_float("min_separation", "Min separation"),
    flag("integer", "Integer"),
    flag("vertical", "Vertical"),
    p(
        "orientation",
        "Orientation",
        PropKind::Enum {
            path: "SliderOrientation",
            values: &["Horizontal", "Vertical"],
        },
        DefaultValue::None,
        PropRole::Builder("orientation"),
    ),
    p(
        "clamping",
        "Clamping",
        PropKind::Enum {
            path: "SliderClamping",
            values: &["Never", "Edits", "Always"],
        },
        DefaultValue::None,
        PropRole::Builder("clamping"),
    ),
    b_bool("logarithmic", "Logarithmic", false),
    b_bool("show_value", "Show value", true),
    b_bool("smart_aim", "Smart aim", true),
    b_float("step_by", "Step"),
    b_float("drag_value_speed", "Drag speed"),
    b_text("prefix", "Prefix"),
    b_text("suffix", "Suffix"),
    b_int("fixed_decimals", "Fixed decimals"),
    b_int("min_decimals", "Min decimals"),
    b_int("max_decimals", "Max decimals"),
    b_expr("handle_shape", "Handle shape"),
    TOOLTIP,
    ADD_SIZED,
    ENABLED,
    VISIBLE,
];

const WIDGET_DEFS: &[WidgetDef] = &[
    WidgetDef {
        id: WidgetId::Label,
        name: "label",
        label: "Label",
        props: LABEL_PROPS,
        events: CLICKS,
        size_props: &["add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Heading,
        name: "heading",
        label: "Heading",
        props: SIZED_TEXT,
        events: CLICKS,
        size_props: &["add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Small,
        name: "small",
        label: "Small text",
        props: SIZED_TEXT,
        events: CLICKS,
        size_props: &["add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Strong,
        name: "strong",
        label: "Strong text",
        props: TEXT_ONLY,
        events: CLICKS,
        size_props: &[],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Weak,
        name: "weak",
        label: "Weak text",
        props: SIZED_TEXT,
        events: CLICKS,
        size_props: &["add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Monospace,
        name: "monospace",
        label: "Monospace text",
        props: TEXT_ONLY,
        events: CLICKS,
        size_props: &[],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Button,
        name: "button",
        label: "Button",
        props: BUTTON_PROPS,
        events: CLICKS,
        size_props: &["min_size", "add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Link,
        name: "link",
        label: "Link",
        props: LINK_PROPS,
        events: CLICKS,
        size_props: &["add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Hyperlink,
        name: "hyperlink",
        label: "Hyperlink",
        props: HYPERLINK_PROPS,
        events: CLICKS,
        size_props: &["add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Checkbox,
        name: "checkbox",
        label: "Checkbox",
        props: CHECKBOX_PROPS,
        events: VALUE_EVENTS,
        size_props: &["add_sized"],
        state: Some(StateDef {
            prop: "value",
            ty: "bool",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::ToggleValue,
        name: "toggle_value",
        label: "Toggle",
        props: TOGGLE_PROPS,
        events: VALUE_EVENTS,
        size_props: &[],
        state: Some(StateDef {
            prop: "value",
            ty: "bool",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::RadioValue,
        name: "radio_value",
        label: "Radio button",
        props: CHOICE_PROPS,
        events: VALUE_EVENTS,
        size_props: &[],
        state: Some(StateDef {
            prop: "value",
            ty: "usize",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::SelectableValue,
        name: "selectable_value",
        label: "Selectable value",
        props: CHOICE_PROPS,
        events: VALUE_EVENTS,
        size_props: &[],
        state: Some(StateDef {
            prop: "value",
            ty: "usize",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::TextEditSingleline,
        name: "text_edit_singleline",
        label: "Text edit",
        props: TEXT_EDIT_PROPS,
        events: TEXT_EVENTS,
        size_props: &["desired_width", "min_size", "add_sized"],
        state: Some(StateDef {
            prop: "value",
            ty: "String",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::TextEditMultiline,
        name: "text_edit_multiline",
        label: "Multiline text edit",
        props: TEXT_EDIT_PROPS,
        events: TEXT_EVENTS,
        size_props: &["desired_width", "desired_rows", "min_size", "add_sized"],
        state: Some(StateDef {
            prop: "value",
            ty: "String",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::Slider,
        name: "slider",
        label: "Slider",
        props: SLIDER_PROPS,
        events: DRAG_EVENTS,
        size_props: &["add_sized"],
        state: Some(StateDef {
            prop: "value",
            ty: "f32",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::DragValue,
        name: "drag_value",
        label: "Drag value",
        props: DRAG_VALUE_PROPS,
        events: DRAG_EVENTS,
        size_props: &["min_size", "add_sized"],
        state: Some(StateDef {
            prop: "value",
            ty: "f32",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::ProgressBar,
        name: "progress_bar",
        label: "Progress bar",
        props: PROGRESS_PROPS,
        events: HOVER,
        size_props: &["desired_width", "desired_height", "add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Spinner,
        name: "spinner",
        label: "Spinner",
        props: SPINNER_PROPS,
        events: HOVER,
        size_props: &["size", "add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Separator,
        name: "separator",
        label: "Separator",
        props: SEPARATOR_PROPS,
        events: HOVER,
        size_props: &["add_sized"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::AddSpace,
        name: "add_space",
        label: "Space",
        props: ADD_SPACE_PROPS,
        events: NO_EVENTS,
        size_props: &["amount"],
        state: None,
    },
    WidgetDef {
        id: WidgetId::ComboBox,
        name: "combo_box",
        label: "Combo box",
        props: COMBO_PROPS,
        events: NO_EVENTS,
        size_props: &["width", "height"],
        state: Some(StateDef {
            prop: "value",
            ty: "usize",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::EndRow,
        name: "end_row",
        label: "End row",
        props: &[],
        events: NO_EVENTS,
        size_props: &[],
        state: None,
    },
    WidgetDef {
        id: WidgetId::Image,
        name: "image",
        label: "Image",
        props: IMAGE_PROPS,
        events: CLICKS,
        size_props: &[
            "fit_to_exact_size",
            "fit_to_original_size",
            "max_width",
            "max_height",
            "max_size",
            "add_sized",
        ],
        state: None,
    },
    WidgetDef {
        id: WidgetId::ColorEdit,
        name: "color_edit_button_srgba",
        label: "Colour picker",
        props: COLOR_EDIT_PROPS,
        events: VALUE_EVENTS,
        size_props: &[],
        state: Some(StateDef {
            prop: "value",
            ty: "egui::Color32",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::ColorEditRgb,
        name: "color_edit_button_rgb",
        label: "Colour picker (RGB)",
        props: COLOR_EDIT_RGB_PROPS,
        events: VALUE_EVENTS,
        size_props: &[],
        state: Some(StateDef {
            prop: "value",
            ty: "[f32; 3]",
            also: None,
        }),
    },
    WidgetDef {
        id: WidgetId::RangeSlider,
        name: "range_slider",
        label: "Range slider",
        props: RANGE_SLIDER_PROPS,
        events: DRAG_EVENTS,
        size_props: &["add_sized"],
        state: Some(StateDef {
            prop: "low",
            ty: "f32",
            also: Some("high"),
        }),
    },
];

/// The registry entry of a widget.
pub fn widget_def(id: WidgetId) -> &'static WidgetDef {
    let index = match id {
        WidgetId::Label => 0,
        WidgetId::Heading => 1,
        WidgetId::Small => 2,
        WidgetId::Strong => 3,
        WidgetId::Weak => 4,
        WidgetId::Monospace => 5,
        WidgetId::Button => 6,
        WidgetId::Link => 7,
        WidgetId::Hyperlink => 8,
        WidgetId::Checkbox => 9,
        WidgetId::ToggleValue => 10,
        WidgetId::RadioValue => 11,
        WidgetId::SelectableValue => 12,
        WidgetId::TextEditSingleline => 13,
        WidgetId::TextEditMultiline => 14,
        WidgetId::Slider => 15,
        WidgetId::DragValue => 16,
        WidgetId::ProgressBar => 17,
        WidgetId::Spinner => 18,
        WidgetId::Separator => 19,
        WidgetId::AddSpace => 20,
        WidgetId::ComboBox => 21,
        WidgetId::EndRow => 22,
        WidgetId::Image => 23,
        WidgetId::ColorEdit => 24,
        WidgetId::ColorEditRgb => 25,
        WidgetId::RangeSlider => 26,
    };
    &WIDGET_DEFS[index]
}

const LAYOUT_PROPS: &[PropDef] = &[];
/// A layout container: only the statements at the start of its body.
const BODY_PROPS: &[PropDef] = &[
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const GRID_PROPS: &[PropDef] = &[
    arg_text("id", "Id", "grid"),
    b_int("num_columns", "Columns"),
    b_bool("striped", "Striped", false),
    b_float("min_col_width", "Min column width"),
    b_float("max_col_width", "Max column width"),
    b_float("min_row_height", "Min row height"),
    b_expr("spacing", "Spacing"),
    b_int("start_row", "Start row"),
];
const SCROLL_PROPS: &[PropDef] = &[
    p(
        "direction",
        "Direction",
        PropKind::Enum {
            path: "",
            values: &["vertical", "horizontal", "both", "neither"],
        },
        DefaultValue::Enum("vertical"),
        PropRole::Ctor,
    ),
    b_float("max_width", "Max width"),
    b_float("max_height", "Max height"),
    b_float("min_scrolled_width", "Min scrolled width"),
    b_float("min_scrolled_height", "Min scrolled height"),
    b_expr("auto_shrink", "Auto shrink"),
    b_expr("id_salt", "Id salt"),
    b_bool("stick_to_right", "Stick to right", false),
    b_bool("stick_to_bottom", "Stick to bottom", false),
    b_bool("animated", "Animated", true),
    b_expr("scroll_bar_visibility", "Scroll bar"),
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const COLLAPSING_PROPS: &[PropDef] = &[
    arg_text("text", "Text", "Section"),
    b_bool("default_open", "Open by default", false),
    b_expr("open", "Open"),
    b_expr("id_salt", "Id salt"),
    b_bool("enabled", "Enabled", true),
    b_bool("show_background", "Show background", false),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const INDENT_PROPS: &[PropDef] = &[
    arg_text("id", "Id", "indent"),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const PANEL_PROPS: &[PropDef] = &[
    arg_text("id", "Id", "panel"),
    b_bool("resizable", "Resizable", true),
    b_bool("show_separator_line", "Separator line", true),
    b_float("default_size", "Default size"),
    b_float("min_size", "Min size"),
    b_float("max_size", "Max size"),
    b_float("exact_size", "Exact size"),
    b_expr("size_range", "Size range"),
    b_expr("frame", "Frame"),
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const CENTRAL_PROPS: &[PropDef] = &[
    p(
        "variant",
        "Variant",
        PropKind::Enum {
            path: "",
            values: &["default", "no_frame", "default_margins"],
        },
        DefaultValue::Enum("default"),
        PropRole::Ctor,
    ),
    b_expr("frame", "Frame"),
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const PANEL_SIZES: &[&str] = &[
    "default_size",
    "min_size",
    "max_size",
    "exact_size",
    "size_range",
];

const WITH_LAYOUT_PROPS: &[PropDef] = &[
    arg_expr(
        "layout",
        "Layout",
        "egui::Layout::top_down(egui::Align::Min)",
    ),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const MENU_BUTTON_PROPS: &[PropDef] = &[
    arg_text("text", "Text", "Menu"),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const ADD_ENABLED_UI_PROPS: &[PropDef] = &[
    p(
        "enabled",
        "Enabled",
        PropKind::Bool,
        DefaultValue::Bool(true),
        PropRole::Arg,
    ),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const FRAME_PROPS: &[PropDef] = &[
    p(
        "variant",
        "Variant",
        PropKind::Enum {
            path: "",
            values: &[
                "new",
                "default",
                "group",
                "side_top_panel",
                "central_panel",
                "window",
                "menu",
                "popup",
                "canvas",
                "dark_canvas",
            ],
        },
        DefaultValue::Enum("new"),
        PropRole::Ctor,
    ),
    p(
        "style",
        "Style",
        PropKind::Expr,
        DefaultValue::None,
        PropRole::Arg,
    ),
    // `impl Into<Margin>` and `impl Into<CornerRadius>` take an `f32` in egui 0.37; anything else
    // (`egui::Margin::symmetric(8, 4)`) stays an expression.
    // In the order `egui::Frame::new().fill(..).stroke(..).corner_radius(..).inner_margin(..)`.
    b_color("fill", "Fill"),
    b_stroke("stroke", "Stroke"),
    b_float("corner_radius", "Corner radius"),
    b_float("inner_margin", "Inner margin"),
    b_float("outer_margin", "Outer margin"),
    b_expr("shadow", "Shadow"),
    b_float("multiply_with_opacity", "Opacity"),
    flag("invisible", "Invisible"),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
/// `ui.columns(count, ..)`: the count is a literal, so the designer knows the columns.
const COLUMNS_PROPS: &[PropDef] = &[p(
    "count",
    "Columns",
    PropKind::Int,
    DefaultValue::Int(2),
    PropRole::Arg,
)];
/// `egui::Sides::new()` and its builder methods, in egui 0.37's order.
const SIDES_PROPS: &[PropDef] = &[
    b_float("height", "Height"),
    b_float("spacing", "Spacing"),
    flag("shrink_left", "Shrink left"),
    flag("shrink_right", "Shrink right"),
    flag("extend", "Extend"),
    p(
        "wrap_mode",
        "Wrap mode",
        WRAP_MODE,
        DefaultValue::None,
        PropRole::Builder("wrap_mode"),
    ),
    flag("truncate", "Truncate"),
    flag("wrap", "Wrap"),
];
/// The helper call before its closure, verbatim: `crate::style::dialog(ctx, "about", None, 540.0, `.
const DIALOG_CALL_PROPS: &[PropDef] = &[
    arg_expr("call", "Call", "dialog(ctx, "),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
const WINDOW_PROPS: &[PropDef] = &[
    arg_text("title", "Title", "Window"),
    b_expr("open", "Open"),
    p(
        "enabled",
        "Enabled",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("enabled"),
    ),
    p(
        "movable",
        "Movable",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("movable"),
    ),
    p(
        "collapsible",
        "Collapsible",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("collapsible"),
    ),
    p(
        "title_bar",
        "Title bar",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("title_bar"),
    ),
    b_expr("resizable", "Resizable"),
    p(
        "default_open",
        "Open by default",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("default_open"),
    ),
    p(
        "vscroll",
        "Vertical scroll",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("vscroll"),
    ),
    p(
        "hscroll",
        "Horizontal scroll",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("hscroll"),
    ),
    b_float("default_width", "Default width"),
    b_float("default_height", "Default height"),
    b_float("min_width", "Min width"),
    b_float("min_height", "Min height"),
    b_float("max_width", "Max width"),
    b_float("max_height", "Max height"),
    b_expr("default_size", "Default size"),
    b_expr("min_size", "Min size"),
    b_expr("max_size", "Max size"),
    b_expr("fixed_size", "Fixed size"),
    b_expr("default_pos", "Default position"),
    // `.anchor(egui::Align2::RIGHT_TOP, [dx, dy])`: both arguments, as written.
    b_expr("anchor", "Anchor"),
    b_expr("fixed_pos", "Fixed position"),
    p(
        "pivot",
        "Pivot",
        ALIGN2_KIND,
        DefaultValue::None,
        PropRole::Builder("pivot"),
    ),
    p(
        "order",
        "Order",
        PropKind::Enum {
            path: "Order",
            values: &["Background", "Middle", "Foreground", "Tooltip", "Debug"],
        },
        DefaultValue::None,
        PropRole::Builder("order"),
    ),
    p(
        "constrain",
        "Keep on screen",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("constrain"),
    ),
    p(
        "interactable",
        "Interactable",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("interactable"),
    ),
    flag("auto_sized", "Auto sized"),
    p(
        "fade_in",
        "Fade in",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("fade_in"),
    ),
    p(
        "fade_out",
        "Fade out",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("fade_out"),
    ),
    b_expr("frame", "Frame"),
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
/// `egui::Align2::<value>`: a corner, a side's middle or the centre.
const ALIGN2_KIND: PropKind = PropKind::Enum {
    path: "Align2",
    values: &[
        "LEFT_TOP",
        "CENTER_TOP",
        "RIGHT_TOP",
        "LEFT_CENTER",
        "CENTER_CENTER",
        "RIGHT_CENTER",
        "LEFT_BOTTOM",
        "CENTER_BOTTOM",
        "RIGHT_BOTTOM",
    ],
};
/// `egui::MenuBar::new().ui(ui, ..)`: spacing statements in its body.
const MENU_BAR_PROPS: &[PropDef] = &[ITEM_SPACING, BUTTON_PADDING, INDENT];
/// `egui::Modal::new(id)`: the id is an `egui::Id` expression.
const MODAL_PROPS: &[PropDef] = &[
    arg_expr("id", "Id", "egui::Id::unique(\"modal\")"),
    b_color("backdrop_color", "Backdrop"),
    b_expr("frame", "Frame"),
    b_expr("area", "Area"),
    SET_MIN_WIDTH,
    SET_MIN_HEIGHT,
    SET_MAX_WIDTH,
    SET_MAX_HEIGHT,
    ITEM_SPACING,
    BUTTON_PADDING,
    INDENT,
];
/// Tabs: the selected-tab field (`self.form.tab`, compared with each tab's value), the tabs as a
/// list of value and label, and how the bar's buttons look. Written by hand (see `emit.rs`).
const TABS_PROPS: &[PropDef] = &[
    arg_expr("field", "Selected tab", "self.form.tab"),
    p(
        "tabs",
        "Tabs",
        PropKind::Items,
        DefaultValue::None,
        PropRole::Body,
    ),
    p(
        "style",
        "Style",
        PropKind::Enum {
            path: "",
            values: &["selectable", "button", "frameless"],
        },
        DefaultValue::Enum("selectable"),
        PropRole::Arg,
    ),
];
/// `egui_extras::TableBuilder`: its builder methods, then its columns (`.column(<value>)` each, a
/// list of [`PropValue::Items`] whose values are `egui_extras::Column` expressions), the header's
/// height when it has a header, and the height of every body row. Written by hand (`emit.rs`).
const TABLE_PROPS: &[PropDef] = &[
    b_expr("id_salt", "Id salt"),
    p(
        "striped",
        "Striped",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("striped"),
    ),
    p(
        "resizable",
        "Resizable columns",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("resizable"),
    ),
    p(
        "vscroll",
        "Vertical scroll",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("vscroll"),
    ),
    p(
        "stick_to_bottom",
        "Stick to bottom",
        PropKind::Bool,
        DefaultValue::None,
        PropRole::Builder("stick_to_bottom"),
    ),
    b_float("min_scrolled_height", "Min scrolled height"),
    b_float("max_scroll_height", "Max scroll height"),
    b_expr("auto_shrink", "Auto shrink"),
    b_expr("cell_layout", "Cell layout"),
    b_expr("sense", "Sense"),
    p(
        "columns",
        "Columns",
        PropKind::Items,
        DefaultValue::None,
        PropRole::Body,
    ),
    p(
        "header",
        "Header height",
        PropKind::Float,
        DefaultValue::None,
        PropRole::Body,
    ),
    p(
        "row_height",
        "Row height",
        PropKind::Float,
        DefaultValue::Float(18.0),
        PropRole::Arg,
    ),
];
const WINDOW_SIZES: &[&str] = &[
    "default_width",
    "default_height",
    "min_width",
    "min_height",
    "max_width",
    "max_height",
    "default_size",
    "min_size",
    "max_size",
    "fixed_size",
];

const CONTAINER_DEFS: &[ContainerDef] = &[
    ContainerDef {
        id: ContainerId::Root,
        name: "root",
        label: "Form",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Horizontal,
        name: "horizontal",
        label: "Horizontal",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::HorizontalWrapped,
        name: "horizontal_wrapped",
        label: "Horizontal (wrapped)",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Vertical,
        name: "vertical",
        label: "Vertical",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::VerticalCentered,
        name: "vertical_centered",
        label: "Vertical (centred)",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Grid,
        name: "grid",
        label: "Grid",
        props: GRID_PROPS,
        size_props: &["min_col_width", "max_col_width", "min_row_height"],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::ScrollArea,
        name: "scroll_area",
        label: "Scroll area",
        props: SCROLL_PROPS,
        size_props: &[
            "max_width",
            "max_height",
            "min_scrolled_width",
            "min_scrolled_height",
        ],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::CollapsingHeader,
        name: "collapsing_header",
        label: "Collapsing header",
        props: COLLAPSING_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Group,
        name: "group",
        label: "Group",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Indent,
        name: "indent",
        label: "Indent",
        props: INDENT_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::PanelTop,
        name: "panel_top",
        label: "Top panel",
        props: PANEL_PROPS,
        size_props: PANEL_SIZES,
        is_panel: true,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::PanelBottom,
        name: "panel_bottom",
        label: "Bottom panel",
        props: PANEL_PROPS,
        size_props: PANEL_SIZES,
        is_panel: true,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::PanelLeft,
        name: "panel_left",
        label: "Left panel",
        props: PANEL_PROPS,
        size_props: PANEL_SIZES,
        is_panel: true,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::PanelRight,
        name: "panel_right",
        label: "Right panel",
        props: PANEL_PROPS,
        size_props: PANEL_SIZES,
        is_panel: true,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::CentralPanel,
        name: "central_panel",
        label: "Central panel",
        props: CENTRAL_PROPS,
        size_props: &[],
        is_panel: true,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::HorizontalCentered,
        name: "horizontal_centered",
        label: "Horizontal (centred)",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::HorizontalTop,
        name: "horizontal_top",
        label: "Horizontal (top)",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::VerticalCenteredJustified,
        name: "vertical_centered_justified",
        label: "Vertical (centred, justified)",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::CenteredAndJustified,
        name: "centered_and_justified",
        label: "Centred and justified",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::Scope,
        name: "scope",
        label: "Scope",
        props: BODY_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::WithLayout,
        name: "with_layout",
        label: "With layout",
        props: WITH_LAYOUT_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::MenuButton,
        name: "menu_button",
        label: "Menu",
        props: MENU_BUTTON_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::AddEnabledUi,
        name: "add_enabled_ui",
        label: "Enabled section",
        props: ADD_ENABLED_UI_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::Frame,
        name: "frame",
        label: "Frame",
        props: FRAME_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Columns,
        name: "columns",
        label: "Columns",
        props: COLUMNS_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Column,
        name: "column",
        label: "Column",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::Sides,
        name: "sides",
        label: "Sides",
        props: SIDES_PROPS,
        size_props: &["height"],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Side,
        name: "side",
        label: "Side",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::Window,
        name: "window",
        label: "Window",
        props: WINDOW_PROPS,
        size_props: WINDOW_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::DialogCall,
        name: "dialog_call",
        label: "Dialog",
        props: DIALOG_CALL_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::Flow,
        name: "flow",
        label: "Control flow",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::FlowGroup,
        name: "flow_group",
        label: "Branches",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::MenuBar,
        name: "menu_bar",
        label: "Menu bar",
        props: MENU_BAR_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Modal,
        name: "modal",
        label: "Modal",
        props: MODAL_PROPS,
        size_props: BODY_SIZES,
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::Tabs,
        name: "tabs",
        label: "Tabs (horizontal)",
        props: TABS_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::TabsVertical,
        name: "tabs_vertical",
        label: "Tabs (vertical)",
        props: TABS_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::TabPage,
        name: "tab_page",
        label: "Page",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::Table,
        name: "table",
        label: "Table",
        props: TABLE_PROPS,
        size_props: &["min_scrolled_height", "max_scroll_height"],
        is_panel: false,
        in_palette: true,
    },
    ContainerDef {
        id: ContainerId::TableRow,
        name: "table_row",
        label: "Row",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: false,
    },
    ContainerDef {
        id: ContainerId::TableCell,
        name: "table_cell",
        label: "Cell",
        props: LAYOUT_PROPS,
        size_props: &[],
        is_panel: false,
        in_palette: false,
    },
];

/// The registry entry of a container.
pub fn container_def(id: ContainerId) -> &'static ContainerDef {
    let index = match id {
        ContainerId::Root => 0,
        ContainerId::Horizontal => 1,
        ContainerId::HorizontalWrapped => 2,
        ContainerId::Vertical => 3,
        ContainerId::VerticalCentered => 4,
        ContainerId::Grid => 5,
        ContainerId::ScrollArea => 6,
        ContainerId::CollapsingHeader => 7,
        ContainerId::Group => 8,
        ContainerId::Indent => 9,
        ContainerId::PanelTop => 10,
        ContainerId::PanelBottom => 11,
        ContainerId::PanelLeft => 12,
        ContainerId::PanelRight => 13,
        ContainerId::CentralPanel => 14,
        ContainerId::HorizontalCentered => 15,
        ContainerId::HorizontalTop => 16,
        ContainerId::VerticalCenteredJustified => 17,
        ContainerId::CenteredAndJustified => 18,
        ContainerId::Scope => 19,
        ContainerId::WithLayout => 20,
        ContainerId::MenuButton => 21,
        ContainerId::AddEnabledUi => 22,
        ContainerId::Frame => 23,
        ContainerId::Columns => 24,
        ContainerId::Column => 25,
        ContainerId::Sides => 26,
        ContainerId::Side => 27,
        ContainerId::Window => 28,
        ContainerId::DialogCall => 29,
        ContainerId::Flow => 30,
        ContainerId::FlowGroup => 31,
        ContainerId::MenuBar => 32,
        ContainerId::Modal => 33,
        ContainerId::Tabs => 34,
        ContainerId::TabsVertical => 35,
        ContainerId::TabPage => 36,
        ContainerId::Table => 37,
        ContainerId::TableRow => 38,
        ContainerId::TableCell => 39,
    };
    &CONTAINER_DEFS[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `widget_def`'s `match` catches a new id with no entry; this catches one left out of the lists.
    #[test]
    fn the_lists_cover_the_tables() {
        assert_eq!(ALL_WIDGETS.len(), WIDGET_DEFS.len());
        assert_eq!(ALL_CONTAINERS.len(), CONTAINER_DEFS.len());
    }

    #[test]
    fn the_first_palette_of_the_spec_is_offered() {
        let names: Vec<&str> = ALL_WIDGETS.iter().map(|&w| widget_def(w).name).collect();
        for name in [
            "label",
            "heading",
            "small",
            "strong",
            "weak",
            "monospace",
            "button",
            "link",
            "hyperlink",
            "checkbox",
            "toggle_value",
            "radio_value",
            "selectable_value",
            "text_edit_singleline",
            "text_edit_multiline",
            "slider",
            "drag_value",
            "progress_bar",
            "spinner",
            "separator",
            "add_space",
            "combo_box",
            "end_row",
            "image",
            "color_edit_button_srgba",
            "color_edit_button_rgb",
            "range_slider",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        let offered: Vec<&str> = ALL_CONTAINERS
            .iter()
            .filter(|&&c| container_def(c).in_palette)
            .map(|&c| container_def(c).name)
            .collect();
        assert_eq!(
            offered,
            [
                "root",
                "horizontal",
                "horizontal_wrapped",
                "vertical",
                "vertical_centered",
                "grid",
                "scroll_area",
                "collapsing_header",
                "group",
                "indent",
                "panel_top",
                "panel_bottom",
                "panel_left",
                "panel_right",
                "central_panel",
                "menu_button",
                "frame",
                "columns",
                "sides",
                "menu_bar",
                "modal",
                "tabs",
                "tabs_vertical",
                "table",
            ]
        );
    }

    /// Text styling needs a `text` argument to style; the response and wrapper roles belong to
    /// widgets, the body statements to containers, and a container's size statements are in its
    /// size group (Reset size clears them).
    #[test]
    fn the_new_roles_sit_where_they_can_be_written() {
        for &id in ALL_WIDGETS {
            let def = widget_def(id);
            if def
                .props
                .iter()
                .any(|p| matches!(p.role, PropRole::Rich(_)))
            {
                assert!(
                    def.prop("text").is_some_and(|t| t.role == PropRole::Arg),
                    "{}",
                    def.name
                );
            }
            for prop in def.props {
                assert!(!prop.role.is_body_statement(), "{}.{}", def.name, prop.name);
            }
        }
        for &id in ALL_CONTAINERS {
            let def = container_def(id);
            for prop in def.props {
                assert!(
                    !matches!(prop.role, PropRole::Rich(_) | PropRole::Response(_))
                        && !prop.role.is_wrapper(),
                    "{}.{}",
                    def.name,
                    prop.name
                );
                if matches!(prop.role, PropRole::UiSet(_)) {
                    assert!(
                        def.size_props.contains(&prop.name),
                        "{}.{}",
                        def.name,
                        prop.name
                    );
                }
            }
        }
        // Tooltip, enabled and visible on every widget with a response, but a combo box's
        // `show_ui` gives the popup's result, so it has no tooltip.
        for &id in ALL_WIDGETS {
            let def = widget_def(id);
            let has = |name: &str| def.prop(name).is_some();
            match id {
                WidgetId::AddSpace | WidgetId::EndRow => {
                    assert!(!has("tooltip") && !has("enabled") && !has("visible"))
                }
                WidgetId::ComboBox => assert!(!has("tooltip") && has("enabled") && has("visible")),
                _ => assert!(
                    has("tooltip") && has("enabled") && has("visible"),
                    "{}",
                    def.name
                ),
            }
        }
    }

    #[test]
    fn every_event_method_reads_back() {
        for &event in ALL_EVENTS {
            assert_eq!(EventId::from_method(event.method()), Some(event));
        }
        assert_eq!(EventId::from_method("nope"), None);
    }
}
