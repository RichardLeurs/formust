//! A form to Rust text: its function, its item and its state struct. An unedited node is emitted
//! verbatim from `src`; only edited and new nodes are generated. [`crate::copy`] puts the text into the
//! `.frm.rs` copy. The `match`es on [`WidgetId`] and [`ContainerId`] have no `_` arm.

use formust_model::{
    ComboItem, ContainerId, EventId, Form, Node, NodeKind, PropDef, PropKind, PropRole, PropValue,
    Props, StateField, WidgetId, available_width_prop, container_def, is_plain_ui,
    table_has_header, tabs_need_wildcard, widget_def,
};

use crate::forms::{self, Call};

/// `#[derive(Default)] pub struct LoginForm { pub email: String }`, or the source's own struct with
/// any new fields added to it, so its attributes and comments survive.
pub(crate) fn state_struct(forms: &[&Form]) -> Option<String> {
    let mut fields: Vec<&StateField> = Vec::new();
    for form in forms {
        for field in &form.state {
            if !fields.iter().any(|f| f.name == field.name) {
                fields.push(field);
            }
        }
    }
    let source = forms.iter().find_map(|f| f.state_src.as_deref());
    match source {
        Some(text) => {
            let new: Vec<&&StateField> = fields
                .iter()
                .filter(|f| !field_in_source(text, &f.name))
                .collect();
            if new.is_empty() {
                return Some(text.to_string());
            }
            let close = text.rfind('}')?;
            let (before, after) = text.split_at(close);
            let trimmed = before.trim_end();
            let spacing = &before[trimmed.len()..];
            let comma = if trimmed.ends_with(',') || trimmed.ends_with('{') {
                ""
            } else {
                ","
            };
            let added: Vec<String> = new
                .iter()
                .map(|f| format!("pub {}: {}", f.name, f.ty))
                .collect();
            let separator = if spacing.contains('\n') {
                format!(",{spacing}    ")
            } else {
                ", ".to_string()
            };
            let start = if spacing.contains('\n') {
                format!("{spacing}    ")
            } else {
                " ".to_string()
            };
            let tail_comma = if spacing.contains('\n') { "," } else { "" };
            Some(format!(
                "{trimmed}{comma}{start}{}{tail_comma}{spacing}{after}",
                added.join(&separator)
            ))
        }
        None => {
            if fields.is_empty() {
                return None;
            }
            let list: Vec<String> = fields
                .iter()
                .map(|f| format!("pub {}: {}", f.name, f.ty))
                .collect();
            Some(format!(
                "#[derive(Default)] pub struct {} {{ {} }}",
                forms[0].state_struct,
                list.join(", ")
            ))
        }
    }
}

/// Does the struct text declare this field? Only the names that came from `.field: ` count.
fn field_in_source(text: &str, name: &str) -> bool {
    let mut rest = text;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + name.len()..].trim_start();
        if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && after.starts_with(':') {
            return true;
        }
        rest = &rest[i + name.len()..];
    }
    false
}

/// The form's function, attributes and signature included: what replaces the function in the copy.
pub fn emit_fn(form: &Form) -> String {
    format!("{}{}", form.sig, emit_body(form))
}

/// The form as an item: `impl X { fn .. { .. } }`, or just the function if it is a free one.
pub fn emit_form_item(form: &Form) -> String {
    let body = emit_body(form);
    if form.impl_header.is_empty() {
        return format!("{}{}", form.sig, body);
    }
    format!(
        "{}{{{}{}{}{}}}",
        form.impl_header, form.impl_pre, form.sig, body, form.impl_post
    )
}

/// The function body with its braces. Byte-identical to the source's while nothing is edited.
pub fn emit_body(form: &Form) -> String {
    if let Some(src) = &form.root.src {
        return src.clone();
    }
    let NodeKind::Container {
        children,
        closing,
        ident,
        shell,
        ..
    } = &form.root.kind
    else {
        return String::new();
    };
    let cx = Cx {
        path: &form.egui_path,
        nl: &form.newline,
        ui: ident,
    };
    // The head is `{` and any inner attributes of the body.
    let mut out = shell
        .as_ref()
        .and_then(|s| s.head.clone())
        .unwrap_or_else(|| "{".to_string());
    let mut body = String::new();
    emit_children(&mut body, children, closing, ident, &cx);
    if let Some(ctx) = &form.legacy_ctx
        && uses_word(&body, ctx)
    {
        // Custom code still reads the `ctx` of the old signature.
        let indent = children
            .first()
            .and_then(|c| {
                c.leading
                    .rfind('\n')
                    .map(|i| c.leading[i + 1..].to_string())
            })
            .unwrap_or_else(|| "        ".to_string());
        out.push_str(&form.newline);
        out.push_str(&indent);
        out.push_str(&format!("let {ctx} = &ui.ctx().clone();"));
    }
    out.push_str(&body);
    out.push('}');
    out
}

/// One statement as it reads in a body whose `Ui` is called `ident`, with its same-line comment:
/// verbatim while unedited and the receiver matches, regenerated otherwise. Custom code is always
/// verbatim. For copying a node to the clipboard; [`crate::clip::parse_statements`] reads it back.
pub fn emit_node(form: &Form, node: &Node, ident: &str) -> String {
    let cx = Cx {
        path: &form.egui_path,
        nl: &form.newline,
        ui: ident,
    };
    let mut text = node_text(node, ident, &cx);
    if node.bare_tail && !text.trim_end().ends_with([';', '}']) {
        text.push(';');
    }
    text.push_str(&node.trailing);
    text
}

