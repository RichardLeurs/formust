//! Source text to form models. See "Importer rules" in `docs/specs.md`.
//!
//! All text in the model is a byte slice of the source, found through `proc_macro2` spans. Every
//! statement's text plus the gaps between statements must add up to the body byte for byte; this is
//! checked for every form, and a form that fails the check becomes one custom block.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::str::FromStr;

use formust_model::{
    ALL_CONTAINERS, ALL_WIDGETS, ComboItem, ContainerId, ContainerShell, EventId, Form, Node,
    NodeId, NodeKind, PropDef, PropKind, PropRole, PropValue, Props, StateField, WidgetId,
    container_def, widget_def,
};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{Expr, ExprMethodCall, FnArg, ImplItem, Item, Lit, Pat, ReturnType, Stmt, Type, UnOp};

use crate::forms::{self, LEGACY_PANEL_METHODS, LEGACY_PANELS};

/// How deep brackets may nest before the input is refused, not parsed: `syn` recurses on them.
pub const MAX_DEPTH: usize = 128;
/// How many tokens may run with no `,`, `;` or brace between them before the input is refused:
/// `syn` also recurses on unbracketed chains such as `- - - x`.
pub const MAX_RUN: usize = 4096;
/// `syn` recursion stays within this, given the two limits above.
const STACK_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct ImportOptions {
    /// Also treat free functions taking a `ui` (and a free legacy `update`/`ui`) as forms, with an
    /// empty owner. The spec's forms are methods, so this is off by default; the recognition
    /// measurement turns it on. A free dialog (a `&Context` function showing a `Window` or calling
    /// a dialog helper, such as the app's own About window) is a form either way.
    pub free_functions: bool,
    /// Read as a `.frm.rs`, a standalone copy of its source: forms are found as in a `.rs`, and the
    /// top-level items and the structs that can be a form's state are kept for the header lines and
    /// the state (see [`crate::frm`]).
    pub frm: bool,
    /// Go beyond the spec's importer table, which makes these custom: `if`/`for`/`while`/`loop`/
    /// `match` holding recognised statements become locked-head containers, and `let x = <widget>;`
    /// a widget. Off by default until the spec decides; see Risk 2.
    pub extended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportError {
    pub message: String,
    /// 1-based, as editors count.
    pub line: usize,
    /// 1-based.
    pub column: usize,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ImportError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Where it applies in the source, when known.
    pub range: Option<Range<usize>>,
}

/// Why a statement became custom code, for the UI and the recognition measurement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomInfo {
    pub node: NodeId,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct ImportedForm {
    pub form: Form,
    /// The body with its braces.
    pub body: Range<usize>,
    /// The whole function, attributes included: what the user deletes from the `.rs`.
    pub fn_range: Range<usize>,
    /// The enclosing impl; equals `fn_range` for a free function.
    pub impl_range: Range<usize>,
    /// [`crate::tokens`] hash of the body.
    pub src_hash: u32,
    /// Inline modules around the impl.
    pub module: Vec<String>,
    /// Byte range of every statement node, in the source.
    pub ranges: BTreeMap<NodeId, Range<usize>>,
    pub custom: Vec<CustomInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct SourceFile {
    pub forms: Vec<ImportedForm>,
    pub diagnostics: Vec<Diagnostic>,
    /// For `.frm.rs` reading: the byte range of each top-level item, in file order. What lies
    /// between them is comments and white space, where the header lines are.
    pub(crate) top: Vec<Range<usize>>,
    /// For `.frm.rs` reading: what can be a state struct, by name.
    pub(crate) states: States,
    /// The type names this file declares or brings in.
    pub(crate) declared: BTreeSet<String>,
}

impl SourceFile {
    pub fn form(&self, id: &str) -> Option<&ImportedForm> {
        self.forms.iter().find(|f| f.form.id() == id)
    }
}

/// Imports the forms of a source text. A syntax error is an `Err`; no input panics.
pub fn import(src: &str) -> Result<SourceFile, ImportError> {
    import_with(src, ImportOptions::default())
}

pub fn import_with(src: &str, options: ImportOptions) -> Result<SourceFile, ImportError> {
    on_big_stack(|| {
        let result = import_on_this_thread(src, options);
        proc_macro2::extra::invalidate_current_thread_spans();
        result
    })
    .unwrap_or_else(|| {
        Err(ImportError {
            message: "the importer failed on this input".to_string(),
            line: 1,
            column: 1,
        })
    })
}

/// Runs `f` on a thread with a large stack, which also keeps the thread-local span table of each
/// parse apart. `None` if `f` panicked or no thread could start; the caller treats it as a failure.
pub(crate) fn on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, f)
            .ok()?
            .join()
            .ok()
    })
}

/// Refuses token trees deep or long enough to overflow the stack in `syn`, without recursing. A
/// brace ends a run only if an item or statement can start after it: `{} + {} + ..` and
/// `if a {} else if b {} else ..` also recurse.
pub(crate) fn check_limits(stream: &TokenStream) -> Result<(), ImportError> {
    struct Level {
        trees: proc_macro2::token_stream::IntoIter,
        run: usize,
        after_brace: bool,
    }
    let new_level = |s: TokenStream| Level {
        trees: s.into_iter(),
        run: 0,
        after_brace: false,
    };
    let mut stack = vec![new_level(stream.clone())];
    while let Some(level) = stack.last_mut() {
        let Some(tree) = level.trees.next() else {
            stack.pop();
            continue;
        };
        let continues = match &tree {
            TokenTree::Punct(p) if matches!(p.as_char(), ',' | ';') => {
                level.run = 0;
                level.after_brace = false;
                continue;
            }
            TokenTree::Punct(_) => true,
            TokenTree::Ident(i) => i == "else",
            _ => false,
        };
        if level.after_brace && !continues {
            level.run = 0;
        }
        level.after_brace =
            matches!(&tree, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace);
        level.run += 1;
        let too_long = level.run > MAX_RUN;
        let span = tree.span();
        if let TokenTree::Group(group) = &tree {
            stack.push(new_level(group.stream()));
        }
        let too_deep = stack.len() > MAX_DEPTH + 1;
        if too_long || too_deep {
            let start = span.start();
            let message = if too_deep {
                "brackets nest too deeply"
            } else {
                "an expression is too long"
            };
            return Err(ImportError {
                message: format!("{message} for the importer"),
                line: start.line,
                column: start.column + 1,
            });
        }
    }
    Ok(())
}

fn import_on_this_thread(src: &str, options: ImportOptions) -> Result<SourceFile, ImportError> {
    let blanked = blank_prefix(src);
    let stream = TokenStream::from_str(&blanked).map_err(|e| {
        let start = e.span().start();
        ImportError {
            message: e.to_string(),
            line: start.line,
            column: start.column + 1,
        }
    })?;
    check_limits(&stream)?;
    drop(stream);
    proc_macro2::extra::invalidate_current_thread_spans();
    let file = syn::parse_file(&blanked).map_err(|e| {
        let start = e.span().start();
        ImportError {
            message: e.to_string(),
            line: start.line,
            column: start.column + 1,
        }
    })?;

    let mut out = SourceFile::default();
    let mut scan = Scan::default();
    scan.items(src, &file.items);
    let mut walk = FileWalk {
        src,
        options,
        scan: &scan,
        out: &mut out,
    };
    walk.items(&file.items, &mut Vec::new());
    if options.frm {
        out.top = file.items.iter().filter_map(range_of).collect();
    }
    report_duplicates(&mut out);
    out.declared = scan.declared;
    if options.frm {
        out.states = scan.states;
    }
    Ok(out)
}

/// `syn::parse_file` cuts a byte order mark and a shebang off its input, which would shift every
/// span. Spaces of the same length keep the offsets right.
pub(crate) fn blank_prefix(src: &str) -> String {
    let mut text = src.to_string();
    const BOM: &str = "\u{feff}";
    let start = if text.starts_with(BOM) {
        text.replace_range(..BOM.len(), "   ");
        BOM.len()
    } else {
        0
    };
    let rest = &text[start..];
    if rest.starts_with("#!") && !after_shebang_is_attribute(&rest[2..]) {
        let end = rest.find('\n').map_or(text.len(), |i| start + i);
        let blank = " ".repeat(end - start);
        text.replace_range(start..end, &blank);
    }
    text
}

/// `#![attr]` is not a shebang, even with comments between the `!` and the `[`.
fn after_shebang_is_attribute(mut s: &str) -> bool {
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("//") {
            match rest.find('\n') {
                Some(i) => s = &rest[i..],
                None => return false,
            }
        } else if s.starts_with("/*") {
            match block_comment_end(s) {
                Some(end) => s = &s[end..],
                None => return false,
            }
        } else {
            return s.starts_with('[');
        }
    }
}

/// The end of the (nested) block comment that `s` starts with.
fn block_comment_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let (mut depth, mut i) = (0usize, 0usize);
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Some(i);
            }
        } else {
            i += 1;
        }
    }
    None
}

fn report_duplicates(out: &mut SourceFile) {
    let mut seen = BTreeMap::new();
    let mut messages = Vec::new();
    for imported in &out.forms {
        let id = imported.form.id();
        if seen.insert(id.clone(), ()).is_some() {
            messages.push(Diagnostic {
                severity: Severity::Warning,
                message: format!(
                    "more than one form is named {id}; only the first can be looked up by name"
                ),
                range: Some(imported.fn_range.clone()),
            });
        }
    }
    out.diagnostics.extend(messages);
}

/// State struct candidates: name to its source text and fields.
pub(crate) type States = BTreeMap<String, (String, Vec<StateField>)>;

/// What the state naming needs to know about a file.
#[derive(Default)]
struct Scan {
    /// Structs with named fields that derive `Default`, the shape of a generated one (the first of
    /// each name; inline modules too, since a copy keeps a form's state beside its owner).
    states: States,
    /// Every type name declared or imported, in inline modules too.
    declared: BTreeSet<String>,
    /// Each struct's named fields, with the last segment of the field's type.
    fields: BTreeMap<String, Vec<(String, String)>>,
}

impl Scan {
    fn items(&mut self, src: &str, items: &[Item]) {
        for item in items {
            match item {
                Item::Struct(s) => {
                    let name = s.ident.to_string();
                    self.declared.insert(name.clone());
                    if let syn::Fields::Named(fields) = &s.fields {
                        let list = fields.named.iter().filter_map(|f| {
                            let last = match &f.ty {
                                Type::Path(p) => p
                                    .path
                                    .segments
                                    .last()
                                    .map_or_else(String::new, |seg| seg.ident.to_string()),
                                _ => String::new(),
                            };
                            Some((f.ident.as_ref()?.to_string(), last))
                        });
                        self.fields
                            .entry(name.clone())
                            .or_insert_with(|| list.collect());
                        let derives_default = s.attrs.iter().any(|a| {
                            a.path().is_ident("derive")
                                && a.to_token_stream()
                                    .into_iter()
                                    .any(|t| has_ident(&t, "Default"))
                        });
                        if derives_default && !self.states.contains_key(&name) {
                            let state: Vec<StateField> = fields
                                .named
                                .iter()
                                .filter_map(|f| {
                                    Some(StateField {
                                        name: f.ident.as_ref()?.to_string(),
                                        ty: slice(src, range_of(&f.ty)?)?.to_string(),
                                    })
                                })
                                .collect();
                            if let Some(text) = range_of(s).and_then(|r| slice(src, r)) {
                                self.states.insert(name, (text.to_string(), state));
                            }
                        }
                    }
                }
                Item::Enum(e) => {
                    self.declared.insert(e.ident.to_string());
                }
                Item::Union(u) => {
                    self.declared.insert(u.ident.to_string());
                }
                Item::Type(t) => {
                    self.declared.insert(t.ident.to_string());
                }
                Item::Trait(t) => {
                    self.declared.insert(t.ident.to_string());
                }
                Item::Use(u) => use_names(&u.tree, &mut self.declared),
                Item::Mod(m) => {
                    self.declared.insert(m.ident.to_string());
                    if let Some((_, inner)) = &m.content {
                        self.items(src, inner);
                    }
                }
                _ => {}
            }
        }
    }
}

fn has_ident(tree: &TokenTree, name: &str) -> bool {
    match tree {
        TokenTree::Ident(i) => i == name,
        TokenTree::Group(g) => g.stream().into_iter().any(|t| has_ident(&t, name)),
        _ => false,
    }
}

fn use_names(tree: &syn::UseTree, names: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => use_names(&p.tree, names),
        syn::UseTree::Name(n) => {
            names.insert(n.ident.to_string());
        }
        syn::UseTree::Rename(r) => {
            names.insert(r.rename.to_string());
        }
        syn::UseTree::Group(g) => g.items.iter().for_each(|t| use_names(t, names)),
        syn::UseTree::Glob(_) => {}
    }
}

/// The struct name for generated state: `<Owner>Form`, or `<Owner>Form2`, `3` and so on if the file
/// already has a type of that name. A type in this file is the user's, never ours: a generated
/// struct is added to the `.frm.rs` copy only.
pub(crate) fn free_state_struct(owner: &str, declared: &BTreeSet<String>) -> String {
    let base = Form::default_state_struct(owner);
    if !declared.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}{n}"))
        .find(|name| !declared.contains(name))
        .expect("an unbounded range")
}

/// The field name: `form`, or `form2`, `3` and so on if the owner has a field of that name of
/// another type. A field already of the state struct's type is ours: the user added it.
fn free_state_field(fields: Option<&Vec<(String, String)>>, state_struct: &str) -> String {
    let free =
        |name: &str| match fields.and_then(|list| list.iter().find(|(field, _)| field == name)) {
            None => true,
            Some((_, ty)) => ty == state_struct,
        };
    let base = Form::DEFAULT_STATE_FIELD;
    if free(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}{n}"))
        .find(|name| free(name))
        .expect("an unbounded range")
}

pub(crate) fn slice(src: &str, range: Range<usize>) -> Option<&str> {
    src.get(range)
}

/// The bytes a syntax node covers, read from its tokens. `None` if it has none.
pub(crate) fn range_of(node: &impl ToTokens) -> Option<Range<usize>> {
    let mut range: Option<Range<usize>> = None;
    for tree in node.to_token_stream() {
        let r = tree.span().byte_range();
        if r.start == r.end {
            continue;
        }
        range = Some(match range {
            None => r,
            Some(old) => old.start.min(r.start)..old.end.max(r.end),
        });
    }
    range
}

pub(crate) fn group_range(span: proc_macro2::extra::DelimSpan) -> Range<usize> {
    span.join().byte_range()
}

struct FileWalk<'a> {
    src: &'a str,
    options: ImportOptions,
    scan: &'a Scan,
    out: &'a mut SourceFile,
}

