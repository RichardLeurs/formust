//! A form to Rust text: its function, its item and its state struct. An unedited node is emitted
//! verbatim from `src`; only edited and new nodes are generated. [`crate::copy`] puts the text into the
//! `.frm.rs` copy. The `match`es on [`WidgetId`] and [`ContainerId`] have no `_` arm.

use formust_model::{
    ContainerId, EventId, Form, Node, NodeKind, PropDef, PropKind, PropRole, PropValue, Props,
    StateField, WidgetId, container_def, widget_def,
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
    let mut after_line_comment = false;
    for (index, child) in children.iter().enumerate() {
        guard_line_comment(out, after_line_comment, &child.leading, cx);
        out.push_str(&child.leading);
        let text = node_text(child, ident, cx);
        let is_last = index + 1 == children.len();
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
            if let Some(src) = &node.src
                && recv == ident
            {
                return src.clone();
            }
            // Control flow has no `Ui` of its own: its children use the one it now sits in.
            let inner = if def.is_flow() { ident } else { inner.as_str() };
            let own_head = shell
                .as_ref()
                .filter(|_| recv == ident || def.is_flow())
                .and_then(|s| s.head.clone());
            let tail = shell.as_ref().map_or("});", |s| s.tail.as_str());
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
                    braces_body(&mut out, children, closing, inner, *braced, cx);
                }
            }
            out.push_str(tail);
            out
        }
    }
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
        parts.push(value.map_or_else(String::new, |v| value_text(&v, def.kind, cx)));
    }
    parts.join(", ")
}

/// The `.method(value)` chain of the properties set, in registry order, then the verbatim extras.
fn chain_text(
    defs: &'static [PropDef],
    props: &Props,
    extra_chain: &[String],
    cx: &Cx<'_>,
) -> String {
    let mut out = String::new();
    for def in defs {
        let Some(value) = props.get(def.name) else {
            continue;
        };
        match def.role {
            PropRole::Builder(method) => {
                out.push_str(&format!(".{method}({})", value_text(value, def.kind, cx)))
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
            | PropRole::Body => {}
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
        WidgetId::ComboBox => combo_box_call(props, extra_chain, ident, cx),
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
        | WidgetId::EndRow => plain_widget_call(def, props, extra_chain, ident, cx),
    };
    let response: String = response_chain.concat();
    let call = format!("{call}{response}");
    match events {
        [] => format!("{call};"),
        [(event, handler)] => format!("if {call}.{}() {{{handler}}}", event.method()),
        many => {
            let mut out = format!("{{ let response = {call};");
            for (event, handler) in many {
                out.push_str(&format!(" if response.{}() {{{handler}}}", event.method()));
            }
            out.push_str(" }");
            out
        }
    }
}

fn plain_widget_call(
    def: WidgetId,
    props: &Props,
    extra_chain: &[String],
    ident: &str,
    cx: &Cx<'_>,
) -> String {
    let cx = &Cx { ui: ident, ..*cx };
    let wdef = widget_def(def);
    let shapes = forms::widget_forms(def);
    let builder_needed = !extra_chain.is_empty()
        || wdef
            .props
            .iter()
            .any(|d| props.contains_key(d.name) && !matches!(d.role, PropRole::Arg))
        || shapes.shorthand.is_empty();
    if !builder_needed && let Some((method, args)) = pick_call(shapes.shorthand, wdef.props, props)
    {
        return format!(
            "{ident}.{method}({})",
            args_text(args, wdef.props, props, cx)
        );
    }
    let Some((ty, ctors)) = shapes.builder else {
        // Only widgets with no builder arrive here, and they have no chain methods to lose.
        let (method, args) = shapes
            .shorthand
            .first()
            .expect("a widget has a builder or a shorthand");
        return format!(
            "{ident}.{method}({})",
            args_text(args, wdef.props, props, cx)
        );
    };
    let (ctor, args) = pick_call(ctors, wdef.props, props)
        .or(ctors.first())
        .expect("a builder has a constructor");
    let args = args_text(args, wdef.props, props, cx);
    let chain = chain_text(wdef.props, props, extra_chain, cx);
    let built = format!("{}{ty}::{ctor}({args}){chain}", cx.path);
    wrap_add(&built, wdef.props, props, ident, cx)
}

/// `ui.add(w)`, or the `add_*` call for the wrapper set. `Form::set_prop` allows only one, and import
/// can't produce two. If a tool sets more anyway, they nest in closures, which is valid code that
/// reads back as custom.
fn wrap_add(
    widget: &str,
    defs: &'static [PropDef],
    props: &Props,
    ident: &str,
    cx: &Cx<'_>,
) -> String {
    let get = |role: PropRole| {
        defs.iter()
            .find(|d| d.role == role)
            .and_then(|d| Some(value_text(props.get(d.name)?, d.kind, cx)))
    };
    let (sized, enabled, visible) = (
        get(PropRole::AddSized),
        get(PropRole::AddEnabled),
        get(PropRole::AddVisible),
    );
    match (sized, enabled, visible) {
        (None, None, None) => format!("{ident}.add({widget})"),
        (Some(s), None, None) => format!("{ident}.add_sized({s}, {widget})"),
        (None, Some(e), None) => format!("{ident}.add_enabled({e}, {widget})"),
        (None, None, Some(v)) => format!("{ident}.add_visible({v}, {widget})"),
        (sized, enabled, visible) => {
            let mut call = match sized {
                Some(s) => format!("{ident}.add_sized({s}, {widget})"),
                None => format!("{ident}.add({widget})"),
            };
            // egui 0.37 has `add_enabled_ui` but no `add_visible_ui`.
            if let Some(v) = visible {
                call = format!(
                    "{ident}.scope(|{ident}| {{ if !({v}) {{ {ident}.set_invisible(); }} {call} }}).inner"
                );
            }
            if let Some(e) = enabled {
                call = format!("{ident}.add_enabled_ui({e}, |{ident}| {call}).inner");
            }
            call
        }
    }
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
    let chain = chain_text(wdef.props, props, extra_chain, cx);
    let binding = props.get("value").map_or_else(
        || "&mut self.form.choice".to_string(),
        |v| value_text(v, PropKind::Expr, cx),
    );
    let mut items = String::new();
    if let Some(PropValue::Items(list)) = props.get("items") {
        for item in list {
            let label = value_text(&item.label, PropKind::Text, cx);
            items.push_str(&format!(
                " {ident}.selectable_value({binding}, {}, {label});",
                item.value
            ));
        }
    }
    format!(
        "{}{ty}::{ctor}({args_out}){chain}.show_ui({ident}, |{ident}| {{{items} }})",
        cx.path
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
        // The importer always keeps their heads; a move can't take them away.
        ContainerId::Flow | ContainerId::FlowGroup => String::new(),
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
                && cdef
                    .props
                    .iter()
                    .all(|d| d.role == PropRole::Arg || !props.contains_key(d.name));
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
            let chain = chain_text(cdef.props, props, extra_chain, cx);
            format!(
                "{}{ty}::{ctor}({args_out}){chain}.show({ident}, {closure}{{",
                cx.path
            )
        }
    }
}