fn uses_word(text: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(i, _)| {
        !text[..i].chars().next_back().is_some_and(is_word)
            && !text[i + word.len()..].chars().next().is_some_and(is_word)
    })
}

#[derive(Clone, Copy)]
struct Cx<'a> {
    path: &'a str,
    nl: &'a str,
    /// The `Ui` in scope where the text goes.
    ui: &'a str,
}

fn emit_children(out: &mut String, children: &[Node], closing: &str, ident: &str, cx: &Cx<'_>) {
    emit_list(out, children, closing, ident, true, cx);
}

/// `statements`: these are statements, and need `;` between them. The branches of an `if` chain and
/// the arms of a `match` don't.
fn emit_list(
    out: &mut String,
    children: &[Node],
    closing: &str,
    ident: &str,
    statements: bool,
    cx: &Cx<'_>,
) {
    // The columns of a `ui.columns(..)` have no text of their own: their statements follow each
    // other in the closure, each on its own `cols[i]`.
    let mut items: Vec<(&Node, &str)> = Vec::new();
    for child in children {
        match &child.kind {
            NodeKind::Container {
                def: ContainerId::Column,
                children: kids,
                ident: column,
                ..
            } => items.extend(kids.iter().map(|k| (k, column.as_str()))),
            _ => items.push((child, ident)),
        }
    }
    let mut after_line_comment = false;
    let count = items.len();
    for (index, (child, ident)) in items.into_iter().enumerate() {
        guard_line_comment(out, after_line_comment, &child.leading, cx);
        out.push_str(&child.leading);
        let text = node_text(child, ident, cx);
        let is_last = index + 1 == count;
        out.push_str(&text);
        if statements && !is_last && child.bare_tail && !text.trim_end().ends_with(';') {
            out.push(';');
        }
        out.push_str(&child.trailing);
        after_line_comment = child.trailing.trim_start().starts_with("//");
    }
    guard_line_comment(out, after_line_comment, closing, cx);
    out.push_str(closing);
}

/// A `//` comment must not swallow what a move put after it.
fn guard_line_comment(out: &mut String, after_line_comment: bool, next: &str, cx: &Cx<'_>) {
    if after_line_comment
        && !next
            .trim_start_matches([' ', '\t'])
            .starts_with(['\r', '\n'])
    {
        out.push_str(cx.nl);
    }
}

fn node_text(node: &Node, ident: &str, cx: &Cx<'_>) -> String {
    match &node.kind {
        NodeKind::Custom { text } => text.clone(),
        NodeKind::Widget {
            def,
            props,
            events,
            extra_chain,
            response_chain,
            recv,
            let_prefix,
        } => {
            if let Some(src) = &node.src
                && recv == ident
            {
                return src.clone();
            }
            let text = widget_text(*def, props, events, extra_chain, response_chain, ident, cx);
            format!("{}{text}", let_prefix.as_deref().unwrap_or(""))
        }
        NodeKind::Container {
            def,
            props,
            children,
            closing,
            extra_chain,
            ident: inner,
            recv,
            closure,
            braced,
            shell,
        } => {
            if *def == ContainerId::Column {
                // A column alone (copied to the clipboard): its statements on its `cols[i]`.
                let mut out = String::new();
                emit_list(&mut out, children, "", inner, true, cx);
                return out;
            }
            if *def == ContainerId::TabPage {
                // A page alone (copied to the clipboard): its statements, on the `Ui` it is in.
                let mut out = String::new();
                emit_list(&mut out, children, "", ident, true, cx);
                return out;
            }
            if def.is_tabs() {
                return tabs_text(node, ident, cx);
            }
            if *def == ContainerId::Table {
                return table_text(node, ident, cx);
            }
            if matches!(def, ContainerId::TableRow | ContainerId::TableCell) {
                // A row or a cell alone (copied to the clipboard): the statements in its cells.
                let mut out = String::new();
                let cells: Vec<&Node> = if *def == ContainerId::TableRow {
                    children.iter().collect()
                } else {
                    vec![node]
                };
                for cell in cells {
                    emit_list(&mut out, cell.children(), "", "ui", true, cx);
                }
                return out;
            }
            if let Some(src) = &node.src
                && recv == ident
            {
                return src.clone();
            }
            if *def == ContainerId::Sides {
                // `egui::Sides::new()..show(ui, ` then the two closures (each a Side), then `);`.
                let head = shell
                    .as_ref()
                    .filter(|_| recv == ident)
                    .and_then(|s| s.head.clone())
                    .unwrap_or_else(|| {
                        container_head(*def, props, extra_chain, closure, ident, cx)
                    });
                let mut out = head;
                emit_list(&mut out, children, closing, inner, false, cx);
                out.push_str(shell.as_ref().map_or(");", |s| s.tail.as_str()));
                return out;
            }
            // Control flow has no `Ui` of its own: its children use the one it now sits in.
            let inner = if def.is_flow() { ident } else { inner.as_str() };
            let own_head = shell
                .as_ref()
                .filter(|_| recv == ident || def.is_flow())
                .and_then(|s| s.head.clone());
            let tail = shell
                .as_ref()
                .map_or(default_tail(*def), |s| s.tail.as_str());
            let mut out = String::new();
            match own_head {
                Some(head) if *braced => {
                    out.push_str(&head);
                    emit_list(
                        &mut out,
                        children,
                        closing,
                        inner,
                        *def != ContainerId::FlowGroup,
                        cx,
                    );
                }
                Some(head) => {
                    out.push_str(&head);
                    unbraced_body(&mut out, children, inner, cx);
                }
                None => {
                    out.push_str(&container_head(
                        *def,
                        props,
                        extra_chain,
                        closure,
                        ident,
                        cx,
                    ));
                    // Spacing and size statements open the body, on the container's own `Ui`.
                    let statements = body_statements(*def, props, inner, cx);
                    if !statements.is_empty() {
                        let indent = body_indent(children, closing);
                        for statement in &statements {
                            match &indent {
                                Some(indent) => {
                                    out.push_str(cx.nl);
                                    out.push_str(indent);
                                }
                                None => out.push(' '),
                            }
                            out.push_str(statement);
                        }
                        if children.is_empty() && closing.is_empty() {
                            out.push(' ');
                        }
                    }
                    braces_body(&mut out, children, closing, inner, *braced, cx);
                }
            }
            out.push_str(tail);
            out
        }
    }
}