impl FileWalk<'_> {
    fn items(&mut self, items: &[Item], module: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Impl(imp) => self.impl_block(imp, module),
                Item::Fn(f) => self.free_fn(f, module),
                Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        module.push(m.ident.to_string());
                        self.items(inner, module);
                        module.pop();
                    }
                }
                _ => {}
            }
        }
    }

    /// A free function is a form with `ImportOptions::free_functions`; without it only a dialog
    /// (a `&Context` function showing a `Window` or calling a dialog helper) is.
    fn free_fn(&mut self, f: &syn::ItemFn, module: &[String]) {
        let Some(param) = form_param(&f.sig, &f.block) else {
            return;
        };
        if !self.options.free_functions && !matches!(param, Param::Ctx { legacy: false, .. }) {
            return;
        }
        let Some(fn_range) = range_of(f) else { return };
        let body = group_range(f.block.brace_token.span);
        let parts = FormParts {
            owner: String::new(),
            impl_header: String::new(),
            impl_pre: String::new(),
            impl_post: String::new(),
            impl_range: fn_range.clone(),
            fn_range,
            body,
            param,
            sig: &f.sig,
            block: &f.block,
            module,
        };
        self.form(parts);
    }

    fn impl_block(&mut self, imp: &syn::ItemImpl, module: &[String]) {
        let Some(owner) = owner_of(&imp.self_ty) else {
            return;
        };
        let methods: Vec<(&syn::ImplItemFn, Param)> = imp
            .items
            .iter()
            .filter_map(|item| match item {
                ImplItem::Fn(f) => form_param(&f.sig, &f.block).map(|p| (f, p)),
                _ => None,
            })
            .collect();
        if methods.is_empty() {
            return;
        }
        let Some(impl_range) = range_of(imp) else {
            return;
        };
        let brace = group_range(imp.brace_token.span);
        let is_trait = imp.trait_.is_some();
        // A trait impl can't be split over two blocks, so it holds one form. Prefer the trait's
        // real entry point; the rest stay verbatim as its other items.
        let chosen: Vec<&(&syn::ImplItemFn, Param)> = if is_trait {
            let main = methods
                .iter()
                .find(|(f, _)| matches!(f.sig.ident.to_string().as_str(), "ui" | "update"))
                .or(methods.first());
            if methods.len() > 1 {
                self.out.diagnostics.push(Diagnostic {
                    severity: Severity::Info,
                    message: format!(
                        "impl for {owner} has several methods taking a ui; only one can be a form"
                    ),
                    range: Some(impl_range.clone()),
                });
            }
            main.into_iter().collect()
        } else {
            methods.iter().collect()
        };

        let Some(header) = slice(self.src, impl_range.start..brace.start) else {
            return;
        };
        let keep_all = is_trait;
        let mut cursor = brace.start + 1;
        let inner_end = brace.end - 1;
        for (index, (f, param)) in chosen.iter().enumerate() {
            let Some(fn_range) = range_of(*f) else {
                continue;
            };
            let body = group_range(f.block.brace_token.span);
            let is_last = index + 1 == chosen.len();
            let (pre, post) = if keep_all {
                let pre = slice(self.src, cursor..fn_range.start)
                    .unwrap_or_default()
                    .to_string();
                let post_end = if is_last { inner_end } else { fn_range.end };
                let post = slice(self.src, fn_range.end..post_end).unwrap_or_default();
                let post = if post.is_empty() && !is_last {
                    self.newline()
                } else {
                    post.to_string()
                };
                cursor = post_end;
                (pre, post)
            } else {
                self.inherent_surroundings(imp, fn_range.clone())
            };
            let parts = FormParts {
                owner: owner.clone(),
                impl_header: header.to_string(),
                impl_pre: pre,
                impl_post: post,
                impl_range: impl_range.clone(),
                fn_range,
                body,
                param: param.clone(),
                sig: &f.sig,
                block: &f.block,
                module,
            };
            self.form(parts);
        }
    }

    fn newline(&self) -> String {
        if self.src.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
        .to_string()
    }

    /// In a `.rs`, an inherent impl's other methods stay in the `.rs`. What comes with this method
    /// is its indent, the comments right above it and a comment after it on the same line.
    fn inherent_surroundings(
        &self,
        imp: &syn::ItemImpl,
        fn_range: Range<usize>,
    ) -> (String, String) {
        let nl = self.newline();
        let inner = group_range(imp.brace_token.span);
        let mut prev_end = inner.start + 1;
        let mut next_start = inner.end - 1;
        for item in &imp.items {
            if let Some(r) = range_of(item) {
                if r.end <= fn_range.start {
                    prev_end = prev_end.max(r.end);
                }
                if r.start >= fn_range.end {
                    next_start = next_start.min(r.start);
                }
            }
        }
        let gap = slice(self.src, prev_end..fn_range.start).unwrap_or_default();
        let (_, gap) = gap.split_at(trailing_comment_len(gap));
        let line_start = self.src[..fn_range.start].rfind('\n').map_or(0, |i| i + 1);
        let indent = slice(self.src, line_start..fn_range.start).unwrap_or_default();
        let indent = if indent.trim().is_empty() {
            indent
        } else {
            "    "
        };
        let pre = if gap.trim().is_empty() {
            format!("{nl}{indent}")
        } else if gap.starts_with(['\r', '\n']) {
            gap.to_string()
        } else {
            format!("{nl}{gap}")
        };
        let after = slice(self.src, fn_range.end..next_start).unwrap_or_default();
        let comment = &after[..trailing_comment_len(after)];
        (pre, format!("{comment}{nl}"))
    }

    fn form(&mut self, parts: FormParts<'_>) {
        let src = self.src;
        let Some(body_text) = slice(src, parts.body.clone()) else {
            return;
        };
        let method = parts.sig.ident.to_string();
        // From the text, not from `syn`'s tokens: the same input as `token_hash`, always.
        let Ok(body_tokens) = TokenStream::from_str(body_text) else {
            return;
        };
        let src_hash = crate::tokens::hash_stream(body_tokens);

        // In a `.rs` a type named like the state struct is the user's, so it is avoided, not read.
        // `.frm.rs` reading fills in the state from the header and the struct it finds.
        let state_struct = free_state_struct(&parts.owner, &self.scan.declared);
        let state_field = free_state_field(self.scan.fields.get(&parts.owner), &state_struct);
        if !self.options.frm
            && !parts.owner.is_empty()
            && (state_struct != Form::default_state_struct(&parts.owner)
                || state_field != Form::DEFAULT_STATE_FIELD)
        {
            self.out.diagnostics.push(Diagnostic {
                severity: Severity::Info,
                message: format!(
                    "{}: {}Form or its `form` field is taken in this file, so generated state is `{state_field}: {state_struct}`",
                    parts.owner, parts.owner
                ),
                range: Some(parts.fn_range.clone()),
            });
        }

        let (ident, legacy_ctx, sig_path) = match &parts.param {
            Param::Ui { ident, path } => (ident.clone(), None, path.clone()),
            Param::Ctx {
                ident,
                path,
                legacy,
                ..
            } => (ident.clone(), legacy.then(|| ident.clone()), path.clone()),
        };
        let mut sig = slice(src, parts.fn_range.start..parts.body.start)
            .unwrap_or_default()
            .to_string();

        let mut cx = BodyCx {
            src,
            next_id: 1,
            ranges: BTreeMap::new(),
            custom: Vec::new(),
            legacy_panels: Vec::new(),
            first_path: None,
            has_return: !matches!(parts.sig.output, ReturnType::Default),
            extended: self.options.extended,
            current_ui: ident.clone(),
            dialog_ctx: match &parts.param {
                Param::Ctx { legacy: false, .. } => Some(ident.clone()),
                _ => None,
            },
        };
        let inner = parts.body.start + 1..parts.body.end - 1;
        let (children, closing, statements) = cx.block(parts.block, inner, &ident, true);
        let root_head = slice(src, parts.body.start..statements)
            .unwrap_or("{")
            .to_string();
        let mut root = Node {
            id: NodeId(0),
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: Some(body_text.to_string()),
            kind: NodeKind::Container {
                def: ContainerId::Root,
                props: Props::new(),
                children,
                closing,
                extra_chain: Vec::new(),
                ident: ident.clone(),
                recv: ident.clone(),
                closure: String::new(),
                braced: true,
                shell: Some(ContainerShell {
                    head: Some(root_head),
                    tail: "}".to_string(),
                }),
            },
        };

        let mut custom = std::mem::take(&mut cx.custom);
        let mut ranges = std::mem::take(&mut cx.ranges);
        let mut next_id = cx.next_id;
        if verbatim_text(&root) != body_text {
            // The safety net: this has never fired on real code, and code must not be lost if it does.
            self.out.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                message: format!(
                    "{}::{method}: statements did not add up to the body; kept as one custom block",
                    parts.owner
                ),
                range: Some(parts.body.clone()),
            });
            let inner_text =
                slice(src, parts.body.start + 1..parts.body.end - 1).unwrap_or_default();
            let id = NodeId(1);
            next_id = 2;
            custom = vec![CustomInfo {
                node: id,
                reason: "safety net: statements did not add up".to_string(),
            }];
            ranges = BTreeMap::new();
            ranges.insert(id, parts.body.start + 1..parts.body.end - 1);
            if let NodeKind::Container {
                children, closing, ..
            } = &mut root.kind
            {
                closing.clear();
                *children = vec![Node {
                    id,
                    leading: String::new(),
                    trailing: String::new(),
                    bare_tail: false,
                    src: Some(inner_text.to_string()),
                    kind: NodeKind::Custom {
                        text: inner_text.to_string(),
                    },
                }];
            }
        } else {
            for id in &cx.legacy_panels {
                clear_head(&mut root, *id);
            }
        }

        if let Param::Ctx {
            path,
            arg,
            legacy: true,
            ..
        } = &parts.param
        {
            // `fn update(&mut self, ctx, frame)` is generated as `fn ui(&mut self, ui, frame)`.
            // A dialog (`fn show(&mut self, ctx)` with a `Window`) keeps its signature and `ctx`.
            let base = parts.fn_range.start;
            let method_range = parts.sig.ident.span().byte_range();
            let ty_path = path.as_deref().unwrap_or("egui::");
            // The later offset first, so the earlier one stays valid.
            sig.replace_range(
                arg.start - base..arg.end - base,
                &format!("ui: &mut {ty_path}Ui"),
            );
            sig.replace_range(method_range.start - base..method_range.end - base, "ui");
            root.src = None;
            if let NodeKind::Container { ident, recv, .. } = &mut root.kind {
                *ident = "ui".to_string();
                *recv = "ui".to_string();
            }
        }

        let egui_path = cx
            .first_path
            .clone()
            .or(sig_path)
            .unwrap_or_else(|| "egui::".to_string());
        let form = Form {
            owner: parts.owner,
            method,
            sig,
            impl_header: parts.impl_header,
            impl_pre: parts.impl_pre,
            impl_post: parts.impl_post,
            root,
            state: Vec::new(),
            state_struct,
            state_field,
            state_src: None,
            next_id,
            newline: self.newline(),
            egui_path,
            legacy_ctx,
            context_root: matches!(parts.param, Param::Ctx { legacy: false, .. }),
        };
        self.out.forms.push(ImportedForm {
            form,
            body: parts.body,
            fn_range: parts.fn_range,
            impl_range: parts.impl_range,
            src_hash,
            module: parts.module.to_vec(),
            ranges,
            custom,
        });
    }
}

struct FormParts<'a> {
    owner: String,
    impl_header: String,
    impl_pre: String,
    impl_post: String,
    impl_range: Range<usize>,
    fn_range: Range<usize>,
    body: Range<usize>,
    param: Param,
    sig: &'a syn::Signature,
    block: &'a syn::Block,
    module: &'a [String],
}

#[derive(Clone, Debug)]
enum Param {
    Ui {
        ident: String,
        path: Option<String>,
    },
    Ctx {
        ident: String,
        path: Option<String>,
        arg: Range<usize>,
        /// A legacy `update` or `ui`, generated as `fn ui(&mut self, ui, ..)`. Otherwise a dialog:
        /// its signature and `ctx` stay, and its root holds `Window`s.
        legacy: bool,
    },
}

/// The parameter that makes a function a form: `ui: &mut Ui` in any function; the legacy
/// `ctx: &Context` in a method named `update` or `ui`; or `ctx: &Context` in a method of any name
/// whose body has a top-level `Window::new(..)..show(ctx, ..)` statement or dialog helper call
/// `dialog(ctx, .., |ui| ..)` (a dialog).
fn form_param(sig: &syn::Signature, block: &syn::Block) -> Option<Param> {
    let name = sig.ident.to_string();
    let legacy = matches!(name.as_str(), "update" | "ui");
    let mut ctx = None;
    for arg in &sig.inputs {
        let FnArg::Typed(typed) = arg else { continue };
        let Pat::Ident(pat) = &*typed.pat else {
            continue;
        };
        let Type::Reference(reference) = &*typed.ty else {
            continue;
        };
        let Type::Path(ty) = &*reference.elem else {
            continue;
        };
        let Some(last) = ty.path.segments.last() else {
            continue;
        };
        let ident = pat.ident.to_string();
        let prefix = path_prefix(&ty.path);
        if last.ident == "Ui" && reference.mutability.is_some() {
            return Some(Param::Ui {
                ident,
                path: prefix,
            });
        }
        if last.ident == "Context"
            && reference.mutability.is_none()
            && ctx.is_none()
            && (legacy || shows_window(block, &ident) || calls_dialog(block, &ident))
        {
            ctx = Some(Param::Ctx {
                ident,
                path: prefix,
                arg: range_of(arg)?,
                legacy,
            });
        }
    }
    ctx
}

/// Does the body have a top-level statement (or tail) `[egui::]Window::new(..)...show(ctx, |ui| ..)`
/// (or `[egui::]Modal::new(id)...show(ctx, |ui| ..)`)?
/// That makes a method taking a `&Context` a dialog form, whatever its name. With no closure
/// (`.show(ctx, add)`, a helper's own body) there is nothing to design, so it is not one.
fn shows_window(block: &syn::Block, ctx: &str) -> bool {
    block.stmts.iter().any(|stmt| {
        let Stmt::Expr(expr, _) = stmt else {
            return false;
        };
        let Some((base, links)) = flatten(expr) else {
            return false;
        };
        let Expr::Call(call) = base else { return false };
        let Expr::Path(path) = &*call.func else {
            return false;
        };
        let Some((_, ty, ctor)) = type_path(&path.path) else {
            return false;
        };
        let Some(show) = links.last() else {
            return false;
        };
        (ty == "Window" || ty == "Modal")
            && ctor == "new"
            && show.method == "show"
            && show.args.len() == 2
            && single_ident(&show.args[0]).as_deref() == Some(ctx)
            && matches!(&show.args[1], Expr::Closure(c) if c.inputs.len() == 1)
    })
}

/// Does the body have a top-level statement (or tail) that is a dialog helper call (see
/// [`is_dialog_call`])? That too makes a method taking a `&Context` a dialog form.
fn calls_dialog(block: &syn::Block, ctx: &str) -> bool {
    block.stmts.iter().any(|stmt| match stmt {
        Stmt::Expr(Expr::Call(call), _) => is_dialog_call(call, ctx),
        _ => false,
    })
}

/// `path::to::helper(ctx, .., |ui| ..)`: a call of a plain path whose first argument is the
/// form's `ctx` and whose last is a closure with one parameter, the dialog's `Ui`.
fn is_dialog_call(call: &syn::ExprCall, ctx: &str) -> bool {
    let Expr::Path(func) = &*call.func else {
        return false;
    };
    let (Some(first), Some(last)) = (call.args.first(), call.args.last()) else {
        return false;
    };
    func.qself.is_none()
        && func.attrs.is_empty()
        && call.attrs.is_empty()
        && call.args.len() >= 2
        && single_ident(first).as_deref() == Some(ctx)
        && matches!(last, Expr::Closure(c) if c.inputs.len() == 1)
}

/// `egui::Ui` to `egui::`; `None` for a bare name.
fn path_prefix(path: &syn::Path) -> Option<String> {
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    let (_, front) = segments.split_last()?;
    if front.is_empty() {
        return None;
    }
    let colons = if path.leading_colon.is_some() {
        "::"
    } else {
        ""
    };
    Some(format!("{colons}{}::", front.join("::")))
}

pub(crate) fn owner_of(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p) => Some(p.path.segments.last()?.ident.to_string()),
        Type::Reference(r) => owner_of(&r.elem),
        Type::Paren(p) => owner_of(&p.elem),
        Type::Group(g) => owner_of(&g.elem),
        _ => None,
    }
}

/// A node's text from its original parts, ignoring `src`, for the check against the source.
fn verbatim_text(node: &Node) -> String {
    match &node.kind {
        // A column has no text of its own: its statements, as they follow each other.
        NodeKind::Container {
            def: ContainerId::Column,
            children,
            ..
        } => {
            let mut text = String::new();
            for child in children {
                text.push_str(&child.leading);
                text.push_str(&verbatim_text(child));
                text.push_str(&child.trailing);
            }
            text
        }
        NodeKind::Container {
            children,
            closing,
            shell: Some(shell),
            ..
        } => {
            let mut text = shell.head.clone().unwrap_or_default();
            for child in children {
                text.push_str(&child.leading);
                text.push_str(&verbatim_text(child));
                text.push_str(&child.trailing);
            }
            text.push_str(closing);
            text.push_str(&shell.tail);
            text
        }
        _ => node.src.clone().unwrap_or_default(),
    }
}

