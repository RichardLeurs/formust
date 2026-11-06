//! Export: the designer's forms to their `.frm.rs`, a standalone copy of the `.rs` with the forms in
//! it (`formust_rust::copy`). The first save copies the `.rs`; later saves patch the copy, so what
//! the user wrote in it stays. Save never writes the `.rs`, which never refers to the copy. Before
//! writing, two checks can stop the export and ask the user:
//!
//! - **The source changed:** a form's source body no longer has the token hash the form is based on
//!   (the header's `src-hash` for a form from the `.frm.rs`, the hash at import for one from the
//!   `.rs`). The user keeps the form, re-imports it from the source, or looks at the difference.
//! - **The form file changed:** the `.frm.rs` (its tab, else the disk) is not the text the designer
//!   read. The user overwrites it, reloads it (dropping the designer's edits), or looks at the
//!   difference.
//!
//! A `.frm.rs` that doesn't parse is never overwritten (`formust_rust::write_frm` refuses too).
//!
//! "Save into .rs" is a separate action, never what Save does: [`plan_into_source`] makes the `.rs`
//! with the forms in it (the copy's text without header lines) and refuses or asks as above (unsaved
//! text in the `.rs` tab, a changed form body); the app always asks for a confirmation, which can
//! show the difference, and [`write_into_source`] writes it only while the disk still has the text
//! the plan was made from. The form file is left as it is.

use std::path::{Path, PathBuf};

use formust_model::Form;
use formust_rust::{
    FileKind, FrmFile, Header, Origin, file_kind, form_path_for, parse_frm, relative_path,
    standalone_base, update_frm, write_frm,
};

use super::Loaded;

/// What stops an export until the user decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    /// The source body of form `form` (its `owner::method`) changed since the form was exported or
    /// imported.
    SourceChanged { form: String },
    /// The form file is not what the designer read.
    FormFileChanged { path: PathBuf },
    /// "Save into .rs": the source body of `form` changed since the designer read it. The user
    /// keeps the designer's form (and goes on to the confirmation), re-imports it, or looks first.
    IntoSourceChanged { form: String },
    /// "Save into .rs": the confirmation, with what would be written.
    IntoSource(IntoSource),
}

/// What "Save into .rs" would write: the `.rs` with the designer's forms in it, the same text the
/// `.frm.rs` copy has, without its header lines (`formust_rust::update_source`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntoSource {
    /// The `.rs`.
    pub path: PathBuf,
    /// Its text on disk now; the write refuses when the disk no longer has exactly this.
    pub old: String,
    pub new: String,
    /// The source ids of the forms whose functions are replaced.
    pub forms: Vec<String>,
    /// Forms the `.rs` doesn't have (only in the form file): not written.
    pub skipped: Vec<String>,
    /// The `.rs` on disk is not the text the designer read (the forms' bodies are; that is checked
    /// before): the confirmation says so.
    pub changed_since_read: bool,
    /// The form file beside the `.rs`, if there is one. It is not written or deleted.
    pub companion: Option<PathBuf>,
}

/// What "Save into .rs" can do now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntoSourceOutcome {
    /// Nothing to write; the text says why.
    Nothing(String),
    /// Ask before writing (always: this overwrites the user's source).
    Confirm(IntoSource),
    /// A form's source body changed since it was read: ask first.
    Ask(Prompt),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing edited and nothing from the form file: nothing to write.
    Nothing,
    Written {
        path: PathBuf,
        text: String,
    },
    Ask(Prompt),
    Failed(String),
}

/// Where a load's forms go and where their source is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub frm: PathBuf,
    pub source: Option<PathBuf>,
}

impl Target {
    /// The form file relative to the source's folder (`login.frm.rs`, `forms/login.frm.rs`), for a
    /// `#[path]` that builds the copy in place of the source; its own name when there is no source.
    pub fn frm_from_source(&self) -> String {
        match &self.source {
            Some(source) => relative_path(source.parent().unwrap_or(Path::new("")), &self.frm),
            None => self
                .frm
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        }
    }
}