/// The end of a new container's text: a Side is only a closure.
fn default_tail(def: ContainerId) -> &'static str {
    match def {
        ContainerId::Side => "}",
        ContainerId::Column => "",
        _ => "});",
    }
}

/// The `Ui` as an argument: `ui`, or `&mut cols[0]` for a column of `ui.columns(..)`.
fn ui_arg(ident: &str) -> String {
    if is_plain_ui(ident) {
        ident.to_string()
    } else {
        format!("&mut {ident}")
    }
}

/// The name for a closure's `Ui` parameter: the same name, or `ui` where the `Ui` is `cols[0]`.
fn ui_param(ident: &str) -> &str {
    if is_plain_ui(ident) { ident } else { "ui" }
}

/// `|ui| child` with its one child still fitting, else with braces added.
fn unbraced_body(out: &mut String, children: &[Node], ident: &str, cx: &Cx<'_>) {
    if let [only] = children {
        let text = node_text(only, ident, cx);
        if !text.trim_end().ends_with(';')
            && only.trailing.is_empty()
            && only.leading.trim().is_empty()
        {
            out.push_str(&only.leading);
            out.push_str(&text);
            return;
        }
    }
    out.push('{');
    braces_body_inner(out, children, ident, cx);
    out.push('}');
}

/// The rest of a body whose head was generated, so it ends in `{`.
fn braces_body(
    out: &mut String,
    children: &[Node],
    closing: &str,
    ident: &str,
    braced: bool,
    cx: &Cx<'_>,
) {
    if braced {
        emit_children(out, children, closing, ident, cx);
    } else {
        braces_body_inner(out, children, ident, cx);
        out.push('}');
    }
}

fn braces_body_inner(out: &mut String, children: &[Node], ident: &str, cx: &Cx<'_>) {
    if children.first().is_some_and(|c| c.leading.is_empty()) {
        out.push(' ');
    }
    emit_children(out, children, " ", ident, cx);
}

/// The value of a property as Rust source.
fn value_text(value: &PropValue, kind: PropKind, cx: &Cx<'_>) -> String {
    match value {
        PropValue::Bool(v) => v.to_string(),
        PropValue::Int(v) => v.to_string(),
        PropValue::Float(v) => float_text(*v),
        PropValue::Text(v) => string_literal(v),
        PropValue::Enum(v) => match kind {
            PropKind::Enum { path, .. } if !path.is_empty() => format!("{}{path}::{v}", cx.path),
            _ => (*v).to_string(),
        },
        PropValue::Color { light, dark: None } => color_literal(*light, cx),
        // `Visuals::dark_mode` is the current mode. The `ui.add(..)` around this is fine: a two-phase
        // borrow lets arguments read `ui`.
        PropValue::Color {
            light,
            dark: Some(dark),
        } => format!(
            "if {}.visuals().dark_mode {{ {} }} else {{ {} }}",
            cx.ui,
            color_literal(*dark, cx),
            color_literal(*light, cx)
        ),
        PropValue::Expr(v) => v.clone(),
        // Items are written by the ComboBox generator; they are no expression of their own.
        PropValue::Items(_) => String::new(),
        PropValue::Stroke { width, light, dark } => format!(
            "{}Stroke::new({}, {})",
            cx.path,
            float_text(*width),
            value_text(
                &PropValue::Color {
                    light: *light,
                    dark: *dark
                },
                PropKind::Color,
                cx
            )
        ),
    }
}

fn color_literal([r, g, b, a]: [u8; 4], cx: &Cx<'_>) -> String {
    if a == 255 {
        format!("{}Color32::from_rgb({r}, {g}, {b})", cx.path)
    } else {
        format!(
            "{}Color32::from_rgba_unmultiplied({r}, {g}, {b}, {a})",
            cx.path
        )
    }
}

fn float_text(value: f64) -> String {
    if value.is_nan() {
        "f32::NAN".to_string()
    } else if value.is_infinite() {
        if value > 0.0 {
            "f32::INFINITY"
        } else {
            "f32::NEG_INFINITY"
        }
        .to_string()
    } else {
        format!("{value:?}")
    }
}

/// A Rust string literal. Printable text, unicode included, stays as it is.
pub(crate) fn string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A call's arguments, using the first shape in `calls` that fits the properties set.
fn pick_call<'c>(calls: &'c [Call], defs: &'static [PropDef], props: &Props) -> Option<&'c Call> {
    calls.iter().find(|(_, args)| {
        let all_have_value = args.iter().all(|name| {
            props.contains_key(name)
                || defs
                    .iter()
                    .any(|d| d.name == *name && d.default.to_value().is_some())
        });
        let none_left_out = defs
            .iter()
            .filter(|d| d.role == PropRole::Arg && props.contains_key(d.name))
            .all(|d| args.contains(&d.name));
        all_have_value && none_left_out
    })
}

