//! What Run Form reads from a source text, beside the forms: the `bool` fields that keep a dialog
//! closed (so the run can start it open), and the free functions whose only parameter is the `ui`
//! (so the run can draw one). Read with `syn` on a large stack, like the importer; a text that does
//! not parse has none of either.

use std::ops::Range;

use proc_macro2::TokenStream;
use syn::visit::Visit;
use syn::{Expr, FnArg, ImplItem, Item, Member, Pat, Stmt, Type, UnOp};

use crate::import::{blank_prefix, check_limits, on_big_stack, owner_of, range_of};

/// A free function `fn name(ui: &mut [path::]Ui)`: a `ui` form with no owner, which Run Form calls
/// as `name(ui)`. `range` is the function's bytes in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreeUi {
    pub name: String,
    pub range: Range<usize>,
}

/// Parses `src` and runs `f` on the file, on a large stack. `None` when the text does not parse or
/// is too deep for the importer.
fn with_file<T: Send>(src: &str, f: impl FnOnce(&syn::File) -> T + Send) -> Option<T> {
    on_big_stack(|| {
        let blanked = blank_prefix(src);
        let stream: TokenStream = blanked.parse().ok()?;
        check_limits(&stream).ok()?;
        drop(stream);
        proc_macro2::extra::invalidate_current_thread_spans();
        let out = syn::parse_file(&blanked).ok().map(|file| f(&file));
        proc_macro2::extra::invalidate_current_thread_spans();
        out
    })
    .flatten()
}

/// Is the signature `fn name(ui: &mut [path::]Ui)`: no receiver, no generics, one parameter, the `Ui`?
fn only_ui(sig: &syn::Signature) -> bool {
    if !sig.generics.params.is_empty() || sig.variadic.is_some() || sig.asyncness.is_some() {
        return false;
    }
    let mut inputs = sig.inputs.iter();
    let (Some(FnArg::Typed(arg)), None) = (inputs.next(), inputs.next()) else {
        return false;
    };
    let Type::Reference(reference) = &*arg.ty else {
        return false;
    };
    let Type::Path(ty) = &*reference.elem else {
        return false;
    };
    reference.mutability.is_some()
        && ty.qself.is_none()
        && ty.path.segments.last().is_some_and(|s| s.ident == "Ui")
}

/// The top-level free functions of `src` whose only parameter is the `ui` (see [`FreeUi`]), in order.
/// Functions in a `mod` are left out: the run calls the function from the file's own module.
pub fn free_ui_fns(src: &str) -> Vec<FreeUi> {
    with_file(src, |file| {
        file.items
            .iter()
            .filter_map(|item| match item {
                Item::Fn(f) if only_ui(&f.sig) => Some(FreeUi {
                    name: f.sig.ident.to_string(),
                    range: range_of(f)?,
                }),
                _ => None,
            })
            .collect()
    })
    .unwrap_or_default()
}

/// The free function of `src` whose only parameter is the `ui` that holds the 1-based `line`.
pub fn free_ui_at(src: &str, line: u32) -> Option<FreeUi> {
    free_ui_fns(src).into_iter().find(|f| {
        line_of(src, f.range.start) <= line && line_of(src, f.range.end.saturating_sub(1)) >= line
    })
}

/// Is `name` a top-level free function of `src` whose only parameter is the `ui`?
pub fn takes_only_ui(src: &str, name: &str) -> bool {
    free_ui_fns(src).iter().any(|f| f.name == name)
}

fn line_of(text: &str, at: usize) -> u32 {
    text.get(..at)
        .map_or(0, |t| t.matches('\n').count() as u32 + 1)
}

/// `self.field` as the field's name.
fn self_field(expr: &Expr) -> Option<String> {
    let Expr::Field(field) = expr else {
        return None;
    };
    let Expr::Path(base) = &*field.base else {
        return None;
    };
    let Member::Named(name) = &field.member else {
        return None;
    };
    base.path.is_ident("self").then(|| name.to_string())
}

