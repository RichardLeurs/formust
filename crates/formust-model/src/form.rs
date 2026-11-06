//! The form tree and the edits the designer makes to it.
//!
//! # Verbatim text
//!
//! A node's `src` is its original source text and is emitted unchanged until it is edited. Every
//! edit below clears `src` on the node *and on its ancestors*, whose text contained the old node.
//! A container whose `src` is gone still emits its own head and tail verbatim (see
//! [`ContainerShell`]) and its unedited children verbatim, so an edit regenerates only what it
//! touches. Fields are public for reading and for tools; change a tree through the methods here,
//! which keep that rule.

use std::collections::BTreeMap;

use crate::registry::{ContainerId, EventId, PropRole, WidgetId, container_def, widget_def};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// The properties set on a node, by registry name. Values equal to a registry default are not
/// stored for optional properties, so generated code stays minimal.
pub type Props = BTreeMap<&'static str, PropValue>;

#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    /// A value from the registry's enum list.
    Enum(&'static str),
    /// An sRGBA colour for light mode, and optionally another for dark mode. With no `dark` it is
    /// the colour in both.
    Color {
        light: [u8; 4],
        dark: Option<[u8; 4]>,
    },
    /// A non-literal value or binding, as source text: `self.name`, `format!("{x}")`.
    Expr(String),
    /// The static items of a ComboBox.
    Items(Vec<ComboItem>),
    /// A line of `width` points in a colour for light mode and optionally another for dark mode:
    /// `egui::Stroke::new(width, colour)`, the colour written as for [`PropValue::Color`].
    Stroke {
        width: f64,
        light: [u8; 4],
        dark: Option<[u8; 4]>,
    },
}

impl PropValue {
    /// One colour for both modes.
    pub fn color(rgba: [u8; 4]) -> PropValue {
        PropValue::Color {
            light: rgba,
            dark: None,
        }
    }

    /// A colour for each mode.
    pub fn color_pair(light: [u8; 4], dark: [u8; 4]) -> PropValue {
        PropValue::Color {
            light,
            dark: Some(dark),
        }
    }

    /// A stroke with one colour for both modes, or with a dark one too.
    pub fn stroke(width: f64, light: [u8; 4], dark: Option<[u8; 4]>) -> PropValue {
        PropValue::Stroke { width, light, dark }
    }
}

/// One `ui.selectable_value(&mut binding, value, label)` in a ComboBox.
#[derive(Clone, Debug, PartialEq)]
pub struct ComboItem {
    /// Source text of the value expression.
    pub value: String,
    /// [`PropValue::Text`] or [`PropValue::Expr`].
    pub label: PropValue,
}

/// A field of the generated `<Owner>Form` struct.
#[derive(Clone, Debug, PartialEq)]
pub struct StateField {
    pub name: String,
    /// Source text of the type.
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    /// The `Self` type of the impl, or empty for a free function.
    pub owner: String,
    pub method: String,
    /// Everything before the body's `{`, verbatim: attributes, doc comments, visibility, signature.
    pub sig: String,
    /// `impl Login ` up to the `{`, verbatim. Empty for a free function.
    pub impl_header: String,
    /// The text between the impl's `{` and `sig`.
    pub impl_pre: String,
    /// The text between the body's `}` and the impl's `}`.
    pub impl_post: String,
    pub root: Node,
    pub state: Vec<StateField>,
    /// The name of the generated state struct, `<Owner>Form` unless that name is taken.
    pub state_struct: String,
    /// The owner's field holding it, `form` unless that name is taken.
    pub state_field: String,
    /// The `<Owner>Form` struct as found in the source. New `state` fields are added to this text, so
    /// its attributes and comments survive.
    pub state_src: Option<String>,
    pub next_id: u32,
    /// `"\n"` or `"\r\n"`, for generated line breaks.
    pub newline: String,
    /// The path prefix of generated egui types: `egui::`, `eframe::egui::` or empty.
    pub egui_path: String,
    /// Set when this form came from a legacy `fn update(&mut self, ctx, frame)`: the name of the
    /// context parameter. Export generates `fn ui`, so the form isn't byte-identical to its source.
    pub legacy_ctx: Option<String>,
    /// Set for a dialog: a method of any name taking `ctx: &Context` whose body shows a `Window`.
    /// The root's ident is that `&Context`, so it only holds `Window`s, control flow and custom
    /// code; widgets go inside a `Window`. The signature is kept, unlike a legacy `update`.
    pub context_root: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: NodeId,
    /// Comments, blank lines and indentation between the previous statement and this one.
    pub leading: String,
    /// A comment on the same line, after this statement. It stays with the statement.
    pub trailing: String,
    /// The source has this as a block's final expression with no `;`, and it isn't block-like. If it
    /// moves away from the end of a block, it needs one.
    pub bare_tail: bool,
    /// The original text; `None` once edited (or for a new node), which regenerates it.
    pub src: Option<String>,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Widget {
        def: WidgetId,
        props: Props,
        events: Vec<(EventId, String)>,
        /// Unknown builder methods, verbatim: part of the widget's constructor chain.
        extra_chain: Vec<String>,
        /// Unknown methods on the `Response`, verbatim: `.on_hover_text("..")`.
        response_chain: Vec<String>,
        /// The `Ui` identifier in `src`. If the parent's differs after a move, `src` can't be reused.
        recv: String,
        /// `let label = ` when the response is bound, with `ImportOptions::extended`.
        let_prefix: Option<String>,
    },
    Container {
        def: ContainerId,
        props: Props,
        children: Vec<Node>,
        /// Text between the last child and the closing `}` of the body.
        closing: String,
        /// Unknown builder methods, verbatim.
        extra_chain: Vec<String>,
        /// The closure's `Ui` parameter, which the children use as their receiver.
        ident: String,
        /// The `Ui` identifier in the head.
        recv: String,
        /// The closure up to its `{`, verbatim: `|ui| ` or `move |inner: &mut Ui| `.
        closure: String,
        /// `false` for a body with no braces, `|ui| self.ui(ui)`: one child, and `head` stops before
        /// it. Emit adds braces if the child's text no longer fits there.
        braced: bool,
        /// Original head and tail. `None` for a new container.
        shell: Option<ContainerShell>,
    },
    /// Locked custom code, a byte slice of the source.
    Custom { text: String },
}