fn args_text(args: &[&str], defs: &'static [PropDef], props: &Props, cx: &Cx<'_>) -> String {
    let mut parts = Vec::new();
    for name in args {
        let Some(def) = defs.iter().find(|d| d.name == *name) else {
            continue;
        };
        let value = props.get(name).cloned().or_else(|| def.default.to_value());
        let text = value.map_or_else(String::new, |v| value_text(&v, def.kind, cx));
        let rich = if *name == "text" {
            rich_chain(defs, props, cx)
        } else {
            String::new()
        };
        if rich.is_empty() {
            parts.push(text);
        } else {
            parts.push(format!("{}RichText::new({text}){rich}", cx.path));
        }
    }
    parts.join(", ")
}

/// The `egui::RichText` methods of the text styling set, in registry order: `.size(18.0).strong()`.
/// Empty when none is set, and the text stays a plain argument.
fn rich_chain(defs: &'static [PropDef], props: &Props, cx: &Cx<'_>) -> String {
    let mut out = String::new();
    for def in defs {
        let (PropRole::Rich(method), Some(value)) = (def.role, props.get(def.name)) else {
            continue;
        };
        match (def.kind, value) {
            (PropKind::Bool, PropValue::Bool(true)) => out.push_str(&format!(".{method}()")),
            (PropKind::Bool, _) => {}
            _ => out.push_str(&format!(".{method}({})", value_text(value, def.kind, cx))),
        }
    }
    out
}

/// `.on_hover_text("..")` and the other properties on the response, in registry order.
fn response_props(defs: &'static [PropDef], props: &Props, cx: &Cx<'_>) -> String {
    let mut out = String::new();
    for def in defs {
        if let (PropRole::Response(method), Some(value)) = (def.role, props.get(def.name)) {
            out.push_str(&format!(".{method}({})", value_text(value, def.kind, cx)));
        }
    }
    out
}

/// The statements a container's properties put at the start of its body, on its own `Ui`
/// `ident`: `ui.set_min_width(120.0);`, `ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);`.
fn body_statements(def: ContainerId, props: &Props, ident: &str, cx: &Cx<'_>) -> Vec<String> {
    let cx = &Cx { ui: ident, ..*cx };
    let mut out = Vec::new();
    for d in container_def(def).props {
        let Some(value) = props.get(d.name) else {
            continue;
        };
        let value = value_text(value, d.kind, cx);
        match d.role {
            PropRole::UiSet(method) => out.push(format!("{ident}.{method}({value});")),
            PropRole::Spacing(field) => {
                out.push(format!("{ident}.spacing_mut().{field} = {value};"))
            }
            PropRole::Arg
            | PropRole::Builder(_)
            | PropRole::Flag(_)
            | PropRole::Ctor
            | PropRole::AddSized
            | PropRole::AddEnabled
            | PropRole::AddVisible
            | PropRole::Body
            | PropRole::Rich(_)
            | PropRole::Response(_)
            | PropRole::InitialFocus => {}
        }
    }
    out
}

/// The indent of a body's statements: its first child's, or one step in from its closing brace.
fn body_indent(children: &[Node], closing: &str) -> Option<String> {
    if let Some(first) = children.first() {
        return first
            .leading
            .rfind('\n')
            .map(|i| first.leading[i + 1..].to_string());
    }
    closing
        .rfind('\n')
        .map(|i| format!("{}    ", &closing[i + 1..]))
}

/// The `.method(value)` chain of the properties set, in registry order, then the verbatim extras.
/// `available`: the property where Fill (an infinite value) is written `<ui>.available_width()`,
/// because the widget would take `f32::INFINITY` literally (see `formust_model::FillWay`).
fn chain_text(
    defs: &'static [PropDef],
    props: &Props,
    extra_chain: &[String],
    available: Option<&str>,
    cx: &Cx<'_>,
) -> String {
    let mut out = String::new();
    for def in defs {
        let Some(value) = props.get(def.name) else {
            continue;
        };
        match def.role {
            PropRole::Builder(method) => {
                let text = match value {
                    PropValue::Float(v)
                        if available == Some(def.name) && v.is_infinite() && *v > 0.0 =>
                    {
                        format!("{}.available_width()", cx.ui)
                    }
                    _ => value_text(value, def.kind, cx),
                };
                out.push_str(&format!(".{method}({text})"))
            }
            PropRole::Flag(method) => {
                if *value == PropValue::Bool(true) {
                    out.push_str(&format!(".{method}()"));
                }
            }
            PropRole::Arg
            | PropRole::Ctor
            | PropRole::AddSized
            | PropRole::AddEnabled
            | PropRole::AddVisible
            | PropRole::Body
            | PropRole::Rich(_)
            | PropRole::Response(_)
            | PropRole::UiSet(_)
            | PropRole::Spacing(_)
            | PropRole::InitialFocus => {}
        }
    }
    for extra in extra_chain {
        out.push_str(extra);
    }
    out
}

