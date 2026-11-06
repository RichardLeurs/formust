//! Formust's form model and widget registry. A form is a tree of egui containers; there is no
//! absolute positioning. This crate has no UI and no parser dependencies: `formust-rust` fills the
//! model from source text and writes it back.

mod form;
mod registry;

pub use form::{
    ComboItem, ContainerShell, EditError, Form, Node, NodeId, NodeKind, PropValue, Props,
    StateField,
};
pub use registry::{
    ALL_CONTAINERS, ALL_EVENTS, ALL_WIDGETS, ContainerDef, ContainerId, DefaultValue, EventId,
    PropDef, PropKind, PropRole, StateDef, WidgetDef, WidgetId, container_def, widget_def,
};