/// A container's own text, around its children. `head` goes when the container's own properties are
/// edited; `tail` stays (it is `});` or the like, with no properties in it).
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerShell {
    /// From the statement's start through the body's `{`.
    pub head: Option<String>,
    /// From the body's `}` to the statement's end.
    pub tail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    NoSuchNode(NodeId),
    /// The root can't be removed, moved or cloned into itself.
    IsRoot,
    /// The target parent isn't a container.
    NotAContainer(NodeId),
    /// A node can't move into its own subtree.
    IntoOwnSubtree,
    /// The registry has no such property on this node's definition.
    NoSuchProp(String),
    /// Custom code has no properties or events.
    IsCustom(NodeId),
    /// A branch of an `if` or an arm of a `match` can't be moved, removed or added to.
    Locked(NodeId),
    /// A dialog's root has a `&Context`, not a `Ui`: widgets go inside a `Window`.
    NeedsUi(NodeId),
}

impl Node {
    pub fn children(&self) -> &[Node] {
        match &self.kind {
            NodeKind::Container { children, .. } => children,
            _ => &[],
        }
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Node>> {
        match &mut self.kind {
            NodeKind::Container { children, .. } => Some(children),
            _ => None,
        }
    }

    /// This node and its descendants, parents first.
    pub fn walk(&self, f: &mut dyn FnMut(&Node)) {
        f(self);
        for child in self.children() {
            child.walk(f);
        }
    }

    fn find(&self, id: NodeId) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children().iter().find_map(|c| c.find(id))
    }

    /// The child indexes leading to `id`, if it is here.
    fn path_to(&self, id: NodeId, path: &mut Vec<usize>) -> bool {
        if self.id == id {
            return true;
        }
        for (i, child) in self.children().iter().enumerate() {
            path.push(i);
            if child.path_to(id, path) {
                return true;
            }
            path.pop();
        }
        false
    }

    /// The node at `path`, clearing `src` on every node passed, including the one returned.
    fn dirty_path(&mut self, path: &[usize]) -> &mut Node {
        self.src = None;
        match path.split_first() {
            None => self,
            Some((&i, rest)) => {
                self.children_mut().expect("a path runs through containers")[i].dirty_path(rest)
            }
        }
    }

    fn renumber(&mut self, next_id: &mut u32) {
        self.id = NodeId(*next_id);
        *next_id += 1;
        if let Some(children) = self.children_mut() {
            for child in children {
                child.renumber(next_id);
            }
        }
    }

    pub(crate) fn indent(&self) -> Option<&str> {
        self.leading.rfind('\n').map(|i| &self.leading[i + 1..])
    }
}

impl Form {
    /// An empty form with a plain `Ui` root, as for a new file.
    pub fn new(owner: &str, method: &str) -> Form {
        let root = Node {
            id: NodeId(0),
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: None,
            kind: NodeKind::Container {
                def: ContainerId::Root,
                props: Props::new(),
                children: Vec::new(),
                closing: "\n    ".to_string(),
                extra_chain: Vec::new(),
                ident: "ui".to_string(),
                recv: "ui".to_string(),
                closure: String::new(),
                braced: true,
                shell: None,
            },
        };
        Form {
            owner: owner.to_string(),
            method: method.to_string(),
            sig: format!("pub fn {method}(&mut self, ui: &mut egui::Ui) "),
            impl_header: format!("impl {owner} "),
            impl_pre: "\n    ".to_string(),
            impl_post: "\n".to_string(),
            root,
            state: Vec::new(),
            state_struct: Form::default_state_struct(owner),
            state_field: Form::DEFAULT_STATE_FIELD.to_string(),
            state_src: None,
            next_id: 1,
            newline: "\n".to_string(),
            egui_path: "egui::".to_string(),
            legacy_ctx: None,
            context_root: false,
        }
    }

    /// The field name when it is free.
    pub const DEFAULT_STATE_FIELD: &'static str = "form";

    /// The struct name when it is free.
    pub fn default_state_struct(owner: &str) -> String {
        format!("{owner}Form")
    }

    /// Whether the state names are the defaults, which a `.frm.rs` header doesn't need to spell out.
    pub fn state_names_are_default(&self) -> bool {
        self.state_struct == Form::default_state_struct(&self.owner)
            && self.state_field == Form::DEFAULT_STATE_FIELD
    }

    /// Form identity: `owner::method`.
    pub fn id(&self) -> String {
        format!("{}::{}", self.owner, self.method)
    }

    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn find(&self, id: NodeId) -> Option<&Node> {
        self.root.find(id)
    }

    /// The parent of `id`, with `id`'s index among its children.
    pub fn parent_of(&self, id: NodeId) -> Option<(NodeId, usize)> {
        let mut path = Vec::new();
        if !self.root.path_to(id, &mut path) {
            return None;
        }
        let (&last, front) = path.split_last()?;
        let mut node = &self.root;
        for &i in front {
            node = &node.children()[i];
        }
        Some((node.id, last))
    }

    fn path(&self, id: NodeId) -> Result<Vec<usize>, EditError> {
        let mut path = Vec::new();
        if self.root.path_to(id, &mut path) {
            Ok(path)
        } else {
            Err(EditError::NoSuchNode(id))
        }
    }

    /// Runs `f` on a node as an edit: its `src` and its ancestors' `src` are cleared. A container's
    /// head is regenerated too, so this suits changes to its own properties. Children keep theirs.
    pub fn edit(&mut self, id: NodeId, f: impl FnOnce(&mut Node)) -> Result<(), EditError> {
        let path = self.path(id)?;
        let node = self.root.dirty_path(&path);
        if let NodeKind::Container {
            def,
            shell: Some(shell),
            ..
        } = &mut node.kind
            && !def.is_flow()
            && *def != ContainerId::Root
        {
            shell.head = None;
        }
        f(node);
        Ok(())
    }