/// The form file and the source of what `loaded` read. A `.rs` gets its companion form file, or the
/// default one: beside it, or in `forms/` in a Cargo target folder (see `formust_rust::paths`).
pub fn target(loaded: &Loaded, exists: &dyn Fn(&Path) -> bool) -> Result<Target, String> {
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    match file_kind(&loaded.path) {
        FileKind::Form => Ok(Target {
            frm: loaded.path.clone(),
            source: loaded.companion.clone(),
        }),
        FileKind::Source | FileKind::Other => {
            let frm = match &loaded.companion {
                Some(c) if file_kind(c) == FileKind::Form => c.clone(),
                _ => form_path_for(&loaded.path, |p| exists(p))
                    .map(|f| f.path)
                    .ok_or_else(|| {
                        format!(
                            "{} is no .rs file, so it has no form file",
                            name(&loaded.path)
                        )
                    })?,
            };
            Ok(Target {
                frm,
                source: Some(loaded.path.clone()),
            })
        }
    }
}

/// What the user has decided so far for this export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decisions {
    /// Write over a form file that changed since the designer read it.
    pub overwrite_form_file: bool,
}

/// The forms to write: every form that came from the form file, and every form edited since it was
/// read (or last exported). When there is no form file yet and something is to be written, all the
/// forms: the first save copies the whole source, so each form gets its header.
pub fn to_export(loaded: &Loaded) -> Vec<usize> {
    let edited: Vec<usize> = loaded
        .forms
        .iter()
        .enumerate()
        .filter(|(i, f)| {
            f.origin == Some(Origin::Frm)
                || loaded
                    .clean
                    .get(*i)
                    .is_none_or(|(form, header)| form != f.form() || header != &f.header)
        })
        .map(|(i, _)| i)
        .collect();
    if edited.is_empty() || loaded.frm_text.is_some() {
        edited
    } else {
        (0..loaded.forms.len()).collect()
    }
}

/// What the next export builds on: the form file's text as it is now (`current`), else `source`, the
/// text of the `.rs`. An old form file written for `include!` becomes a copy of the source here.
fn base(current: Option<&str>, frm_name: &str, source: Option<&str>) -> Result<FrmFile, String> {
    let existing = match current.map(parse_frm) {
        None => None,
        Some(Ok(file)) => Some(file),
        Some(Err(e)) => {
            return Err(format!(
                "{frm_name} has a syntax error at line {}, column {}: {}. It was not overwritten; fix it in the code editor first.",
                e.line, e.column, e.message
            ));
        }
    };
    standalone_base(existing.as_ref(), source).map_err(|e| {
        format!(
            "the source doesn't parse at line {}, column {}: {}",
            e.line, e.column, e.message
        )
    })
}

/// The forms of `indexes` with their headers, as `update_frm` takes them.
fn forms_of(loaded: &Loaded, indexes: &[usize]) -> Vec<(Form, Option<Header>)> {
    indexes
        .iter()
        .map(|&i| {
            (
                loaded.forms[i].form().clone(),
                loaded.forms[i].header.clone(),
            )
        })
        .collect()
}

/// The form file's text as Save would write it, with the form at `shown` included even when Save
/// would leave it out (an unedited form of the source). `source` is the `.rs` text. Nothing is
/// written: Run Form compiles this text, so the unsaved edits run and the form file on disk stays as
/// it is.
pub fn preview_text(loaded: &Loaded, shown: usize, source: Option<&str>) -> Result<String, String> {
    let name = loaded
        .path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let base = base(loaded.frm_text.as_deref(), &name, source)?;
    let mut indexes = to_export(loaded);
    if !indexes.contains(&shown) {
        indexes.push(shown);
        indexes.sort_unstable();
    }
    Ok(update_frm(&base, &forms_of(loaded, &indexes)))
}