/// Drops a legacy panel's head, so it is regenerated with the 0.37 names, and `src` above it.
fn clear_head(node: &mut Node, id: NodeId) -> bool {
    if node.id == id {
        node.src = None;
        if let NodeKind::Container {
            shell: Some(shell), ..
        } = &mut node.kind
        {
            shell.head = None;
        }
        return true;
    }
    if let NodeKind::Container { children, .. } = &mut node.kind {
        for child in children {
            if clear_head(child, id) {
                node.src = None;
                return true;
            }
        }
    }
    false
}

/// The length of a comment at the start of a gap that sits on the same line as the statement before
/// it, with the blanks before it. Zero if there is none.
pub(crate) fn trailing_comment_len(gap: &str) -> usize {
    let blanks = gap.len() - gap.trim_start_matches([' ', '\t']).len();
    let rest = &gap[blanks..];
    if rest.starts_with("//") {
        let line = rest.find('\n').unwrap_or(rest.len());
        let line = if rest[..line].ends_with('\r') {
            line - 1
        } else {
            line
        };
        return blanks + line;
    }
    if rest.starts_with("/*")
        && let Some(end) = block_comment_end(rest)
        && !rest[..end].contains('\n')
    {
        return blanks + end;
    }
    0
}

struct BodyCx<'a> {
    src: &'a str,
    next_id: u32,
    ranges: BTreeMap<NodeId, Range<usize>>,
    custom: Vec<CustomInfo>,
    legacy_panels: Vec<NodeId>,
    /// The path prefix of the first egui type seen, for generated code in this form.
    first_path: Option<String>,
    has_return: bool,
    extended: bool,
    /// The `Ui` in scope for the statement being read.
    current_ui: String,
    /// A dialog form's `ctx`: where it is the current ident, a dialog helper call is a container.
    dialog_ctx: Option<String>,
}

/// A recognised widget before it gets its text and id.
struct WidgetParts {
    def: WidgetId,
    props: Props,
    extra_chain: Vec<String>,
    response_chain: Vec<String>,
    recv: String,
}

/// A closure that can be a container's body.
struct ClosureParts<'e> {
    ident: String,
    body: ClosureBody<'e>,
    /// From the closure's start to the body.
    text: String,
    /// The body, with its braces if it has them.
    body_range: Range<usize>,
}

enum ClosureBody<'e> {
    Block(&'e syn::Block),
    /// `|ui| self.ui(ui)`.
    Expr(&'e Expr),
}

/// The chain `base.a().b()`, innermost call first.
fn flatten(expr: &Expr) -> Option<(&Expr, Vec<&ExprMethodCall>)> {
    let mut links = Vec::new();
    let mut current = expr;
    while let Expr::MethodCall(call) = current {
        links.push(call);
        if links.len() > 256 {
            return None;
        }
        current = &call.receiver;
    }
    links.reverse();
    Some((current, links))
}

/// A `Ui` receiver or argument: a name, or a column of `ui.columns(..)` as `cols[0]` (also passed
/// as `&mut cols[0]`), written without spaces so it compares with the column's ident.
fn ui_ref(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Index(ix) if ix.attrs.is_empty() => {
            let base = single_ident(&ix.expr)?;
            let Expr::Lit(lit) = &*ix.index else {
                return None;
            };
            let Lit::Int(i) = &lit.lit else { return None };
            if !i.suffix().is_empty() || !lit.attrs.is_empty() {
                return None;
            }
            Some(format!("{base}[{}]", i.base10_digits()))
        }
        Expr::Reference(r)
            if r.attrs.is_empty()
                && r.mutability.is_some()
                && matches!(*r.expr, Expr::Index(_)) =>
        {
            ui_ref(&r.expr)
        }
        _ => single_ident(expr),
    }
}

fn single_ident(expr: &Expr) -> Option<String> {
    if let Expr::Path(p) = expr
        && p.qself.is_none()
        && p.attrs.is_empty()
    {
        return p.path.get_ident().map(|i| i.to_string());
    }
    None
}

/// `egui::Slider::new` to (`egui::`, `Slider`, `new`).
fn type_path(path: &syn::Path) -> Option<(String, String, String)> {
    let segments: Vec<&syn::PathSegment> = path.segments.iter().collect();
    if segments.len() < 2 || segments.iter().any(|s| !s.arguments.is_none()) {
        return None;
    }
    let ctor = segments[segments.len() - 1].ident.to_string();
    let ty = segments[segments.len() - 2].ident.to_string();
    let mut front = &segments[..segments.len() - 2];
    // `egui::containers::Sides` is `egui::Sides` too; the prefix is egui's.
    if ty == "Sides"
        && front.len() >= 2
        && front[front.len() - 1].ident == "containers"
        && front[front.len() - 2].ident == "egui"
    {
        front = &front[..front.len() - 1];
    }
    let prefix = if front.is_empty() {
        String::new()
    } else {
        if front.last()?.ident != "egui" {
            return None;
        }
        let colons = if path.leading_colon.is_some() {
            "::"
        } else {
            ""
        };
        let names: Vec<String> = front.iter().map(|s| s.ident.to_string()).collect();
        format!("{colons}{}::", names.join("::"))
    };
    Some((prefix, ty, ctor))
}

/// The word used in a Custom reason for an expression's kind.
fn expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::If(_) => "if (not a widget event)",
        Expr::ForLoop(_) => "for loop",
        Expr::While(_) => "while loop",
        Expr::Loop(_) => "loop",
        Expr::Match(_) => "match",
        Expr::Block(_) => "block",
        Expr::Unsafe(_) => "unsafe block",
        Expr::Assign(_) => "assignment",
        Expr::Binary(_) => "binary expression",
        Expr::Call(_) => "helper call: function",
        Expr::Macro(_) => "macro",
        Expr::Return(_) => "return",
        Expr::Break(_) => "break",
        Expr::Continue(_) => "continue",
        Expr::Closure(_) => "closure",
        Expr::Try(_) => "try (?)",
        Expr::Field(_) => "field access",
        Expr::Path(_) => "path expression",
        Expr::Lit(_) => "literal",
        Expr::Reference(_) => "reference",
        Expr::Unary(_) => "unary expression",
        Expr::Struct(_) => "struct literal",
        Expr::Tuple(_) => "tuple",
        _ => "other expression",
    }
}

impl<'a> BodyCx<'a> {
    fn alloc(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    fn text(&self, range: Range<usize>) -> &'a str {
        self.src.get(range).unwrap_or_default()
    }

    fn expr_text(&self, expr: &impl ToTokens) -> String {
        range_of(expr)
            .map(|r| self.text(r).to_string())
            .unwrap_or_default()
    }

    /// The statements of a block, with the text between them. `inner` is the block's text between
    /// its braces. Returns the nodes, the text after the last one and where the statements begin:
    /// a block's inner attributes (`#![allow(x)]`) belong to the block, so they join its head.
    fn block(
        &mut self,
        block: &syn::Block,
        inner: Range<usize>,
        ident: &str,
        top: bool,
    ) -> (Vec<Node>, String, usize) {
        self.block_from(block, inner, ident, top, 0)
    }

    /// [`Self::block`] from statement `skip` on: the ones before it belong to the head (a
    /// container's body statements, see [`Self::body_props`]).
    fn block_from(
        &mut self,
        block: &syn::Block,
        inner: Range<usize>,
        ident: &str,
        top: bool,
        skip: usize,
    ) -> (Vec<Node>, String, usize) {
        let mut nodes: Vec<Node> = Vec::new();
        let start = match skip.checked_sub(1).and_then(|i| block.stmts.get(i)) {
            Some(last) => range_of(last).map_or(inner.start, |r| r.end),
            None => {
                let first = block
                    .stmts
                    .first()
                    .and_then(range_of)
                    .map_or(inner.end, |r| r.start);
                inner.start + inner_attrs_end(self.text(inner.start..first))
            }
        };
        let mut cursor = start;
        let count = block.stmts.len();
        let mut index = skip;
        while index < count {
            let stmt = &block.stmts[index];
            let Some(range) = range_of(stmt) else {
                index += 1;
                continue;
            };
            let gap = self.text(cursor..range.start);
            let leading = take_trailing(&mut nodes, gap);
            let is_tail = index + 1 == count;
            // A function's returned tail stays what it is.
            let may_end = !(top && self.has_return);
            let (mut node, used) = match self.tabs_at(&block.stmts[index..], ident, may_end) {
                Some(found) => found,
                None => (self.statement(stmt, range.clone(), ident, top, is_tail), 1),
            };
            let last = &block.stmts[index + used - 1];
            node.leading = leading.to_string();
            node.bare_tail = is_bare_tail(last);
            nodes.push(node);
            cursor = range_of(last).map_or(range.end, |r| r.end);
            index += used;
        }
        let closing = take_trailing(&mut nodes, self.text(cursor..inner.end)).to_string();
        (nodes, closing, start)
    }

    fn statement(
        &mut self,
        stmt: &Stmt,
        range: Range<usize>,
        ident: &str,
        top: bool,
        is_tail: bool,
    ) -> Node {
        let id = self.alloc();
        self.ranges.insert(id, range.clone());
        let text = self.text(range.clone());
        let outer_ui = std::mem::replace(&mut self.current_ui, ident.to_string());
        let result = if text.starts_with('#') {
            Err("attribute on statement".to_string())
        } else {
            match stmt {
                Stmt::Local(local) => match self.let_widget(stmt, local, ident) {
                    Some(kind) => Ok(kind),
                    None => Err(local_reason(local, ident)),
                },
                Stmt::Item(item) => Err(format!("item: {}", item_kind(item))),
                Stmt::Macro(mac) => Err(macro_reason(&mac.mac)),
                Stmt::Expr(expr, semi) => {
                    if top
                        && is_tail
                        && semi.is_none()
                        && self.has_return
                        && !matches!(expr, Expr::If(_) | Expr::ForLoop(_) | Expr::Match(_))
                    {
                        Err("result used: returned from the function".to_string())
                    } else {
                        let returned = top && is_tail && semi.is_none() && self.has_return;
                        self.expression(stmt, expr, id, ident).or_else(|reason| {
                            let flow = if self.extended && !returned {
                                self.flow(expr, range.clone(), ident)
                            } else {
                                None
                            };
                            flow.ok_or(reason)
                        })
                    }
                }
            }
        };
        self.current_ui = outer_ui;
        match result {
            Ok(kind) => Node {
                id,
                leading: String::new(),
                trailing: String::new(),
                bare_tail: false,
                src: Some(text.to_string()),
                kind,
            },
            Err(reason) => {
                self.custom.push(CustomInfo { node: id, reason });
                Node {
                    id,
                    leading: String::new(),
                    trailing: String::new(),
                    bare_tail: false,
                    src: Some(text.to_string()),
                    kind: NodeKind::Custom {
                        text: text.to_string(),
                    },
                }
            }
        }
    }

    fn expression(
        &mut self,
        stmt: &Stmt,
        expr: &Expr,
        id: NodeId,
        ident: &str,
    ) -> Result<NodeKind, String> {
        match expr {
            Expr::If(if_expr) => self.if_event(stmt, if_expr, ident),
            Expr::Block(block) => self.event_block(stmt, block, ident),
            Expr::MethodCall(_) => self.call(stmt, expr, id, ident),
            Expr::Call(call)
                if self.dialog_ctx.as_deref() == Some(ident) && is_dialog_call(call, ident) =>
            {
                self.dialog_call(stmt, call, ident)
            }
            Expr::Macro(mac) => Err(macro_reason(&mac.mac)),
            // `ui.add_enabled_ui(e, |ui| ui.heading("x")).inner;`: a widget with a wrapper.
            Expr::Field(_) => self.call(stmt, expr, id, ident),
            other => Err(expr_kind(other).to_string()),
        }
    }

    /// `let label = <widget>;`, with `ImportOptions::extended`.
    fn let_widget(&mut self, stmt: &Stmt, local: &syn::Local, ident: &str) -> Option<NodeKind> {
        if !self.extended || !local.attrs.is_empty() {
            return None;
        }
        let init = local.init.as_ref().filter(|i| i.diverge.is_none())?;
        let parts = self.widget(&init.expr, ident).ok()?;
        self.refuse_comments(stmt, &[]).ok()?;
        let start = range_of(stmt)?.start;
        let prefix = self.text(start..range_of(&init.expr)?.start).to_string();
        let mut kind = widget_kind(parts, Vec::new());
        if let NodeKind::Widget { let_prefix, .. } = &mut kind {
            *let_prefix = Some(prefix);
        }
        Some(kind)
    }

    /// Control flow as a container, if a body holds something recognised. With only custom code
    /// inside, the statement stays one custom block: there would be nothing to design in it, and
    /// the extra nodes would only flatter the measurement.
    fn flow(&mut self, expr: &Expr, range: Range<usize>, ident: &str) -> Option<NodeKind> {
        let saved_id = self.next_id;
        let saved_custom = self.custom.len();
        let kind = self.flow_kind(expr, range, ident);
        let kept = kind.as_ref().is_some_and(|k| match k {
            NodeKind::Container { children, .. } => children.iter().any(has_recognised),
            _ => false,
        });
        if kept {
            return kind;
        }
        self.next_id = saved_id;
        self.custom.truncate(saved_custom);
        self.ranges.retain(|id, _| id.0 < saved_id);
        self.legacy_panels.retain(|id| id.0 < saved_id);
        None
    }

    fn flow_kind(&mut self, expr: &Expr, range: Range<usize>, ident: &str) -> Option<NodeKind> {
        match expr {
            Expr::If(first) => {
                if first.else_branch.is_none() {
                    return Some(self.flow_body(&first.then_branch, range, ident).kind);
                }
                let mut branches = Vec::new();
                let mut start = range.start;
                let mut cursor = range.start;
                let mut current = first;
                loop {
                    let end = group_range(current.then_branch.brace_token.span).end;
                    let mut node = self.flow_body(&current.then_branch, start..end, ident);
                    node.leading = self.text(cursor..start).to_string();
                    branches.push(node);
                    cursor = end;
                    let Some((else_token, next)) = &current.else_branch else {
                        break;
                    };
                    start = else_token.span.byte_range().start;
                    match &**next {
                        Expr::If(next_if) => current = next_if,
                        Expr::Block(block) => {
                            let end = group_range(block.block.brace_token.span).end;
                            let mut node = self.flow_body(&block.block, start..end, ident);
                            node.leading = self.text(cursor..start).to_string();
                            branches.push(node);
                            cursor = end;
                            break;
                        }
                        _ => return None,
                    }
                }
                let tail = self.text(cursor..range.end).to_string();
                Some(group_kind(
                    branches,
                    String::new(),
                    String::new(),
                    tail,
                    ident,
                ))
            }
            Expr::ForLoop(e) => Some(self.flow_body(&e.body, range, ident).kind),
            Expr::While(e) => Some(self.flow_body(&e.body, range, ident).kind),
            Expr::Loop(e) => Some(self.flow_body(&e.body, range, ident).kind),
            Expr::Match(e) => {
                let brace = group_range(e.brace_token.span);
                let head = self.text(range.start..brace.start + 1).to_string();
                let mut arms: Vec<Node> = Vec::new();
                let mut cursor = brace.start + 1;
                for arm in &e.arms {
                    let arm_range = range_of(arm)?;
                    let gap = self.text(cursor..arm_range.start);
                    let leading = take_trailing(&mut arms, gap).to_string();
                    let mut node = match &*arm.body {
                        Expr::Block(block) if block.label.is_none() && block.attrs.is_empty() => {
                            self.flow_body(&block.block, arm_range.clone(), ident)
                        }
                        other => self.flow_arm_expr(other, arm_range.clone(), ident)?,
                    };
                    node.leading = leading;
                    arms.push(node);
                    cursor = arm_range.end;
                }
                let closing =
                    take_trailing(&mut arms, self.text(cursor..brace.end - 1)).to_string();
                let tail = self.text(brace.end - 1..range.end).to_string();
                Some(group_kind(arms, head, closing, tail, ident))
            }
            _ => None,
        }
    }