    /// Sets a property. A value equal to an optional property's default is removed instead.
    pub fn set_prop(&mut self, id: NodeId, name: &str, value: PropValue) -> Result<(), EditError> {
        let node = self.find(id).ok_or(EditError::NoSuchNode(id))?;
        // The column count is the columns themselves: they are added or removed with it.
        if let (
            NodeKind::Container {
                def: ContainerId::Columns,
                ..
            },
            "count",
            PropValue::Int(n),
        ) = (&node.kind, name, &value)
        {
            let n = usize::try_from(*n).unwrap_or(0);
            return self.set_columns(id, n);
        }
        // A table's columns and header are its cells and its first row.
        if let NodeKind::Container {
            def: ContainerId::Table,
            ..
        } = &node.kind
        {
            match (name, &value) {
                ("columns", PropValue::Items(items)) => {
                    let items = items.clone();
                    return self.set_table_columns_list(id, items);
                }
                ("header", PropValue::Float(h)) => {
                    let h = *h;
                    return self.set_table_header(id, Some(h));
                }
                _ => {}
            }
        }
        // The tabs are the pages: their count changes through `add_tab` and `remove_tab`.
        if let (NodeKind::Container { def, .. }, "tabs", PropValue::Items(items)) =
            (&node.kind, name, &value)
            && def.is_tabs()
        {
            let items = items.clone();
            return self.set_tabs(id, items);
        }
        let def = match &node.kind {
            NodeKind::Widget { def, .. } => widget_def(*def).prop(name),
            NodeKind::Container { def, .. } => container_def(*def).prop(name),
            NodeKind::Custom { .. } => return Err(EditError::IsCustom(id)),
        }
        .ok_or_else(|| EditError::NoSuchProp(name.to_string()))?;
        // `add_sized`, enabled and visible together nest as `ui.add_enabled_ui(..)` and
        // `ui.scope(..)` around the widget, which the importer reads back: any mix is allowed.
        let drop_it = def.role != PropRole::Arg && def.default.to_value().as_ref() == Some(&value);
        self.edit(id, |node| {
            let props = match &mut node.kind {
                NodeKind::Widget { props, .. } | NodeKind::Container { props, .. } => props,
                NodeKind::Custom { .. } => unreachable!("checked above"),
            };
            if drop_it {
                props.remove(def.name);
            } else {
                props.insert(def.name, value);
            }
        })
    }

    /// Removes a property, back to its default.
    pub fn clear_prop(&mut self, id: NodeId, name: &str) -> Result<(), EditError> {
        // A table without a header loses its header row.
        if name == "header"
            && let Some(NodeKind::Container {
                def: ContainerId::Table,
                ..
            }) = self.find(id).map(|n| &n.kind)
        {
            return self.set_table_header(id, None);
        }
        self.edit(id, |node| {
            if let NodeKind::Widget { props, .. } | NodeKind::Container { props, .. } =
                &mut node.kind
            {
                props.remove(name);
            }
        })
    }

    /// Sets the handler body of an event, adding the event if the widget hasn't got it.
    pub fn set_event(&mut self, id: NodeId, event: EventId, body: &str) -> Result<(), EditError> {
        match &self.find(id).ok_or(EditError::NoSuchNode(id))?.kind {
            NodeKind::Widget { .. } => {}
            NodeKind::Container { .. } | NodeKind::Custom { .. } => {
                return Err(EditError::IsCustom(id));
            }
        }
        self.edit(id, |node| {
            if let NodeKind::Widget { events, .. } = &mut node.kind {
                match events.iter_mut().find(|(e, _)| *e == event) {
                    Some((_, text)) => *text = body.to_string(),
                    None => events.push((event, body.to_string())),
                }
            }
        })
    }

    /// A new handler's default body: `self.on_<x>_clicked();`, with `<x>` taken from `name`.
    pub fn default_handler(name: &str, event: EventId) -> String {
        format!(" self.on_{}_{}(); ", snake(name), event.method())
    }

    /// A new widget with its default properties, not yet in the tree.
    pub fn new_widget(&mut self, def: WidgetId) -> Node {
        let mut props = Props::new();
        for prop in widget_def(def).props {
            let binds = prop.role == PropRole::Body && prop.default.to_value().is_some();
            if (prop.role == PropRole::Arg || binds)
                && let Some(value) = prop.default.to_value()
            {
                // The registry's bindings say `self.form`, which may be taken here.
                let value = match value {
                    PropValue::Expr(text) => PropValue::Expr(
                        text.replace("self.form.", &format!("self.{}.", self.state_field)),
                    ),
                    other => other,
                };
                props.insert(prop.name, value);
            }
        }
        Node {
            id: self.alloc_id(),
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: None,
            kind: NodeKind::Widget {
                def,
                props,
                events: Vec::new(),
                extra_chain: Vec::new(),
                response_chain: Vec::new(),
                recv: String::new(),
                let_prefix: None,
            },
        }
    }

    /// A new stateful widget bound to `self.form.<field>`, adding the field to the generated struct.
    /// Widgets with no state (see `WidgetDef::state`) get no field.
    pub fn new_bound_widget(&mut self, def: WidgetId, field: &str) -> Node {
        let mut node = self.new_widget(def);
        if let Some(state) = widget_def(def).state {
            // A second binding (a RangeSlider's `high`) is named after its registry default.
            let also = state.also.map(|prop| {
                let hint = match widget_def(def).prop(prop).map(|p| p.default) {
                    Some(crate::registry::DefaultValue::Expr(e)) => {
                        e.rsplit('.').next().unwrap_or(prop).to_string()
                    }
                    _ => prop.to_string(),
                };
                (prop, hint)
            });
            for (prop, hint) in std::iter::once((state.prop, field.to_string())).chain(also) {
                let name = self.unique_field(&hint);
                self.add_state_field(&name, state.ty);
                if let NodeKind::Widget { props, .. } = &mut node.kind {
                    props.insert(
                        prop,
                        PropValue::Expr(format!("&mut self.{}.{name}", self.state_field)),
                    );
                }
            }
        }
        node
    }

    /// Adds a field to the generated state struct. A type starting `egui::` is written with the
    /// form's own egui path (`eframe::egui::Color32`), so it resolves where the form's code does.
    pub(crate) fn add_state_field(&mut self, name: &str, ty: &str) {
        let ty = match ty.strip_prefix("egui::") {
            Some(rest) => format!("{}{rest}", self.egui_path),
            None => ty.to_string(),
        };
        self.state.push(StateField {
            name: name.to_string(),
            ty,
        });
    }