fn widget_text(
    def: WidgetId,
    props: &Props,
    events: &[(EventId, String)],
    extra_chain: &[String],
    response_chain: &[String],
    ident: &str,
    cx: &Cx<'_>,
) -> String {
    let call = match def {
        WidgetId::ComboBox => {
            let build = |ui: &str| combo_box_call(props, extra_chain, ui, cx);
            wrap_ui(&build, widget_def(def).props, props, ident, cx)
        }
        WidgetId::Label
        | WidgetId::Heading
        | WidgetId::Small
        | WidgetId::Strong
        | WidgetId::Weak
        | WidgetId::Monospace
        | WidgetId::Button
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
        | WidgetId::ProgressBar
        | WidgetId::Spinner
        | WidgetId::Separator
        | WidgetId::AddSpace
        | WidgetId::EndRow
        | WidgetId::Image
        | WidgetId::ColorEdit
        | WidgetId::ColorEditRgb
        | WidgetId::RangeSlider => plain_widget_call(def, props, extra_chain, ident, cx),
    };
    let tooltip = response_props(widget_def(def).props, props, &Cx { ui: ident, ..*cx });
    let response: String = response_chain.concat();
    let call = format!("{call}{tooltip}{response}");
    let focus = widget_def(def).props.iter().any(|d| {
        d.role == PropRole::InitialFocus && props.get(d.name) == Some(&PropValue::Bool(true))
    });
    match events {
        [] if !focus => format!("{call};"),
        [(event, handler)] if !focus => format!("if {call}.{}() {{{handler}}}", event.method()),
        many => {
            let mut out = format!("{{ let response = {call};");
            if focus {
                out.push_str(&format!(" {}", focus_statement(ident)));
            }
            for (event, handler) in many {
                out.push_str(&format!(" if response.{}() {{{handler}}}", event.method()));
            }
            out.push_str(" }");
            out
        }
    }
}

/// Initial focus: the widget takes the focus while nothing else has it.
pub(crate) fn focus_statement(ident: &str) -> String {
    format!("if {ident}.memory(|m| m.focused().is_none()) {{ response.request_focus(); }}")
}

fn plain_widget_call(
    def: WidgetId,
    props: &Props,
    extra_chain: &[String],
    ident: &str,
    cx: &Cx<'_>,
) -> String {
    let wdef = widget_def(def);
    let shapes = forms::widget_forms(def);
    // Text styling goes into the text argument and a tooltip onto the response: neither needs the
    // builder. The `add_*` wrappers do, when there is one.
    let builder_needed = !extra_chain.is_empty()
        || wdef.props.iter().any(|d| {
            props.contains_key(d.name)
                && !matches!(
                    d.role,
                    PropRole::Arg
                        | PropRole::Rich(_)
                        | PropRole::Response(_)
                        | PropRole::InitialFocus
                )
        })
        || shapes.shorthand.is_empty();
    if !builder_needed && let Some((method, args)) = pick_call(shapes.shorthand, wdef.props, props)
    {
        let cx = &Cx { ui: ident, ..*cx };
        return format!(
            "{ident}.{method}({})",
            args_text(args, wdef.props, props, cx)
        );
    }
    let Some((ty, ctors)) = shapes.builder else {
        // Only widgets with no builder arrive here, and they have no chain methods to lose. Enabled
        // and visible wrap the shorthand in a `Ui` of its own.
        let (method, args) = shapes
            .shorthand
            .first()
            .expect("a widget has a builder or a shorthand");
        let build = |ui: &str| {
            let cx = &Cx { ui, ..*cx };
            format!("{ui}.{method}({})", args_text(args, wdef.props, props, cx))
        };
        return wrap_ui(&build, wdef.props, props, ident, cx);
    };
    let (ctor, args) = pick_call(ctors, wdef.props, props)
        .or(ctors.first())
        .expect("a builder has a constructor");
    // The widget's own text, with the `Ui` its arguments read (a dark colour, Fill) named `ui`.
    let built = |ui: &str| {
        let cx = &Cx { ui, ..*cx };
        let args = args_text(args, wdef.props, props, cx);
        let chain = chain_text(
            wdef.props,
            props,
            extra_chain,
            available_width_prop(def),
            cx,
        );
        format!("{}{ty}::{ctor}({args}){chain}", cx.path)
    };
    wrap_add(&built, wdef.props, props, ident, cx)
}

/// `ui.add(w)`, or the `add_*` call for the wrapper set. More than one nest in closures,
/// `ui.add_enabled_ui(e, |ui| ui.scope(|ui| { if !(v) { ui.set_invisible(); } ui.add_sized(s, w) }).inner).inner`,
/// which the importer reads back as the widget with all three. `widget` gives the widget's text for
/// the `Ui` it is added to: inside those closures that is their parameter, which is `ui` when the
/// outer `Ui` is a column `cols[0]` (no closure parameter can have that name).
fn wrap_add(
    widget: &dyn Fn(&str) -> String,
    defs: &'static [PropDef],
    props: &Props,
    ident: &str,
    cx: &Cx<'_>,
) -> String {
    let get = |role: PropRole, ui: &str| {
        let cx = &Cx { ui, ..*cx };
        defs.iter().find(|d| d.role == role).and_then(|d| {
            Some(match props.get(d.name)? {
                // Fill: as wide as there is room, as high as a normal widget.
                PropValue::Float(v)
                    if role == PropRole::AddSized && v.is_infinite() && *v > 0.0 =>
                {
                    format!("[{ui}.available_width(), {ui}.spacing().interact_size.y]")
                }
                value => value_text(value, d.kind, cx),
            })
        })
    };
    let has = |role: PropRole| get(role, ident).is_some();
    let (sized, enabled, visible) = (
        has(PropRole::AddSized),
        has(PropRole::AddEnabled),
        has(PropRole::AddVisible),
    );
    match (sized, enabled, visible) {
        (false, false, false) => format!("{ident}.add({})", widget(ident)),
        (true, false, false) => format!(
            "{ident}.add_sized({}, {})",
            get(PropRole::AddSized, ident).unwrap_or_default(),
            widget(ident)
        ),
        (false, true, false) => format!(
            "{ident}.add_enabled({}, {})",
            get(PropRole::AddEnabled, ident).unwrap_or_default(),
            widget(ident)
        ),
        (false, false, true) => format!(
            "{ident}.add_visible({}, {})",
            get(PropRole::AddVisible, ident).unwrap_or_default(),
            widget(ident)
        ),
        _ => {
            // Inside the closures the `Ui` is their parameter.
            let p = ui_param(ident);
            let mut call = match get(PropRole::AddSized, p) {
                Some(s) => format!("{p}.add_sized({s}, {})", widget(p)),
                None => format!("{p}.add({})", widget(p)),
            };
            // egui 0.37 has `add_enabled_ui` but no `add_visible_ui`.
            if let Some(v) = get(PropRole::AddVisible, p) {
                let outer = if enabled { p } else { ident };
                call = format!(
                    "{outer}.scope(|{p}| {{ if !({v}) {{ {p}.set_invisible(); }} {call} }}).inner"
                );
            }
            if let Some(e) = get(PropRole::AddEnabled, ident) {
                call = format!("{ident}.add_enabled_ui({e}, |{p}| {call}).inner");
            }
            call
        }
    }
}

