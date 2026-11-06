//! The `.frm.rs` copy: the complete text of the source file with the designer's forms in it. Each
//! exported form replaces its function in the copy, byte ranges from the importer show where, and
//! everything else (the struct, other impls, helpers, `use` and `mod` lines, comments) stays byte for
//! byte. A header line per form goes at the top. The copy compiles and runs by itself; Save never
//! writes the source and the source needs no line for it.
//!
//! "Save into .rs" (an explicit action of its own) writes the same text into the source itself,
//! without the header lines: [`update_source`] makes it and [`write_source`] writes it.
//!
//! New state (the fields that new stateful widgets bind to as `self.form.<field>`) goes into the copy
//! too: a `#[derive(Default)]` state struct after the owner's struct, a `form: LoginForm` field in the
//! owner's struct, and `form: Default::default()` in the owner's struct literals that list their
//! fields (`Login { .. }`, or `Self { .. }` in an impl of the owner). A unit or tuple owner can't take
//! the field, and literals inside macros or in other files are not seen.

use std::ops::Range;
use std::path::Path;

use formust_model::Form;
use syn::visit::Visit;

use crate::emit::{emit_fn, emit_form_item, state_struct};
use crate::frm::{FrmFile, Header, parse_frm};
use crate::import::{
    ImportError, blank_prefix, group_range, import, on_big_stack, owner_of, range_of,
};

/// The text of the `.frm.rs` after exporting `forms` into `base`, the copy as it is now (see
/// [`standalone_base`]). Each form replaces the function of its source id (the header's, else its
/// own) and is appended at the end when the copy doesn't have it; the copy's other forms and all its
/// other text stay. `parse_frm(x).emit() == x` for a file this wrote.
pub fn update_frm(base: &FrmFile, forms: &[(Form, Option<Header>)]) -> String {
    patch(base, forms, true)
}

/// The forms written into `base` (see [`update_frm`]). `headers`: the copy's header block is
/// rewritten; without it (the `.rs` itself, see [`update_source`]) header lines are neither written
/// nor removed.
fn patch(base: &FrmFile, forms: &[(Form, Option<Header>)], headers: bool) -> String {
    let text = base.text.as_str();
    let nl = if text.contains("\r\n") {
        "\r\n"
    } else {
        forms.first().map_or("\n", |(f, _)| f.newline.as_str())
    };
    let id_of = |form: &Form, header: &Option<Header>| {
        header.as_ref().map_or_else(|| form.id(), |h| h.id.clone())
    };
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    // Which passed form each of the copy's forms is now, if any.
    let mut taken: Vec<Option<usize>> = vec![None; base.forms.len()];
    let mut appended: Vec<usize> = Vec::new();
    let mut replaced: Vec<Range<usize>> = Vec::new();
    for (i, (form, header)) in forms.iter().enumerate() {
        let id = id_of(form, header);
        let at =
            (0..base.forms.len()).find(|&j| taken[j].is_none() && base.forms[j].source_id() == id);
        match at {
            Some(j) => {
                taken[j] = Some(i);
                let range = base.forms[j].imported.fn_range.clone();
                let new = emit_fn(form);
                if text.get(range.clone()) != Some(new.as_str()) {
                    replaced.push(range.clone());
                    edits.push((range, new));
                }
            }
            None => appended.push(i),
        }
    }

    // State: per owner, the struct, the owner's field and its struct literals.
    let sites = sites_of(text);
    let mut owners: Vec<&str> = Vec::new();
    let mut tail_items: Vec<String> = Vec::new();
    for (form, _) in forms {
        let owner = form.owner.as_str();
        if owner.is_empty() || owners.contains(&owner) {
            continue;
        }
        owners.push(owner);
        let same_owner: Vec<&Form> = forms
            .iter()
            .map(|(f, _)| f)
            .filter(|f| f.owner == owner)
            .collect();
        let Some(struct_text) = state_struct(&same_owner) else {
            continue;
        };
        let (name, field) = (&form.state_struct, &form.state_field);
        let owner_site = sites.structs.iter().find(|s| s.name == owner);
        match sites.structs.iter().find(|s| &s.name == name) {
            Some(site) => {
                if text.get(site.item.clone()) != Some(struct_text.as_str()) {
                    edits.push((site.item.clone(), struct_text));
                }
            }
            None => match owner_site {
                Some(site) => edits.push((
                    site.item.end..site.item.end,
                    format!(
                        "{nl}{nl}{}{struct_text}",
                        line_indent(text, site.item.start)
                    ),
                )),
                None => tail_items.push(struct_text),
            },
        }
        if let Some(fields) = owner_site.and_then(|s| s.fields.as_ref())
            && !fields.names.contains(field)
        {
            edits.push(fields.insert(&format!("{field}: {name}"), nl));
        }
        for literal in sites.literals.iter().filter(|l| l.ty == owner && !l.rest) {
            let inside_replaced = replaced
                .iter()
                .any(|r| r.start <= literal.fields.at && literal.fields.at <= r.end);
            if !literal.fields.names.contains(field) && !inside_replaced {
                edits.push(
                    literal
                        .fields
                        .insert(&format!("{field}: Default::default()"), nl),
                );
            }
        }
    }

    if !headers {
        return finish(text, edits, tail_items, &appended, forms, nl);
    }
    // The header lines: the old ones go, the new block is the first lines of the file.
    for range in &base.header_lines {
        edits.push((range.clone(), String::new()));
    }
    let mut block = String::new();
    let mut header_line = |form: &Form, header: &Header| {
        // The form's names, whatever the header was read with.
        let mut line = header.clone();
        line.state = (!form.state_names_are_default())
            .then(|| (form.state_struct.clone(), form.state_field.clone()));
        block.push_str(&line.to_line());
        block.push_str(nl);
    };
    for (j, old) in base.forms.iter().enumerate() {
        match taken[j] {
            Some(i) => {
                if let Some(header) = &forms[i].1 {
                    header_line(&forms[i].0, header);
                }
            }
            None => {
                if let Some(header) = &old.header {
                    header_line(&old.imported.form, header);
                }
            }
        }
    }
    for &i in &appended {
        if let Some(header) = &forms[i].1 {
            header_line(&forms[i].0, header);
        }
    }
    let start = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    edits.push((start..start, block));
    finish(text, edits, tail_items, &appended, forms, nl)
}