    pub(crate) fn unique_field(&self, wanted: &str) -> String {
        let base = snake(wanted);
        let taken = |name: &str| self.state.iter().any(|f| f.name == name);
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|i| format!("{base}_{i}"))
            .find(|n| !taken(n))
            .expect("an unbounded range")
    }

    /// A new, empty container with its default properties, not yet in the tree. Columns get their
    /// two columns and Sides their two closures.
    pub fn new_container(&mut self, def: ContainerId) -> Node {
        let mut node = self.bare_container(def);
        let parts: Vec<Node> = match def {
            ContainerId::Columns => {
                if let NodeKind::Container { ident, closure, .. } = &mut node.kind {
                    *ident = "cols".to_string();
                    *closure = "|cols| ".to_string();
                }
                (0..2).map(|i| self.new_column("cols", i)).collect()
            }
            ContainerId::Tabs | ContainerId::TabsVertical => {
                let field = match container_def(def).prop("field").map(|p| p.default) {
                    Some(crate::registry::DefaultValue::Expr(e)) => e.to_string(),
                    _ => "self.form.tab".to_string(),
                }
                .replace("self.form.", &format!("self.{}.", self.state_field));
                let tabs = vec![
                    ComboItem {
                        value: "0".to_string(),
                        label: PropValue::Text("Tab 1".to_string()),
                    },
                    ComboItem {
                        value: "1".to_string(),
                        label: PropValue::Text("Tab 2".to_string()),
                    },
                ];
                if let NodeKind::Container { props, .. } = &mut node.kind {
                    props.insert("field", PropValue::Expr(field));
                    props.insert("tabs", PropValue::Items(tabs));
                }
                (0..2).map(|_| self.new_page(def)).collect()
            }
            ContainerId::Table => {
                let columns: Vec<ComboItem> =
                    (0..2).map(|_| crate::tables::column_item()).collect();
                if let NodeKind::Container { props, .. } = &mut node.kind {
                    props.insert("columns", PropValue::Items(columns));
                    props.insert("header", PropValue::Float(20.0));
                }
                (0..3).map(|_| self.new_table_row(2)).collect()
            }
            ContainerId::Sides => (0..2)
                .map(|i| {
                    let mut side = self.bare_container(ContainerId::Side);
                    if let NodeKind::Container { recv, .. } = &mut side.kind {
                        *recv = "ui".to_string();
                    }
                    if i == 1 {
                        side.leading = ", ".to_string();
                    }
                    side
                })
                .collect(),
            _ => Vec::new(),
        };
        if let NodeKind::Container { children, .. } = &mut node.kind {
            *children = parts;
        }
        node
    }

    /// Column `index` of a Columns whose closure names its slice `cols`: its children use
    /// `cols[index]`.
    pub(crate) fn new_column(&mut self, cols: &str, index: usize) -> Node {
        let mut column = self.bare_container(ContainerId::Column);
        if let NodeKind::Container {
            ident,
            recv,
            closure,
            ..
        } = &mut column.kind
        {
            *ident = format!("{cols}[{index}]");
            *recv = cols.to_string();
            closure.clear();
        }
        column
    }

    pub(crate) fn bare_container(&mut self, def: ContainerId) -> Node {
        let mut props = Props::new();
        for prop in container_def(def).props {
            if prop.role == PropRole::Arg
                && let Some(value) = prop.default.to_value()
            {
                props.insert(prop.name, value);
            }
        }
        Node {
            id: self.alloc_id(),
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: None,
            kind: NodeKind::Container {
                def,
                props,
                children: Vec::new(),
                closing: String::new(),
                extra_chain: Vec::new(),
                ident: "ui".to_string(),
                recv: String::new(),
                closure: "|ui| ".to_string(),
                braced: true,
                shell: None,
            },
        }
    }

    /// Inserts `node` under `parent` at `index` (clamped). Its layout whitespace is fitted to the new
    /// place; any comments in its `leading` stay.
    pub fn insert(
        &mut self,
        parent: NodeId,
        index: usize,
        mut node: Node,
    ) -> Result<(), EditError> {
        let path = self.path(parent)?;
        let is_container = matches!(
            self.find(parent).map(|n| &n.kind),
            Some(NodeKind::Container { .. })
        );
        if !is_container {
            return Err(EditError::NotAContainer(parent));
        }
        if self.is_group(parent) {
            return Err(EditError::Locked(parent));
        }
        if !self.fits(parent, &node) {
            return Err(EditError::NeedsUi(parent));
        }
        self.fresh_columns_name(parent, &mut node);
        let newline = self.newline.clone();
        let root_indent = self.root_child_indent();
        let returns = self.returns_value();
        // A column or side has no line of its own: its statements are indented from its holder's.
        let holder_indent = match self.find(parent).map(|n| &n.kind) {
            Some(NodeKind::Container {
                def: ContainerId::Column | ContainerId::Side,
                ..
            }) => self
                .parent_of(parent)
                .and_then(|(h, _)| self.find(h))
                .map(|h| h.indent().unwrap_or("").to_string()),
            // A page is an arm of the Tabs' `match`: one step in from the `match`, which a
            // vertical bar puts two closures deeper.
            Some(NodeKind::Container {
                def: ContainerId::TabPage,
                ..
            }) => self
                .parent_of(parent)
                .and_then(|(h, _)| self.find(h))
                .map(|h| {
                    let deeper = match &h.kind {
                        NodeKind::Container {
                            def: ContainerId::TabsVertical,
                            ..
                        } => "            ",
                        _ => "    ",
                    };
                    format!("{}{deeper}", h.indent().unwrap_or(""))
                }),
            // A cell's statements: one step in from its `col(|ui| {` line, which is one step in
            // from a header's `.header(..)` line and two from a body row's table.
            Some(NodeKind::Container {
                def: ContainerId::TableCell,
                ..
            }) => self.table_cell_indent(parent),
            _ => None,
        };
        if let Some(indent) = &holder_indent
            && let Some((holder, _)) = self.parent_of(parent)
        {
            // An empty Columns closes on a line of its own once it has a statement.
            let holder_path = self.path(holder)?;
            let h = self.root.dirty_path(&holder_path);
            if let NodeKind::Container {
                def: ContainerId::Columns,
                closing,
                ..
            } = &mut h.kind
                && !closing.contains('\n')
            {
                *closing = format!("{newline}{indent}");
            }
        }
        let parent_node = self.root.dirty_path(&path);
        let parent_is_root = path.is_empty();
        let mut index = index.min(parent_node.children().len());
        // The tail expression of a function that returns a value stays last: it is the result.
        if parent_is_root
            && returns
            && index == parent_node.children().len()
            && parent_node.children().last().is_some_and(|n| n.bare_tail)
        {
            index -= 1;
        }
        fit_layout(
            parent_node,
            parent_is_root,
            &root_indent,
            holder_indent.as_deref(),
            &newline,
            index,
            &mut node,
        );
        parent_node
            .children_mut()
            .expect("checked above")
            .insert(index, node);
        Ok(())
    }

    /// A new Columns (no source text) inside another Columns' column must not name its slice like
    /// the outer one: the inner name would hide the outer, and the outer's statements would no longer
    /// read as one column each. It becomes `cols2`, `cols3`, ...
    fn fresh_columns_name(&self, parent: NodeId, node: &mut Node) {
        let NodeKind::Container {
            def: ContainerId::Columns,
            ident,
            closure,
            children,
            shell: None,
            ..
        } = &mut node.kind
        else {
            return;
        };
        let mut taken = Vec::new();
        let mut at = Some(parent);
        while let Some(id) = at {
            if let Some(NodeKind::Container {
                def: ContainerId::Columns,
                ident,
                ..
            }) = self.find(id).map(|n| &n.kind)
            {
                taken.push(ident.clone());
            }
            at = self.parent_of(id).map(|(p, _)| p);
        }
        if !taken.contains(ident) {
            return;
        }
        let name = (2..)
            .map(|k| format!("cols{k}"))
            .find(|n| !taken.contains(n))
            .expect("an unbounded range");
        *closure = format!("|{name}| ");
        for (i, column) in children.iter_mut().enumerate() {
            if let NodeKind::Container { ident, recv, .. } = &mut column.kind {
                *ident = format!("{name}[{i}]");
                *recv = name.clone();
            }
        }
        *ident = name;
    }

    /// Does the form's function return a value (`-> T` in its signature)? Then a body's final
    /// expression is that value.
    pub fn returns_value(&self) -> bool {
        let sig = self.sig.trim_end();
        sig.rfind(')')
            .is_some_and(|close| sig[close..].contains("->"))
    }

    /// A holder of parts that can't be moved or removed on their own: the branches of an `if` or
    /// `match`, the columns of a Columns, the two closures of a Sides.
    fn is_group(&self, id: NodeId) -> bool {
        matches!(
            self.find(id).map(|n| &n.kind),
            Some(NodeKind::Container { def, .. }) if def.is_group()
        )
    }

    /// The indent of the root's children when it has none yet: the signature's, plus four spaces.
    fn root_child_indent(&self) -> String {
        let tail = match self.impl_pre.rfind('\n') {
            Some(i) => &self.impl_pre[i + 1..],
            None => "",
        };
        format!("{tail}    ")
    }

    /// Removes a node and returns it. The user asked to delete it, so its comments go with it.
    pub fn remove(&mut self, id: NodeId) -> Result<Node, EditError> {
        let path = self.path(id)?;
        let Some((&last, front)) = path.split_last() else {
            return Err(EditError::IsRoot);
        };
        if self
            .parent_of(id)
            .is_some_and(|(parent, _)| self.is_group(parent))
        {
            return Err(EditError::Locked(id));
        }
        let parent = self.root.dirty_path(front);
        Ok(parent
            .children_mut()
            .expect("a path runs through containers")
            .remove(last))
    }

    /// Moves a node to `index` under `new_parent`, reparenting if needed. `index` is its position
    /// after the move. Its `src` stays: moving doesn't change a statement's text.
    pub fn move_node(
        &mut self,
        id: NodeId,
        new_parent: NodeId,
        index: usize,
    ) -> Result<(), EditError> {
        if id == self.root.id {
            return Err(EditError::IsRoot);
        }
        let moving = self.find(id).ok_or(EditError::NoSuchNode(id))?;
        if moving.find(new_parent).is_some() {
            return Err(EditError::IntoOwnSubtree);
        }
        match self.find(new_parent).map(|n| &n.kind) {
            None => return Err(EditError::NoSuchNode(new_parent)),
            Some(NodeKind::Container { .. }) => {}
            Some(_) => return Err(EditError::NotAContainer(new_parent)),
        }
        if self.is_group(new_parent) {
            return Err(EditError::Locked(new_parent));
        }
        // Checked before the node leaves, so a refused move loses nothing.
        if !self.fits(new_parent, moving) {
            return Err(EditError::NeedsUi(new_parent));
        }
        let node = self.remove(id)?;
        self.insert(new_parent, index, node)
    }

    /// A deep copy of a node's subtree with fresh ids, not yet in the tree. The text stays verbatim.
    pub fn clone_subtree(&mut self, id: NodeId) -> Result<Node, EditError> {
        let copy = self.find(id).ok_or(EditError::NoSuchNode(id))?.clone();
        Ok(self.adopt(copy))
    }

    /// A node from elsewhere (another form, a paste) with fresh ids of this form, ready to insert.
    pub fn adopt(&mut self, mut node: Node) -> Node {
        node.renumber(&mut self.next_id);
        node
    }

    /// Removes a widget's handler for `event`, if it has one.
    pub fn remove_event(&mut self, id: NodeId, event: EventId) -> Result<(), EditError> {
        match &self.find(id).ok_or(EditError::NoSuchNode(id))?.kind {
            NodeKind::Widget { events, .. } if events.iter().any(|(e, _)| *e == event) => {}
            NodeKind::Widget { .. } => return Ok(()),
            NodeKind::Container { .. } | NodeKind::Custom { .. } => {
                return Err(EditError::IsCustom(id));
            }
        }
        self.edit(id, |node| {
            if let NodeKind::Widget { events, .. } = &mut node.kind {
                events.retain(|(e, _)| *e != event);
            }
        })
    }

    /// Can `parent` hold widgets and `Ui` containers? Not a dialog's root, nor control flow sitting
    /// directly in it: their ident is a `&Context` (see [`Form::context_root`]).
    pub fn takes_widgets(&self, parent: NodeId) -> bool {
        if !self.context_root {
            return true;
        }
        let mut current = parent;
        loop {
            if current == self.root.id {
                return false;
            }
            match self.find(current).map(|n| &n.kind) {
                Some(NodeKind::Container { def, .. }) if def.is_flow() => {}
                _ => return true,
            }
            match self.parent_of(current) {
                Some((up, _)) => current = up,
                None => return true,
            }
        }
    }

    /// Can `node` go under `parent`? Where [`Form::takes_widgets`] is false only a `Window`, a
    /// dialog helper call, control flow and custom code can.
    pub fn fits(&self, parent: NodeId, node: &Node) -> bool {
        self.takes_widgets(parent)
            || matches!(
                &node.kind,
                NodeKind::Custom { .. }
                    | NodeKind::Container {
                        def: ContainerId::Window
                            | ContainerId::Modal
                            | ContainerId::DialogCall
                            | ContainerId::Flow
                            | ContainerId::FlowGroup,
                        ..
                    }
            )
    }

    /// Where new widgets go by default: the root, or in a dialog its first `Window` or dialog helper
    /// call (the root itself when it has none).
    pub fn widget_root(&self) -> NodeId {
        fn first_window(n: &Node) -> Option<NodeId> {
            n.children().iter().find_map(|c| match &c.kind {
                NodeKind::Container {
                    def: ContainerId::Window | ContainerId::Modal | ContainerId::DialogCall,
                    ..
                } => Some(c.id),
                NodeKind::Container { def, .. } if def.is_flow() => first_window(c),
                _ => None,
            })
        }
        if self.context_root {
            first_window(&self.root).unwrap_or(self.root.id)
        } else {
            self.root.id
        }
    }

    /// Can `id` take children here: a container that isn't the locked holder of `if`/`match` branches.
    pub fn accepts_children(&self, id: NodeId) -> bool {
        matches!(self.find(id).map(|n| &n.kind), Some(NodeKind::Container { def, .. }) if !def.is_group())
    }

    /// Is `id` a branch of an `if` chain or `match`, a column or a side, which can't be moved or
    /// removed on its own?
    pub fn is_locked_branch(&self, id: NodeId) -> bool {
        self.parent_of(id)
            .is_some_and(|(parent, _)| self.is_group(parent))
    }

    /// Every node id, parents first.
    pub fn node_ids(&self) -> Vec<NodeId> {
        let mut ids = Vec::new();
        self.root.walk(&mut |n| ids.push(n.id));
        ids
    }
}