/// Enabled and visible for a call that is no `ui.add` of a widget (a shorthand with no builder, a
/// ComboBox): `ui.add_enabled_ui(e, |ui| call).inner`, and
/// `ui.scope(|ui| { if !(v) { ui.set_invisible(); } call }).inner`, the shapes `wrap_add` nests
/// in. The importer reads both back. `call` gives the call on a `Ui` (see [`wrap_add`]).
fn wrap_ui(
    call: &dyn Fn(&str) -> String,
    defs: &'static [PropDef],
    props: &Props,
    ident: &str,
    cx: &Cx<'_>,
) -> String {
    let get = |role: PropRole, ui: &str| {
        let cx = &Cx { ui, ..*cx };
        defs.iter()
            .find(|d| d.role == role)
            .and_then(|d| Some(value_text(props.get(d.name)?, d.kind, cx)))
    };
    let enabled = get(PropRole::AddEnabled, ident);
    let visible = get(PropRole::AddVisible, ident);
    if enabled.is_none() && visible.is_none() {
        return call(ident);
    }
    let p = ui_param(ident);
    let mut text = call(p);
    if let Some(v) = get(PropRole::AddVisible, p) {
        let outer = if enabled.is_some() { p } else { ident };
        text =
            format!("{outer}.scope(|{p}| {{ if !({v}) {{ {p}.set_invisible(); }} {text} }}).inner");
    }
    if let Some(e) = enabled {
        text = format!("{ident}.add_enabled_ui({e}, |{p}| {text}).inner");
    }
    text
}

fn combo_box_call(props: &Props, extra_chain: &[String], ident: &str, cx: &Cx<'_>) -> String {
    let cx = &Cx { ui: ident, ..*cx };
    let wdef = widget_def(WidgetId::ComboBox);
    let (ty, ctors) = forms::widget_forms(WidgetId::ComboBox)
        .builder
        .expect("ComboBox has a builder");
    let fallback: &Call = &("from_id_salt", &["id_salt"]);
    let (ctor, args) = pick_call(ctors, wdef.props, props).unwrap_or(fallback);
    let args_out = if args.iter().any(|a| props.contains_key(a)) {
        args_text(args, wdef.props, props, cx)
    } else {
        string_literal("combo")
    };
    let chain = chain_text(
        wdef.props,
        props,
        extra_chain,
        available_width_prop(WidgetId::ComboBox),
        cx,
    );
    let binding = props.get("value").map_or_else(
        || "&mut self.form.choice".to_string(),
        |v| value_text(v, PropKind::Expr, cx),
    );
    let p = ui_param(ident);
    let mut items = String::new();
    if let Some(PropValue::Items(list)) = props.get("items") {
        for item in list {
            let label = value_text(&item.label, PropKind::Text, &Cx { ui: p, ..*cx });
            items.push_str(&format!(
                " {p}.selectable_value({binding}, {}, {label});",
                item.value
            ));
        }
    }
    format!(
        "{}{ty}::{ctor}({args_out}){chain}.show_ui({}, |{p}| {{{items} }})",
        cx.path,
        ui_arg(ident)
    )
}