/// Exports `loaded`. `read` gives a file's current text (an open tab's, else the disk's). On
/// success `loaded` is clean, its forms come from the form file, and they carry their new headers.
pub fn export(
    loaded: &mut Loaded,
    read: &dyn Fn(&Path) -> Option<String>,
    exists: &dyn Fn(&Path) -> bool,
    decisions: Decisions,
) -> Outcome {
    let target = match target(loaded, exists) {
        Ok(t) => t,
        Err(e) => return Outcome::Failed(e),
    };
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    let indexes = to_export(loaded);
    if indexes.is_empty() {
        return Outcome::Nothing;
    }
    // The source as it is now, for the headers, the conflict check and the first copy.
    let source_text = target.source.as_deref().and_then(read);
    let source = match target.source.as_deref().zip(source_text.as_deref()) {
        Some((p, text)) => match formust_rust::import(text) {
            Ok(s) => Some(s),
            Err(e) => {
                return Outcome::Failed(format!(
                    "{} doesn't parse at line {}, column {}: {}. Fix it before exporting, so the form file's header can be checked against it.",
                    name(p),
                    e.line,
                    e.column,
                    e.message
                ));
            }
        },
        None => None,
    };
    // The form file as it is now: never overwritten when it doesn't parse or changed under us.
    let current = read(&target.frm);
    if current != loaded.frm_text && !decisions.overwrite_form_file {
        return Outcome::Ask(Prompt::FormFileChanged {
            path: target.frm.clone(),
        });
    }
    let base = match base(
        current.as_deref(),
        &name(&target.frm),
        source_text.as_deref(),
    ) {
        Ok(base) => base,
        Err(e) => return Outcome::Failed(e),
    };
    let header_file = target
        .source
        .as_deref()
        .map(|s| relative_path(target.frm.parent().unwrap_or(Path::new("")), s));
    let mut out: Vec<(Form, Option<Header>)> = Vec::new();
    for &i in &indexes {
        let f = &loaded.forms[i];
        let id = f.source_id();
        let now = source.as_ref().and_then(|s| s.form(&id));
        if let (Some(now), Some(base)) = (now, f.base_hash())
            && now.src_hash != base
        {
            return Outcome::Ask(Prompt::SourceChanged { form: id });
        }
        let header = match (&f.header, now, &header_file) {
            (Some(h), Some(now), _) => Some(Header {
                src_hash: now.src_hash,
                ..h.clone()
            }),
            (Some(h), None, _) => Some(h.clone()),
            (None, Some(now), Some(file)) => Some(Header::for_source(file, now)),
            (None, _, _) => None,
        };
        out.push((f.form().clone(), header));
    }
    let text = update_frm(&base, &out);
    if let Some(dir) = target.frm.parent()
        && !dir.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return Outcome::Failed(format!("can't create {}: {e}", dir.display()));
    }
    if let Err(e) = write_frm(&target.frm, &text) {
        return Outcome::Failed(format!("{}: {e}", name(&target.frm)));
    }
    for (&i, (_, header)) in indexes.iter().zip(out) {
        let f = &mut loaded.forms[i];
        f.header = header;
        f.origin = Some(Origin::Frm);
        f.conflict = Some(formust_rust::Conflict::Unchanged);
    }
    if file_kind(&loaded.path) != FileKind::Form {
        loaded.companion = Some(target.frm.clone());
    }
    loaded.frm_text = Some(text.clone());
    loaded.mark_clean();
    loaded.stale = false;
    Outcome::Written {
        path: target.frm,
        text,
    }
}

