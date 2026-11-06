//! Formust's form model and widget registry. A form is a tree of egui containers; there is no
//! absolute positioning. This crate has no UI and no parser dependencies: `formust-rust` fills the
//! model from source text and writes it back.

mod form;
mod layout;
mod registry;
mod tables;
mod tabs;

pub use form::{
    ComboItem, ContainerShell, EditError, Form, Node, NodeId, NodeKind, PropValue, Props,
    StateField,
};
pub use layout::{
    ALIGN2, ALL_ANCHORS, Anchor, FillWay, MAX_COLUMNS, SizeRequest, SizeTarget,
    available_width_prop, fill_way, is_fill, is_fill_size, is_fill_text, is_fill_value,
    is_focusable, is_plain_ui, is_ui_text, literal_range, num, number, panel_size_range,
    parse_vec2, parse_window_anchor, resize_axes, resize_body_inset, resize_from_start,
    resize_targets, split_pair, vec2_text, window_anchor_text, wrapper_anchor,
};
pub use registry::{
    ALL_CONTAINERS, ALL_EVENTS, ALL_WIDGETS, ContainerDef, ContainerId, DefaultValue, EventId,
    PropDef, PropKind, PropRole, StateDef, WidgetDef, WidgetId, container_def, widget_def,
};
pub use tables::{MAX_TABLE_COLUMNS, MAX_TABLE_ROWS, TABLE_COLUMN, table_has_header};
pub use tabs::tabs_need_wildcard;