/// `edits` applied to `text`, then the items that had no place in it (`tail_items`, the forms of
/// `appended`) at the end.
fn finish(
    text: &str,
    edits: Vec<(Range<usize>, String)>,
    tail_items: Vec<String>,
    appended: &[usize],
    forms: &[(Form, Option<Header>)],
    nl: &str,
) -> String {
    let mut out = apply(text, edits);
    let mut tail: Vec<String> = tail_items;
    tail.extend(appended.iter().map(|&i| emit_form_item(&forms[i].0)));
    for item in tail {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(nl);
        }
        if !out.trim().is_empty() {
            out.push_str(nl);
        }
        out.push_str(&item);
        out.push_str(nl);
    }
    out
}

/// What a new export builds on: the copy as it is (`existing`, read with [`parse_frm`]), or the
/// source's text when there is no copy yet. A form file an older Formust wrote for `include!` (only
/// the forms, see [`FrmFile::included`]) becomes a copy: the source without its `include!` of a
/// `.frm.rs` and without the functions that file replaced, then that file's items. `None` for the
/// source means it can't be read; the existing file is then kept as it is.
pub fn standalone_base(
    existing: Option<&FrmFile>,
    source: Option<&str>,
) -> Result<FrmFile, ImportError> {
    match (existing, source) {
        (Some(file), Some(src)) if file.included => migrate(file, src),
        (Some(file), _) => Ok(file.clone()),
        (None, Some(src)) => parse_frm(src),
        (None, None) => parse_frm(""),
    }
}

/// What [`update_source`] made of a `.rs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceUpdate {
    /// The new text of the `.rs`.
    pub text: String,
    /// The source id (`owner::method`) of each form whose function was replaced, in the order the
    /// forms were given. Empty when the `.rs` already has every form as given.
    pub written: Vec<String>,
}