/// "Save into .rs", first step: what writing the designer's forms into the `.rs` of `loaded` would
/// do. Nothing is written here. `disk` reads a file from the disk (not from a tab), `tab_unsaved`
/// says the `.rs` has unsaved text in its tab.
///
/// Refused: a load that isn't a `.rs` (a `.frm.rs` tab, a file with an error), unsaved text in the
/// `.rs` tab, a `.rs` that can't be read or doesn't parse, and forms the `.rs` can't take (see
/// `formust_rust::update_source`). A form whose source body changed since the designer read it asks
/// first ([`Prompt::IntoSourceChanged`]). Forms the `.rs` already has as the designer has them are
/// left out, and so are forms only the form file has.
pub fn plan_into_source(
    loaded: &Loaded,
    disk: &dyn Fn(&Path) -> Option<String>,
    tab_unsaved: bool,
) -> IntoSourceOutcome {
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    let rs = name(&loaded.path);
    if let Some(e) = &loaded.error {
        return IntoSourceOutcome::Failed(e.clone());
    }
    if file_kind(&loaded.path) != FileKind::Source {
        return IntoSourceOutcome::Failed(format!(
            "{rs} is not a .rs source; open the .rs in the designer to save its forms into it"
        ));
    }
    if loaded.forms.is_empty() {
        return IntoSourceOutcome::Nothing(format!("{rs} has no forms"));
    }
    if tab_unsaved {
        return IntoSourceOutcome::Failed(format!(
            "{rs} has unsaved changes in the code editor; save or revert them first"
        ));
    }
    let Some(old) = disk(&loaded.path) else {
        return IntoSourceOutcome::Failed(format!("{rs} can't be read"));
    };
    let source = match formust_rust::import(&old) {
        Ok(s) => s,
        Err(e) => {
            return IntoSourceOutcome::Failed(format!(
                "{rs} doesn't parse at line {}, column {}: {}",
                e.line, e.column, e.message
            ));
        }
    };
    let mut forms: Vec<(Form, Option<Header>)> = Vec::new();
    let mut skipped = Vec::new();
    for f in &loaded.forms {
        let id = f.source_id();
        let Some(now) = source.form(&id) else {
            skipped.push(id);
            continue;
        };
        if formust_rust::emit_fn(f.form()) == formust_rust::emit_fn(&now.form) {
            continue;
        }
        if let Some(base) = f.base_hash()
            && base != now.src_hash
        {
            return IntoSourceOutcome::Ask(Prompt::IntoSourceChanged { form: id });
        }
        forms.push((f.form().clone(), f.header.clone()));
    }
    let nothing = || {
        IntoSourceOutcome::Nothing(format!(
            "{rs} already has the designer's forms; nothing to write"
        ))
    };
    if forms.is_empty() {
        return nothing();
    }
    let update = match formust_rust::update_source(&old, &forms) {
        Ok(u) => u,
        Err(e) => return IntoSourceOutcome::Failed(format!("{rs}: {e}")),
    };
    if update.written.is_empty() {
        return nothing();
    }
    let changed_since_read = loaded.stale || loaded.source_seen != Some(super::text_hash(&old));
    let companion = loaded
        .companion
        .clone()
        .filter(|c| file_kind(c) == FileKind::Form)
        .or_else(|| form_path_for(&loaded.path, |p| p.exists()).map(|f| f.path))
        .filter(|p| p.exists());
    IntoSourceOutcome::Confirm(IntoSource {
        path: loaded.path.clone(),
        old,
        new: update.text,
        forms: update.written,
        skipped,
        changed_since_read,
        companion,
    })
}

/// "Save into .rs", after the confirmation: writes `plan.new` over the `.rs`, atomically, only while
/// the disk still has exactly `plan.old` (`formust_rust::write_source`).
pub fn write_into_source(plan: &IntoSource) -> Result<(), String> {
    let name = plan
        .path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    formust_rust::write_source(&plan.path, &plan.old, &plan.new).map_err(|e| match e {
        formust_rust::SourceWriteError::ChangedOnDisk => format!(
            "{name} changed on disk after the confirmation; it was not overwritten. Try again to see the new difference."
        ),
        e => format!("{name}: {e}"),
    })
}