/// Fits a node's whitespace to its new place: the indent of its new siblings, and a new line if they
/// are on separate lines. Only the last line of `leading` is touched, so comments survive; the
/// node's own lines are left alone, because reindenting text can change multi-line strings.
fn fit_layout(
    parent: &mut Node,
    parent_is_root: bool,
    root_indent: &str,
    parent_indent_override: Option<&str>,
    newline: &str,
    index: usize,
    node: &mut Node,
) {
    let siblings = parent.children();
    let neighbour = siblings
        .get(index)
        .or_else(|| index.checked_sub(1).and_then(|i| siblings.get(i)));
    let indent: Option<String> = match neighbour {
        Some(sibling) => sibling.indent().map(str::to_string),
        None => {
            let base = if parent_is_root {
                root_indent.to_string()
            } else {
                let own = parent.indent().unwrap_or("");
                format!("{}    ", parent_indent_override.unwrap_or(own))
            };
            Some(base)
        }
    };
    match indent {
        Some(indent) => match node.leading.rfind('\n') {
            Some(i) => {
                node.leading.truncate(i + 1);
                node.leading.push_str(&indent);
            }
            None => {
                let comment = node.leading.trim().to_string();
                node.leading = if comment.is_empty() {
                    format!("{newline}{indent}")
                } else {
                    format!("{newline}{indent}{comment}{newline}{indent}")
                };
            }
        },
        // Siblings on one line: keep this node on it too.
        None => {
            if node.leading.is_empty() {
                node.leading.push(' ');
            }
        }
    }
    // An empty body on one line (`|ui| {}`) gets its closing brace on a line of its own.
    let parent_indent = if parent_is_root {
        root_indent.strip_suffix("    ").unwrap_or("").to_string()
    } else {
        let own = parent.indent().unwrap_or("");
        parent_indent_override.unwrap_or(own).to_string()
    };
    let empty = siblings.is_empty();
    if let NodeKind::Container { closing, .. } = &mut parent.kind
        && empty
        && !closing.contains('\n')
    {
        *closing = format!("{newline}{parent_indent}");
    }
}

