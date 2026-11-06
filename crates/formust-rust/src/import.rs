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

/// Does the body have a top-level statement (or tail) `[egui::]Window::new(..)...show(ctx, |ui| ..)`?
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
        ty == "Window"
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
    let front = &segments[..segments.len() - 2];
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
        let mut nodes: Vec<Node> = Vec::new();
        let first = block
            .stmts
            .first()
            .and_then(range_of)
            .map_or(inner.end, |r| r.start);
        let start = inner.start + inner_attrs_end(self.text(inner.start..first));
        let mut cursor = start;
        let count = block.stmts.len();
        for (index, stmt) in block.stmts.iter().enumerate() {
            let Some(range) = range_of(stmt) else {
                continue;
            };
            let gap = self.text(cursor..range.start);
            let leading = take_trailing(&mut nodes, gap);
            let is_tail = index + 1 == count;
            let mut node = self.statement(stmt, range.clone(), ident, top, is_tail);
            node.leading = leading.to_string();
            node.bare_tail = is_bare_tail(stmt);
            nodes.push(node);
            cursor = range.end;
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
            Expr::Field(_) => Err(base_reason(expr)),
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
        let parts = self.widget(&init.expr, ident).map_err(|_| plain())?;
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
        if let Some(name) = single_ident(base) {
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
        let show_at = links
            .iter()
            .position(|l| matches!(l.method.to_string().as_str(), "show" | "show_inside"));
        let Some(show_at) = show_at else {
            return Some(Err(format!("container show variant: {ty}::{ctor}")));
        };
        if show_at + 1 != links.len() {
            return Some(Err("container result used".to_string()));
        }
        let show = links[show_at];
        let args: Vec<&Expr> = show.args.iter().collect();
        if args.len() != 2 {
            return Some(Err(format!(
                "container with unexpected arguments: {ty}.show"
            )));
        }
        let Some(recv) = single_ident(args[0]) else {
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
        Some(self.finish_container(stmt, def, props, extra_chain, args[1], &recv))
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
        props: Props,
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
                let (children, closing, statements) =
                    self.block(block, body.start + 1..body.end - 1, &closure.ident, false);
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
        let Some(name) = single_ident(base) else {
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
            parts
                .response_chain
                .push(self.text(previous_end..end).trim_start().to_string());
            previous_end = end;
        }
        Ok(parts)
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
                    props.insert(prop.name, self.value(arg, prop));
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
            props.insert(prop.name, self.value(arg, prop));
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
        if single_ident(&show.args[0]).as_deref() != Some(ident) {
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

    /// An argument's value: typed if it is a literal of the property's kind, else its source text.
    fn value(&self, expr: &Expr, prop: &PropDef) -> PropValue {
        literal_value(expr, prop, &self.current_ui)
            .unwrap_or_else(|| PropValue::Expr(self.expr_text(expr)))
    }
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
        PropKind::Expr | PropKind::Items => None,
    }
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
        || single_ident(&call.receiver).as_deref() != Some(ui)
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
    if single_ident(base).as_deref() == Some(ident) && !links.is_empty() {
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