/// A single identifier, as a name.
fn ident(expr: &Expr) -> Option<String> {
    let Expr::Path(path) = expr else { return None };
    path.path.get_ident().map(ToString::to_string)
}

/// Collects, in a dialog's body, the `self` fields that `.open(..)` takes (`&mut self.f`, or a local
/// `let mut x = self.f;` then `&mut x`).
#[derive(Default)]
struct OpenArgs {
    /// `let [mut] x = self.f;`: the local and the field.
    locals: Vec<(String, String)>,
    /// What `.open(&mut ..)` takes: a field (`Ok`) or a local (`Err`).
    taken: Vec<Result<String, String>>,
}

impl<'ast> Visit<'ast> for OpenArgs {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let (Pat::Ident(pat), Some(init)) = (&local.pat, &local.init)
            && pat.subpat.is_none()
            && pat.by_ref.is_none()
            && init.diverge.is_none()
            && let Some(field) = self_field(&init.expr)
        {
            self.locals.push((pat.ident.to_string(), field));
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "open"
            && call.args.len() == 1
            && let Some(Expr::Reference(arg)) = call.args.first()
            && arg.mutability.is_some()
        {
            if let Some(field) = self_field(&arg.expr) {
                self.taken.push(Ok(field));
            } else if let Some(local) = ident(&arg.expr) {
                self.taken.push(Err(local));
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

/// A top-level statement that gates the dialog on a field: `if self.f { .. }` (with no `else`), or
/// `if !self.f { return; }`.
fn gate(stmt: &Stmt) -> Option<String> {
    let Stmt::Expr(Expr::If(test), _) = stmt else {
        return None;
    };
    if test.else_branch.is_some() {
        return None;
    }
    if let Some(field) = self_field(&test.cond) {
        return Some(field);
    }
    let Expr::Unary(not) = &*test.cond else {
        return None;
    };
    let returns = matches!(
        test.then_branch.stmts.as_slice(),
        [Stmt::Expr(Expr::Return(r), _)] if r.expr.is_none()
    );
    (matches!(not.op, UnOp::Not(_)) && returns)
        .then(|| self_field(&not.expr))
        .flatten()
}

/// The `bool` fields of `owner` that keep its dialog `method` closed, so that a run can set them to
/// `true` and the dialog starts open: what the dialog's `.open(..)` takes (`&mut self.open`, or
/// `let mut open = self.open;` then `&mut open`), and a top-level `if self.open { .. }` or
/// `if !self.open { return; }`. Only fields declared `bool` in `struct owner` here count, so the
/// run's `owner.f = true;` always compiles. In order, without repeats.
pub fn open_flags(src: &str, owner: &str, method: &str) -> Vec<String> {
    with_file(src, |file| {
        let bools: Vec<String> = file
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Struct(s) if s.ident == owner => Some(s),
                _ => None,
            })
            .flat_map(|s| match &s.fields {
                syn::Fields::Named(named) => named.named.iter().collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .filter(|field| matches!(&field.ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("bool")))
            .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
            .collect();
        let mut flags = Vec::new();
        let methods = file.items.iter().filter_map(|item| match item {
            Item::Impl(imp) if owner_of(&imp.self_ty).as_deref() == Some(owner) => Some(imp),
            _ => None,
        });
        for imp in methods {
            for item in &imp.items {
                let ImplItem::Fn(f) = item else { continue };
                if f.sig.ident != method {
                    continue;
                }
                let mut found: Vec<String> = f.block.stmts.iter().filter_map(gate).collect();
                let mut args = OpenArgs::default();
                args.visit_block(&f.block);
                for taken in args.taken {
                    match taken {
                        Ok(field) => found.push(field),
                        Err(local) => found.extend(
                            args.locals
                                .iter()
                                .filter(|(name, _)| *name == local)
                                .map(|(_, field)| field.clone()),
                        ),
                    }
                }
                for field in found {
                    if bools.contains(&field) && !flags.contains(&field) {
                        flags.push(field);
                    }
                }
            }
        }
        flags
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIALOG: &str = "#[derive(Default)]\npub struct D {\n    open: bool,\n    shown: bool,\n    name: String,\n    count: u32,\n}\n\nimpl D {\n    pub fn show(&mut self, ctx: &egui::Context) {\n        let mut open = self.open;\n        egui::Window::new(\"About\").open(&mut open).show(ctx, |ui| {\n            ui.label(\"Hello\");\n        });\n        self.open = open;\n    }\n\n    pub fn direct(&mut self, ctx: &egui::Context) {\n        egui::Window::new(\"W\").open(&mut self.shown).show(ctx, |ui| {\n            ui.label(\"x\");\n        });\n    }\n\n    pub fn gated(&mut self, ctx: &egui::Context) {\n        if self.shown {\n            egui::Modal::new(egui::Id::new(\"m\")).show(ctx, |ui| {\n                ui.label(\"x\");\n            });\n        }\n    }\n\n    pub fn early(&mut self, ctx: &egui::Context) {\n        if !self.open {\n            return;\n        }\n        egui::Window::new(\"E\").show(ctx, |ui| {\n            ui.label(\"x\");\n        });\n    }\n\n    pub fn not_bool(&mut self, ctx: &egui::Context) {\n        if self.count > 0 {}\n        let mut n = self.name.clone();\n        egui::Window::new(\"N\").open(&mut n.is_empty()).show(ctx, |ui| {});\n    }\n}\n";

    #[test]
    fn a_local_copy_of_a_field_given_to_open_is_the_flag() {
        assert_eq!(open_flags(DIALOG, "D", "show"), ["open"]);
    }

    #[test]
    fn a_field_given_to_open_or_gating_the_dialog_is_the_flag() {
        assert_eq!(open_flags(DIALOG, "D", "direct"), ["shown"]);
        assert_eq!(open_flags(DIALOG, "D", "gated"), ["shown"]);
        assert_eq!(open_flags(DIALOG, "D", "early"), ["open"]);
    }

    #[test]
    fn only_bool_fields_of_the_owner_count() {
        assert!(open_flags(DIALOG, "D", "not_bool").is_empty());
        assert!(open_flags(DIALOG, "Other", "show").is_empty());
        assert!(open_flags(DIALOG, "D", "missing").is_empty());
        // A field of another type with the same name: not set, since `= true` would not compile.
        let text = DIALOG.replace("    open: bool,", "    open: Option<bool>,");
        assert!(open_flags(&text, "D", "show").is_empty());
        assert!(open_flags("fn broken(", "D", "show").is_empty());
    }

    #[test]
    fn free_functions_taking_only_the_ui_are_found_with_their_lines() {
        let text = "pub fn form(ui: &mut egui::Ui) {\n    ui.label(\"x\");\n}\n\nfn two(ui: &mut egui::Ui, n: u32) {}\nfn shared(ui: &egui::Ui) {}\nfn generic<T>(ui: &mut Ui) {}\nmod inner {\n    pub fn deep(ui: &mut egui::Ui) {}\n}\nimpl A {\n    fn method(ui: &mut egui::Ui) {}\n}\nfn bare(ui: &mut Ui) {}\n";
        let names: Vec<String> = free_ui_fns(text).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["form", "bare"]);
        assert_eq!(free_ui_at(text, 2).map(|f| f.name).as_deref(), Some("form"));
        assert_eq!(free_ui_at(text, 4), None);
        assert_eq!(
            free_ui_at(text, 14).map(|f| f.name).as_deref(),
            Some("bare")
        );
        assert!(takes_only_ui(text, "form"));
        assert!(!takes_only_ui(text, "two"));
        assert!(!takes_only_ui(text, "deep"));
    }
}
