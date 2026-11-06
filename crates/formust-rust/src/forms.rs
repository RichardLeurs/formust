//! The call shapes of the palette: how each widget and container is written in egui 0.37, read by
//! both the importer and the emitter. The `match`es are exhaustive, so a new [`WidgetId`] doesn't
//! compile until it has a shape here.

use formust_model::{ContainerId, WidgetId};

/// A constructor or `ui` method and the properties its positional arguments fill.
pub type Call = (&'static str, &'static [&'static str]);

#[derive(Clone, Copy, Debug)]
pub struct WidgetForms {
    /// `ui.<method>(args)`. The first that fits is generated, so longer ones come first.
    pub shorthand: &'static [Call],
    /// The type for `ui.add(<Type>::<ctor>(args).chain())` and its constructors.
    pub builder: Option<(&'static str, &'static [Call])>,
}

pub fn widget_forms(id: WidgetId) -> WidgetForms {
    const fn text_only(method: &'static [Call]) -> WidgetForms {
        WidgetForms {
            shorthand: method,
            builder: None,
        }
    }
    match id {
        WidgetId::Label => WidgetForms {
            shorthand: &[("label", &["text"])],
            builder: Some(("Label", &[("new", &["text"])])),
        },
        WidgetId::Heading => text_only(&[("heading", &["text"])]),
        WidgetId::Small => text_only(&[("small", &["text"])]),
        WidgetId::Strong => text_only(&[("strong", &["text"])]),
        WidgetId::Weak => text_only(&[("weak", &["text"])]),
        WidgetId::Monospace => text_only(&[("monospace", &["text"])]),
        WidgetId::Button => WidgetForms {
            shorthand: &[("button", &["text"])],
            builder: Some(("Button", &[("new", &["text"])])),
        },
        WidgetId::Link => WidgetForms {
            shorthand: &[("link", &["text"])],
            builder: Some(("Link", &[("new", &["text"])])),
        },
        WidgetId::Hyperlink => WidgetForms {
            shorthand: &[("hyperlink_to", &["text", "url"]), ("hyperlink", &["url"])],
            builder: Some((
                "Hyperlink",
                &[("from_label_and_url", &["text", "url"]), ("new", &["url"])],
            )),
        },
        WidgetId::Checkbox => WidgetForms {
            shorthand: &[("checkbox", &["value", "text"])],
            builder: Some(("Checkbox", &[("new", &["value", "text"])])),
        },
        WidgetId::ToggleValue => text_only(&[("toggle_value", &["value", "text"])]),
        WidgetId::RadioValue => text_only(&[("radio_value", &["value", "alternative", "text"])]),
        WidgetId::SelectableValue => {
            text_only(&[("selectable_value", &["value", "alternative", "text"])])
        }
        WidgetId::TextEditSingleline => WidgetForms {
            shorthand: &[("text_edit_singleline", &["value"])],
            builder: Some(("TextEdit", &[("singleline", &["value"])])),
        },
        WidgetId::TextEditMultiline => WidgetForms {
            shorthand: &[("text_edit_multiline", &["value"])],
            builder: Some(("TextEdit", &[("multiline", &["value"])])),
        },
        WidgetId::Slider => WidgetForms {
            shorthand: &[],
            builder: Some(("Slider", &[("new", &["value", "range"])])),
        },
        WidgetId::DragValue => WidgetForms {
            shorthand: &[],
            builder: Some(("DragValue", &[("new", &["value"])])),
        },
        WidgetId::ProgressBar => WidgetForms {
            shorthand: &[],
            builder: Some(("ProgressBar", &[("new", &["progress"])])),
        },
        WidgetId::Spinner => WidgetForms {
            shorthand: &[("spinner", &[])],
            builder: Some(("Spinner", &[("new", &[])])),
        },
        WidgetId::Separator => WidgetForms {
            shorthand: &[("separator", &[])],
            builder: Some(("Separator", &[("default", &[])])),
        },
        WidgetId::AddSpace => text_only(&[("add_space", &["amount"])]),
        // Written out by hand: its body holds the items, and its chain ends in `show_ui`.
        WidgetId::ComboBox => WidgetForms {
            shorthand: &[],
            builder: Some((
                "ComboBox",
                &[
                    ("new", &["id_salt", "label"]),
                    ("from_label", &["label"]),
                    ("from_id_salt", &["id_salt"]),
                ],
            )),
        },
        WidgetId::EndRow => text_only(&[("end_row", &[])]),
        WidgetId::Image => WidgetForms {
            shorthand: &[("image", &["source"])],
            builder: Some(("Image", &[("new", &["source"]), ("from_uri", &["uri"])])),
        },
        WidgetId::ColorEdit => text_only(&[("color_edit_button_srgba", &["value"])]),
        WidgetId::ColorEditRgb => text_only(&[("color_edit_button_rgb", &["value"])]),
        WidgetId::RangeSlider => WidgetForms {
            shorthand: &[],
            builder: Some(("RangeSlider", &[("new", &["low", "high", "range"])])),
        },
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ContainerForms {
    /// `ui.<method>(args, |ui| { .. })`.
    pub shorthand: Option<Call>,
    /// `<Type>::<ctor>(args).chain().show(ui, |ui| { .. })`.
    pub builder: Option<(&'static str, &'static [Call])>,
}

pub fn container_forms(id: ContainerId) -> ContainerForms {
    const fn method(call: Call) -> ContainerForms {
        ContainerForms {
            shorthand: Some(call),
            builder: None,
        }
    }
    const NONE: ContainerForms = ContainerForms {
        shorthand: None,
        builder: None,
    };
    match id {
        // A dialog helper call is any function; the importer reads it by shape (`dialog_call`).
        // A column and a side have no call of their own: their holder writes them.
        ContainerId::Root
        | ContainerId::DialogCall
        | ContainerId::Column
        | ContainerId::Side
        | ContainerId::Flow
        | ContainerId::FlowGroup => NONE,
        // Tabs are a bar and a `match`, read and written by hand (`import::tabs_at`, `emit`).
        ContainerId::Tabs | ContainerId::TabsVertical | ContainerId::TabPage => NONE,
        // `egui_extras::TableBuilder`, its rows and cells: read and written by hand.
        ContainerId::Table | ContainerId::TableRow | ContainerId::TableCell => NONE,
        // `egui::MenuBar::new().ui(ui, |ui| ..)`: shown with `ui`, not `show` (see `show_method`).
        ContainerId::MenuBar => ContainerForms {
            shorthand: None,
            builder: Some(("MenuBar", &[("new", &[])])),
        },
        ContainerId::Modal => ContainerForms {
            shorthand: None,
            builder: Some(("Modal", &[("new", &["id"])])),
        },
        // `ui.columns(2, |cols| { cols[0].label(..); })`: read and written by hand around the columns.
        ContainerId::Columns => method(("columns", &["count"])),
        // `egui::Sides::new().show(ui, |ui| .., |ui| ..)`: two closures, read and written by hand.
        ContainerId::Sides => ContainerForms {
            shorthand: None,
            builder: Some(("Sides", &[("new", &[])])),
        },
        ContainerId::HorizontalCentered => method(("horizontal_centered", &[])),
        ContainerId::HorizontalTop => method(("horizontal_top", &[])),
        ContainerId::VerticalCenteredJustified => method(("vertical_centered_justified", &[])),
        ContainerId::CenteredAndJustified => method(("centered_and_justified", &[])),
        ContainerId::Scope => method(("scope", &[])),
        ContainerId::WithLayout => method(("with_layout", &["layout"])),
        ContainerId::MenuButton => method(("menu_button", &["text"])),
        ContainerId::AddEnabledUi => method(("add_enabled_ui", &["enabled"])),
        ContainerId::Frame => ContainerForms {
            shorthand: None,
            builder: Some((
                "Frame",
                &[
                    ("new", &[]),
                    ("default", &[]),
                    ("group", &["style"]),
                    ("side_top_panel", &["style"]),
                    ("central_panel", &["style"]),
                    ("window", &["style"]),
                    ("menu", &["style"]),
                    ("popup", &["style"]),
                    ("canvas", &["style"]),
                    ("dark_canvas", &["style"]),
                ],
            )),
        },
        ContainerId::Window => ContainerForms {
            shorthand: None,
            builder: Some(("Window", &[("new", &["title"])])),
        },
        ContainerId::Horizontal => method(("horizontal", &[])),
        ContainerId::HorizontalWrapped => method(("horizontal_wrapped", &[])),
        ContainerId::Vertical => method(("vertical", &[])),
        ContainerId::VerticalCentered => method(("vertical_centered", &[])),
        ContainerId::Group => method(("group", &[])),
        ContainerId::Indent => method(("indent", &["id"])),
        ContainerId::Grid => ContainerForms {
            shorthand: None,
            builder: Some(("Grid", &[("new", &["id"])])),
        },
        // `direction` is the constructor's name: the emitter and importer read it from PropRole::Ctor.
        ContainerId::ScrollArea => ContainerForms {
            shorthand: None,
            builder: Some((
                "ScrollArea",
                &[
                    ("vertical", &[]),
                    ("horizontal", &[]),
                    ("both", &[]),
                    ("neither", &[]),
                ],
            )),
        },
        ContainerId::CollapsingHeader => ContainerForms {
            shorthand: Some(("collapsing", &["text"])),
            builder: Some(("CollapsingHeader", &[("new", &["text"])])),
        },
        ContainerId::PanelTop => ContainerForms {
            shorthand: None,
            builder: Some(("Panel", &[("top", &["id"])])),
        },
        ContainerId::PanelBottom => ContainerForms {
            shorthand: None,
            builder: Some(("Panel", &[("bottom", &["id"])])),
        },
        ContainerId::PanelLeft => ContainerForms {
            shorthand: None,
            builder: Some(("Panel", &[("left", &["id"])])),
        },
        ContainerId::PanelRight => ContainerForms {
            shorthand: None,
            builder: Some(("Panel", &[("right", &["id"])])),
        },
        ContainerId::CentralPanel => ContainerForms {
            shorthand: None,
            builder: Some((
                "CentralPanel",
                &[
                    ("default", &[]),
                    ("no_frame", &[]),
                    ("default_margins", &[]),
                ],
            )),
        },
    }
}

/// The method that shows a builder container with its closure: `.ui(ui, ..)` for a MenuBar,
/// `.show(ui, ..)` (or `.show(ctx, ..)`) for the others.
pub fn show_method(id: ContainerId) -> &'static str {
    match id {
        ContainerId::MenuBar => "ui",
        _ => "show",
    }
}

/// The older panel types, recognised on import only.
pub const LEGACY_PANELS: &[(&str, &str, ContainerId)] = &[
    ("TopBottomPanel", "top", ContainerId::PanelTop),
    ("TopBottomPanel", "bottom", ContainerId::PanelBottom),
    ("SidePanel", "left", ContainerId::PanelLeft),
    ("SidePanel", "right", ContainerId::PanelRight),
];

/// The older panel size methods, mapped to the 0.37 properties.
pub const LEGACY_PANEL_METHODS: &[(&str, &str)] = &[
    ("default_height", "default_size"),
    ("default_width", "default_size"),
    ("min_height", "min_size"),
    ("min_width", "min_size"),
    ("max_height", "max_size"),
    ("max_width", "max_size"),
    ("exact_height", "exact_size"),
    ("exact_width", "exact_size"),
    ("height_range", "size_range"),
    ("width_range", "size_range"),
];