/// `Save file!` to `save_file`: a usable identifier part.
pub(crate) fn snake(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    match out.chars().next() {
        None => "item".to_string(),
        Some(c) if c.is_numeric() => format!("item_{out}"),
        Some(_) => out,
    }
}

impl ContainerId {
    /// Control flow: its head is locked source text, with no generator behind it.
    pub fn is_flow(self) -> bool {
        matches!(self, ContainerId::Flow | ContainerId::FlowGroup)
    }

    /// Holds fixed parts (branches, columns, sides, tab pages) rather than free children.
    pub fn is_group(self) -> bool {
        matches!(
            self,
            ContainerId::FlowGroup
                | ContainerId::Columns
                | ContainerId::Sides
                | ContainerId::Tabs
                | ContainerId::TabsVertical
                | ContainerId::Table
                | ContainerId::TableRow
        )
    }

    /// One of those parts, whose place is fixed.
    pub fn is_part(self) -> bool {
        matches!(
            self,
            ContainerId::Column
                | ContainerId::Side
                | ContainerId::TabPage
                | ContainerId::TableRow
                | ContainerId::TableCell
        )
    }

    /// Tabs, with the bar above or to the left of the pages.
    pub fn is_tabs(self) -> bool {
        matches!(self, ContainerId::Tabs | ContainerId::TabsVertical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ALL_CONTAINERS, ALL_WIDGETS, DefaultValue, PropKind};

    fn sample() -> (Form, NodeId, NodeId, NodeId) {
        let mut form = Form::new("Login", "ui");
        let root = form.root.id;
        let row = form.new_container(ContainerId::Horizontal);
        let row_id = row.id;
        form.insert(root, 0, row).unwrap();
        let a = form.new_widget(WidgetId::Label);
        let a_id = a.id;
        form.insert(row_id, 0, a).unwrap();
        let b = form.new_widget(WidgetId::Button);
        let b_id = b.id;
        form.insert(root, 1, b).unwrap();
        (form, row_id, a_id, b_id)
    }

    #[test]
    fn every_widget_and_container_has_its_own_registry_entry() {
        for &id in ALL_WIDGETS {
            assert_eq!(widget_def(id).id, id);
        }
        for &id in ALL_CONTAINERS {
            assert_eq!(container_def(id).id, id);
        }
    }

    #[test]
    fn the_registry_is_consistent() {
        for &id in ALL_WIDGETS {
            let def = widget_def(id);
            for name in def.size_props {
                assert!(def.prop(name).is_some(), "{}: size prop {name}", def.name);
            }
            if let Some(state) = def.state {
                assert!(def.prop(state.prop).is_some(), "{}: state prop", def.name);
            }
        }
        for &id in ALL_CONTAINERS {
            let def = container_def(id);
            for name in def.size_props {
                assert!(def.prop(name).is_some(), "{}: size prop {name}", def.name);
            }
        }
        // An enum's default is one of its values.
        let all = ALL_WIDGETS.iter().flat_map(|&w| widget_def(w).props.iter());
        let all = all.chain(
            ALL_CONTAINERS
                .iter()
                .flat_map(|&c| container_def(c).props.iter()),
        );
        for prop in all {
            if let (PropKind::Enum { values, .. }, DefaultValue::Enum(v)) =
                (prop.kind, prop.default)
            {
                assert!(values.contains(&v), "{}: default {v}", prop.name);
            }
        }
    }

    #[test]
    fn find_and_parent() {
        let (form, row, a, b) = sample();
        assert!(form.find(a).is_some());
        assert!(form.find(NodeId(999)).is_none());
        assert_eq!(form.parent_of(a), Some((row, 0)));
        assert_eq!(form.parent_of(b), Some((form.root.id, 1)));
        assert_eq!(form.parent_of(form.root.id), None);
    }

    #[test]
    fn an_edit_clears_src_on_the_node_and_its_ancestors_only() {
        let (mut form, row, a, b) = sample();
        for id in [form.root.id, row, a, b] {
            let path = form.path(id).unwrap();
            let mut node = &mut form.root;
            for &i in &path {
                node = &mut node.children_mut().unwrap()[i];
            }
            node.src = Some("text".to_string());
        }
        form.set_prop(a, "text", PropValue::Text("New".to_string()))
            .unwrap();
        assert_eq!(form.find(a).unwrap().src, None);
        assert_eq!(form.find(row).unwrap().src, None);
        assert_eq!(form.root.src, None);
        assert_eq!(form.find(b).unwrap().src.as_deref(), Some("text"));
    }

    #[test]
    fn an_edit_to_a_child_keeps_the_container_head() {
        let (mut form, row, a, _) = sample();
        let shell = ContainerShell {
            head: Some("ui.horizontal(|ui| {".to_string()),
            tail: "});".to_string(),
        };
        form.edit(row, |n| {
            if let NodeKind::Container { shell: s, .. } = &mut n.kind {
                *s = Some(shell.clone());
            }
        })
        .unwrap();
        form.set_prop(a, "text", PropValue::Text("New".to_string()))
            .unwrap();
        let NodeKind::Container { shell: Some(s), .. } = &form.find(row).unwrap().kind else {
            panic!()
        };
        assert!(s.head.is_some());
        form.edit(row, |_| {}).unwrap();
        let NodeKind::Container { shell: Some(s), .. } = &form.find(row).unwrap().kind else {
            panic!()
        };
        assert!(s.head.is_none());
        assert_eq!(s.tail, "});");
    }

    #[test]
    fn a_default_value_on_an_optional_prop_is_not_stored() {
        let (mut form, _, _, b) = sample();
        form.set_prop(b, "selected", PropValue::Bool(true)).unwrap();
        let has = |form: &Form| matches!(&form.find(b).unwrap().kind, NodeKind::Widget { props, .. } if props.contains_key("selected"));
        assert!(has(&form));
        form.set_prop(b, "selected", PropValue::Bool(false))
            .unwrap();
        assert!(!has(&form));
    }

    /// Size, enabled and visible go together (they nest in the export; see `emit::wrap_add`).
    #[test]
    fn a_widget_takes_several_add_wrappers() {
        let (mut form, _, _, b) = sample();
        form.set_prop(b, "enabled", PropValue::Bool(false)).unwrap();
        form.set_prop(b, "add_sized", PropValue::Expr("[1.0, 1.0]".to_string()))
            .unwrap();
        form.set_prop(b, "visible", PropValue::Expr("self.on".to_string()))
            .unwrap();
        let NodeKind::Widget { props, .. } = &form.find(b).unwrap().kind else {
            panic!()
        };
        assert_eq!(props.len(), 4, "{props:?}");
        form.clear_prop(b, "enabled").unwrap();
    }

    #[test]
    fn unknown_props_and_custom_nodes_are_refused() {
        let (mut form, _, a, _) = sample();
        assert_eq!(
            form.set_prop(a, "nonsense", PropValue::Bool(true)),
            Err(EditError::NoSuchProp("nonsense".to_string()))
        );
        let id = form.alloc_id();
        let custom = Node {
            id,
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: Some("x();".to_string()),
            kind: NodeKind::Custom {
                text: "x();".to_string(),
            },
        };
        form.insert(form.root.id, 0, custom).unwrap();
        assert_eq!(
            form.set_prop(id, "text", PropValue::Bool(true)),
            Err(EditError::IsCustom(id))
        );
        assert_eq!(
            form.set_event(id, EventId::Clicked, ""),
            Err(EditError::IsCustom(id))
        );
    }

    #[test]
    fn move_reparents_and_reorders() {
        let (mut form, row, a, b) = sample();
        form.move_node(b, row, 0).unwrap();
        assert_eq!(form.parent_of(b), Some((row, 0)));
        assert_eq!(form.parent_of(a), Some((row, 1)));
        form.move_node(b, row, 1).unwrap();
        assert_eq!(form.parent_of(a), Some((row, 0)));
        assert_eq!(form.parent_of(b), Some((row, 1)));
        form.move_node(a, form.root.id, 99).unwrap();
        assert_eq!(form.parent_of(a), Some((form.root.id, 1)));
    }

    #[test]
    fn illegal_moves_are_refused_and_change_nothing() {
        let (mut form, row, a, b) = sample();
        let before = form.clone();
        assert_eq!(form.move_node(row, a, 0), Err(EditError::IntoOwnSubtree));
        assert_eq!(form.move_node(row, row, 0), Err(EditError::IntoOwnSubtree));
        assert_eq!(form.move_node(a, b, 0), Err(EditError::NotAContainer(b)));
        assert_eq!(form.move_node(form.root.id, row, 0), Err(EditError::IsRoot));
        assert_eq!(form.remove(form.root.id).err(), Some(EditError::IsRoot));
        assert_eq!(
            form.move_node(NodeId(99), row, 0),
            Err(EditError::NoSuchNode(NodeId(99)))
        );
        assert_eq!(
            form.insert(a, 0, before.root.clone()),
            Err(EditError::NotAContainer(a))
        );
        assert_eq!(form, before);
    }

    #[test]
    fn branches_of_a_flow_group_are_locked_but_their_bodies_are_not() {
        let (mut form, _, a, _) = sample();
        let mut group = form.new_container(ContainerId::FlowGroup);
        let branch = form.new_container(ContainerId::Flow);
        let (group_id, branch_id) = (group.id, branch.id);
        if let NodeKind::Container { children, .. } = &mut group.kind {
            children.push(branch);
        }
        form.insert(form.root.id, 0, group).unwrap();
        assert_eq!(
            form.move_node(a, group_id, 0),
            Err(EditError::Locked(group_id))
        );
        assert_eq!(
            form.remove(branch_id).err(),
            Some(EditError::Locked(branch_id))
        );
        form.move_node(a, branch_id, 0).unwrap();
        assert_eq!(form.parent_of(a), Some((branch_id, 0)));
    }

    #[test]
    fn an_edit_keeps_a_flow_head_because_nothing_can_regenerate_it() {
        let (mut form, _, _, _) = sample();
        let mut flow = form.new_container(ContainerId::Flow);
        let id = flow.id;
        if let NodeKind::Container { shell, .. } = &mut flow.kind {
            *shell = Some(ContainerShell {
                head: Some("if x {".to_string()),
                tail: "}".to_string(),
            });
        }
        form.insert(form.root.id, 0, flow).unwrap();
        form.edit(id, |_| {}).unwrap();
        let NodeKind::Container { shell: Some(s), .. } = &form.find(id).unwrap().kind else {
            panic!()
        };
        assert_eq!(s.head.as_deref(), Some("if x {"));
    }

    #[test]
    fn a_moved_node_keeps_its_id_and_comment() {
        let (mut form, row, a, _) = sample();
        form.edit(a, |n| n.leading = "\n    // about a\n    ".to_string())
            .unwrap();
        form.move_node(a, form.root.id, 0).unwrap();
        let node = form.find(a).unwrap();
        assert!(node.leading.contains("// about a"), "{:?}", node.leading);
        assert!(form.find(row).unwrap().children().is_empty());
    }

    #[test]
    fn a_clone_has_fresh_unique_ids_and_the_same_text() {
        let (mut form, row, a, _) = sample();
        form.edit(a, |n| n.trailing = " // t".to_string()).unwrap();
        let copy = form.clone_subtree(row).unwrap();
        assert_ne!(copy.id, row);
        assert_eq!(copy.children()[0].trailing, " // t");
        form.insert(form.root.id, 0, copy).unwrap();
        let mut ids = form.node_ids();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count, "ids must stay unique");
    }