/// Why forms can't be written into a `.rs`. Nothing is written then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// The `.rs` doesn't parse.
    Parse(ImportError),
    /// The `.rs` has no function for this form (its source id).
    Missing(String),
    /// The form has state (`self.form.<field>`) but its owner has no struct with named fields in
    /// this file to take the field. `owner` is the owner's name, or the function's for a free one.
    NoStateField { owner: String },
    /// The new text doesn't read back to the forms given: a bug here.
    NotIdempotent(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Parse(e) => write!(
                f,
                "the .rs doesn't parse at line {}, column {}: {}",
                e.line, e.column, e.message
            ),
            SourceError::Missing(id) => write!(f, "the .rs has no {id}"),
            SourceError::NoStateField { owner } => write!(
                f,
                "{owner} has new state (self.form.…) but no struct with named fields in this file to hold it"
            ),
            SourceError::NotIdempotent(what) => write!(
                f,
                "the new text doesn't read back to the same forms ({what}); nothing was written"
            ),
        }
    }
}

impl std::error::Error for SourceError {}

/// The text of the `.rs` itself with `forms` written into it: what the `.frm.rs` copy is, without
/// its header lines. Each form whose function differs from the one in `source` (matched by the
/// header's id, else the form's own) replaces it; a form the `.rs` has as given is left alone, so an
/// unedited legacy `update` stays `update`. New state goes in as in the copy (a state struct after
/// the owner's struct, the owner's field, its struct literals). Everything else stays byte for byte.
///
/// The result is checked before it is returned: it parses, and each replaced function reads back as
/// the form's text, so importing the result gives the same forms.
pub fn update_source(
    source: &str,
    forms: &[(Form, Option<Header>)],
) -> Result<SourceUpdate, SourceError> {
    let base = parse_frm(source).map_err(SourceError::Parse)?;
    let mut chosen: Vec<(Form, Option<Header>)> = Vec::new();
    let mut written = Vec::new();
    for (form, header) in forms {
        let id = header.as_ref().map_or_else(|| form.id(), |h| h.id.clone());
        let Some(there) = base.forms.iter().find(|f| f.source_id() == id) else {
            return Err(SourceError::Missing(id));
        };
        if emit_fn(form) == emit_fn(&there.imported.form) {
            continue;
        }
        chosen.push((form.clone(), header.clone()));
        written.push(id);
    }
    if chosen.is_empty() {
        return Ok(SourceUpdate {
            text: source.to_string(),
            written,
        });
    }
    // State needs the owner's struct here, with named fields; the copy would append the struct at
    // the end and leave `self.form` undeclared, which a source must not get.
    let sites = sites_of(source);
    let mut owners: Vec<&str> = Vec::new();
    for (form, _) in &chosen {
        let owner = form.owner.as_str();
        if owners.contains(&owner) {
            continue;
        }
        owners.push(owner);
        let same: Vec<&Form> = chosen
            .iter()
            .map(|(f, _)| f)
            .filter(|f| f.owner == owner)
            .collect();
        if state_struct(&same).is_none() {
            continue;
        }
        let fields = sites
            .structs
            .iter()
            .find(|s| !owner.is_empty() && s.name == owner)
            .and_then(|s| s.fields.as_ref());
        if fields.is_none() {
            return Err(SourceError::NoStateField {
                owner: if owner.is_empty() {
                    form.method.clone()
                } else {
                    owner.to_string()
                },
            });
        }
    }
    let text = patch(&base, &chosen, false);
    let back = crate::import::import(&text)
        .map_err(|e| SourceError::NotIdempotent(format!("it doesn't parse: {e}")))?;
    for (form, _) in &chosen {
        let want = emit_fn(form);
        if !back
            .forms
            .iter()
            .any(|f| text.get(f.fn_range.clone()) == Some(want.as_str()))
        {
            return Err(SourceError::NotIdempotent(form.id()));
        }
    }
    Ok(SourceUpdate { text, written })
}

/// Why [`write_source`] didn't write.
#[derive(Debug)]
pub enum SourceWriteError {
    /// The file is not the text the new one was made from: it changed since.
    ChangedOnDisk,
    /// It isn't text.
    NotText,
    /// What was to be written doesn't parse. A bug; the file stays.
    NewDoesNotParse(ImportError),
    Io(std::io::Error),
}

