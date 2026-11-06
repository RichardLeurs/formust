//! The `.frm.rs` file: a standalone copy of its `.rs` with the designer's forms in it (written by
//! [`crate::copy`]). Here: its header lines, parsing through the shared importer, the conflict check
//! against the `.rs`, and the refusal to overwrite a file that doesn't parse.
//!
//! The header records the source's hash only to notice that the source changed. Nothing couples the
//! two files: the copy compiles on its own, and the source needs no line for it.

use std::io::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};

use formust_model::Form;

pub use crate::copy::{standalone_base, update_frm};
use crate::import::{
    Diagnostic, ImportError, ImportOptions, ImportedForm, Severity, SourceFile, free_state_struct,
    import_with,
};

const VERSION: u32 = 1;

/// `// formust 1 | login.rs | Login::ui | src-hash 8f3a21c0`. `file` is the source relative to the
/// form file's folder (`../login.rs` for a form in a `forms` subfolder; see [`crate::paths`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub file: String,
    /// The form's identity in the source. A legacy `App::update` keeps it after becoming `App::ui`.
    pub id: String,
    pub src_hash: u32,
    /// `(struct, field)` of the form's state, only when they aren't `<Owner>Form` and `form`, which
    /// a default header leaves out. Written ` | state LoginForm2.form2`.
    pub state: Option<(String, String)>,
}

impl Header {
    pub fn to_line(&self) -> String {
        let line = format!(
            "// formust {VERSION} | {} | {} | src-hash {:08x}",
            self.file, self.id, self.src_hash
        );
        match &self.state {
            Some((state_struct, field)) => format!("{line} | state {state_struct}.{field}"),
            None => line,
        }
    }

    /// Reads a header line; `None` if it isn't one. A newer version isn't read either.
    pub fn parse(line: &str) -> Option<Header> {
        let rest = line.trim().strip_prefix("// formust ")?;
        let mut parts = rest.split(" | ");
        let version: u32 = parts.next()?.trim().parse().ok()?;
        if version != VERSION {
            return None;
        }
        let mut parts: Vec<&str> = parts.collect();
        let mut state = None;
        if let Some(text) = parts
            .last()
            .and_then(|last| last.trim().strip_prefix("state "))
        {
            let (state_struct, field) = text.split_once('.')?;
            let is_name =
                |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_');
            if !is_name(state_struct) || !is_name(field) {
                return None;
            }
            state = Some((state_struct.to_string(), field.to_string()));
            parts.pop();
        }
        // A file name may hold " | ", so the fields are counted from the end.
        let [file @ .., id, hash] = parts.as_slice() else {
            return None;
        };
        if file.is_empty() {
            return None;
        }
        let src_hash = u32::from_str_radix(hash.trim().strip_prefix("src-hash ")?, 16).ok()?;
        Some(Header {
            file: file.join(" | "),
            id: (*id).to_string(),
            src_hash,
            state,
        })
    }