    /// A body of control flow: its head is everything up to the `{`.
    fn flow_body(&mut self, block: &syn::Block, range: Range<usize>, ident: &str) -> Node {
        let id = self.alloc();
        self.ranges.insert(id, range.clone());
        let brace = group_range(block.brace_token.span);
        let (children, closing, statements) =
            self.block(block, brace.start + 1..brace.end - 1, ident, false);
        let head = self.text(range.start..statements).to_string();
        let tail = self.text(brace.end - 1..range.end).to_string();
        Node {
            id,
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: Some(self.text(range).to_string()),
            kind: flow_kind_node(children, closing, head, tail, true, ident),
        }
    }

    /// `A => ui.label("x"),`: an arm with no braces.
    fn flow_arm_expr(&mut self, body: &Expr, range: Range<usize>, ident: &str) -> Option<Node> {
        let id = self.alloc();
        self.ranges.insert(id, range.clone());
        let body_range = range_of(body)?;
        let stmt = Stmt::Expr(body.clone(), None);
        let mut child = self.statement(&stmt, body_range.clone(), ident, false, true);
        child.bare_tail = is_bare_tail(&stmt);
        let head = self.text(range.start..body_range.start).to_string();
        let tail = self.text(body_range.end..range.end).to_string();
        Some(Node {
            id,
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: Some(self.text(range).to_string()),
            kind: flow_kind_node(vec![child], String::new(), head, tail, false, ident),
        })
    }

    /// `if <widget>.clicked() { .. }`.
    fn if_event(
        &mut self,
        stmt: &Stmt,
        expr: &syn::ExprIf,
        ident: &str,
    ) -> Result<NodeKind, String> {
        let not_event = || "if (not a widget event)".to_string();
        if expr.else_branch.is_some() {
            return Err(if self.event_of(&expr.cond).is_some() {
                "if with else on a widget event".to_string()
            } else {
                not_event()
            });
        }
        let (event, receiver) = self.event_of(&expr.cond).ok_or_else(not_event)?;
        let parts = self.widget(receiver, ident).map_err(|_| not_event())?;
        let handler_range = group_range(expr.then_branch.brace_token.span);
        self.refuse_comments(stmt, std::slice::from_ref(&handler_range))?;
        let handler = self
            .text(handler_range.start + 1..handler_range.end - 1)
            .to_string();
        Ok(widget_kind(parts, vec![(event, handler)]))
    }

    /// The generated form of a widget with several events:
    /// `{ let response = <widget>; if response.clicked() { .. } if response.changed() { .. } }`.
    fn event_block(
        &mut self,
        stmt: &Stmt,
        block: &syn::ExprBlock,
        ident: &str,
    ) -> Result<NodeKind, String> {
        let plain = || "block".to_string();
        if block.label.is_some() || !block.attrs.is_empty() {
            return Err(plain());
        }
        let stmts = &block.block.stmts;
        let Some((Stmt::Local(local), rest)) = stmts.split_first() else {
            return Err(plain());
        };
        let Pat::Ident(pat) = &local.pat else {
            return Err(plain());
        };
        let Some(init) = &local.init else {
            return Err(plain());
        };
        if init.diverge.is_some() || rest.is_empty() {
            return Err(plain());
        }
        let name = pat.ident.to_string();
        let mut events = Vec::new();
        let mut handlers = Vec::new();
        // Initial focus: `if <ui>.memory(|m| m.focused().is_none()) { response.request_focus(); }`
        // first, exactly as the emitter writes it (spaces aside).
        let squeeze = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let focus_text =
            squeeze(&crate::emit::focus_statement(ident).replace("response.", &format!("{name}.")));
        let focus = rest
            .first()
            .and_then(range_of)
            .is_some_and(|r| squeeze(self.text(r)) == focus_text);
        let rest = if focus { &rest[1..] } else { rest };
        for stmt in rest {
            let Stmt::Expr(Expr::If(branch), None) = stmt else {
                return Err(plain());
            };
            if branch.else_branch.is_some() {
                return Err(plain());
            }
            let (event, receiver) = self.event_of(&branch.cond).ok_or_else(plain)?;
            if single_ident(receiver).as_deref() != Some(name.as_str()) {
                return Err(plain());
            }
            let range = group_range(branch.then_branch.brace_token.span);
            events.push((event, self.text(range.start + 1..range.end - 1).to_string()));
            handlers.push(range);
        }
        let mut parts = self.widget(&init.expr, ident).map_err(|_| plain())?;
        if focus {
            let prop = widget_def(parts.def)
                .props
                .iter()
                .find(|p| p.role == PropRole::InitialFocus)
                .ok_or_else(plain)?;
            parts.props.insert(prop.name, PropValue::Bool(true));
        } else if events.is_empty() {
            return Err(plain());
        }
        self.refuse_comments(stmt, &handlers)?;
        Ok(widget_kind(parts, events))
    }