fn container_head(
    def: ContainerId,
    props: &Props,
    extra_chain: &[String],
    closure: &str,
    ident: &str,
    cx: &Cx<'_>,
) -> String {
    let cx = &Cx { ui: ident, ..*cx };
    let cdef = container_def(def);
    let shapes = forms::container_forms(def);
    let closure = if closure.is_empty() { "|ui| " } else { closure };
    match def {
        ContainerId::Root => "{".to_string(),
        // The importer always keeps their heads; a move can't take them away. A column has none.
        ContainerId::Flow | ContainerId::FlowGroup | ContainerId::Column => String::new(),
        // Written whole by `tabs_text` and `table_text`.
        ContainerId::Tabs
        | ContainerId::TabsVertical
        | ContainerId::TabPage
        | ContainerId::Table
        | ContainerId::TableRow
        | ContainerId::TableCell => String::new(),
        ContainerId::Side => format!("{closure}{{"),
        ContainerId::Sides => {
            let (ty, _) = shapes.builder.expect("Sides has a builder");
            let chain = chain_text(cdef.props, props, extra_chain, None, cx);
            format!("{}{ty}::new(){chain}.show({}, ", cx.path, ui_arg(ident))
        }
        // The helper call up to its closure, as written: `crate::style::dialog(ctx, "about", `.
        ContainerId::DialogCall => {
            let call = match props.get("call") {
                Some(PropValue::Expr(call) | PropValue::Text(call)) => call.clone(),
                _ => cdef
                    .prop("call")
                    .and_then(|p| p.default.to_value())
                    .map(|v| value_text(&v, PropKind::Expr, cx))
                    .unwrap_or_default(),
            };
            format!("{call}{closure}{{")
        }
        ContainerId::HorizontalCentered
        | ContainerId::HorizontalTop
        | ContainerId::VerticalCenteredJustified
        | ContainerId::CenteredAndJustified
        | ContainerId::Scope
        | ContainerId::WithLayout
        | ContainerId::MenuButton
        | ContainerId::AddEnabledUi
        | ContainerId::Frame
        | ContainerId::Columns
        | ContainerId::MenuBar
        | ContainerId::Modal
        | ContainerId::Window
        | ContainerId::Horizontal
        | ContainerId::HorizontalWrapped
        | ContainerId::Vertical
        | ContainerId::VerticalCentered
        | ContainerId::Group
        | ContainerId::Indent
        | ContainerId::Grid
        | ContainerId::ScrollArea
        | ContainerId::CollapsingHeader
        | ContainerId::PanelTop
        | ContainerId::PanelBottom
        | ContainerId::PanelLeft
        | ContainerId::PanelRight
        | ContainerId::CentralPanel => {
            let only_args = extra_chain.is_empty()
                && cdef.props.iter().all(|d| {
                    d.role == PropRole::Arg
                        || d.role.is_body_statement()
                        || !props.contains_key(d.name)
                });
            if only_args && let Some((method, args)) = shapes.shorthand {
                let list = args_text(args, cdef.props, props, cx);
                let comma = if list.is_empty() { "" } else { ", " };
                return format!("{ident}.{method}({list}{comma}{closure}{{");
            }
            let (ty, ctors) = shapes
                .builder
                .expect("a container has a builder or a shorthand");
            let ctor_prop = cdef.props.iter().find(|d| d.role == PropRole::Ctor);
            let (ctor, args_out) = match ctor_prop {
                Some(prop) => {
                    let value = props
                        .get(prop.name)
                        .cloned()
                        .or_else(|| prop.default.to_value());
                    let name = match value {
                        Some(PropValue::Enum(v)) => v,
                        _ => ctors[0].0,
                    };
                    let args = ctors
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map_or(&[][..], |(_, a)| *a);
                    (name, args_text(args, cdef.props, props, cx))
                }
                None => {
                    let (name, args) = pick_call(ctors, cdef.props, props).unwrap_or(&ctors[0]);
                    (*name, args_text(args, cdef.props, props, cx))
                }
            };
            let chain = chain_text(cdef.props, props, extra_chain, None, cx);
            format!(
                "{}{ty}::{ctor}({args_out}){chain}.{}({}, {closure}{{",
                cx.path,
                forms::show_method(def),
                ui_arg(ident)
            )
        }
    }
}

// ---- Tabs ------------------------------------------------------------------------------------

/// The last line of a node's `leading`: its indent.
fn indent_of(node: &Node) -> &str {
    node.leading
        .rfind('\n')
        .map_or("", |i| &node.leading[i + 1..])
}

/// A Tabs node: its bar and `match` head, its pages (one arm each) and its tail. Unedited, the
/// source text; edited, the head and tail are regenerated and each unedited page stays verbatim.
fn tabs_text(node: &Node, ident: &str, cx: &Cx<'_>) -> String {
    let NodeKind::Container {
        def,
        props,
        children,
        closing,
        ident: own,
        recv,
        shell,
        ..
    } = &node.kind
    else {
        return String::new();
    };
    if let Some(src) = &node.src
        && recv == ident
    {
        return src.clone();
    }
    let vertical = *def == ContainerId::TabsVertical;
    // A horizontal bar's pages are in the body the Tabs is in; a vertical one's in its closure.
    let inner = if vertical { own.as_str() } else { ident };
    let indent = indent_of(node);
    let (arm, at_match) = if vertical {
        (format!("{indent}            "), format!("{indent}        "))
    } else {
        (format!("{indent}    "), indent.to_string())
    };
    let tabs = tab_items(props);
    let (head, tail) = match shell
        .as_ref()
        .filter(|_| recv == ident)
        .and_then(|s| Some((s.head.clone()?, s.tail.clone())))
    {
        Some(parts) => parts,
        None => tabs_parts(*def, props, ident, indent, cx.path, cx.nl),
    };
    let mut out = head;
    for (k, page) in children.iter().enumerate() {
        if page.leading.is_empty() {
            out.push_str(cx.nl);
            out.push_str(&arm);
        } else {
            out.push_str(&page.leading);
        }
        out.push_str(&page_text(page, tabs.get(k), inner, cx));
        out.push_str(&page.trailing);
    }
    if closing.is_empty() {
        if tabs_need_wildcard(&tabs) {
            out.push_str(&format!("{}{arm}_ => {{}}", cx.nl));
        }
        out.push_str(cx.nl);
        out.push_str(&at_match);
    } else {
        out.push_str(closing);
    }
    out.push_str(&tail);
    out
}

/// One arm: `<value> => { .. }`, verbatim while unedited and on the same `Ui`.
fn page_text(page: &Node, tab: Option<&ComboItem>, inner: &str, cx: &Cx<'_>) -> String {
    let NodeKind::Container {
        children,
        closing,
        recv,
        shell,
        ..
    } = &page.kind
    else {
        return String::new();
    };
    if let Some(src) = &page.src
        && recv == inner
    {
        return src.clone();
    }
    let head = shell
        .as_ref()
        .and_then(|s| s.head.clone())
        .unwrap_or_else(|| format!("{} => {{", tab.map_or("_", |t| t.value.trim())));
    let mut out = head;
    emit_children(&mut out, children, closing, inner, cx);
    out.push_str(shell.as_ref().map_or("}", |s| s.tail.as_str()));
    out
}

