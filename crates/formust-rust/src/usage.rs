//! "Use in code": what to know about the exported `.frm.rs`. It is a standalone copy of the `.rs`
//! with the designer's forms in it, so the `.rs` needs no change and the copy needs no `include!`.
//! Shown are how a program can use the copy, what Save does for the form's new state, and the
//! handler stubs the form calls that the copy doesn't define yet (Formust doesn't write those).

use formust_model::{ContainerId, Form, NodeKind, WidgetId};

/// What the user may want to know or do, for one form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseInCode {
    /// The copy and the source, in words: both stand alone.
    pub standalone: String,
    /// `#[path = "login.frm.rs"] mod login;`: a program that has `mod login;` builds the copy in
    /// place of the source with this. `None` for a crate root (`main.rs`, `lib.rs`).
    pub module: Option<String>,
    /// What Save does for the form's new state, or what the user must do when it can't.
    pub state: Option<String>,
    /// `impl Login { fn on_go_clicked(&mut self) {} }` for the handlers the copy doesn't define.
    pub stubs: Option<String>,
    /// What the form's code needs from the program beyond egui itself: image loaders for an
    /// image, the `egui_extras` dependency for a table. Formust adds neither.
    pub requirements: Vec<String>,
}

/// The notes for `form`, exported to the copy `copy_path` (relative to the source's folder) of the
/// source `source_name`. `copy` is the copy's text as Save writes it, to leave out what it already
/// has.
pub fn use_in_code(
    form: &Form,
    copy_path: &str,
    source_name: &str,
    copy: Option<&str>,
) -> UseInCode {
    let copy = copy.unwrap_or_default();
    let copy_name = copy_path.rsplit('/').next().unwrap_or(copy_path);
    let standalone = format!(
        "{copy_name} is a standalone copy of {source_name} with this form in it: it compiles and runs by itself, needs no include!, and {source_name} needs no change. Formust never writes {source_name}."
    );
    let stem = source_name.strip_suffix(".rs").unwrap_or(source_name);
    let module = (!matches!(stem, "main" | "lib") && !stem.is_empty())
        .then(|| format!("#[path = \"{copy_path}\"]\nmod {stem};"));
    let has_state = !form.state.is_empty() || form.state_src.is_some();
    let state = (has_state && !form.owner.is_empty()).then(|| {
        let (owner, field, ty) = (&form.owner, &form.state_field, &form.state_struct);
        if declares_field(copy, owner, field) {
            format!(
                "The new fields are in struct {ty}. Save adds it to the copy with a `{field}: {ty}` field in struct {owner}, and `{field}: Default::default()` where the copy builds a {owner} with its fields listed."
            )
        } else if declares_type(copy, owner) {
            format!(
                "struct {owner} has no named fields, so it can't hold the form's state (`{field}: {ty}`). Give it named fields, or bind the new widgets to values of your own."
            )
        } else {
            format!(
                "struct {owner} is not in {copy_name}: add `{field}: {ty},` to it where it is declared."
            )
        }
    });
    let mut handlers: Vec<String> = Vec::new();
    form.root.walk(&mut |n| {
        if let NodeKind::Widget { events, .. } = &n.kind {
            for (_, body) in events {
                for name in self_calls(body) {
                    if !handlers.contains(&name) && !defines_fn(copy, &name) {
                        handlers.push(name);
                    }
                }
            }
        }
    });
    let stubs = (!handlers.is_empty() && !form.owner.is_empty()).then(|| {
        let fns: Vec<String> = handlers
            .iter()
            .map(|h| format!("    fn {h}(&mut self) {{}}"))
            .collect();
        format!("impl {} {{\n{}\n}}", form.owner, fns.join("\n"))
    });
    UseInCode {
        standalone,
        module,
        state,
        stubs,
        requirements: requirements(form),
    }
}