    /// `<receiver>.clicked()`.
    fn event_of<'e>(&self, cond: &'e Expr) -> Option<(EventId, &'e Expr)> {
        let Expr::MethodCall(call) = cond else {
            return None;
        };
        if !call.args.is_empty() || call.turbofish.is_some() {
            return None;
        }
        Some((
            EventId::from_method(&call.method.to_string())?,
            &call.receiver,
        ))
    }

    fn refuse_comments(&self, stmt: &Stmt, exclude: &[Range<usize>]) -> Result<(), String> {
        if has_comment(self.src, stmt.to_token_stream(), exclude) {
            Err("comment inside statement".to_string())
        } else {
            Ok(())
        }
    }

    /// A statement that is a method chain: a container, a widget, or a reason why it is neither.
    fn call(
        &mut self,
        stmt: &Stmt,
        expr: &Expr,
        id: NodeId,
        ident: &str,
    ) -> Result<NodeKind, String> {
        let Some((base, links)) = flatten(expr) else {
            return Err("very long method chain".to_string());
        };
        if let Some(kind) = self.table(stmt, base, &links, ident) {
            return Ok(kind);
        }
        if let Some(result) = self.container(stmt, base, &links, id, ident) {
            return result;
        }
        let parts = self.widget(expr, ident)?;
        self.refuse_comments(stmt, &[])?;
        Ok(widget_kind(parts, Vec::new()))
    }

    /// `Some` when this chain starts like a known container.
    fn container(
        &mut self,
        stmt: &Stmt,
        base: &Expr,
        links: &[&ExprMethodCall],
        id: NodeId,
        ident: &str,
    ) -> Option<Result<NodeKind, String>> {
        if let Some(name) = ui_ref(base) {
            if name != ident {
                return None;
            }
            let first = links.first()?;
            let method = first.method.to_string();
            let def = ALL_CONTAINERS.iter().copied().find(|&c| {
                forms::container_forms(c)
                    .shorthand
                    .is_some_and(|(m, _)| m == method)
            })?;
            if links.len() > 1 {
                return Some(Err("container result used".to_string()));
            }
            if def == ContainerId::Columns {
                return Some(self.columns(stmt, first, ident));
            }
            let (_, arg_props) = forms::container_forms(def).shorthand?;
            let args: Vec<&Expr> = first.args.iter().collect();
            if args.len() != arg_props.len() + 1 {
                return Some(Err(format!(
                    "container with unexpected arguments: ui.{method}"
                )));
            }
            let mut props = Props::new();
            let cdef = container_def(def);
            for (name, arg) in arg_props.iter().zip(&args) {
                let prop = cdef.prop(name)?;
                props.insert(prop.name, self.value(arg, prop));
            }
            return Some(self.finish_container(
                stmt,
                def,
                props,
                Vec::new(),
                args[args.len() - 1],
                ident,
            ));
        }

        let Expr::Call(call) = base else { return None };
        let Expr::Path(path) = &*call.func else {
            return None;
        };
        let (prefix, ty, ctor) = type_path(&path.path)?;
        let legacy = LEGACY_PANELS
            .iter()
            .find(|(t, c, _)| *t == ty && *c == ctor)
            .map(|(_, _, id)| *id);
        let def = legacy.or_else(|| {
            ALL_CONTAINERS.iter().copied().find(|&c| {
                forms::container_forms(c)
                    .builder
                    .is_some_and(|(t, ctors)| t == ty && ctors.iter().any(|(n, _)| *n == ctor))
            })
        })?;
        let (_, ctors) = forms::container_forms(def).builder?;
        let arg_props: &[&str] = match legacy {
            Some(_) => &["id"],
            None => ctors.iter().find(|(n, _)| *n == ctor)?.1,
        };
        let cdef = container_def(def);
        if call.args.len() != arg_props.len() {
            return Some(Err(format!(
                "container with unexpected arguments: {ty}::{ctor}"
            )));
        }
        let mut props = Props::new();
        for (name, arg) in arg_props.iter().zip(call.args.iter()) {
            let prop = cdef.prop(name)?;
            props.insert(prop.name, self.value(arg, prop));
        }
        if let Some(prop) = cdef.props.iter().find(|p| p.role == PropRole::Ctor) {
            let PropKind::Enum { values, .. } = prop.kind else {
                return None;
            };
            let value = values.iter().copied().find(|v| *v == ctor)?;
            if prop.default.to_value() != Some(PropValue::Enum(value)) {
                props.insert(prop.name, PropValue::Enum(value));
            }
        }

        // The last link must be `.show(ui, closure)`; anything after it uses the result.
        let shown_with = forms::show_method(def);
        let show_at = links.iter().position(|l| {
            let m = l.method.to_string();
            m == shown_with || (shown_with == "show" && m == "show_inside")
        });
        let Some(show_at) = show_at else {
            return Some(Err(format!("container show variant: {ty}::{ctor}")));
        };
        if show_at + 1 != links.len() {
            return Some(Err("container result used".to_string()));
        }
        let show = links[show_at];
        let args: Vec<&Expr> = show.args.iter().collect();
        let closures = if def == ContainerId::Sides { 2 } else { 1 };
        if args.len() != closures + 1 {
            return Some(Err(format!(
                "container with unexpected arguments: {ty}.show"
            )));
        }
        let Some(recv) = ui_ref(args[0]) else {
            return Some(Err(format!(
                "container receiver is not the current ui: {ty}"
            )));
        };
        // A legacy `update` has `ctx` where the root's panels take their `ui`; `ident` is it there.
        if recv != ident {
            return Some(Err(format!(
                "container receiver is not the current ui: {ty}"
            )));
        }
        let mut extra_chain = Vec::new();
        let mut previous_end = range_of(base)?.end;
        for link in &links[..show_at] {
            let end = range_of(*link)?.end;
            let text = self.text(previous_end..end).trim_start().to_string();
            previous_end = end;
            let method = link.method.to_string();
            let mapped = if cdef.is_panel {
                legacy_panel_method(&method)
            } else {
                None
            };
            let as_prop = find_builder_prop(cdef.props, mapped.unwrap_or(&method), &props);
            match (as_prop, link.args.len()) {
                (Some(prop), 1) if matches!(prop.role, PropRole::Builder(_)) => {
                    let value = self.value(&link.args[0], prop);
                    props.insert(prop.name, value);
                    if mapped.is_some() {
                        self.legacy_panels.push(id);
                    }
                }
                (Some(prop), n) if n > 1 && is_expr_builder(prop) => {
                    props.insert(prop.name, PropValue::Expr(self.args_text(link)));
                }
                (Some(prop), 0) if matches!(prop.role, PropRole::Flag(_)) => {
                    props.insert(prop.name, PropValue::Bool(true));
                }
                _ => extra_chain.push(text),
            }
        }
        if legacy.is_some() {
            self.legacy_panels.push(id);
        }
        self.first_path.get_or_insert(prefix);
        if def == ContainerId::Sides {
            return Some(self.sides(stmt, props, extra_chain, args[1], args[2], &recv));
        }
        Some(self.finish_container(stmt, def, props, extra_chain, args[1], &recv))
    }

    /// `ui.columns(n, |cols| { cols[0].label(..); cols[1].button(..); })`: a Columns with one
    /// Column per column, each holding the statements on its `cols[i]`. Only the exact shape is
    /// read: a literal count, a braced body whose every statement uses one column (`cols[i]` with a
    /// literal index below the count, and `cols` nowhere else), in column order. Anything else
    /// (a computed count, a loop over the columns, statements that go back to an earlier column,
    /// `columns_const`) stays custom code.
    fn columns(
        &mut self,
        stmt: &Stmt,
        call: &ExprMethodCall,
        ident: &str,
    ) -> Result<NodeKind, String> {
        let args: Vec<&Expr> = call.args.iter().collect();
        if args.len() != 2 {
            return Err("container with unexpected arguments: ui.columns".to_string());
        }
        let not_literal = || "columns: the count is not a literal number".to_string();
        let Expr::Lit(lit) = args[0] else {
            return Err(not_literal());
        };
        let Lit::Int(int) = &lit.lit else {
            return Err(not_literal());
        };
        if !matches!(int.suffix(), "" | "usize") {
            return Err(not_literal());
        }
        let count: usize = int.base10_parse().map_err(|_| not_literal())?;
        if count == 0 || count > formust_model::MAX_COLUMNS {
            return Err(format!(
                "columns: {count} columns (the designer reads 1 to {})",
                formust_model::MAX_COLUMNS
            ));
        }
        let closure = self.closure(args[1])?;
        let ClosureBody::Block(block) = closure.body else {
            return Err("columns: the closure has no braces".to_string());
        };
        let cols = closure.ident.clone();
        let mut columns_of = Vec::new();
        for st in &block.stmts {
            let i = column_of(st.to_token_stream(), &cols)
                .ok_or_else(|| format!("columns: a statement doesn't use exactly one {cols}[i]"))?;
            if i >= count {
                return Err(format!("columns: {cols}[{i}] is past the {count} columns"));
            }
            if columns_of.last().is_some_and(|&last| i < last) {
                return Err("columns: the statements are not in column order".to_string());
            }
            columns_of.push(i);
        }
        self.refuse_comments(stmt, std::slice::from_ref(&closure.body_range))?;
        let range = range_of(stmt).ok_or("statement has no tokens")?;
        let body = closure.body_range.clone();
        let inner = body.start + 1..body.end - 1;
        let first = block
            .stmts
            .first()
            .and_then(range_of)
            .map_or(inner.end, |r| r.start);
        if inner_attrs_end(self.text(inner.start..first)) != 0 {
            return Err("columns: the body has inner attributes".to_string());
        }
        let mut nodes: Vec<Node> = Vec::new();
        let mut node_columns: Vec<usize> = Vec::new();
        let mut cursor = inner.start;
        let total = block.stmts.len();
        let mut k = 0;
        while k < total {
            let (st, i) = (&block.stmts[k], columns_of[k]);
            let Some(r) = range_of(st) else {
                k += 1;
                continue;
            };
            let gap = self.text(cursor..r.start);
            let leading = take_trailing(&mut nodes, gap).to_string();
            let column = format!("{cols}[{i}]");
            // Tabs on one column: its bar and its `match` are both on `cols[i]`.
            let same_next = columns_of.get(k + 1) == Some(&i);
            let tabs = if same_next {
                self.tabs_at(&block.stmts[k..], &column, true)
            } else {
                self.tabs_at(&block.stmts[k..k + 1], &column, true)
            };
            let (mut node, used) = match tabs {
                Some(found) => found,
                None => (
                    self.statement(st, r.clone(), &column, false, k + 1 == total),
                    1,
                ),
            };
            let last = &block.stmts[k + used - 1];
            node.leading = leading;
            node.bare_tail = is_bare_tail(last);
            nodes.push(node);
            node_columns.push(i);
            cursor = range_of(last).map_or(r.end, |r| r.end);
            k += used;
        }
        let closing = take_trailing(&mut nodes, self.text(cursor..inner.end)).to_string();
        let mut parts: Vec<Vec<Node>> = (0..count).map(|_| Vec::new()).collect();
        for (node, i) in nodes.into_iter().zip(node_columns) {
            parts[i].push(node);
        }
        // A column's range runs from its first statement to its last (empty: where it would be).
        let mut at = inner.start;
        let mut spans = Vec::new();
        for kids in &parts {
            let first = kids
                .first()
                .and_then(|k| self.ranges.get(&k.id))
                .map(|r| r.start);
            let last = kids
                .last()
                .and_then(|k| self.ranges.get(&k.id))
                .map(|r| r.end);
            let span = first.unwrap_or(at)..last.unwrap_or(at);
            at = span.end;
            spans.push(span);
        }
        let children = parts
            .into_iter()
            .zip(spans)
            .enumerate()
            .map(|(i, (kids, span))| Node {
                id: {
                    let id = self.alloc();
                    self.ranges.insert(id, span);
                    id
                },
                leading: String::new(),
                trailing: String::new(),
                bare_tail: false,
                src: None,
                kind: NodeKind::Container {
                    def: ContainerId::Column,
                    props: Props::new(),
                    children: kids,
                    closing: String::new(),
                    extra_chain: Vec::new(),
                    ident: format!("{cols}[{i}]"),
                    recv: cols.clone(),
                    closure: String::new(),
                    braced: true,
                    shell: None,
                },
            })
            .collect();
        let mut props = Props::new();
        props.insert("count", PropValue::Int(count as i64));
        Ok(NodeKind::Container {
            def: ContainerId::Columns,
            props,
            children,
            closing,
            extra_chain: Vec::new(),
            ident: cols,
            recv: ident.to_string(),
            closure: closure.text,
            braced: true,
            shell: Some(ContainerShell {
                head: Some(self.text(range.start..inner.start).to_string()),
                tail: self.text(inner.end..range.end).to_string(),
            }),
        })
    }

    /// Tabs at the start of `stmts`, in exactly the shape the emitter writes (whitespace aside,
    /// and no comment outside the pages): a horizontal bar of `selectable_value`s (or buttons) on
    /// one field followed by `match field { value => { .. } .. }` with the same values in the same
    /// order (two statements), or the vertical form, one `ui.horizontal(..)` with the bar in a
    /// `ui.vertical`, a separator, and the `match` in another `ui.vertical`. A final `_ => {}` arm
    /// is allowed. Returns the node and the number of statements it takes. Anything else is left
    /// to the ordinary rules, with nothing recorded.
    fn tabs_at(&mut self, stmts: &[Stmt], ident: &str, may_end: bool) -> Option<(Node, usize)> {
        let squeeze = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let Some(Stmt::Expr(Expr::MethodCall(outer), Some(_))) = stmts.first() else {
            return None;
        };
        if ui_ref(&outer.receiver).as_deref() != Some(ident)
            || outer.method != "horizontal"
            || outer.args.len() != 1
            || outer.turbofish.is_some()
            || !outer.attrs.is_empty()
        {
            return None;
        }
        let closure = self.closure(&outer.args[0]).ok()?;
        let ClosureBody::Block(body) = closure.body else {
            return None;
        };
        // The vertical form: `vertical(bar); separator(); vertical(match)` in one statement.
        let on = |stmt: &Stmt, method: &str, args: usize| -> Option<ExprMethodCall> {
            match stmt {
                Stmt::Expr(Expr::MethodCall(m), Some(_))
                    if m.method == method
                        && m.args.len() == args
                        && m.attrs.is_empty()
                        && ui_ref(&m.receiver).as_deref() == Some(closure.ident.as_str()) =>
                {
                    Some(m.clone())
                }
                _ => None,
            }
        };
        let vertical = match body.stmts.as_slice() {
            [a, b, c] => (|| {
                let bar = on(a, "vertical", 1)?;
                on(b, "separator", 0)?;
                let pages = on(c, "vertical", 1)?;
                Some((bar, pages))
            })(),
            _ => None,
        };
        let (def, used, bar, match_expr, page_ui) = match &vertical {
            Some((bar, pages)) => {
                let bar = self.closure(&bar.args[0]).ok()?;
                let ClosureBody::Block(bar_block) = bar.body else {
                    return None;
                };
                let pages = self.closure(&pages.args[0]).ok()?;
                let ClosureBody::Block(pages_block) = pages.body else {
                    return None;
                };
                let [Stmt::Expr(Expr::Match(m), None)] = pages_block.stmts.as_slice() else {
                    return None;
                };
                (
                    ContainerId::TabsVertical,
                    1,
                    (bar.ident.clone(), bar_block),
                    m,
                    pages.ident.clone(),
                )
            }
            None => {
                let Some(Stmt::Expr(Expr::Match(m), None)) = stmts.get(1) else {
                    return None;
                };
                (
                    ContainerId::Tabs,
                    2,
                    (closure.ident.clone(), body),
                    m,
                    ident.to_string(),
                )
            }
        };
        if !may_end && stmts.len() == used {
            return None;
        }
        let (style, path, items) = self.tab_bar(bar.1, &bar.0)?;
        let field = &items.first()?.0;
        if items.iter().any(|(f, ..)| squeeze(f) != squeeze(field))
            || squeeze(&self.expr_text(&*match_expr.expr)) != squeeze(field)
            || !match_expr.attrs.is_empty()
        {
            return None;
        }
        // The arms: one per tab, in the bar's order, each a braced page; then maybe `_ => {}`.
        let mut arms: Vec<&syn::Arm> = match_expr.arms.iter().collect();
        let wildcard = arms.last().is_some_and(|arm| {
            matches!(arm.pat, Pat::Wild(_))
                && matches!(&*arm.body, Expr::Block(b) if b.block.stmts.is_empty() && b.label.is_none() && b.attrs.is_empty())
        });
        if wildcard {
            arms.pop();
        }
        if arms.len() != items.len() {
            return None;
        }
        let mut blocks = Vec::new();
        for (arm, (_, value, _)) in arms.iter().zip(&items) {
            let Expr::Block(block) = &*arm.body else {
                return None;
            };
            // A guard is part of the pattern in syn 3, so it fails the comparison with the value.
            if !arm.attrs.is_empty()
                || block.label.is_some()
                || !block.attrs.is_empty()
                || squeeze(&self.expr_text(&arm.pat)) != squeeze(value)
            {
                return None;
            }
            blocks.push(&block.block);
        }
        let bodies: Vec<Range<usize>> = blocks
            .iter()
            .map(|b| group_range(b.brace_token.span))
            .collect();
        let start = range_of(&stmts[0])?.start;
        let end = range_of(&stmts[used - 1])?.end;
        // No comment outside the pages: the bar and the `match` are regenerated on an edit.
        if used == 2 {
            let first_end = range_of(&stmts[0])?.end;
            let second_start = range_of(&stmts[1])?.start;
            if has_comment(self.src, stmts[0].to_token_stream(), &[])
                || !self.text(first_end..second_start).trim().is_empty()
                || has_comment(self.src, stmts[1].to_token_stream(), &bodies)
            {
                return None;
            }
        } else if has_comment(self.src, stmts[0].to_token_stream(), &bodies) {
            return None;
        }
        let mut props = Props::new();
        let label_def = container_def(def).prop("tabs")?;
        let tabs: Vec<ComboItem> = items
            .iter()
            .map(|(_, value, label)| ComboItem {
                value: value.trim().to_string(),
                label: match lit(label) {
                    Some(Lit::Str(s)) if s.suffix().is_empty() => PropValue::Text(s.value()),
                    _ => PropValue::Expr(self.expr_text(*label)),
                },
            })
            .collect();
        let _ = label_def;
        props.insert("field", PropValue::Expr(field.trim().to_string()));
        props.insert("tabs", PropValue::Items(tabs));
        props.insert("style", PropValue::Enum(style));
        let brace = group_range(match_expr.brace_token.span);
        let head_src = self.text(start..brace.start + 1);
        let tail_src = self.text(brace.end - 1..end);
        let (head, tail) = crate::emit::tabs_parts(
            def,
            &props,
            ident,
            "",
            path.as_deref().unwrap_or("egui::"),
            "\n",
        );
        if squeeze(&head) != squeeze(head_src) || squeeze(&tail) != squeeze(tail_src) {
            return None;
        }
        // The shape is right: now the nodes.
        if let Some(prefix) = path {
            self.first_path.get_or_insert(prefix);
        }
        let id = self.alloc();
        self.ranges.insert(id, start..end);
        let mut pages = Vec::new();
        let mut cursor = brace.start + 1;
        for (arm, block) in arms.iter().zip(&blocks) {
            let arm_range = range_of(*arm)?;
            let body = group_range(block.brace_token.span);
            let page_id = self.alloc();
            self.ranges.insert(page_id, arm_range.clone());
            let (children, closing, statements) =
                self.block(block, body.start + 1..body.end - 1, &page_ui, false);
            pages.push(Node {
                id: page_id,
                leading: self.text(cursor..arm_range.start).to_string(),
                trailing: String::new(),
                bare_tail: false,
                src: Some(self.text(arm_range.clone()).to_string()),
                kind: NodeKind::Container {
                    def: ContainerId::TabPage,
                    props: Props::new(),
                    children,
                    closing,
                    extra_chain: Vec::new(),
                    ident: page_ui.clone(),
                    recv: page_ui.clone(),
                    closure: String::new(),
                    braced: true,
                    shell: Some(ContainerShell {
                        head: Some(self.text(arm_range.start..statements).to_string()),
                        tail: self.text(body.end - 1..arm_range.end).to_string(),
                    }),
                },
            });
            cursor = arm_range.end;
        }
        let node = Node {
            id,
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: Some(self.text(start..end).to_string()),
            kind: NodeKind::Container {
                def,
                props,
                children: pages,
                closing: self.text(cursor..brace.end - 1).to_string(),
                extra_chain: Vec::new(),
                ident: page_ui,
                recv: ident.to_string(),
                closure: String::new(),
                braced: true,
                shell: Some(ContainerShell {
                    head: Some(head_src.to_string()),
                    tail: tail_src.to_string(),
                }),
            },
        };
        Some((node, used))
    }

    /// The statements of a tab bar on `ui`: all `ui.selectable_value(&mut field, value, label);`,
    /// or all `if ui.add([egui::]Button::new(label)[.frame(false)].selected(field == value))
    /// .clicked() { field = value; }`. Returns the style, the egui path of `Button` and, per tab,
    /// the field's text, the value's text and the label.
    #[allow(clippy::type_complexity)]
    fn tab_bar<'b>(
        &self,
        block: &'b syn::Block,
        ui: &str,
    ) -> Option<(
        &'static str,
        Option<String>,
        Vec<(String, String, &'b Expr)>,
    )> {
        let squeeze = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let mut style = None;
        let mut path = None;
        let mut items = Vec::new();
        for stmt in &block.stmts {
            let (this, item) = match stmt {
                Stmt::Expr(Expr::MethodCall(m), Some(_))
                    if m.method == "selectable_value"
                        && m.args.len() == 3
                        && m.turbofish.is_none()
                        && ui_ref(&m.receiver).as_deref() == Some(ui) =>
                {
                    let Expr::Reference(r) = &m.args[0] else {
                        return None;
                    };
                    r.mutability?;
                    (
                        "selectable",
                        (
                            self.expr_text(&*r.expr),
                            self.expr_text(&m.args[1]),
                            &m.args[2],
                        ),
                    )
                }
                Stmt::Expr(Expr::If(branch), None) if branch.else_branch.is_none() => {
                    let Expr::MethodCall(clicked) = &*branch.cond else {
                        return None;
                    };
                    let Expr::MethodCall(add) = &*clicked.receiver else {
                        return None;
                    };
                    if clicked.method != "clicked"
                        || !clicked.args.is_empty()
                        || add.method != "add"
                        || add.args.len() != 1
                        || ui_ref(&add.receiver).as_deref() != Some(ui)
                    {
                        return None;
                    }
                    let (base, links) = flatten(&add.args[0])?;
                    let Expr::Call(call) = base else { return None };
                    let Expr::Path(p) = &*call.func else {
                        return None;
                    };
                    let (prefix, ty, ctor) = type_path(&p.path)?;
                    if ty != "Button" || ctor != "new" || call.args.len() != 1 {
                        return None;
                    }
                    let (frameless, selected) = match links.as_slice() {
                        [sel] => (false, *sel),
                        [frame, sel] if frame.method == "frame" => (true, *sel),
                        _ => return None,
                    };
                    if selected.method != "selected" || selected.args.len() != 1 {
                        return None;
                    }
                    let Expr::Binary(eq) = &selected.args[0] else {
                        return None;
                    };
                    if !matches!(eq.op, syn::BinOp::Eq(_)) {
                        return None;
                    }
                    let field = self.expr_text(&*eq.left);
                    let value = self.expr_text(&*eq.right);
                    // The handler sets the field to the value, and nothing else.
                    let [Stmt::Expr(Expr::Assign(set), Some(_))] =
                        branch.then_branch.stmts.as_slice()
                    else {
                        return None;
                    };
                    if squeeze(&self.expr_text(&*set.left)) != squeeze(&field)
                        || squeeze(&self.expr_text(&*set.right)) != squeeze(&value)
                    {
                        return None;
                    }
                    if path.get_or_insert(prefix.clone()) != &prefix {
                        return None;
                    }
                    (
                        if frameless { "frameless" } else { "button" },
                        (field, value, &call.args[0]),
                    )
                }
                _ => return None,
            };
            if *style.get_or_insert(this) != this {
                return None;
            }
            items.push(item);
        }
        Some((style?, path, items))
    }

    /// `egui_extras::TableBuilder::new(ui)..column(c)..[.header(h, |mut header| { header.col(|ui| {
    /// .. }); .. })].body(|mut body| { body.row(h, |mut row| { row.col(|ui| { .. }); .. }); .. });`
    /// in exactly the shape the emitter writes (whitespace aside, no comment outside the cells, the
    /// same number of cells as columns in every row, one height for all body rows, only known
    /// builder methods): a Table with its rows and cells. Anything else is left to the ordinary
    /// rules, with nothing recorded.
    fn table(
        &mut self,
        stmt: &Stmt,
        base: &Expr,
        links: &[&ExprMethodCall],
        ident: &str,
    ) -> Option<NodeKind> {
        let squeeze = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let Stmt::Expr(_, Some(_)) = stmt else {
            return None;
        };
        let Expr::Call(call) = base else { return None };
        let Expr::Path(path) = &*call.func else {
            return None;
        };
        let names: Vec<String> = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        let [.., ty, ctor] = names.as_slice() else {
            return None;
        };
        if ty != "TableBuilder"
            || ctor != "new"
            || call.args.len() != 1
            || ui_ref(&call.args[0]).as_deref() != Some(ident)
        {
            return None;
        }
        let defs = container_def(ContainerId::Table).props;
        let mut props = Props::new();
        let mut columns = Vec::new();
        let mut header: Option<(&Expr, &Expr)> = None;
        let mut body: Option<&Expr> = None;
        for link in links {
            if body.is_some() || link.turbofish.is_some() {
                return None;
            }
            let method = link.method.to_string();
            match (method.as_str(), link.args.len()) {
                ("column", 1) if header.is_none() => columns.push(ComboItem {
                    value: self.expr_text(&link.args[0]),
                    label: PropValue::Text(String::new()),
                }),
                ("header", 2) if header.is_none() => header = Some((&link.args[0], &link.args[1])),
                ("body", 1) => body = Some(&link.args[0]),
                (_, 1) if header.is_none() && columns.is_empty() => {
                    let prop = find_builder_prop(defs, &method, &props)?;
                    if !matches!(prop.role, PropRole::Builder(_)) {
                        return None;
                    }
                    props.insert(prop.name, self.value(&link.args[0], prop));
                }
                _ => return None,
            }
        }
        let body = body?;
        if columns.is_empty() || columns.len() > formust_model::MAX_TABLE_COLUMNS {
            return None;
        }
        // The cells of a row closure: `<param>.col(|ui| { .. });` each.
        let cells_of = |closure: &'_ Expr| -> Option<Vec<Expr>> {
            let Expr::Closure(c) = closure else {
                return None;
            };
            let Expr::Block(block) = &*c.body else {
                return None;
            };
            let [Pat::Ident(_)] = c.inputs.iter().collect::<Vec<_>>().as_slice() else {
                return None;
            };
            let mut out = Vec::new();
            for st in &block.block.stmts {
                let Stmt::Expr(Expr::MethodCall(m), Some(_)) = st else {
                    return None;
                };
                if m.method != "col" || m.args.len() != 1 {
                    return None;
                }
                out.push(m.args[0].clone());
            }
            (out.len() == columns.len()).then_some(out)
        };
        let mut rows: Vec<(Range<usize>, Vec<Expr>)> = Vec::new();
        if let Some((height, closure)) = header {
            let prop = container_def(ContainerId::Table).prop("header")?;
            props.insert(prop.name, self.value(height, prop));
            rows.push((range_of(closure)?, cells_of(closure)?));
        }
        let Expr::Closure(body_closure) = body else {
            return None;
        };
        let Expr::Block(body_block) = &*body_closure.body else {
            return None;
        };
        let mut row_height: Option<&Expr> = None;
        for st in &body_block.block.stmts {
            let Stmt::Expr(Expr::MethodCall(m), Some(_)) = st else {
                return None;
            };
            if m.method != "row" || m.args.len() != 2 {
                return None;
            }
            match row_height {
                Some(h) if squeeze(&self.expr_text(h)) != squeeze(&self.expr_text(&m.args[0])) => {
                    return None;
                }
                _ => row_height = Some(&m.args[0]),
            }
            rows.push((range_of(st)?, cells_of(&m.args[1])?));
        }
        let row_height = row_height?;
        if rows.len() - usize::from(header.is_some()) > formust_model::MAX_TABLE_ROWS {
            return None;
        }
        let prop = container_def(ContainerId::Table).prop("row_height")?;
        props.insert(prop.name, self.value(row_height, prop));
        props.insert("columns", PropValue::Items(columns));
        // Every cell a closure with braces; no comment outside the cells.
        let mut bodies = Vec::new();
        for (_, cells) in &rows {
            for cell in cells {
                let closure = self.closure(cell).ok()?;
                let ClosureBody::Block(_) = closure.body else {
                    return None;
                };
                bodies.push(closure.body_range.clone());
            }
        }
        if has_comment(self.src, stmt.to_token_stream(), &bodies) {
            return None;
        }
        // The nodes; undone if the text doesn't come out the same.
        let saved_id = self.next_id;
        let saved_custom = self.custom.len();
        let mut row_nodes = Vec::new();
        for (row_range, cells) in &rows {
            let row_id = self.alloc();
            self.ranges.insert(row_id, row_range.clone());
            let mut cell_nodes = Vec::new();
            for cell in cells {
                let closure = self.closure(cell).ok()?;
                let ClosureBody::Block(block) = closure.body else {
                    return None;
                };
                let whole = range_of(cell)?;
                let brace = closure.body_range.clone();
                let cell_id = self.alloc();
                self.ranges.insert(cell_id, whole.clone());
                let (children, closing, statements) =
                    self.block(block, brace.start + 1..brace.end - 1, &closure.ident, false);
                cell_nodes.push(Node {
                    id: cell_id,
                    leading: String::new(),
                    trailing: String::new(),
                    bare_tail: false,
                    src: Some(self.text(whole.clone()).to_string()),
                    kind: NodeKind::Container {
                        def: ContainerId::TableCell,
                        props: Props::new(),
                        children,
                        closing,
                        extra_chain: Vec::new(),
                        ident: closure.ident.clone(),
                        recv: closure.ident.clone(),
                        closure: closure.text.clone(),
                        braced: true,
                        shell: Some(ContainerShell {
                            head: Some(self.text(whole.start..statements).to_string()),
                            tail: self.text(brace.end - 1..whole.end).to_string(),
                        }),
                    },
                });
            }
            row_nodes.push(Node {
                id: row_id,
                leading: String::new(),
                trailing: String::new(),
                bare_tail: false,
                src: Some(self.text(row_range.clone()).to_string()),
                kind: NodeKind::Container {
                    def: ContainerId::TableRow,
                    props: Props::new(),
                    children: cell_nodes,
                    closing: String::new(),
                    extra_chain: Vec::new(),
                    ident: ident.to_string(),
                    recv: ident.to_string(),
                    closure: String::new(),
                    braced: true,
                    shell: None,
                },
            });
        }
        let kind = NodeKind::Container {
            def: ContainerId::Table,
            props,
            children: row_nodes,
            closing: String::new(),
            extra_chain: Vec::new(),
            ident: ident.to_string(),
            recv: ident.to_string(),
            closure: String::new(),
            braced: true,
            shell: None,
        };
        let check = Node {
            id: NodeId(u32::MAX),
            leading: String::new(),
            trailing: String::new(),
            bare_tail: false,
            src: None,
            kind,
        };
        let range = range_of(stmt)?;
        let written = crate::emit::table_text_for(&check, ident, "egui::", "\n");
        if squeeze(&written) != squeeze(self.text(range)) {
            self.next_id = saved_id;
            self.custom.truncate(saved_custom);
            self.ranges.retain(|id, _| id.0 < saved_id);
            self.legacy_panels.retain(|id| id.0 < saved_id);
            return None;
        }
        Some(check.kind)
    }

    /// `egui::Sides::new()..show(ui, |ui| { .. }, |ui| { .. })`: a Sides holding its two closures as
    /// Side containers, left then right. Both need braces.
    fn sides(
        &mut self,
        stmt: &Stmt,
        props: Props,
        extra_chain: Vec<String>,
        left: &Expr,
        right: &Expr,
        recv: &str,
    ) -> Result<NodeKind, String> {
        let mut closures = Vec::new();
        for e in [left, right] {
            let closure = self.closure(e).map_err(|e| format!("Sides: {e}"))?;
            let ClosureBody::Block(block) = closure.body else {
                return Err("Sides: a closure has no braces".to_string());
            };
            let whole = range_of(e).ok_or("closure has no tokens")?;
            closures.push((
                closure.ident,
                closure.text,
                block,
                closure.body_range,
                whole,
            ));
        }
        let bodies: Vec<Range<usize>> = closures.iter().map(|c| c.3.clone()).collect();
        self.refuse_comments(stmt, &bodies)?;
        let range = range_of(stmt).ok_or("statement has no tokens")?;
        let mut sides = Vec::new();
        let mut cursor: Option<usize> = None;
        for (ident, closure, block, body, whole) in closures {
            let id = self.alloc();
            self.ranges.insert(id, whole.clone());
            let (children, closing, statements) =
                self.block(block, body.start + 1..body.end - 1, &ident, false);
            let leading =
                cursor.map_or_else(String::new, |c| self.text(c..whole.start).to_string());
            sides.push(Node {
                id,
                leading,
                trailing: String::new(),
                bare_tail: false,
                src: Some(self.text(whole.clone()).to_string()),
                kind: NodeKind::Container {
                    def: ContainerId::Side,
                    props: Props::new(),
                    children,
                    closing,
                    extra_chain: Vec::new(),
                    ident,
                    recv: recv.to_string(),
                    closure,
                    braced: true,
                    shell: Some(ContainerShell {
                        head: Some(self.text(whole.start..statements).to_string()),
                        tail: self.text(body.end - 1..whole.end).to_string(),
                    }),
                },
            });
            cursor = Some(whole.end);
        }
        let left_start = range_of(left).ok_or("closure has no tokens")?.start;
        Ok(NodeKind::Container {
            def: ContainerId::Sides,
            props,
            children: sides,
            closing: String::new(),
            extra_chain,
            ident: recv.to_string(),
            recv: recv.to_string(),
            closure: String::new(),
            braced: true,
            shell: Some(ContainerShell {
                head: Some(self.text(range.start..left_start).to_string()),
                tail: self
                    .text(cursor.unwrap_or(range.end)..range.end)
                    .to_string(),
            }),
        })
    }

    /// A dialog helper call, `crate::style::dialog(ctx, "about", |ui| { .. })`: the call up to its
    /// closure is kept verbatim as the `call` property, the closure's body holds the children.
    fn dialog_call(
        &mut self,
        stmt: &Stmt,
        call: &syn::ExprCall,
        ident: &str,
    ) -> Result<NodeKind, String> {
        let closure = call.args.last().ok_or("helper call: function")?;
        let start = range_of(call).ok_or("statement has no tokens")?.start;
        let end = range_of(closure).ok_or("closure has no tokens")?.start;
        let head = self.text(start..end).to_string();
        let mut props = Props::new();
        props.insert("call", PropValue::Expr(head));
        self.finish_container(
            stmt,
            ContainerId::DialogCall,
            props,
            Vec::new(),
            closure,
            ident,
        )
    }

    fn finish_container(
        &mut self,
        stmt: &Stmt,
        def: ContainerId,
        mut props: Props,
        extra_chain: Vec<String>,
        closure: &Expr,
        recv: &str,
    ) -> Result<NodeKind, String> {
        let closure = self.closure(closure)?;
        self.refuse_comments(stmt, std::slice::from_ref(&closure.body_range))?;
        let range = range_of(stmt).ok_or("statement has no tokens")?;
        let body = closure.body_range.clone();
        let (head, tail, children, closing, braced) = match closure.body {
            ClosureBody::Block(block) => {
                let tail = self.text(body.end - 1..range.end).to_string();
                // Spacing and size statements opening the body are the container's properties;
                // their text joins the head, which an edit of the properties regenerates.
                let (statements_props, skip) =
                    self.body_props(block, body.start + 1, &closure.ident, def);
                props.extend(statements_props);
                let (children, closing, statements) = self.block_from(
                    block,
                    body.start + 1..body.end - 1,
                    &closure.ident,
                    false,
                    skip,
                );
                let head = self.text(range.start..statements).to_string();
                (head, tail, children, closing, true)
            }
            ClosureBody::Expr(expr) => {
                let head = self.text(range.start..body.start).to_string();
                let tail = self.text(body.end..range.end).to_string();
                let child_stmt = Stmt::Expr(expr.clone(), None);
                let mut child =
                    self.statement(&child_stmt, body.clone(), &closure.ident, false, true);
                child.bare_tail = is_bare_tail(&child_stmt);
                (head, tail, vec![child], String::new(), false)
            }
        };
        Ok(NodeKind::Container {
            def,
            props,
            children,
            closing,
            extra_chain,
            ident: closure.ident,
            recv: recv.to_string(),
            closure: closure.text,
            braced,
            shell: Some(ContainerShell {
                head: Some(head),
                tail,
            }),
        })
    }

    /// The statements at the start of a container's body that are its properties:
    /// `<ui>.set_min_width(<v>);` ([`PropRole::UiSet`]) and `<ui>.spacing_mut().<f> = <v>;`
    /// ([`PropRole::Spacing`]), on the body's own `Ui`. They stop at the first other statement, a
    /// comment or attribute before one, a comment inside one, or a property seen twice, so nothing
    /// but these statements and white space goes into the head. Returns them and their count.
    fn body_props(
        &self,
        block: &syn::Block,
        inner_start: usize,
        ui: &str,
        def: ContainerId,
    ) -> (Props, usize) {
        let defs = container_def(def).props;
        let mut props = Props::new();
        let mut cursor = inner_start;
        let mut count = 0;
        for stmt in &block.stmts {
            let Some(range) = range_of(stmt) else { break };
            if !self.text(cursor..range.start).trim().is_empty()
                || has_comment(self.src, stmt.to_token_stream(), &[])
            {
                break;
            }
            let Some((prop, value)) = body_statement(stmt, ui, defs) else {
                break;
            };
            if props.contains_key(prop.name) {
                break;
            }
            props.insert(prop.name, self.value(value, prop));
            cursor = range.end;
            count += 1;
        }
        (props, count)
    }

    fn closure<'e>(&self, expr: &'e Expr) -> Result<ClosureParts<'e>, String> {
        let Expr::Closure(closure) = expr else {
            return Err("container with a non-closure argument".to_string());
        };
        let param = closure.inputs.first().filter(|_| closure.inputs.len() == 1);
        let param = match param {
            Some(Pat::Ident(p)) => Some(p),
            Some(Pat::Type(t)) => match &*t.pat {
                Pat::Ident(p) => Some(p),
                _ => None,
            },
            _ => None,
        };
        let Some(param) = param else {
            return Err("container closure parameter is not a plain name".to_string());
        };
        if !matches!(closure.output, ReturnType::Default) {
            return Err("container closure with a return type".to_string());
        }
        let (body, body_range) = match &*closure.body {
            Expr::Block(block) if block.label.is_none() && block.attrs.is_empty() => (
                ClosureBody::Block(&block.block),
                group_range(block.block.brace_token.span),
            ),
            Expr::Block(_) => {
                return Err("container closure body has a label or attribute".to_string());
            }
            other => (
                ClosureBody::Expr(other),
                range_of(other).ok_or("closure body has no tokens")?,
            ),
        };
        let start = range_of(closure).ok_or("closure has no tokens")?.start;
        Ok(ClosureParts {
            ident: param.ident.to_string(),
            body,
            text: self.text(start..body_range.start).to_string(),
            body_range,
        })
    }

    /// `ui.<known>(..)` or `ui.add(<Widget>::new(..).chain())`, with any methods on the response.
    fn widget(&mut self, expr: &Expr, ident: &str) -> Result<WidgetParts, String> {
        let Some((base, links)) = flatten(expr) else {
            return Err("very long method chain".to_string());
        };
        if let Expr::Call(_) = base {
            return self.combo_box(base, &links, ident);
        }
        // `ui.add_enabled_ui(e, |ui| <widget>).inner` and the visible `ui.scope(..).inner`: what
        // the emitter writes for a widget that has no `ui.add` form, or more than one wrapper.
        if let Expr::Field(field) = base
            && let Some(result) = self.ui_wrapper(field, ident)
        {
            let mut parts = result?;
            let mut previous_end = range_of(base).ok_or("chain has no tokens")?.end;
            for link in &links {
                let end = range_of(*link).ok_or("chain has no tokens")?.end;
                let text = self.text(previous_end..end).trim_start().to_string();
                previous_end = end;
                self.response_link(&mut parts, link, text);
            }
            return Ok(parts);
        }
        let Some(name) = ui_ref(base) else {
            return Err(base_reason(base));
        };
        if name != ident {
            return Err(if name == "self" {
                "helper call: method on self".to_string()
            } else {
                "method on something other than the current ui".to_string()
            });
        }
        let Some(first) = links.first() else {
            return Err(expr_kind(expr).to_string());
        };
        let method = first.method.to_string();
        let mut parts = match method.as_str() {
            "add" | "add_sized" | "add_enabled" | "add_visible" => {
                self.add_call(first, &method, ident)?
            }
            _ => self.shorthand(first, &method, ident)?,
        };
        let mut previous_end = range_of(*first).ok_or("chain has no tokens")?.end;
        for link in &links[1..] {
            let end = range_of(*link).ok_or("chain has no tokens")?.end;
            let text = self.text(previous_end..end).trim_start().to_string();
            previous_end = end;
            self.response_link(&mut parts, link, text);
        }
        Ok(parts)
    }

    /// A method on a widget's response: a property such as the tooltip
    /// (`.on_hover_text("..")`) while nothing unknown came before it, else kept verbatim.
    fn response_link(&self, parts: &mut WidgetParts, link: &ExprMethodCall, text: String) {
        let method = link.method.to_string();
        let prop = widget_def(parts.def)
            .props
            .iter()
            .find(|p| matches!(p.role, PropRole::Response(m) if m == method));
        if let Some(prop) = prop
            && parts.response_chain.is_empty()
            && link.args.len() == 1
            && link.turbofish.is_none()
            && !parts.props.contains_key(prop.name)
        {
            let value = self.value(&link.args[0], prop);
            parts.props.insert(prop.name, value);
            return;
        }
        parts.response_chain.push(text);
    }

    /// `<ui>.add_enabled_ui(<e>, |<ui>| <widget>).inner` or
    /// `<ui>.scope(|<ui>| { if !(<v>) { <ui>.set_invisible(); } <widget> }).inner`, `field` being
    /// the `.inner`. `None` if it isn't one of these shapes; an error if it is, around something
    /// that can't be a widget with that property.
    fn ui_wrapper(
        &mut self,
        field: &syn::ExprField,
        ident: &str,
    ) -> Option<Result<WidgetParts, String>> {
        let syn::Member::Named(member) = &field.member else {
            return None;
        };
        let Expr::MethodCall(call) = &*field.base else {
            return None;
        };
        if member != "inner"
            || !field.attrs.is_empty()
            || ui_ref(&call.receiver).as_deref() != Some(ident)
            || call.turbofish.is_some()
        {
            return None;
        }
        // Inside the closures the `Ui` is their parameter: the same name, or any name where the
        // outer `Ui` is a column `cols[0]`, which can't name a parameter.
        let inner_ident = if formust_model::is_plain_ui(ident) {
            ident.to_string()
        } else {
            match call.args.last() {
                Some(Expr::Closure(c)) => match c.inputs.first() {
                    Some(Pat::Ident(p)) => p.ident.to_string(),
                    _ => return None,
                },
                _ => return None,
            }
        };
        let outer_ident = ident;
        let ident = inner_ident.as_str();
        let used = || Err("container result used".to_string());
        // The closure `|ui| ..` on the same `Ui` name, and its body's statements.
        let body = |closure: &Expr| -> Option<Vec<Stmt>> {
            let Expr::Closure(c) = closure else {
                return None;
            };
            let [Pat::Ident(param)] = c.inputs.iter().collect::<Vec<_>>()[..] else {
                return None;
            };
            if param.ident != ident
                || param.by_ref.is_some()
                || param.mutability.is_some()
                || param.subpat.is_some()
                || c.capture.is_some()
                || !matches!(c.output, ReturnType::Default)
            {
                return None;
            }
            match &*c.body {
                Expr::Block(b) if b.label.is_none() && b.attrs.is_empty() => {
                    Some(b.block.stmts.clone())
                }
                Expr::Block(_) => None,
                other => Some(vec![Stmt::Expr(other.clone(), None)]),
            }
        };
        let (role, cond, inner) = match (call.method.to_string().as_str(), call.args.len()) {
            ("add_enabled_ui", 2) => {
                let stmts = body(&call.args[1])?;
                let [Stmt::Expr(inner, None)] = &stmts[..] else {
                    return Some(used());
                };
                (PropRole::AddEnabled, call.args[0].clone(), inner.clone())
            }
            ("scope", 1) => {
                let stmts = body(&call.args[0])?;
                let [Stmt::Expr(Expr::If(test), None), Stmt::Expr(inner, None)] = &stmts[..] else {
                    return None;
                };
                let Expr::Unary(not) = &*test.cond else {
                    return None;
                };
                let hides = match &test.then_branch.stmts[..] {
                    [Stmt::Expr(Expr::MethodCall(m), Some(_))] => {
                        m.method == "set_invisible"
                            && m.args.is_empty()
                            && single_ident(&m.receiver).as_deref() == Some(ident)
                    }
                    _ => false,
                };
                if !matches!(not.op, UnOp::Not(_)) || test.else_branch.is_some() || !hides {
                    return None;
                }
                let cond = match &*not.expr {
                    Expr::Paren(p) if p.attrs.is_empty() => (*p.expr).clone(),
                    other => other.clone(),
                };
                (PropRole::AddVisible, cond, inner.clone())
            }
            _ => return None,
        };
        let mut parts = match self.widget(&inner, ident) {
            Ok(parts) => parts,
            Err(_) => return Some(used()),
        };
        parts.recv = outer_ident.to_string();
        let Some(prop) = widget_def(parts.def)
            .props
            .iter()
            .find(|p| p.role == role)
            .filter(|p| !parts.props.contains_key(p.name))
        else {
            return Some(used());
        };
        let value = self.value(&cond, prop);
        parts.props.insert(prop.name, value);
        Some(Ok(parts))
    }

    fn shorthand(
        &mut self,
        call: &ExprMethodCall,
        method: &str,
        ident: &str,
    ) -> Result<WidgetParts, String> {
        for &def in ALL_WIDGETS {
            for (name, arg_props) in forms::widget_forms(def).shorthand {
                if *name != method {
                    continue;
                }
                if call.args.len() != arg_props.len() {
                    return Err(format!("ui method with unexpected arguments: ui.{method}"));
                }
                let wdef = widget_def(def);
                let mut props = Props::new();
                for (prop_name, arg) in arg_props.iter().zip(call.args.iter()) {
                    let prop = wdef.prop(prop_name).ok_or("registry has no such prop")?;
                    self.widget_arg(wdef.props, prop, arg, &mut props);
                }
                return Ok(WidgetParts {
                    def,
                    props,
                    extra_chain: Vec::new(),
                    response_chain: Vec::new(),
                    recv: ident.to_string(),
                });
            }
        }
        Err(format!("ui method not in the palette: ui.{method}"))
    }

    /// `ui.add(w)`, `ui.add_sized(size, w)`, `ui.add_enabled(on, w)`, `ui.add_visible(on, w)`.
    fn add_call(
        &mut self,
        call: &ExprMethodCall,
        method: &str,
        ident: &str,
    ) -> Result<WidgetParts, String> {
        let expected = if method == "add" { 1 } else { 2 };
        if call.args.len() != expected {
            return Err(format!("ui method with unexpected arguments: ui.{method}"));
        }
        let builder = &call.args[expected - 1];
        let mut parts = self.builder(builder, ident)?;
        if expected == 2 {
            let role = match method {
                "add_sized" => PropRole::AddSized,
                "add_enabled" => PropRole::AddEnabled,
                _ => PropRole::AddVisible,
            };
            let wdef = widget_def(parts.def);
            let Some(prop) = wdef.props.iter().find(|p| p.role == role) else {
                return Err(format!(
                    "ui method not in the palette: ui.{method} with {}",
                    wdef.name
                ));
            };
            parts
                .props
                .insert(prop.name, self.value(&call.args[0], prop));
        }
        Ok(parts)
    }

    /// `Widget::ctor(args).chain()`.
    fn builder(&mut self, expr: &Expr, ident: &str) -> Result<WidgetParts, String> {
        let unknown = "ui.add of a value, not a known builder".to_string();
        let Some((base, links)) = flatten(expr) else {
            return Err(unknown);
        };
        let Expr::Call(call) = base else {
            return Err(unknown);
        };
        let Expr::Path(path) = &*call.func else {
            return Err(unknown);
        };
        let Some((prefix, ty, ctor)) = type_path(&path.path) else {
            return Err(unknown);
        };
        let found = ALL_WIDGETS.iter().copied().find_map(|def| {
            let (t, ctors) = forms::widget_forms(def).builder?;
            if t != ty || def == WidgetId::ComboBox {
                return None;
            }
            ctors
                .iter()
                .find(|(c, args)| *c == ctor && args.len() == call.args.len())
                .map(|(_, args)| (def, *args))
        });
        let Some((def, arg_props)) = found else {
            return Err(format!(
                "ui.add of a widget not in the palette: {ty}::{ctor}"
            ));
        };
        let wdef = widget_def(def);
        let mut props = Props::new();
        for (name, arg) in arg_props.iter().zip(call.args.iter()) {
            let prop = wdef.prop(name).ok_or("registry has no such prop")?;
            self.widget_arg(wdef.props, prop, arg, &mut props);
        }
        let extra_chain = self.builder_chain(wdef.props, base, &links, &mut props)?;
        self.first_path.get_or_insert(prefix);
        Ok(WidgetParts {
            def,
            props,
            extra_chain,
            response_chain: Vec::new(),
            recv: ident.to_string(),
        })
    }

    /// Known methods become properties; the others are kept verbatim. A method seen twice stays
    /// verbatim too, as a property could hold only one of the two.
    fn builder_chain(
        &mut self,
        defs: &'static [PropDef],
        base: &Expr,
        links: &[&ExprMethodCall],
        props: &mut Props,
    ) -> Result<Vec<String>, String> {
        let mut extra = Vec::new();
        let mut previous_end = range_of(base).ok_or("chain has no tokens")?.end;
        for link in links {
            let end = range_of(*link).ok_or("chain has no tokens")?.end;
            let text = self.text(previous_end..end).trim_start().to_string();
            previous_end = end;
            let method = link.method.to_string();
            match (find_builder_prop(defs, &method, props), link.args.len()) {
                (Some(prop), 1) if matches!(prop.role, PropRole::Builder(_)) => {
                    let value = self.value(&link.args[0], prop);
                    props.insert(prop.name, value);
                }
                (Some(prop), n) if n > 1 && is_expr_builder(prop) => {
                    props.insert(prop.name, PropValue::Expr(self.args_text(link)));
                }
                (Some(prop), 0) if matches!(prop.role, PropRole::Flag(_)) => {
                    props.insert(prop.name, PropValue::Bool(true));
                }
                _ => extra.push(text),
            }
        }
        Ok(extra)
    }

    /// `ComboBox::from_label("x").selected_text(..).show_ui(ui, |ui| { static items });`.
    fn combo_box(
        &mut self,
        base: &Expr,
        links: &[&ExprMethodCall],
        ident: &str,
    ) -> Result<WidgetParts, String> {
        let not_palette =
            |ty: &str, ctor: &str| format!("builder not in the palette: {ty}::{ctor}");
        let Expr::Call(call) = base else {
            return Err(base_reason(base));
        };
        let Expr::Path(path) = &*call.func else {
            return Err("helper call: function".to_string());
        };
        let Some((prefix, ty, ctor)) = type_path(&path.path) else {
            return Err("helper call: function".to_string());
        };
        if ty != "ComboBox" {
            return Err(not_palette(&ty, &ctor));
        }
        let wdef = widget_def(WidgetId::ComboBox);
        let (_, ctors) = forms::widget_forms(WidgetId::ComboBox)
            .builder
            .ok_or("no ComboBox form")?;
        let Some((_, arg_props)) = ctors
            .iter()
            .find(|(c, a)| *c == ctor && a.len() == call.args.len())
        else {
            return Err(not_palette(&ty, &ctor));
        };
        let mut props = Props::new();
        for (name, arg) in arg_props.iter().zip(call.args.iter()) {
            let prop = wdef.prop(name).ok_or("registry has no such prop")?;
            props.insert(prop.name, self.value(arg, prop));
        }
        let Some((show, chain)) = links.split_last() else {
            return Err("ComboBox with no show_ui".to_string());
        };
        if show.method != "show_ui" || show.args.len() != 2 {
            return Err(if links.iter().any(|l| l.method == "show_ui") {
                "ComboBox result used".to_string()
            } else {
                format!("ComboBox variant: {}", show.method)
            });
        }
        if ui_ref(&show.args[0]).as_deref() != Some(ident) {
            return Err("container receiver is not the current ui: ComboBox".to_string());
        }
        let closure = self
            .closure(&show.args[1])
            .map_err(|e| format!("ComboBox: {e}"))?;
        let ClosureBody::Block(body) = closure.body else {
            return Err("ComboBox items are not static".to_string());
        };
        let mut items = Vec::new();
        let mut binding: Option<String> = None;
        for stmt in &body.stmts {
            let Stmt::Expr(Expr::MethodCall(item), _) = stmt else {
                return Err("ComboBox items are not static".to_string());
            };
            if item.method != "selectable_value"
                || item.args.len() != 3
                || single_ident(&item.receiver).as_deref() != Some(closure.ident.as_str())
            {
                return Err("ComboBox items are not static".to_string());
            }
            let value = self.expr_text(&item.args[0]);
            if binding.as_ref().is_some_and(|b| *b != value) {
                return Err("ComboBox items are not static".to_string());
            }
            binding = Some(value);
            let label_prop = PropDef {
                name: "label",
                label: "Label",
                kind: PropKind::Text,
                default: formust_model::DefaultValue::None,
                role: PropRole::Arg,
            };
            items.push(ComboItem {
                value: self.expr_text(&item.args[1]),
                label: self.value(&item.args[2], &label_prop),
            });
        }
        let extra_chain = self.builder_chain(wdef.props, base, chain, &mut props)?;
        if let Some(binding) = binding {
            props.insert("value", PropValue::Expr(binding));
        }
        props.insert("items", PropValue::Items(items));
        self.first_path.get_or_insert(prefix);
        Ok(WidgetParts {
            def: WidgetId::ComboBox,
            props,
            extra_chain,
            response_chain: Vec::new(),
            recv: ident.to_string(),
        })
    }

    /// A widget's positional argument. The `text` of a widget with text styling may be
    /// `egui::RichText::new(<text>).size(..).strong()..`: then the text and each method are
    /// properties, if every method is one of them, each once; otherwise the argument is kept whole.
    fn widget_arg(
        &mut self,
        defs: &'static [PropDef],
        prop: &'static PropDef,
        arg: &Expr,
        props: &mut Props,
    ) {
        if prop.name == "text"
            && defs.iter().any(|d| matches!(d.role, PropRole::Rich(_)))
            && let Some((text, styles, prefix)) = self.rich_text(defs, prop, arg)
        {
            props.insert(prop.name, text);
            props.extend(styles);
            self.first_path.get_or_insert(prefix);
            return;
        }
        props.insert(prop.name, self.value(arg, prop));
    }

    /// `[egui::]RichText::new(<text>)` and its known methods: the text, the style properties and the
    /// path prefix.
    fn rich_text(
        &self,
        defs: &'static [PropDef],
        text_prop: &PropDef,
        arg: &Expr,
    ) -> Option<(PropValue, Props, String)> {
        let (base, links) = flatten(arg)?;
        let Expr::Call(call) = base else { return None };
        let Expr::Path(path) = &*call.func else {
            return None;
        };
        let (prefix, ty, ctor) = type_path(&path.path)?;
        if ty != "RichText" || ctor != "new" || call.args.len() != 1 || !call.attrs.is_empty() {
            return None;
        }
        let mut styles = Props::new();
        for link in &links {
            let method = link.method.to_string();
            let prop = defs
                .iter()
                .find(|d| matches!(d.role, PropRole::Rich(m) if m == method))?;
            if styles.contains_key(prop.name) || link.turbofish.is_some() {
                return None;
            }
            let value = match (prop.kind, link.args.len()) {
                (PropKind::Bool, 0) => PropValue::Bool(true),
                (PropKind::Bool, _) | (_, 0) => return None,
                (_, 1) => self.value(&link.args[0], prop),
                _ => return None,
            };
            styles.insert(prop.name, value);
        }
        Some((self.value(&call.args[0], text_prop), styles, prefix))
    }

    /// All arguments of a call as written, from the first to the last: `egui::Align2::LEFT_TOP, [0.0, 0.0]`.
    fn args_text(&self, call: &ExprMethodCall) -> String {
        let first = call.args.first().and_then(range_of);
        let last = call.args.last().and_then(range_of);
        match (first, last) {
            (Some(a), Some(b)) => self.text(a.start..b.end).to_string(),
            _ => String::new(),
        }
    }

    /// An argument's value: typed if it is a literal of the property's kind, else its source text.
    fn value(&self, expr: &Expr, prop: &PropDef) -> PropValue {
        literal_value(expr, prop, &self.current_ui)
            .unwrap_or_else(|| PropValue::Expr(self.expr_text(expr)))
    }
}