/// The two sides of a prompt: the source's form against the designer's (source changed), or the
/// form file as it is against what the designer would write (form file changed).
pub fn diff_for(
    loaded: &Loaded,
    prompt: &Prompt,
    read: &dyn Fn(&Path) -> Option<String>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<super::Diff> {
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    if let Prompt::IntoSource(plan) = prompt {
        return Some(super::Diff {
            title: format!("{}: on disk and after Save into .rs", name(&plan.path)),
            old_label: format!("{} now", name(&plan.path)),
            old: plan.old.clone(),
            new_label: "what Save into .rs would write".to_string(),
            new: plan.new.clone(),
        });
    }
    let t = target(loaded, exists).ok()?;
    match prompt {
        Prompt::IntoSource(_) => None,
        Prompt::SourceChanged { form } | Prompt::IntoSourceChanged { form } => {
            let source = t.source.as_deref()?;
            let now = formust_rust::import(&read(source)?).ok()?;
            let theirs = formust_rust::emit_form_item(&now.form(form)?.form);
            let ours = formust_rust::emit_form_item(
                loaded.forms.iter().find(|f| &f.source_id() == form)?.form(),
            );
            Some(super::Diff {
                title: format!("{form}: source and form"),
                old_label: name(source),
                old: theirs,
                new_label: "the designer's form".to_string(),
                new: ours,
            })
        }
        Prompt::FormFileChanged { path } => {
            let current = read(path);
            let source = t.source.as_deref().and_then(read);
            let base = base(current.as_deref(), &name(path), source.as_deref()).ok()?;
            let ours = update_frm(&base, &forms_of(loaded, &to_export(loaded)));
            let current = current.unwrap_or_default();
            Some(super::Diff {
                title: format!("{}: on disk and in the designer", name(path)),
                old_label: format!("{} now", name(path)),
                old: current,
                new_label: "what Save would write".to_string(),
                new: ours,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::designer::{Designer, load};
    use formust_model::WidgetId;

    const LOGIN: &str = "pub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Name\");\n        ui.text_edit_singleline(&mut self.name);\n    }\n}\n";

    fn dir(name: &str) -> PathBuf {
        let d = crate::target_tmp().join("designer-export").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn disk(p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }

    fn opened(path: &Path) -> Designer {
        let text = std::fs::read_to_string(path).unwrap();
        let companion = crate::files::companions(path).into_iter().next();
        let ctext = companion.as_deref().and_then(disk);
        let mut d = Designer::default();
        d.store(
            load(path, &text, companion.as_deref().zip(ctext.as_deref())),
            1,
        );
        d
    }

    fn add_separator(d: &mut Designer) {
        d.edit(None, |f| {
            let n = f.new_widget(WidgetId::Separator);
            f.insert(f.root.id, 1, n)
        })
        .unwrap();
    }

    #[test]
    fn an_edited_source_form_is_written_beside_it_with_a_header() {
        let dir = dir("beside");
        let rs = dir.join("login.rs");
        std::fs::write(&rs, LOGIN).unwrap();
        let mut d = opened(&rs);
        let loaded = d.loaded.as_mut().unwrap();
        assert_eq!(
            export(loaded, &disk, &|p| p.exists(), Decisions::default()),
            Outcome::Nothing,
            "nothing edited"
        );
        add_separator(&mut d);
        assert!(d.is_dirty());
        let out = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        );
        let Outcome::Written { path, text } = out else {
            panic!("{out:?}")
        };
        assert_eq!(path, dir.join("login.frm.rs"));
        assert!(
            text.starts_with("// formust 1 | login.rs | Login::ui | src-hash "),
            "{text}"
        );
        assert!(
            text.contains("ui.separator();") && text.contains("ui.label(\"Name\");"),
            "{text}"
        );
        // A standalone copy: the whole source with the edit, under its header, and no include!.
        let (header, rest) = text.split_once('\n').unwrap();
        assert!(header.starts_with("// formust 1 | login.rs"), "{text}");
        assert_eq!(
            rest,
            LOGIN.replace(
                "ui.label(\"Name\");\n",
                "ui.label(\"Name\");\n        ui.separator();\n"
            )
        );
        assert!(!text.contains("include!"), "{text}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(
            std::fs::read_to_string(&rs).unwrap(),
            LOGIN,
            "the .rs is never written"
        );
        assert!(!d.is_dirty());
        // Reopened, the form file wins and has the edit.
        let again = opened(&rs);
        let f = &again.forms()[0];
        assert_eq!(f.origin, Some(Origin::Frm));
        assert_eq!(f.conflict, Some(formust_rust::Conflict::Unchanged));
        assert_eq!(f.form().root.children().len(), 3);
    }

    #[test]
    fn a_source_in_a_cargo_target_folder_exports_into_forms() {
        let dir = dir("target-folder");
        let bin = dir.join("src/bin");
        std::fs::create_dir_all(bin.join("forms")).unwrap();
        std::fs::write(bin.join("forms/main.rs"), "fn main() {}").unwrap();
        let rs = bin.join("login.rs");
        std::fs::write(&rs, LOGIN).unwrap();
        let mut d = opened(&rs);
        let t = target(d.loaded.as_ref().unwrap(), &|p| p.exists()).unwrap();
        assert_eq!(
            t.frm,
            bin.join("forms2/login.frm.rs"),
            "forms/ holds a main.rs, so forms2/"
        );
        assert_eq!(t.frm_from_source(), "forms2/login.frm.rs");
        add_separator(&mut d);
        let out = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        );
        let Outcome::Written { text, .. } = out else {
            panic!("{out:?}")
        };
        assert!(
            text.starts_with("// formust 1 | ../login.rs | Login::ui"),
            "{text}"
        );
    }

    /// The copy is the user's too: what they write in it stays through later saves, and the source
    /// is never touched.
    #[test]
    fn a_later_save_patches_the_copy_and_keeps_what_was_written_in_it() {
        let dir = dir("patch");
        let rs = dir.join("login.rs");
        std::fs::write(&rs, LOGIN).unwrap();
        let mut d = opened(&rs);
        add_separator(&mut d);
        let Outcome::Written { path, text } = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        ) else {
            panic!()
        };
        let by_hand = format!("{text}\nimpl Login {{\n    fn mine(&self) {{}}\n}}\n");
        std::fs::write(&path, &by_hand).unwrap();
        let mut d = opened(&rs);
        assert_eq!(d.forms()[0].origin, Some(Origin::Frm));
        let n = d.forms()[0].form().root.children().len();
        add_separator(&mut d);
        let Outcome::Written { text, .. } = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        ) else {
            panic!()
        };
        assert!(text.contains("fn mine(&self) {}"), "{text}");
        assert_eq!(text.matches("ui.separator();").count(), 2, "{text}");
        assert_eq!(text.matches("// formust 1").count(), 1, "{text}");
        assert_eq!(
            opened(&rs).forms()[0].form().root.children().len(),
            n + 1,
            "one form, read once"
        );
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), LOGIN);
    }

    /// New state goes into the copy: the state struct, the owner's field and its literal.
    #[test]
    fn new_state_is_added_to_the_copy_not_asked_of_the_source() {
        let dir = dir("state");
        let rs = dir.join("login.rs");
        let src = format!(
            "{LOGIN}\nimpl Login {{\n    pub fn new() -> Self {{\n        Self {{ name: String::new() }}\n    }}\n}}\n"
        );
        std::fs::write(&rs, &src).unwrap();
        let mut d = opened(&rs);
        d.edit(None, |f| {
            let n = f.new_bound_widget(WidgetId::Checkbox, "remember");
            f.insert(f.root.id, 0, n)
        })
        .unwrap();
        let Outcome::Written { text, .. } = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        ) else {
            panic!()
        };
        assert!(
            text.contains("pub struct Login { name: String, form: LoginForm }"),
            "{text}"
        );
        assert!(
            text.contains("#[derive(Default)] pub struct LoginForm { pub remember: bool }"),
            "{text}"
        );
        assert!(
            text.contains("Self { name: String::new(), form: Default::default() }"),
            "{text}"
        );
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), src);
        let again = opened(&rs);
        assert_eq!(again.forms()[0].form().state.len(), 1);
        assert_eq!(again.forms()[0].form().state_struct, "LoginForm");
    }

    /// A form file an older Formust wrote for `include!` becomes a standalone copy on the next save.
    #[test]
    fn an_old_included_form_file_becomes_a_copy_on_save() {
        let dir = dir("old");
        let rs = dir.join("login.rs");
        let source = "pub struct Login { name: String }\ninclude!(\"login.frm.rs\");\n";
        std::fs::write(&rs, source).unwrap();
        std::fs::write(
            dir.join("login.frm.rs"),
            "// formust 1 | login.rs | Login::ui | src-hash 00000000\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Name\");\n    }\n}\n",
        )
        .unwrap();
        let mut d = opened(&rs);
        assert_eq!(d.forms().len(), 1);
        add_separator(&mut d);
        let Outcome::Written { text, .. } = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        ) else {
            panic!()
        };
        assert!(!text.contains("include!"), "{text}");
        assert!(
            text.contains("pub struct Login { name: String }") && text.contains("ui.separator();"),
            "{text}"
        );
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), source);
    }

    #[test]
    fn a_changed_source_or_form_file_asks_first() {
        let dir = dir("conflicts");
        let rs = dir.join("login.rs");
        std::fs::write(&rs, LOGIN).unwrap();
        let mut d = opened(&rs);
        add_separator(&mut d);
        // The source changes while the designer has edits.
        std::fs::write(&rs, LOGIN.replace("\"Name\"", "\"Your name\"")).unwrap();
        let out = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        );
        assert_eq!(
            out,
            Outcome::Ask(Prompt::SourceChanged {
                form: "Login::ui".into()
            })
        );
        assert!(!dir.join("login.frm.rs").exists(), "nothing written");
        // Keeping the form takes the new hash.
        d.keep_form("Login::ui", &disk);
        let Outcome::Written { path, .. } = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        ) else {
            panic!()
        };

        // Someone edits the form file; the designer's next export asks.
        add_separator(&mut d);
        let hand = std::fs::read_to_string(&path)
            .unwrap()
            .replace("ui.separator();", "ui.separator(); // by hand");
        std::fs::write(&path, &hand).unwrap();
        let out = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        );
        assert_eq!(
            out,
            Outcome::Ask(Prompt::FormFileChanged { path: path.clone() })
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), hand);
        let out = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions {
                overwrite_form_file: true,
            },
        );
        assert!(matches!(out, Outcome::Written { .. }), "{out:?}");
    }

    fn plan(d: &Designer, tab_unsaved: bool) -> IntoSourceOutcome {
        plan_into_source(d.loaded.as_ref().unwrap(), &disk, tab_unsaved)
    }

    /// Save into .rs: the plan is the `.rs` with the edited function replaced and nothing else; the
    /// write replaces the file, which reads back to the same form; a second plan has nothing to do.
    #[test]
    fn save_into_source_writes_the_edited_function_into_the_rs() {
        let dir = dir("into-rs");
        let rs = dir.join("login.rs");
        let src = format!("// Top comment.\n{LOGIN}\nfn helper() {{  }}\n");
        std::fs::write(&rs, &src).unwrap();
        let mut d = opened(&rs);
        assert!(matches!(plan(&d, false), IntoSourceOutcome::Nothing(_)));
        add_separator(&mut d);
        let IntoSourceOutcome::Confirm(p) = plan(&d, false) else {
            panic!("{:?}", plan(&d, false))
        };
        assert_eq!(p.path, rs);
        assert_eq!(p.old, src);
        assert_eq!(p.forms, ["Login::ui"]);
        assert!(!p.changed_since_read && p.companion.is_none() && p.skipped.is_empty());
        assert_eq!(
            p.new,
            src.replace(
                "ui.label(\"Name\");\n",
                "ui.label(\"Name\");\n        ui.separator();\n"
            )
        );
        // Nothing was written by planning.
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), src);
        write_into_source(&p).unwrap();
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), p.new);
        assert!(!dir.join("login.frm.rs").exists(), "no form file is made");
        // Read again from the new .rs: the same form, and nothing left to write.
        let again = opened(&rs);
        assert_eq!(
            formust_rust::emit_fn(again.forms()[0].form()),
            formust_rust::emit_fn(d.forms()[0].form())
        );
        assert!(matches!(plan(&again, false), IntoSourceOutcome::Nothing(_)));
    }

    #[test]
    fn save_into_source_refuses_or_asks_when_the_rs_is_not_as_read() {
        let dir = dir("into-rs-conflicts");
        let rs = dir.join("login.rs");
        std::fs::write(&rs, LOGIN).unwrap();
        let mut d = opened(&rs);
        add_separator(&mut d);
        // Unsaved text in the .rs tab.
        assert!(
            matches!(plan(&d, true), IntoSourceOutcome::Failed(m) if m.contains("unsaved changes")),
        );
        // Changed outside the form: confirmed with a note.
        let outside = format!("{LOGIN}\nfn added_on_disk() {{}}\n");
        std::fs::write(&rs, &outside).unwrap();
        let IntoSourceOutcome::Confirm(p) = plan(&d, false) else {
            panic!()
        };
        assert!(p.changed_since_read);
        assert!(p.new.contains("fn added_on_disk() {}") && p.new.contains("ui.separator();"));
        // The disk changes again before the answer: the write refuses and the file stays.
        std::fs::write(&rs, LOGIN).unwrap();
        assert!(
            write_into_source(&p)
                .unwrap_err()
                .contains("changed on disk")
        );
        assert_eq!(std::fs::read_to_string(&rs).unwrap(), LOGIN);
        // The form's own body changed: asked first.
        std::fs::write(&rs, LOGIN.replace("\"Name\"", "\"Your name\"")).unwrap();
        assert_eq!(
            plan(&d, false),
            IntoSourceOutcome::Ask(Prompt::IntoSourceChanged {
                form: "Login::ui".into()
            })
        );
        let Some(diff) = diff_for(
            d.loaded.as_ref().unwrap(),
            &Prompt::IntoSourceChanged {
                form: "Login::ui".into(),
            },
            &disk,
            &|p| p.exists(),
        ) else {
            panic!()
        };
        assert!(diff.old.contains("Your name") && diff.new.contains("ui.separator();"));
        // Keeping the designer's form takes the new hash; then the plan goes on.
        d.keep_form("Login::ui", &disk);
        assert!(matches!(plan(&d, false), IntoSourceOutcome::Confirm(_)));
    }

    /// A `.frm.rs` tab is not a source; a companion form file is named in the plan and left alone.
    #[test]
    fn save_into_source_names_the_form_file_and_never_writes_it() {
        let dir = dir("into-rs-companion");
        let rs = dir.join("login.rs");
        std::fs::write(&rs, LOGIN).unwrap();
        let mut d = opened(&rs);
        add_separator(&mut d);
        let Outcome::Written { path: frm, text } = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions::default(),
        ) else {
            panic!()
        };
        // From the .frm.rs tab: refused.
        let from_frm = opened(&frm);
        assert!(
            matches!(plan(&from_frm, false), IntoSourceOutcome::Failed(m) if m.contains("not a .rs source"))
        );
        // From the .rs: the copy's form (from the form file) goes into the .rs.
        let d = opened(&rs);
        assert_eq!(d.forms()[0].origin, Some(Origin::Frm));
        let IntoSourceOutcome::Confirm(p) = plan(&d, false) else {
            panic!()
        };
        assert_eq!(p.companion.as_deref(), Some(frm.as_path()));
        let Some(shown) = diff_for(
            d.loaded.as_ref().unwrap(),
            &Prompt::IntoSource(p.clone()),
            &disk,
            &|p| p.exists(),
        ) else {
            panic!()
        };
        assert_eq!(
            (shown.old.as_str(), shown.new.as_str()),
            (LOGIN, p.new.as_str())
        );
        write_into_source(&p).unwrap();
        assert_eq!(
            std::fs::read_to_string(&frm).unwrap(),
            text,
            "the form file is left as it is"
        );
        let (_, rest) = text.split_once('\n').unwrap();
        assert_eq!(p.new, rest, "the .rs is the copy without its header line");
    }

    #[test]
    fn a_form_file_that_does_not_parse_is_never_overwritten() {
        let dir = dir("broken");
        let rs = dir.join("login.rs");
        std::fs::write(&rs, LOGIN).unwrap();
        let mut d = opened(&rs);
        add_separator(&mut d);
        let frm = dir.join("login.frm.rs");
        std::fs::write(&frm, "impl Login { fn ui(").unwrap();
        // Read as it is now, the broken file is what the designer would overwrite.
        d.loaded.as_mut().unwrap().frm_text = Some("impl Login { fn ui(".into());
        let out = export(
            d.loaded.as_mut().unwrap(),
            &disk,
            &|p| p.exists(),
            Decisions {
                overwrite_form_file: true,
            },
        );
        assert!(
            matches!(&out, Outcome::Failed(m) if m.contains("syntax error") && m.contains("not overwritten")),
            "{out:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&frm).unwrap(),
            "impl Login { fn ui("
        );
    }
}