    /// The header of a form freshly imported from `file`.
    pub fn for_source(file: &str, imported: &ImportedForm) -> Header {
        Header {
            file: file.to_string(),
            id: imported.form.id(),
            src_hash: imported.src_hash,
            state: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FrmForm {
    pub imported: ImportedForm,
    pub header: Option<Header>,
}

impl FrmForm {
    /// The identity to match against the `.rs`: the header's, else the form's own.
    pub fn source_id(&self) -> String {
        self.header
            .as_ref()
            .map_or_else(|| self.imported.form.id(), |h| h.id.clone())
    }
}

/// A `.frm.rs` as read: a standalone copy of its source, the designer's forms in it, and a header
/// line per form at the top.
#[derive(Clone, Debug, Default)]
pub struct FrmFile {
    pub forms: Vec<FrmForm>,
    pub diagnostics: Vec<Diagnostic>,
    /// The whole file. Everything that isn't a form or a header line is the user's and is kept.
    pub text: String,
    /// The byte ranges of the header lines read, each with its line end.
    pub(crate) header_lines: Vec<Range<usize>>,
    /// Written by an older Formust for `include!`: it holds forms of types it doesn't declare, so it
    /// is no copy of its source. The next export turns it into one (see [`crate::copy`]).
    pub included: bool,
}

impl FrmFile {
    /// The file's text written again from its forms. Stable: a parsed file gives its own text back.
    pub fn emit(&self) -> String {
        let forms: Vec<(Form, Option<Header>)> = self
            .forms
            .iter()
            .map(|f| (f.imported.form.clone(), f.header.clone()))
            .collect();
        update_frm(self, &forms)
    }
}

/// Reads a `.frm.rs` through the shared importer, as any `.rs`. A syntax error is an `Err`: the
/// caller must open the code editor, block the designer and not overwrite the file.
pub fn parse_frm(src: &str) -> Result<FrmFile, ImportError> {
    let file = import_with(
        src,
        ImportOptions {
            frm: true,
            ..ImportOptions::default()
        },
    )?;
    Ok(assemble(src, file))
}

fn assemble(src: &str, file: SourceFile) -> FrmFile {
    let SourceFile {
        mut forms,
        mut diagnostics,
        top,
        states,
        declared,
    } = file;
    // The header lines: line comments between the top-level items.
    let mut found: Vec<(Header, Range<usize>)> = Vec::new();
    let mut gaps: Vec<Range<usize>> = Vec::new();
    let mut cursor = 0;
    for item in &top {
        gaps.push(cursor..item.start.max(cursor));
        cursor = cursor.max(item.end);
    }
    gaps.push(cursor..src.len());
    for gap in gaps {
        let mut at = gap.start;
        for line in src.get(gap).unwrap_or_default().split_inclusive('\n') {
            // A byte order mark stays where it is.
            let skip = if at == 0 && line.starts_with('\u{feff}') {
                '\u{feff}'.len_utf8()
            } else {
                0
            };
            let range = at + skip..at + line.len();
            if let Some(header) = Header::parse(&line[skip..]) {
                found.push((header, range));
            } else if line[skip..].trim_start().starts_with("// formust ") {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "this header can't be read and stays as a comment: {}",
                        line.trim()
                    ),
                    range: Some(range),
                });
            }
            at += line.len();
        }
    }
    // Each header goes to the form of its id, else to an unclaimed form of its owner that no other
    // header names (a legacy `X::update` is `X::ui` in the copy).
    let ids: Vec<String> = found.iter().map(|(h, _)| h.id.clone()).collect();
    let mut headers: Vec<Option<Header>> = vec![None; forms.len()];
    let mut header_lines = Vec::new();
    for (header, range) in found {
        let free = |i: usize, headers: &[Option<Header>]| headers[i].is_none();
        let exact =
            (0..forms.len()).find(|&i| free(i, &headers) && forms[i].form.id() == header.id);
        let owner = header.id.split("::").next().unwrap_or_default();
        let target = exact.or_else(|| {
            (0..forms.len()).find(|&i| {
                free(i, &headers)
                    && forms[i].form.owner == owner
                    && !ids.contains(&forms[i].form.id())
            })
        });
        match target {
            Some(i) => {
                headers[i] = Some(header);
                header_lines.push(range);
            }
            None => diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                message: format!(
                    "the header for {} has no form in this file; it stays as a comment",
                    header.id
                ),
                range: Some(range),
            }),
        }
    }
    // Each form's state: the header's names or the defaults, if a struct of that name is here.
    for (imported, header) in forms.iter_mut().zip(&headers) {
        let form = &mut imported.form;
        let explicit = header.as_ref().and_then(|h| h.state.clone());
        let (state_struct, field) = explicit.clone().unwrap_or_else(|| {
            (
                Form::default_state_struct(&form.owner),
                Form::DEFAULT_STATE_FIELD.to_string(),
            )
        });
        if let Some((text, fields)) = states.get(&state_struct) {
            form.state_src = Some(text.clone());
            form.state = fields.clone();
        } else if explicit.is_none() {
            // Nothing claimed, so new state needs a name no item here has.
            form.state_struct = free_state_struct(&form.owner, &declared);
            form.state_field = field;
            continue;
        }
        form.state_struct = state_struct;
        form.state_field = field;
    }
    let included = forms
        .iter()
        .any(|f| !f.form.owner.is_empty() && !declared.contains(&f.form.owner));
    let forms = forms
        .into_iter()
        .zip(headers)
        .map(|(imported, header)| FrmForm { imported, header })
        .collect();
    FrmFile {
        forms,
        diagnostics,
        text: src.to_string(),
        header_lines,
        included,
    }
}