/// What the program must provide for the form's code to work as designed.
pub fn requirements(form: &Form) -> Vec<String> {
    let (mut image, mut table) = (false, false);
    form.root.walk(&mut |n| match &n.kind {
        NodeKind::Widget {
            def: WidgetId::Image,
            ..
        } => image = true,
        NodeKind::Container {
            def: ContainerId::Table,
            ..
        } => table = true,
        _ => {}
    });
    let mut out = Vec::new();
    if image {
        out.push("Images: egui shows an image only once the program has installed image loaders, for example egui_extras::install_image_loaders(&cc.egui_ctx) with egui_extras's image features (\"file://\" also needs its \"file\" feature). Formust adds no dependency; an `include_image!` path is read relative to the source file.".to_string());
    }
    if table {
        out.push("Tables: TableBuilder is in the egui_extras crate. Your Cargo.toml needs egui_extras at egui's version; Formust doesn't add it.".to_string());
    }
    out
}

/// `self.on_x_clicked()` calls with no arguments, the shape of a generated handler.
fn self_calls(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("self.") {
        let after = &rest[i + 5..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty()
            && after[name.len()..].trim_start().starts_with("()")
            && name.starts_with("on_")
        {
            out.push(name.clone());
        }
        rest = &rest[i + 5..];
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Does the text have `fn <name>` as whole words?
fn defines_fn(text: &str, name: &str) -> bool {
    find_words(text, &["fn", name]).is_some()
}

/// The byte offset of `words` in order, separated by whitespace only, as whole words.
fn find_words(text: &str, words: &[&str]) -> Option<usize> {
    let first = words.first()?;
    let mut from = 0;
    while let Some(i) = text[from..].find(first) {
        let start = from + i;
        from = start + first.len();
        if text[..start].chars().next_back().is_some_and(is_word) {
            continue;
        }
        let mut at = start + first.len();
        let mut ok = true;
        for word in &words[1..] {
            let rest = &text[at..];
            let trimmed = rest.trim_start();
            if trimmed.len() == rest.len() || !trimmed.starts_with(word) {
                ok = false;
                break;
            }
            at += rest.len() - trimmed.len() + word.len();
        }
        if ok && !text[at..].chars().next().is_some_and(is_word) {
            return Some(start);
        }
    }
    None
}

/// Is `struct <owner>` preceded by a `#[derive(..Default..)]`?
fn derives_default(text: &str, owner: &str) -> bool {
    let Some(at) = find_words(text, &["struct", owner]) else {
        return false;
    };
    // The attributes are on the lines just above (and `pub` may sit in between).
    let before = &text[..at];
    let lines: Vec<&str> = before.lines().rev().take(6).collect();
    lines
        .iter()
        .take_while(|l| {
            l.trim().is_empty()
                || l.trim().starts_with("#[")
                || l.trim().starts_with("///")
                || l.trim() == "pub"
                || l.trim().starts_with("pub")
        })
        .any(|l| {
            let l = l.trim();
            l.starts_with("#[derive") && l.contains("Default")
        })
}

/// Does the source give `owner` a `Default`: a `#[derive(Default)]` on its `struct`, or an
/// `impl Default for <owner>`? Text search, so a `Default` from another file is not seen.
pub fn has_default(text: &str, owner: &str) -> bool {
    derives_default(text, owner) || find_words(text, &["Default", "for", owner]).is_some()
}

/// Is `owner` declared as a `struct` or an `enum` in this text?
pub fn declares_type(text: &str, owner: &str) -> bool {
    find_words(text, &["struct", owner]).is_some() || find_words(text, &["enum", owner]).is_some()
}

/// Does `struct <owner> { .. }` already have the field?
fn declares_field(text: &str, owner: &str, field: &str) -> bool {
    let Some(at) = find_words(text, &["struct", owner]) else {
        return false;
    };
    let rest = &text[at..];
    let Some(open) = rest.find('{') else {
        return false;
    };
    let Some(close) = rest[open..].find('}') else {
        return false;
    };
    let body = &rest[open + 1..open + close];
    body.split(',').any(|part| {
        let part = part.trim();
        let part = part.strip_prefix("pub ").unwrap_or(part).trim_start();
        part.strip_prefix(field)
            .is_some_and(|r| r.trim_start().starts_with(':'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frm::{Header, parse_frm, update_frm};
    use crate::import;
    use formust_model::{EventId, WidgetId};

    const LOGIN: &str = "pub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        if ui.button(\"Go\").clicked() { self.go(); }\n    }\n    fn go(&mut self) {}\n}\n";

    /// The copy as Save writes it with `form`.
    fn saved(src: &str, form: &Form) -> String {
        let source = import(src).unwrap();
        update_frm(
            &parse_frm(src).unwrap(),
            &[(
                form.clone(),
                Some(Header::for_source("login.rs", &source.forms[0])),
            )],
        )
    }

    #[test]
    fn the_notes_for_a_form_with_new_state_and_a_new_handler() {
        let mut form = import(LOGIN).unwrap().forms.remove(0).form;
        let edit = form.new_bound_widget(WidgetId::TextEditSingleline, "email");
        form.insert(form.root.id, 0, edit).unwrap();
        let button = form.new_widget(WidgetId::Button);
        let id = button.id;
        form.insert(form.root.id, 1, button).unwrap();
        form.set_event(
            id,
            EventId::Clicked,
            &Form::default_handler("Save", EventId::Clicked),
        )
        .unwrap();
        let copy = saved(LOGIN, &form);
        let u = use_in_code(&form, "login.frm.rs", "login.rs", Some(&copy));
        assert!(
            u.standalone.contains("standalone copy of login.rs")
                && u.standalone.contains("needs no include!"),
            "{}",
            u.standalone
        );
        assert_eq!(
            u.module.as_deref(),
            Some("#[path = \"login.frm.rs\"]\nmod login;")
        );
        assert!(
            u.state
                .as_deref()
                .is_some_and(|s| s.contains("`form: LoginForm` field in struct Login")),
            "{:?}",
            u.state
        );
        assert_eq!(
            u.stubs.as_deref(),
            Some("impl Login {\n    fn on_save_clicked(&mut self) {}\n}")
        );
    }

    #[test]
    fn nothing_is_asked_for_that_the_copy_already_has() {
        let src = "#[derive(Default)]\npub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        if ui.button(\"Go\").clicked() { self.on_go_clicked(); }\n    }\n    fn on_go_clicked(&mut self) {}\n}\n";
        let form = import(src).unwrap().forms.remove(0).form;
        let u = use_in_code(&form, "forms/main.frm.rs", "main.rs", Some(src));
        assert_eq!(u.state, None, "no state");
        assert_eq!(u.stubs, None);
        assert_eq!(u.module, None, "a crate root is no module");
    }

    #[test]
    fn a_unit_owner_cant_take_the_state() {
        let src = "#[derive(Default)]\nstruct App;\nimpl eframe::App for App {\n    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) { ui.label(\"x\"); }\n}\n";
        let mut form = import(src).unwrap().forms.remove(0).form;
        let edit = form.new_bound_widget(WidgetId::Checkbox, "on");
        form.insert(form.root.id, 0, edit).unwrap();
        let copy = saved(src, &form);
        let u = use_in_code(&form, "app.frm.rs", "app.rs", Some(&copy));
        assert!(
            u.state
                .as_deref()
                .is_some_and(|s| s.contains("has no named fields")),
            "{:?}",
            u.state
        );
        assert!(derives_default(src, "App"));
        assert!(!derives_default("struct App;", "App"));
    }

    #[test]
    fn words_are_matched_whole() {
        assert!(defines_fn("fn on_go(&mut self)", "on_go"));
        assert!(!defines_fn("fn on_go_now(&mut self)", "on_go"));
        assert!(!defines_fn("xfn on_go()", "on_go"));
        assert_eq!(
            self_calls(" self.on_a(); self.b(); self.on_c (); self.on_d(1);"),
            ["on_a", "on_c"]
        );
    }
}
