//! Form files on disk. `login.rs` pairs with `login.frm.rs`, matched on the whole `.frm.rs` suffix:
//! `Path::extension` only sees `rs`, so it can't tell the two apart.
//!
//! The form file is a standalone copy of its source, so it can sit beside it: nothing includes it and
//! nothing compiles it unless the user makes it a module or a target. The exception: Cargo turns
//! every `.rs` file directly in `src/bin`, `examples`, `tests` and `benches` into a target of its
//! own, and `login.frm` is no valid crate name, so the build breaks (and a valid name would build the
//! copy as a second program). Forms of sources there go one folder down, into `forms/`, which Cargo
//! leaves alone while it has no `main.rs`.
//!
//! Pairing is the default, not a rule: a form may be saved under any name, and its header names
//! its source relative to itself.

use std::path::{Component, Path, PathBuf};

use crate::frm::{FrmFile, Header, parse_frm};
use crate::import::{Diagnostic, ImportError, Severity, SourceFile, import};

pub const FORM_SUFFIX: &str = ".frm.rs";

/// The folders whose direct `.rs` files Cargo builds as targets, relative to the package.
pub const TARGET_FOLDERS: &[&str] = &["src/bin", "examples", "tests", "benches"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// A Rust source: `login.rs`.
    Source,
    /// A Formust form file: `login.frm.rs`.
    Form,
    Other,
}

/// What a path is, by its name alone.
pub fn file_kind(path: &Path) -> FileKind {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return FileKind::Other;
    };
    if name.len() > FORM_SUFFIX.len() && name.ends_with(FORM_SUFFIX) {
        FileKind::Form
    } else if name.len() > 3 && name.ends_with(".rs") {
        FileKind::Source
    } else {
        FileKind::Other
    }
}

/// Is this folder one whose direct `.rs` files Cargo builds as targets? Judged by its name, since
/// that is what Cargo does by default; a folder of that name elsewhere just gets a subfolder too.
pub fn is_target_folder(dir: &Path) -> bool {
    let names: Vec<&str> = dir
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    match names.as_slice() {
        [.., "src", "bin"] => true,
        [.., last] => matches!(*last, "examples" | "tests" | "benches"),
        [] => false,
    }
}

/// Where a source's form goes by default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormPath {
    pub path: PathBuf,
    /// For the header: the source relative to the form's folder.
    pub header_file: String,
    /// Set when the form can't sit next to its source, saying why.
    pub diagnostic: Option<Diagnostic>,
}

/// The default form file of a source: `x.rs` to `x.frm.rs` beside it, or to `forms/x.frm.rs` in a
/// Cargo target folder. `forms` holding a `main.rs` would be a target itself, so then `forms2`,
/// `forms3` and so on. `exists` is asked about those `main.rs` files only. `None` if `source` isn't
/// a plain `.rs` path.
pub fn form_path_for(source: &Path, exists: impl Fn(&Path) -> bool) -> Option<FormPath> {
    if file_kind(source) != FileKind::Source {
        return None;
    }
    let name = source.file_name()?.to_str()?;
    let stem = &name[..name.len() - ".rs".len()];
    let dir = source.parent().unwrap_or(Path::new(""));
    let form_name = format!("{stem}{FORM_SUFFIX}");
    if !is_target_folder(dir) {
        return Some(FormPath {
            path: dir.join(&form_name),
            header_file: name.to_string(),
            diagnostic: None,
        });
    }
    let sub = (1..)
        .map(|n| {
            if n == 1 {
                "forms".to_string()
            } else {
                format!("forms{n}")
            }
        })
        .find(|sub| !exists(&dir.join(sub).join("main.rs")))
        .expect("an unbounded range");
    let message = format!(
        "{} is a Cargo target folder, where every .rs file becomes a target and {form_name} would break the build; the form goes in {sub}/{form_name}",
        dir.display()
    );
    Some(FormPath {
        path: dir.join(&sub).join(&form_name),
        header_file: format!("../{name}"),
        diagnostic: Some(Diagnostic {
            severity: Severity::Info,
            message,
            range: None,
        }),
    })
}

/// The source of a form file: the one its header names, else by name: `x.rs` beside it, or one
/// folder up when it sits in a `forms` folder of a Cargo target folder and only that one exists.
/// `None` if `form` isn't a `.frm.rs` path and has no header.
pub fn source_path_for(
    form: &Path,
    header: Option<&Header>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let dir = form.parent().unwrap_or(Path::new(""));
    if let Some(header) = header {
        return Some(normalize(&dir.join(&header.file)));
    }
    if file_kind(form) != FileKind::Form {
        return None;
    }
    let name = form.file_name()?.to_str()?;
    let source_name = format!("{}.rs", &name[..name.len() - FORM_SUFFIX.len()]);
    let beside = dir.join(&source_name);
    if !exists(&beside)
        && let Some(parent) = dir.parent()
        && dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("forms"))
        && is_target_folder(parent)
        && exists(&parent.join(&source_name))
    {
        return Some(parent.join(source_name));
    }
    Some(beside)
}

/// `to` relative to the folder `from_dir`, with `/`, for a header. Both must be
/// relative to the same place, or both absolute.
pub fn relative_path(from_dir: &Path, to: &Path) -> String {
    let base = normalize(from_dir);
    let from: Vec<Component<'_>> = base
        .components()
        .filter(|c| *c != Component::CurDir)
        .collect();
    let target = normalize(to);
    let to: Vec<Component<'_>> = target
        .components()
        .filter(|c| *c != Component::CurDir)
        .collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().to_string()),
    );
    parts.join("/")
}

/// `a/b/../c` to `a/c`, without asking the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// A file read for what it is, so a folder scan imports each form once.
#[derive(Debug)]
pub enum ReadFile {
    Source(SourceFile),
    Form(FrmFile),
}

/// Reads a file as a source or as a form file. A form file is one named `.frm.rs`, or any file with a
/// Formust header line (one saved under another name). A `.frm.rs` with no header is still a form
/// file: its forms are matched to the source by `Owner::method`, with a warning that their
/// conflict check can't run. It is never imported as a source.
pub fn read_file(path: &Path, text: &str) -> Result<ReadFile, ImportError> {
    let named_form = file_kind(path) == FileKind::Form;
    let has_header = text.lines().any(|line| Header::parse(line).is_some());
    if !named_form && !has_header {
        return import(text).map(ReadFile::Source);
    }
    let mut file = parse_frm(text)?;
    if file.forms.iter().all(|f| f.header.is_none()) {
        file.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            message: format!(
                "{} has no Formust header: its forms are matched to the source by name, and changes to the source can't be detected",
                path.display()
            ),
            range: None,
        });
    }
    Ok(ReadFile::Form(file))
}