/// `<ui>.<m>(<v>);` for a [`PropRole::UiSet`] property `m`, or `<ui>.spacing_mut().<f> = <v>;` for a
/// [`PropRole::Spacing`] one: the property and the value's expression.
fn body_statement<'s>(
    stmt: &'s Stmt,
    ui: &str,
    defs: &'static [PropDef],
) -> Option<(&'static PropDef, &'s Expr)> {
    let Stmt::Expr(expr, Some(_)) = stmt else {
        return None;
    };
    match expr {
        Expr::MethodCall(call)
            if call.attrs.is_empty()
                && call.turbofish.is_none()
                && call.args.len() == 1
                && ui_ref(&call.receiver).as_deref() == Some(ui) =>
        {
            let method = call.method.to_string();
            let prop = defs
                .iter()
                .find(|d| matches!(d.role, PropRole::UiSet(m) if m == method))?;
            Some((prop, &call.args[0]))
        }
        Expr::Assign(assign) if assign.attrs.is_empty() => {
            let Expr::Field(field) = &*assign.left else {
                return None;
            };
            let (syn::Member::Named(name), Expr::MethodCall(call)) = (&field.member, &*field.base)
            else {
                return None;
            };
            if call.method != "spacing_mut"
                || !call.args.is_empty()
                || call.turbofish.is_some()
                || !field.attrs.is_empty()
                || ui_ref(&call.receiver).as_deref() != Some(ui)
            {
                return None;
            }
            let name = name.to_string();
            let prop = defs
                .iter()
                .find(|d| matches!(d.role, PropRole::Spacing(f) if f == name))?;
            Some((prop, &assign.right))
        }
        _ => None,
    }
}