    #[test]
    fn new_bound_widgets_get_unique_fields_of_the_right_type() {
        let mut form = Form::new("Login", "ui");
        let a = form.new_bound_widget(WidgetId::TextEditSingleline, "Email address");
        let b = form.new_bound_widget(WidgetId::TextEditSingleline, "Email address");
        form.new_bound_widget(WidgetId::Checkbox, "remember");
        let d = form.new_bound_widget(WidgetId::Label, "ignored");
        assert_eq!(
            form.state
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["email_address", "email_address_2", "remember"]
        );
        assert_eq!(form.state[2].ty, "bool");
        let value = |n: &Node| match &n.kind {
            NodeKind::Widget { props, .. } => props.get("value").cloned(),
            _ => None,
        };
        assert_eq!(
            value(&a),
            Some(PropValue::Expr("&mut self.form.email_address".to_string()))
        );
        assert_eq!(
            value(&b),
            Some(PropValue::Expr(
                "&mut self.form.email_address_2".to_string()
            ))
        );
        assert_eq!(value(&d), None);
    }

    #[test]
    fn inserting_fits_the_new_nodes_indent_to_its_siblings() {
        let mut form = Form::new("F", "ui");
        let root = form.root.id;
        let a = form.new_widget(WidgetId::Label);
        form.insert(root, 0, a).unwrap();
        assert_eq!(form.root.children()[0].leading, "\n        ");
        let b = form.new_widget(WidgetId::Label);
        form.insert(root, 1, b).unwrap();
        assert_eq!(form.root.children()[1].leading, "\n        ");
    }