fn tab_items(props: &Props) -> Vec<ComboItem> {
    match props.get("tabs") {
        Some(PropValue::Items(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// The text of a Tabs before its first arm (the bar, then `match <field> {`) and after the
/// match's closing brace, for a Tabs in a body whose `Ui` is `recv`, indented `indent`. The
/// importer reads only what this writes, so the two always agree.
pub(crate) fn tabs_parts(
    def: ContainerId,
    props: &Props,
    recv: &str,
    indent: &str,
    path: &str,
    nl: &str,
) -> (String, String) {
    let cx = Cx { path, nl, ui: "ui" };
    let field = match props.get("field") {
        Some(PropValue::Expr(e) | PropValue::Text(e)) => e.trim().to_string(),
        _ => "self.form.tab".to_string(),
    };
    let style = match props.get("style") {
        Some(PropValue::Enum(s)) => *s,
        _ => "selectable",
    };
    let item = |tab: &ComboItem| {
        let label = value_text(&tab.label, PropKind::Text, &cx);
        let value = tab.value.trim();
        match style {
            "button" | "frameless" => {
                let frame = if style == "frameless" {
                    ".frame(false)"
                } else {
                    ""
                };
                format!(
                    "if ui.add({path}Button::new({label}){frame}.selected({field} == {value})).clicked() {{ {field} = {value}; }}"
                )
            }
            _ => format!("ui.selectable_value(&mut {field}, {value}, {label});"),
        }
    };
    let tabs = tab_items(props);
    let i4 = format!("{indent}    ");
    let i8 = format!("{indent}        ");
    let mut head = format!("{recv}.horizontal(|ui| {{");
    if def == ContainerId::TabsVertical {
        head.push_str(&format!("{nl}{i4}ui.vertical(|ui| {{"));
        for tab in &tabs {
            head.push_str(&format!("{nl}{i8}{}", item(tab)));
        }
        head.push_str(&format!(
            "{nl}{i4}}});{nl}{i4}ui.separator();{nl}{i4}ui.vertical(|ui| {{{nl}{i8}match {field} {{"
        ));
        let tail = format!("}}{nl}{i4}}});{nl}{indent}}});");
        (head, tail)
    } else {
        for tab in &tabs {
            head.push_str(&format!("{nl}{i4}{}", item(tab)));
        }
        head.push_str(&format!("{nl}{indent}}});{nl}{indent}match {field} {{"));
        (head, "}".to_string())
    }
}

// ---- Tables ----------------------------------------------------------------------------------

/// A Table node: `egui_extras::TableBuilder::new(ui)`, its builder methods and columns, the header
/// row and the body rows. Unedited, the source text. Edited, the table's own lines are written
/// afresh (whitespace as the designer writes it) and every cell's statements stay as they are.
fn table_text(node: &Node, ident: &str, cx: &Cx<'_>) -> String {
    if let (Some(src), NodeKind::Container { recv, .. }) = (&node.src, &node.kind)
        && recv == ident
    {
        return src.clone();
    }
    table_parts(node, ident, indent_of(node), cx)
}

/// The text of a table at `indent` in a body whose `Ui` is `ident`, the cells' statements verbatim.
pub(crate) fn table_text_for(node: &Node, ident: &str, path: &str, nl: &str) -> String {
    let cx = Cx {
        path,
        nl,
        ui: ident,
    };
    table_parts(node, ident, indent_of(node), &cx)
}

fn table_parts(node: &Node, ident: &str, indent: &str, cx: &Cx<'_>) -> String {
    let NodeKind::Container {
        props,
        children,
        extra_chain,
        ..
    } = &node.kind
    else {
        return String::new();
    };
    let nl = cx.nl;
    let cdef = container_def(ContainerId::Table);
    let chain = chain_text(
        cdef.props,
        props,
        extra_chain,
        None,
        &Cx { ui: ident, ..*cx },
    );
    let mut out = format!("egui_extras::TableBuilder::new({}){chain}", ui_arg(ident));
    if let Some(PropValue::Items(columns)) = props.get("columns") {
        for column in columns {
            out.push_str(&format!(".column({})", column.value.trim()));
        }
    }
    let height = |name: &str| match props.get(name) {
        Some(v @ (PropValue::Float(_) | PropValue::Int(_) | PropValue::Expr(_))) => {
            value_text(v, PropKind::Float, cx)
        }
        _ => "18.0".to_string(),
    };
    let i4 = format!("{indent}    ");
    let i8 = format!("{indent}        ");
    let cells = |out: &mut String, row: &Node, owner: &str, at: &str| {
        for cell in row.children() {
            let NodeKind::Container {
                children, closing, ..
            } = &cell.kind
            else {
                continue;
            };
            out.push_str(&format!("{nl}{at}{owner}.col(|ui| {{"));
            let closing = if closing.is_empty() && !children.is_empty() {
                format!("{nl}{at}")
            } else {
                closing.clone()
            };
            emit_children(out, children, &closing, "ui", cx);
            out.push_str("});");
        }
    };
    let header = table_has_header(props);
    let mut rows = children.iter();
    if header && let Some(row) = rows.next() {
        out.push_str(&format!(".header({}, |mut header| {{", height("header")));
        cells(&mut out, row, "header", &i4);
        out.push_str(&format!("{nl}{indent}}})"));
    }
    out.push_str(".body(|mut body| {");
    let row_height = height("row_height");
    for row in rows {
        out.push_str(&format!("{nl}{i4}body.row({row_height}, |mut row| {{"));
        cells(&mut out, row, "row", &i8);
        out.push_str(&format!("{nl}{i4}}});"));
    }
    out.push_str(&format!("{nl}{indent}}});"));
    out
}