/// The one column a statement of `ui.columns(..)` uses: every `cols` in it is `cols[<literal>]` with
/// the same literal. `None` for none, several, or another use of `cols`.
fn column_of(stream: TokenStream, cols: &str) -> Option<usize> {
    fn walk(stream: TokenStream, cols: &str, found: &mut Option<usize>) -> bool {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        let mut k = 0;
        while k < tokens.len() {
            match &tokens[k] {
                TokenTree::Ident(i) if i == cols => {
                    let Some(TokenTree::Group(g)) = tokens.get(k + 1) else {
                        return false;
                    };
                    if g.delimiter() != Delimiter::Bracket {
                        return false;
                    }
                    let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                    let [TokenTree::Literal(lit)] = &inner[..] else {
                        return false;
                    };
                    let text = lit.to_string();
                    let text = text.strip_suffix("usize").unwrap_or(&text);
                    let Ok(index) = text.parse::<usize>() else {
                        return false;
                    };
                    if found.is_some_and(|f| f != index) {
                        return false;
                    }
                    *found = Some(index);
                    k += 2;
                }
                TokenTree::Group(g) => {
                    if !walk(g.stream(), cols, found) {
                        return false;
                    }
                    k += 1;
                }
                _ => k += 1,
            }
        }
        true
    }
    let mut found = None;
    walk(stream, cols, &mut found).then_some(found).flatten()
}