    #[test]
    fn an_adopted_node_gets_fresh_ids_and_events_can_be_removed() {
        let (mut form, row, _, b) = sample();
        let mut other = Form::new("Other", "ui");
        let foreign = other.new_container(ContainerId::Vertical);
        let adopted = form.adopt(foreign);
        assert!(form.find(adopted.id).is_none() && adopted.id.0 >= 4);
        form.insert(row, 0, adopted).unwrap();
        form.set_event(b, EventId::Clicked, " go(); ").unwrap();
        form.set_event(b, EventId::Hovered, " hover(); ").unwrap();
        form.remove_event(b, EventId::Clicked).unwrap();
        let NodeKind::Widget { events, .. } = &form.find(b).unwrap().kind else {
            panic!()
        };
        assert_eq!(events, &vec![(EventId::Hovered, " hover(); ".to_string())]);
        assert_eq!(
            form.remove_event(row, EventId::Clicked),
            Err(EditError::IsCustom(row))
        );
        assert!(form.accepts_children(row) && !form.accepts_children(b));
        assert!(!form.is_locked_branch(b));
    }

    /// Adding at the end of a function that returns its last expression puts the new node before
    /// that expression, so the function still returns it.
    #[test]
    fn the_result_expression_stays_last() {
        let mut form = Form::new("F", "ui");
        form.sig = "pub fn ui(&mut self, ui: &mut egui::Ui) -> egui::Response ".to_string();
        let mut tail = form.new_widget(WidgetId::Label);
        tail.bare_tail = true;
        let tail_id = tail.id;
        form.insert(form.root.id, 0, tail).unwrap();
        let b = form.new_widget(WidgetId::Button);
        let b_id = b.id;
        form.insert(form.root.id, usize::MAX, b).unwrap();
        assert_eq!(form.parent_of(b_id).unwrap().1, 0);
        assert_eq!(form.parent_of(tail_id).unwrap().1, 1);
        form.sig = "pub fn ui(&mut self, ui: &mut egui::Ui) ".to_string();
        assert!(!form.returns_value());
    }

    #[test]
    fn default_handler_uses_the_widgets_name() {
        assert_eq!(
            Form::default_handler("Save file!", EventId::Clicked),
            " self.on_save_file_clicked(); "
        );
        assert_eq!(
            Form::default_handler("", EventId::Changed),
            " self.on_item_changed(); "
        );
    }
}
