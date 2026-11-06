//! Formust's Rust importer and emitter. `.rs` and `.frm.rs` text go through one `syn` importer, and
//! the `.frm.rs` written back is a standalone copy of the `.rs` with the designer's forms in it. On
//! request ("Save into .rs") the same forms go into the `.rs` itself ([`update_source`]).
//! Unrecognised code stays as a locked custom block, a byte slice of the source; an unedited node
//! is emitted verbatim. No UI dependencies.

pub mod clip;
pub mod copy;
pub mod emit;
pub mod forms;
pub mod frm;
pub mod import;
pub mod paths;
pub mod run;
pub mod stats;
pub mod tokens;
pub mod usage;

pub use copy::{SourceError, SourceUpdate, SourceWriteError, update_source, write_source};
pub use emit::{emit_body, emit_fn, emit_form_item, emit_node};
pub use frm::{
    Conflict, FrmFile, FrmForm, Header, MergedForm, Origin, WriteError, check_conflict, merge,
    parse_frm, parse_frm_with, standalone_base, update_frm, write_frm,
};
pub use import::{
    CustomInfo, Diagnostic, ImportError, ImportOptions, ImportedForm, Severity, SourceFile, import,
    import_with,
};
pub use paths::{
    FORM_SUFFIX, FileKind, FormPath, ReadFile, file_kind, form_path_for, read_file, relative_path,
    source_path_for,
};
pub use tokens::{token_hash, tokens_equal};
pub use usage::{UseInCode, use_in_code};