impl std::fmt::Display for SourceWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceWriteError::ChangedOnDisk => {
                write!(f, "the file changed on disk; it was not overwritten")
            }
            SourceWriteError::NotText => write!(f, "the file is not text; it was not overwritten"),
            SourceWriteError::NewDoesNotParse(e) => write!(
                f,
                "the new text has a syntax error ({e}); nothing was written"
            ),
            SourceWriteError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SourceWriteError {}

/// Writes `text` over the `.rs` at `path`, but only while the file is still exactly `expected` (the
/// text `text` was made from) and only if `text` parses. A symlink is followed, so the file it
/// points to is written and the link stays. The write goes to a temporary file beside it, which
/// takes the old file's permissions, and is renamed over it, so a failure leaves the old one whole.
pub fn write_source(path: &Path, expected: &str, text: &str) -> Result<(), SourceWriteError> {
    let path = std::fs::canonicalize(path).map_err(SourceWriteError::Io)?;
    let bytes = std::fs::read(&path).map_err(SourceWriteError::Io)?;
    let now = String::from_utf8(bytes).map_err(|_| SourceWriteError::NotText)?;
    if now != expected {
        return Err(SourceWriteError::ChangedOnDisk);
    }
    crate::import::import(text).map_err(SourceWriteError::NewDoesNotParse)?;
    let permissions = std::fs::metadata(&path).map(|m| m.permissions()).ok();
    crate::frm::write_atomic(&path, text, permissions).map_err(SourceWriteError::Io)
}

fn migrate(old: &FrmFile, src: &str) -> Result<FrmFile, ImportError> {
    let source = import(src)?;
    let mut cuts: Vec<Range<usize>> = Vec::new();
    for form in &old.forms {
        if let Some(there) = source.form(&form.source_id()) {
            // A trait impl moved as a whole into the old form file.
            let cut = if there.form.impl_header.contains(" for ") {
                there.impl_range.clone()
            } else {
                there.fn_range.clone()
            };
            cuts.push(cut);
        }
    }
    cuts.extend(sites_of(src).includes);
    let rest = apply(src, cuts.into_iter().map(|c| (c, String::new())).collect());
    let nl = if src.contains("\r\n") { "\r\n" } else { "\n" };
    // The header lines first, so they are read as a copy's, then the source, then the old items.
    let mut text = String::new();
    for range in &old.header_lines {
        text.push_str(old.text.get(range.clone()).unwrap_or_default());
        if !text.ends_with('\n') {
            text.push_str(nl);
        }
    }
    text.push_str(&rest);
    let items = apply(
        &old.text,
        old.header_lines
            .iter()
            .map(|r| (r.clone(), String::new()))
            .collect(),
    );
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str(nl);
    }
    text.push_str(nl);
    text.push_str(items.trim_start());
    parse_frm(&text)
}

/// `edits` applied to `text`. They must not overlap; an empty range is an insertion.
fn apply(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by(|a, b| (b.0.start, b.0.end).cmp(&(a.0.start, a.0.end)));
    let mut out = text.to_string();
    let mut limit = usize::MAX;
    for (range, new) in edits {
        if range.end > limit || out.get(range.clone()).is_none() {
            // Overlapping or out of place: left out rather than cut into other text.
            continue;
        }
        limit = range.start;
        out.replace_range(range, &new);
    }
    out
}

/// A list of named fields between braces, a struct's or a struct literal's.
#[derive(Clone, Debug)]
struct FieldList {
    names: Vec<String>,
    /// Where a new field goes.
    at: usize,
    /// The list has fields and no trailing comma.
    comma: bool,
    /// The list ends with a comma (or is empty): the new field does too.
    trailing: bool,
    /// The fields stand on lines of their own, with this indent.
    indent: Option<String>,
    /// `{}` with nothing between the braces.
    empty: bool,
}

