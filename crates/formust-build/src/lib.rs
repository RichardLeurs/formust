//! Formust's build types and cargo driver. The neutral types (`Diagnostic`, `Severity`, `BuildResult`,
//! `Progress`) describe what a build reports. The driver runs `cargo check`, `build` or `run` on a
//! background thread with `--message-format=json` and streams progress, diagnostics and output through a
//! channel. `tools` holds the user's external commands and their macros. No UI dependencies.

pub mod driver;
pub mod message;
pub mod metadata;
pub mod tools;

pub use driver::{
    Action, Build, Event, NOT_FOUND_HINT, Outcome, Request, Selection, Stream, locate_cargo,
    manifest_for, start,
};
pub use message::{CargoDiagnostic, Message, Span, parse_message, severity_for, strip_ansi};
pub use metadata::{Metadata, Package, Target, metadata, parse_metadata};

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Fatal,
    Error,
    Warning,
    Note,
    Hint,
}

/// One compiler message. `line` and `col` are 1-based; a message without a place has an empty `file`
/// and `line` 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug)]
pub struct BuildResult {
    pub success: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub raw_output: String,
}

/// Where a build is, for a progress display.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// What the compiler is doing now, e.g. `Compiling egui v0.37.0` or `Checking formust-build v0.1.0`.
    pub label: String,
    /// 0..1 when the amount of work is known; `None` shows an indeterminate bar.
    pub fraction: Option<f32>,
    /// Units compiled so far in this run (callers remember it to size the next build).
    pub done: usize,
}