/// The result of comparing a `.frm.rs` form with the `.rs` it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// The `.rs` body is as it was at export.
    Unchanged,
    /// The `.rs` body has changed: offer keep `.frm.rs`, re-import, or diff.
    Changed { stored: u32, current: u32 },
    /// The `.rs` has no such form: it was removed there, or added to the copy only.
    MissingInSource,
    /// The `.frm.rs` form has no header to compare with.
    NoHeader,
}

pub fn check_conflict(form: &FrmForm, source: &SourceFile) -> Conflict {
    let Some(header) = &form.header else {
        return Conflict::NoHeader;
    };
    match source.form(&header.id) {
        None => Conflict::MissingInSource,
        Some(current) if current.src_hash == header.src_hash => Conflict::Unchanged,
        Some(current) => Conflict::Changed {
            stored: header.src_hash,
            current: current.src_hash,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Frm,
    Rs,
}

/// One form after both files are read.
#[derive(Clone, Debug)]
pub struct MergedForm {
    pub imported: ImportedForm,
    pub origin: Origin,
    pub header: Header,
    /// Only for forms from the `.frm.rs`.
    pub conflict: Option<Conflict>,
}

/// Both files: the `.frm.rs` wins for each form it defines; forms found only in the `.rs` come from
/// there. `.rs` forms come first in `.rs` order, then forms only the `.frm.rs` has.
pub fn merge(file_name: &str, source: &SourceFile, frm: &FrmFile) -> Vec<MergedForm> {
    let mut merged = Vec::new();
    for imported in &source.forms {
        let id = imported.form.id();
        match frm.forms.iter().find(|f| f.source_id() == id) {
            Some(form) => {
                let mut form_here = from_frm(file_name, form, source);
                // With no state yet, names must be free in the `.rs`, which this `.frm.rs` can't see.
                let target = &mut form_here.imported.form;
                if target.state.is_empty()
                    && target.state_src.is_none()
                    && form.header.as_ref().is_none_or(|h| h.state.is_none())
                {
                    target.state_struct.clone_from(&imported.form.state_struct);
                    target.state_field.clone_from(&imported.form.state_field);
                }
                merged.push(form_here);
            }
            None => merged.push(MergedForm {
                imported: imported.clone(),
                origin: Origin::Rs,
                header: Header::for_source(file_name, imported),
                conflict: None,
            }),
        }
    }
    for form in &frm.forms {
        if source.form(&form.source_id()).is_none() {
            merged.push(from_frm(file_name, form, source));
        }
    }
    merged
}

fn from_frm(file_name: &str, form: &FrmForm, source: &SourceFile) -> MergedForm {
    MergedForm {
        imported: form.imported.clone(),
        origin: Origin::Frm,
        header: form
            .header
            .clone()
            .unwrap_or_else(|| Header::for_source(file_name, &form.imported)),
        conflict: Some(check_conflict(form, source)),
    }
}

#[derive(Debug)]
pub enum WriteError {
    /// The file on disk doesn't parse, so it may hold hand edits this run can't see. Never overwritten.
    ExistingDoesNotParse(ImportError),
    /// It isn't text.
    ExistingNotText,
    /// What we were about to write doesn't parse. That is a bug here; the file on disk stays.
    NewDoesNotParse(ImportError),
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::ExistingDoesNotParse(e) => write!(
                f,
                "the existing .frm.rs has a syntax error ({e}); it was not overwritten"
            ),
            WriteError::ExistingNotText => write!(
                f,
                "the existing .frm.rs is not text; it was not overwritten"
            ),
            WriteError::NewDoesNotParse(e) => write!(
                f,
                "the generated .frm.rs has a syntax error ({e}); nothing was written"
            ),
            WriteError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for WriteError {}

/// Writes a `.frm.rs`, refusing when the file on disk or the new text doesn't parse. The write goes to
/// a temporary file first and is renamed over the old one, so a failure leaves the old one whole.
pub fn write_frm(path: &Path, text: &str) -> Result<(), WriteError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let existing = String::from_utf8(bytes).map_err(|_| WriteError::ExistingNotText)?;
            parse_frm(&existing).map_err(WriteError::ExistingDoesNotParse)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(WriteError::Io(e)),
    }
    parse_frm(text).map_err(WriteError::NewDoesNotParse)?;
    let mut temp = PathBuf::from(path);
    let name = path
        .file_name()
        .map_or_else(|| "form.frm.rs".into(), |n| n.to_os_string());
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(".tmp");
    temp.set_file_name(temp_name);
    let result = std::fs::File::create(&temp)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(WriteError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::import;
    use formust_model::{PropValue, WidgetId};

    const SRC: &str = "pub struct A;\nimpl A {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"a\");\n    }\n}\npub struct B;\nimpl B {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"b\");\n    }\n}\nfn helper() {}\n";

    /// Exporting one form into a copy replaces its function there and keeps everything else.
    #[test]
    fn an_export_replaces_its_own_form_and_keeps_the_rest() {
        let source = import(SRC).unwrap();
        let header = |i: usize| Some(Header::for_source("ab.rs", &source.forms[i]));
        let all = |forms: &[ImportedForm]| -> Vec<(Form, Option<Header>)> {
            forms
                .iter()
                .enumerate()
                .map(|(i, f)| (f.form.clone(), header(i)))
                .collect()
        };
        let first = update_frm(&parse_frm(SRC).unwrap(), &all(&source.forms));
        assert!(
            first.starts_with("// formust 1 | ab.rs | A::ui | src-hash "),
            "{first}"
        );
        assert!(first.ends_with(SRC), "the copy is the source: {first}");
        let file = parse_frm(&first).unwrap();
        assert!(!file.included);

        let mut b = file.forms[1].imported.form.clone();
        let node = b.new_widget(WidgetId::Separator);
        b.insert(b.root.id, 1, node).unwrap();
        let second = update_frm(&file, &[(b, file.forms[1].header.clone())]);
        let reread = parse_frm(&second).unwrap();
        assert_eq!(
            reread
                .forms
                .iter()
                .map(|f| f.source_id())
                .collect::<Vec<_>>(),
            ["A::ui", "B::ui"]
        );
        assert!(second.contains("ui.separator();"), "{second}");
        assert!(second.contains("fn helper() {}"), "{second}");
        assert_eq!(second.matches("// formust 1").count(), 2, "{second}");

        // Exporting B again replaces it in place; A stays verbatim.
        let mut b2 = reread.forms[1].imported.form.clone();
        let first_child = b2.root.children()[0].id;
        b2.set_prop(first_child, "text", PropValue::Text("changed".into()))
            .unwrap();
        let third = update_frm(&reread, &[(b2, reread.forms[1].header.clone())]);
        let again = parse_frm(&third).unwrap();
        assert_eq!(again.forms.len(), 2);
        assert!(
            third.contains("ui.label(\"changed\");") && third.contains("ui.label(\"a\");"),
            "{third}"
        );
        // Idempotent: the same export twice is the same text.
        assert_eq!(again.emit(), third);
        assert_eq!(update_frm(&again, &[]), third);
    }
}