/// The spaces and tabs that start the line of `at`.
fn line_indent(src: &str, at: usize) -> String {
    let start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    src[start..at]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

impl FieldList {
    fn new(
        src: &str,
        brace: Range<usize>,
        fields: &[(String, Range<usize>)],
        comma: Option<Range<usize>>,
    ) -> FieldList {
        let line_indent = |at: usize| line_indent(src, at);
        match fields.last() {
            None => {
                let inner = src
                    .get(brace.start + 1..brace.end.saturating_sub(1))
                    .unwrap_or_default();
                FieldList {
                    names: Vec::new(),
                    at: brace.start + 1,
                    comma: false,
                    trailing: true,
                    indent: inner
                        .contains('\n')
                        .then(|| format!("{}    ", line_indent(brace.start))),
                    empty: inner.is_empty(),
                }
            }
            Some((_, last)) => {
                let first = &fields[0].1;
                let multiline = src
                    .get(brace.start + 1..first.start)
                    .is_some_and(|gap| gap.contains('\n'));
                FieldList {
                    names: fields.iter().map(|(n, _)| n.clone()).collect(),
                    at: comma.as_ref().map_or(last.end, |c| c.end),
                    comma: comma.is_none(),
                    trailing: comma.is_some(),
                    indent: multiline.then(|| line_indent(last.start)),
                    empty: false,
                }
            }
        }
    }

    /// The edit that adds `field` (`form: LoginForm`) to the list.
    fn insert(&self, field: &str, nl: &str) -> (Range<usize>, String) {
        let comma = if self.comma { "," } else { "" };
        let trail = if self.trailing { "," } else { "" };
        let text = match &self.indent {
            Some(indent) => format!("{comma}{nl}{indent}{field}{trail}"),
            None if self.names.is_empty() && self.empty => format!(" {field} "),
            None if self.names.is_empty() => format!(" {field}"),
            None => format!("{comma} {field}{trail}"),
        };
        (self.at..self.at, text)
    }
}

/// A struct declared in the file.
#[derive(Clone, Debug)]
struct StructSite {
    name: String,
    item: Range<usize>,
    /// `None` for a unit or tuple struct.
    fields: Option<FieldList>,
}

/// `Login { .. }`, or `Self { .. }` in an impl of `Login`.
#[derive(Clone, Debug)]
struct Literal {
    ty: String,
    fields: FieldList,
    /// It ends in `..rest`, which fills in the other fields.
    rest: bool,
}

/// Where a copy can take new state, and the `include!`s an older form file needed.
#[derive(Default)]
struct Sites {
    structs: Vec<StructSite>,
    literals: Vec<Literal>,
    /// Top-level `include!("….frm.rs");` items.
    includes: Vec<Range<usize>>,
}

fn sites_of(src: &str) -> Sites {
    on_big_stack(|| {
        let blanked = blank_prefix(src);
        let sites = match syn::parse_file(&blanked) {
            Ok(file) => {
                let mut collect = Collect {
                    src,
                    owners: Vec::new(),
                    out: Sites::default(),
                };
                for item in &file.items {
                    if let syn::Item::Macro(m) = item
                        && m.mac.path.is_ident("include")
                        && m.mac.tokens.to_string().contains(".frm.rs")
                        && let Some(range) = range_of(item)
                    {
                        collect.out.includes.push(range);
                    }
                }
                collect.visit_file(&file);
                collect.out
            }
            Err(_) => Sites::default(),
        };
        proc_macro2::extra::invalidate_current_thread_spans();
        sites
    })
    .unwrap_or_default()
}

struct Collect<'a> {
    src: &'a str,
    /// The self types of the impls around, innermost last.
    owners: Vec<Option<String>>,
    out: Sites,
}

fn named(member: &syn::Member) -> Option<String> {
    match member {
        syn::Member::Named(ident) => Some(ident.to_string()),
        syn::Member::Unnamed(_) => None,
    }
}