/// A statement's trailing same-line comment moves from the gap to the node before it. The rest of
/// the gap is the next statement's `leading`.
fn take_trailing<'g>(nodes: &mut [Node], gap: &'g str) -> &'g str {
    let Some(last) = nodes.last_mut() else {
        return gap;
    };
    let (trailing, rest) = gap.split_at(trailing_comment_len(gap));
    last.trailing = trailing.to_string();
    rest
}

/// Where a run of inner attributes ends in the text before a block's first statement. Comments
/// before and among them come along; they can't be told apart from what they sit next to.
fn inner_attrs_end(gap: &str) -> usize {
    if !gap.contains("#!") {
        return 0;
    }
    let Ok(stream) = TokenStream::from_str(gap) else {
        return 0;
    };
    let mut trees = stream.into_iter().peekable();
    let mut end = 0;
    while let (
        Some(TokenTree::Punct(hash)),
        Some(TokenTree::Punct(bang)),
        Some(TokenTree::Group(group)),
    ) = (trees.next(), trees.next(), trees.next())
    {
        if hash.as_char() != '#' || bang.as_char() != '!' || group.delimiter() != Delimiter::Bracket
        {
            break;
        }
        end = group.span().byte_range().end;
    }
    end
}

/// An expression with no `;` that isn't block-like can only stand at the end of a block.
fn is_bare_tail(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(expr, None) => !matches!(
            expr,
            Expr::If(_)
                | Expr::Match(_)
                | Expr::Block(_)
                | Expr::Unsafe(_)
                | Expr::While(_)
                | Expr::Loop(_)
                | Expr::ForLoop(_)
                | Expr::TryBlock(_)
                | Expr::Const(_)
        ),
        Stmt::Macro(mac) => {
            mac.semi_token.is_none() && !matches!(mac.mac.delimiter, syn::MacroDelimiter::Brace(_))
        }
        _ => false,
    }
}

fn widget_kind(parts: WidgetParts, events: Vec<(EventId, String)>) -> NodeKind {
    NodeKind::Widget {
        def: parts.def,
        props: parts.props,
        events,
        extra_chain: parts.extra_chain,
        response_chain: parts.response_chain,
        recv: parts.recv,
        let_prefix: None,
    }
}

fn flow_kind_node(
    children: Vec<Node>,
    closing: String,
    head: String,
    tail: String,
    braced: bool,
    ident: &str,
) -> NodeKind {
    NodeKind::Container {
        def: ContainerId::Flow,
        props: Props::new(),
        children,
        closing,
        extra_chain: Vec::new(),
        ident: ident.to_string(),
        recv: ident.to_string(),
        closure: String::new(),
        braced,
        shell: Some(ContainerShell {
            head: Some(head),
            tail,
        }),
    }
}

fn group_kind(
    children: Vec<Node>,
    head: String,
    closing: String,
    tail: String,
    ident: &str,
) -> NodeKind {
    NodeKind::Container {
        def: ContainerId::FlowGroup,
        props: Props::new(),
        children,
        closing,
        extra_chain: Vec::new(),
        ident: ident.to_string(),
        recv: ident.to_string(),
        closure: String::new(),
        braced: true,
        shell: Some(ContainerShell {
            head: Some(head),
            tail,
        }),
    }
}

/// Is there a widget or a real container anywhere under here? Control flow doesn't count by itself.
fn has_recognised(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Widget { .. } => true,
        NodeKind::Container { def, children, .. } => {
            !def.is_flow() || children.iter().any(has_recognised)
        }
        NodeKind::Custom { .. } => false,
    }
}

/// Finds the property a builder method sets, unless it is already set: `.text("a").text("b")`
/// leaves the second in the chain.
fn find_builder_prop(
    defs: &'static [PropDef],
    method: &str,
    props: &Props,
) -> Option<&'static PropDef> {
    defs.iter()
        .find(|p| matches!(p.role, PropRole::Builder(m) | PropRole::Flag(m) if m == method))
        .filter(|p| !props.contains_key(p.name))
}

/// A builder property holding an expression, which takes a call's arguments as one text when the
/// method has several (`Window::anchor(align, offset)`); the emitter writes them back as they are.
fn is_expr_builder(prop: &PropDef) -> bool {
    matches!(prop.role, PropRole::Builder(_)) && prop.kind == PropKind::Expr
}

fn legacy_panel_method(method: &str) -> Option<&'static str> {
    LEGACY_PANEL_METHODS
        .iter()
        .find(|(old, _)| *old == method)
        .map(|(_, new)| *new)
}

fn literal_value(expr: &Expr, prop: &PropDef, ui: &str) -> Option<PropValue> {
    let expr = match expr {
        Expr::Paren(p) if p.attrs.is_empty() => &*p.expr,
        other => other,
    };
    match prop.kind {
        PropKind::Text => match lit(expr)? {
            Lit::Str(s) if s.suffix().is_empty() => Some(PropValue::Text(s.value())),
            _ => None,
        },
        PropKind::Bool => match lit(expr)? {
            Lit::Bool(b) => Some(PropValue::Bool(b.value)),
            _ => None,
        },
        PropKind::Int => {
            let (negative, inner) = strip_neg(expr);
            match lit(inner)? {
                Lit::Int(i) if i.suffix().is_empty() => {
                    let value: i64 = i.base10_parse().ok()?;
                    Some(PropValue::Int(if negative { -value } else { value }))
                }
                _ => None,
            }
        }
        PropKind::Float => {
            let (negative, inner) = strip_neg(expr);
            match lit(inner)? {
                Lit::Float(f) if f.suffix().is_empty() => {
                    let value: f64 = f.base10_parse().ok()?;
                    value.is_finite().then_some(PropValue::Float(if negative {
                        -value
                    } else {
                        value
                    }))
                }
                _ => None,
            }
        }
        PropKind::Enum { values, .. } => {
            let Expr::Path(p) = expr else { return None };
            let last = p.path.segments.last()?.ident.to_string();
            let value = values.iter().copied().find(|v| *v == last)?;
            Some(PropValue::Enum(value))
        }
        PropKind::Color => match color_literal(expr) {
            Some(rgba) => Some(PropValue::color(rgba)),
            None => color_pair(expr, ui),
        },
        PropKind::Stroke => stroke_literal(expr, ui),
        PropKind::Expr | PropKind::Items | PropKind::Vec2 => None,
    }
}

/// `[egui::]Stroke::new(<float>, <colour>)` with a literal width and a colour [`literal_value`]
/// reads (one, or a light and dark pair on the current `ui`). Others stay `Expr`.
fn stroke_literal(expr: &Expr, ui: &str) -> Option<PropValue> {
    let Expr::Call(call) = expr else { return None };
    let Expr::Path(path) = &*call.func else {
        return None;
    };
    let (_, ty, ctor) = type_path(&path.path)?;
    if ty != "Stroke" || ctor != "new" || call.args.len() != 2 || !call.attrs.is_empty() {
        return None;
    }
    let width = match lit(&call.args[0])? {
        Lit::Float(f) if f.suffix().is_empty() => f.base10_parse::<f64>().ok()?,
        _ => return None,
    };
    let color = &call.args[1];
    let (light, dark) = match color_literal(color) {
        Some(rgba) => (rgba, None),
        None => match color_pair(color, ui)? {
            PropValue::Color { light, dark } => (light, dark),
            _ => return None,
        },
    };
    width
        .is_finite()
        .then_some(PropValue::stroke(width, light, dark))
}

fn lit(expr: &Expr) -> Option<&Lit> {
    match expr {
        Expr::Lit(l) if l.attrs.is_empty() => Some(&l.lit),
        _ => None,
    }
}

fn strip_neg(expr: &Expr) -> (bool, &Expr) {
    match expr {
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) && u.attrs.is_empty() => (true, &u.expr),
        other => (false, other),
    }
}

/// `if ui.visuals().dark_mode { <dark> } else { <light> }`, the form emit writes for a pair, on the
/// current `ui`. Any other test stays `Expr`: it needn't mean the mode.
fn color_pair(expr: &Expr, ui: &str) -> Option<PropValue> {
    let Expr::If(branch) = expr else { return None };
    if !branch.attrs.is_empty() {
        return None;
    }
    let Expr::Field(field) = &*branch.cond else {
        return None;
    };
    let syn::Member::Named(member) = &field.member else {
        return None;
    };
    let Expr::MethodCall(call) = &*field.base else {
        return None;
    };
    if member != "dark_mode"
        || call.method != "visuals"
        || !call.args.is_empty()
        || ui_ref(&call.receiver).as_deref() != Some(ui)
    {
        return None;
    }
    let dark = only_color(&branch.then_branch)?;
    let (_, otherwise) = branch.else_branch.as_ref()?;
    let Expr::Block(light) = &**otherwise else {
        return None;
    };
    if light.label.is_some() || !light.attrs.is_empty() {
        return None;
    }
    Some(PropValue::color_pair(only_color(&light.block)?, dark))
}

fn only_color(block: &syn::Block) -> Option<[u8; 4]> {
    match block.stmts.as_slice() {
        [Stmt::Expr(expr, None)] => color_literal(expr),
        _ => None,
    }
}

/// `Color32::from_rgb(1, 2, 3)` and `from_rgba_unmultiplied(1, 2, 3, 4)`. Other colours stay `Expr`.
fn color_literal(expr: &Expr) -> Option<[u8; 4]> {
    let Expr::Call(call) = expr else { return None };
    let Expr::Path(path) = &*call.func else {
        return None;
    };
    let (_, ty, ctor) = type_path(&path.path)?;
    if ty != "Color32" {
        return None;
    }
    let mut channels = Vec::new();
    for arg in &call.args {
        match lit(arg)? {
            Lit::Int(i) if i.suffix().is_empty() => channels.push(i.base10_parse::<u8>().ok()?),
            _ => return None,
        }
    }
    match (ctor.as_str(), channels.as_slice()) {
        ("from_rgb", [r, g, b]) => Some([*r, *g, *b, 255]),
        ("from_rgba_unmultiplied", [r, g, b, a]) => Some([*r, *g, *b, *a]),
        _ => None,
    }
}

fn base_reason(base: &Expr) -> String {
    match base {
        Expr::Field(field) => {
            let mut inner = &*field.base;
            while let Expr::Field(f) = inner {
                inner = &f.base;
            }
            if single_ident(inner).as_deref() == Some("self") {
                "helper call: method on a field of self".to_string()
            } else if matches!(inner, Expr::MethodCall(_)) {
                "container result used".to_string()
            } else {
                "method on something other than the current ui".to_string()
            }
        }
        Expr::Path(_) => "helper call: associated function".to_string(),
        Expr::Macro(_) => "macro".to_string(),
        other => format!("method on {}", expr_kind(other)),
    }
}

fn macro_reason(mac: &syn::Macro) -> String {
    let name = mac
        .path
        .segments
        .last()
        .map_or_else(String::new, |s| s.ident.to_string());
    format!("macro: {name}!")
}

fn local_reason(local: &syn::Local, ident: &str) -> String {
    let Some(init) = &local.init else {
        return "let: plain".to_string();
    };
    // Is the value a widget or container result? Those are the bindings worth recognising later.
    let Some((base, links)) = flatten(&init.expr) else {
        return "let: plain".to_string();
    };
    if ui_ref(base).as_deref() == Some(ident) && !links.is_empty() {
        let method = links[0].method.to_string();
        let is_container = ALL_CONTAINERS.iter().any(|&c| {
            forms::container_forms(c)
                .shorthand
                .is_some_and(|(m, _)| m == method)
        });
        if is_container {
            return "let: container result".to_string();
        }
        let is_widget = matches!(
            method.as_str(),
            "add" | "add_sized" | "add_enabled" | "add_visible"
        ) || ALL_WIDGETS.iter().any(|&w| {
            forms::widget_forms(w)
                .shorthand
                .iter()
                .any(|(m, _)| *m == method)
        });
        if is_widget {
            return "let: widget response".to_string();
        }
        return "let: value from ui".to_string();
    }
    "let: plain".to_string()
}

fn item_kind(item: &Item) -> &'static str {
    match item {
        Item::Use(_) => "use",
        Item::Fn(_) => "fn",
        Item::Const(_) => "const",
        Item::Static(_) => "static",
        Item::Struct(_) => "struct",
        Item::Enum(_) => "enum",
        _ => "other",
    }
}

/// Whether a comment sits between two tokens of this stream. Doc comments are tokens and don't
/// count. `exclude` lists the parts kept verbatim by other means (container bodies, event handlers),
/// which may hold comments freely.
fn has_comment(src: &str, stream: TokenStream, exclude: &[Range<usize>]) -> bool {
    let mut cursor = None;
    gap_with_comment(src, stream, exclude, &mut cursor)
}

fn gap_with_comment(
    src: &str,
    stream: TokenStream,
    exclude: &[Range<usize>],
    cursor: &mut Option<usize>,
) -> bool {
    for tree in stream {
        let whole = tree.span().byte_range();
        if let Some(part) = exclude
            .iter()
            .find(|e| e.start <= whole.start && whole.end <= e.end && whole.start != whole.end)
        {
            if text_between(src, *cursor, part.start) {
                return true;
            }
            *cursor = Some(part.end);
            continue;
        }
        match tree {
            TokenTree::Group(group) => {
                if group.delimiter() == Delimiter::None {
                    if gap_with_comment(src, group.stream(), exclude, cursor) {
                        return true;
                    }
                    continue;
                }
                if text_between(src, *cursor, whole.start) {
                    return true;
                }
                *cursor = Some(whole.start + 1);
                if gap_with_comment(src, group.stream(), exclude, cursor)
                    || text_between(src, *cursor, whole.end - 1)
                {
                    return true;
                }
                *cursor = Some(whole.end);
            }
            _ => {
                if whole.start == whole.end {
                    continue;
                }
                if text_between(src, *cursor, whole.start) {
                    return true;
                }
                *cursor = Some(whole.end);
            }
        }
    }
    false
}

fn text_between(src: &str, cursor: Option<usize>, end: usize) -> bool {
    let Some(start) = cursor else { return false };
    if end <= start {
        return false;
    }
    src.get(start..end)
        .is_some_and(|gap| !gap.trim().is_empty())
}