impl<'ast> Visit<'ast> for Collect<'_> {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        self.owners.push(owner_of(&i.self_ty));
        syn::visit::visit_item_impl(self, i);
        self.owners.pop();
    }

    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        if let Some(item) = range_of(i) {
            let fields = match &i.fields {
                syn::Fields::Named(list) => {
                    let names: Vec<(String, Range<usize>)> = list
                        .named
                        .iter()
                        .filter_map(|f| Some((f.ident.as_ref()?.to_string(), range_of(f)?)))
                        .collect();
                    let comma = list
                        .named
                        .pairs()
                        .last()
                        .and_then(|p| p.punct().and_then(|c| range_of(*c)));
                    Some(FieldList::new(
                        self.src,
                        group_range(list.brace_token.span),
                        &names,
                        comma,
                    ))
                }
                syn::Fields::Unnamed(_) | syn::Fields::Unit => None,
            };
            self.out.structs.push(StructSite {
                name: i.ident.to_string(),
                item,
                fields,
            });
        }
        syn::visit::visit_item_struct(self, i);
    }

    fn visit_expr_struct(&mut self, i: &'ast syn::ExprStruct) {
        let ty = match i.path.segments.last() {
            Some(seg) if i.path.segments.len() == 1 && seg.ident == "Self" => {
                self.owners.last().cloned().flatten()
            }
            Some(seg) if i.qself.is_none() => Some(seg.ident.to_string()),
            _ => None,
        };
        if let Some(ty) = ty {
            let fields: Vec<(String, Range<usize>)> = i
                .fields
                .iter()
                .filter_map(|f| Some((named(&f.member)?, range_of(f)?)))
                .collect();
            let comma = i
                .fields
                .pairs()
                .last()
                .and_then(|p| p.punct().and_then(|c| range_of(*c)));
            self.out.literals.push(Literal {
                ty,
                fields: FieldList::new(self.src, group_range(i.brace_token.span), &fields, comma),
                rest: i.dot2_token.is_some(),
            });
        }
        syn::visit::visit_expr_struct(self, i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use formust_model::WidgetId;

    fn with_email(src: &str) -> String {
        let base = parse_frm(src).unwrap();
        let source = import(src).unwrap();
        let mut form = source.forms[0].form.clone();
        let node = form.new_bound_widget(WidgetId::TextEditSingleline, "email");
        form.insert(form.root.id, 0, node).unwrap();
        update_frm(
            &base,
            &[(form, Some(Header::for_source("a.rs", &source.forms[0])))],
        )
    }

    #[test]
    fn new_state_goes_into_the_owner_and_its_literals() {
        let src = "pub struct A {\n    name: String,\n}\n\nimpl A {\n    pub fn new() -> Self {\n        Self { name: String::new() }\n    }\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"x\");\n    }\n}\n\nimpl Default for A {\n    fn default() -> Self {\n        A {\n            name: \"a\".into(),\n        }\n    }\n}\n\nfn other() -> A { A { name: String::new(), ..A::default() } }\n";
        let out = with_email(src);
        assert!(
            out.contains("pub struct A {\n    name: String,\n    form: AForm,\n}\n\n#[derive(Default)] pub struct AForm { pub email: String }\n\nimpl A {"),
            "{out}"
        );
        assert!(
            out.contains("Self { name: String::new(), form: Default::default() }"),
            "{out}"
        );
        assert!(
            out.contains("A {\n            name: \"a\".into(),\n            form: Default::default(),\n        }"),
            "{out}"
        );
        assert!(
            out.contains("A { name: String::new(), ..A::default() }"),
            "a literal with `..` is left alone: {out}"
        );
        // Read back, the state is the form's, and a second export changes nothing.
        let file = parse_frm(&out).unwrap();
        assert_eq!(file.forms[0].imported.form.state_struct, "AForm");
        assert_eq!(file.forms[0].imported.form.state.len(), 1);
        assert_eq!(file.emit(), out);
    }

    #[test]
    fn single_line_and_empty_field_lists_take_the_field_too() {
        let src = "struct A { name: String }\nimpl A {\n    fn ui(&mut self, ui: &mut egui::Ui) { ui.label(\"x\"); }\n}\n";
        let out = with_email(src);
        assert!(
            out.contains("struct A { name: String, form: AForm }"),
            "{out}"
        );
        let src = "struct A {}\nimpl A {\n    fn ui(&mut self, ui: &mut egui::Ui) { ui.label(\"x\"); }\n}\n";
        let out = with_email(src);
        assert!(out.contains("struct A { form: AForm }"), "{out}");
    }

    #[test]
    fn a_form_in_an_inline_module_is_exported_where_it_is() {
        let src = "mod inner {\n    #[derive(Default)]\n    pub struct A {\n        on: bool,\n    }\n    impl A {\n        pub fn ui(&mut self, ui: &mut egui::Ui) {\n            ui.checkbox(&mut self.on, \"on\");\n        }\n    }\n}\n";
        let out = with_email(src);
        assert!(
            out.contains("    pub struct A {\n        on: bool,\n        form: AForm,\n    }\n\n    #[derive(Default)] pub struct AForm { pub email: String }\n    impl A {\n        pub fn ui(&mut self, ui: &mut egui::Ui) {\n            ui.text_edit_singleline(&mut self.form.email);"),
            "{out}"
        );
        assert!(out.ends_with("    }\n}\n"), "{out}");
        assert_eq!(parse_frm(&out).unwrap().emit(), out);
    }

    /// The copy's text without its header lines.
    fn without_headers(copy: &str) -> String {
        copy.split_inclusive('\n')
            .filter(|l| !l.starts_with("// formust "))
            .collect()
    }

    const TWO: &str = "use eframe::egui;\n\n/// The owner.\npub struct A {\n    name: String,\n}\n\nimpl A {\n    pub fn new() -> Self {\n        Self { name: String::new() }\n    }\n\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"x\"); // keep this comment\n    }\n\n    pub fn other(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"y\");\n    }\n}\n\nfn helper() -> u32 {   42 }\n";

    /// Save into .rs writes what the copy holds, without the headers: the edited function and the
    /// new state, nothing else changed, and the other form left as it was.
    #[test]
    fn into_source_is_the_copy_without_headers_and_keeps_the_rest() {
        let source = import(TWO).unwrap();
        let mut form = source.forms[0].form.clone();
        let node = form.new_bound_widget(WidgetId::TextEditSingleline, "email");
        form.insert(form.root.id, 1, node).unwrap();
        let header = Some(Header::for_source("a.rs", &source.forms[0]));
        let other = (source.forms[1].form.clone(), None);
        let out = update_source(TWO, &[(form.clone(), header.clone()), other]).unwrap();
        assert_eq!(out.written, ["A::ui"], "the unedited form is not written");
        let copy = update_frm(&parse_frm(TWO).unwrap(), &[(form.clone(), header)]);
        assert_eq!(out.text, without_headers(&copy));
        assert!(!out.text.contains("// formust"), "{}", out.text);
        assert!(
            out.text
                .contains("ui.text_edit_singleline(&mut self.form.email);")
                && out.text.contains("form: AForm,")
                && out
                    .text
                    .contains("Self { name: String::new(), form: Default::default() }")
                && out
                    .text
                    .contains("#[derive(Default)] pub struct AForm { pub email: String }"),
            "{}",
            out.text
        );
        // Byte for byte outside the edits.
        assert!(out.text.starts_with(
            "use eframe::egui;\n\n/// The owner.\npub struct A {\n    name: String,\n"
        ));
        assert!(out.text.ends_with("    pub fn other(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"y\");\n    }\n}\n\nfn helper() -> u32 {   42 }\n"));
        assert!(out.text.contains("ui.label(\"x\"); // keep this comment"));

        // Re-imported, the forms are the same, and writing them again changes nothing.
        let back = import(&out.text).unwrap();
        assert_eq!(emit_fn(&back.forms[0].form), emit_fn(&form));
        let again: Vec<(Form, Option<Header>)> =
            back.forms.iter().map(|f| (f.form.clone(), None)).collect();
        let second = update_source(&out.text, &again).unwrap();
        assert!(second.written.is_empty());
        assert_eq!(second.text, out.text);
    }

    #[test]
    fn into_source_leaves_an_unedited_legacy_update_alone_and_renames_an_edited_one() {
        let src = "pub struct Old { n: u32 }\nimpl eframe::App for Old {\n    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {\n        egui::CentralPanel::default().show(ctx, |ui| {\n            ui.label(\"a\");\n        });\n    }\n}\n";
        let source = import(src).unwrap();
        let form = source.forms[0].form.clone();
        let out = update_source(src, &[(form.clone(), None)]).unwrap();
        assert!(out.written.is_empty());
        assert_eq!(out.text, src, "byte for byte");

        let mut edited = form;
        let node = edited.new_widget(WidgetId::Separator);
        let root = edited.widget_root();
        edited.insert(root, 0, node).unwrap();
        let out = update_source(src, &[(edited.clone(), None)]).unwrap();
        assert_eq!(out.written, ["Old::update"]);
        assert!(
            out.text.contains("fn ui(&mut self, ui: &mut egui::Ui"),
            "{}",
            out.text
        );
        assert!(!out.text.contains("fn update("), "{}", out.text);
        let back = import(&out.text).unwrap();
        assert_eq!(emit_fn(&back.forms[0].form), emit_fn(&edited));
    }

    #[test]
    fn into_source_refuses_what_it_cannot_write() {
        // State for a tuple owner has nowhere to go.
        let src = "pub struct T(u32);\nimpl T {\n    fn ui(&mut self, ui: &mut egui::Ui) { ui.label(\"x\"); }\n}\n";
        let source = import(src).unwrap();
        let mut form = source.forms[0].form.clone();
        let node = form.new_bound_widget(WidgetId::Checkbox, "on");
        form.insert(form.root.id, 0, node).unwrap();
        assert_eq!(
            update_source(src, &[(form, None)]),
            Err(SourceError::NoStateField { owner: "T".into() })
        );
        // A form the source doesn't have.
        let other = import(TWO).unwrap().forms[1].form.clone();
        let mut renamed = other.clone();
        renamed.owner = "Nobody".into();
        assert_eq!(
            update_source(src, &[(renamed, None)]),
            Err(SourceError::Missing("Nobody::other".into()))
        );
        assert!(matches!(
            update_source("fn (", &[]),
            Err(SourceError::Parse(_))
        ));
    }

    #[test]
    fn write_source_checks_the_disk_and_keeps_permissions_and_links() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("formust-write-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rs = dir.join("a.rs");
        std::fs::write(&rs, TWO).unwrap();
        std::fs::set_permissions(&rs, std::fs::Permissions::from_mode(0o640)).unwrap();
        let new = TWO.replace("\"x\"", "\"z\"");
        // Not what the new text was made from: refused, the file stays.
        assert!(matches!(
            write_source(&rs, "something else", &new),
            Err(SourceWriteError::ChangedOnDisk)
        ));
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), TWO);
        // A new text that doesn't parse: refused.
        assert!(matches!(
            write_source(&rs, TWO, "fn ("),
            Err(SourceWriteError::NewDoesNotParse(_))
        ));
        // Through a symlink: the file is written, the link stays a link.
        let link = dir.join("link.rs");
        std::os::unix::fs::symlink(&rs, &link).unwrap();
        write_source(&link, TWO, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), new);
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let mode = std::fs::metadata(&rs).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert!(!dir.join(".a.rs.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_included_form_file_becomes_a_copy() {
        let src = "#[derive(Default)]\npub struct A {\n    pub form: AForm,\n}\n\ninclude!(\"a.frm.rs\");\n\nfn keep() {}\n";
        let old = "// formust 1 | a.rs | A::ui | src-hash 00000000\n#[derive(Default)] pub struct AForm { pub email: String }\nimpl A {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.text_edit_singleline(&mut self.form.email);\n    }\n}\nfn by_hand() {}\n";
        let old = parse_frm(old).unwrap();
        assert!(old.included);
        let base = standalone_base(Some(&old), Some(src)).unwrap();
        assert!(!base.included);
        assert!(!base.text.contains("include!"), "{}", base.text);
        assert!(
            base.text.contains("fn keep() {}") && base.text.contains("fn by_hand() {}"),
            "{}",
            base.text
        );
        assert_eq!(base.forms[0].source_id(), "A::ui");
        let forms: Vec<(Form, Option<Header>)> = base
            .forms
            .iter()
            .map(|f| (f.imported.form.clone(), f.header.clone()))
            .collect();
        let out = update_frm(&base, &forms);
        assert!(
            out.starts_with("// formust 1 | a.rs | A::ui | src-hash 00000000\n"),
            "{out}"
        );
        assert_eq!(out.matches("form: AForm").count(), 1, "{out}");
    }
}
