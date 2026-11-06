//! Parses the JSON lines of `cargo ... --message-format=json` into [`Message`]s.
//!
//! rustc names a file relative to the workspace root for workspace members and absolutely for
//! everything else (path dependencies outside the workspace, the registry, the standard library).
//! Every path here is made absolute against the workspace root. A line that is not a cargo message
//! (program output under `cargo run`, a truncated line, anything else) gives `None`; nothing here panics.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::{Diagnostic, Severity};

/// One line of cargo's JSON output, the kinds the driver uses.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Diagnostic(Box<CargoDiagnostic>),
    /// A unit is done (`fresh` when it was up to date). `executable` is set for binaries, tests and
    /// examples that were linked.
    Artifact {
        target: String,
        fresh: bool,
        executable: Option<PathBuf>,
    },
    /// `build-finished`: the compiling is over (under `cargo run` the program starts after it).
    BuildFinished {
        success: bool,
    },
}

/// A rustc message with everything cargo's JSON gives that a [`Diagnostic`] has no room for.
#[derive(Debug, Clone, PartialEq)]
pub struct CargoDiagnostic {
    /// The place to show: see [`parse_message`] for how a macro expansion is followed.
    pub diagnostic: Diagnostic,
    /// rustc's level as written: `error`, `warning`, `note`, `help`, `failure-note`,
    /// `error: internal compiler error`.
    pub level: String,
    /// The error or lint code, like `E0308` or `unused_variables`.
    pub code: Option<String>,
    /// rustc's own text of the message with its children (notes, help), for display. No ANSI codes.
    pub rendered: String,
    /// Every top-level span as written by rustc (not followed through macros), made absolute.
    pub spans: Vec<Span>,
    /// The cargo target the message is for, like `lib`, `bin`, `test` or `example`.
    pub target_kind: String,
    pub target_name: String,
    /// The `Cargo.toml` of the package the message is for.
    pub manifest_path: PathBuf,
}

/// A span of source. Lines and columns are 1-based; rustc counts columns in characters; the end is
/// exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub primary: bool,
    pub label: Option<String>,
}

/// The severity of a rustc level. `help` is a hint, `failure-note` (the `rustc --explain` line) a
/// note, and an internal compiler error fatal. An unknown level is a note, so nothing is lost.
pub fn severity_for(level: &str) -> Severity {
    match level {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        "help" => Severity::Hint,
        "note" | "failure-note" => Severity::Note,
        l if l.starts_with("error: internal compiler error") => Severity::Fatal,
        _ => Severity::Note,
    }
}

/// Parses one line of cargo's JSON output. Relative file names are taken from `workspace_root`.
///
/// The diagnostic's place is the first primary span. When that span is in a file outside the
/// message's package (a macro from a dependency or the standard library), the place is the
/// outermost macro call site in the package, if the expansion has one. A message without spans has
/// an empty file and line 0.
pub fn parse_message(line: &str, workspace_root: &Path) -> Option<Message> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    match value.get("reason")?.as_str()? {
        "compiler-message" => {
            compiler_message(&value, workspace_root).map(|d| Message::Diagnostic(Box::new(d)))
        }
        "compiler-artifact" => Some(Message::Artifact {
            target: text(value.get("target").and_then(|t| t.get("name"))),
            fresh: value.get("fresh").and_then(Value::as_bool).unwrap_or(false),
            executable: value
                .get("executable")
                .and_then(Value::as_str)
                .map(PathBuf::from),
        }),
        "build-finished" => Some(Message::BuildFinished {
            success: value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        _ => None,
    }
}

fn compiler_message(value: &Value, root: &Path) -> Option<CargoDiagnostic> {
    let message = value.get("message")?;
    let level = message.get("level")?.as_str()?.to_string();
    let manifest_path = value
        .get("manifest_path")
        .and_then(Value::as_str)
        .map(|p| resolve(root, p))
        .unwrap_or_default();
    let package_dir = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());
    let raw_spans: Vec<&Value> = message
        .get("spans")
        .and_then(Value::as_array)
        .map(|s| s.iter().collect())
        .unwrap_or_default();
    let place = raw_spans
        .iter()
        .find(|s| s.get("is_primary").and_then(Value::as_bool) == Some(true))
        .and_then(|s| span(located(s, root, &package_dir), root));
    let (file, line, col) = place.map(|s| (s.file, s.line, s.col)).unwrap_or_default();
    let target = value.get("target");
    Some(CargoDiagnostic {
        diagnostic: Diagnostic {
            file,
            line,
            col,
            severity: severity_for(&level),
            message: text(message.get("message")),
        },
        level,
        code: message
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string),
        rendered: strip_ansi(&text(message.get("rendered"))),
        spans: raw_spans.iter().filter_map(|s| span(s, root)).collect(),
        target_kind: text(target.and_then(|t| t.get("kind")).and_then(|k| k.get(0))),
        target_name: text(target.and_then(|t| t.get("name"))),
        manifest_path,
    })
}

/// The span to show for `span`: itself when it is in the package, else the outermost call site in
/// the package along its macro expansions, else itself.
fn located<'a>(span: &'a Value, root: &Path, package_dir: &Path) -> &'a Value {
    if in_package(span, root, package_dir) {
        return span;
    }
    let mut best = span;
    let mut current = span;
    // Bounded: serde_json's recursion limit keeps the chain short, but a loop must never hang.
    for _ in 0..1024 {
        let Some(call) = current.get("expansion").and_then(|e| e.get("span")) else {
            break;
        };
        if in_package(call, root, package_dir) {
            best = call;
        }
        current = call;
    }
    best
}

fn in_package(span: &Value, root: &Path, package_dir: &Path) -> bool {
    let name = text(span.get("file_name"));
    // `<anon>`, `<::core::macros::panic macros>` and the like are not files.
    !name.is_empty() && !name.starts_with('<') && resolve(root, &name).starts_with(package_dir)
}

fn span(value: &Value, root: &Path) -> Option<Span> {
    let number = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let name = value.get("file_name")?.as_str()?;
    Some(Span {
        file: if name.starts_with('<') {
            PathBuf::from(name)
        } else {
            resolve(root, name)
        },
        line: number("line_start")?,
        col: number("column_start")?,
        end_line: number("line_end").unwrap_or(0),
        end_col: number("column_end").unwrap_or(0),
        primary: value
            .get("is_primary")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        label: value
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// `name` absolute against `root`, with `.` and `..` taken out lexically.
fn resolve(root: &Path, name: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in root.join(name).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// `text` without ANSI escape sequences (CSI `ESC [ ... final` and OSC `ESC ] ... BEL/ST`).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // Parameters and intermediates, then one final byte in `@`..`~`.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // A two-character escape such as `ESC c`.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACROS: &str = include_str!("../fixtures/macros.jsonl");
    const TARGETS: &str = include_str!("../fixtures/targets.jsonl");

    fn diagnostics(fixture: &str, root: &str) -> Vec<CargoDiagnostic> {
        fixture
            .lines()
            .filter_map(|l| match parse_message(l, Path::new(root)) {
                Some(Message::Diagnostic(d)) => Some(*d),
                _ => None,
            })
            .collect()
    }

    fn place(d: &CargoDiagnostic) -> (String, u32, u32) {
        (
            d.diagnostic.file.display().to_string(),
            d.diagnostic.line,
            d.diagnostic.col,
        )
    }

    // Real output of cargo 1.98.1 (`fixtures/macros.jsonl`, paths replaced by `/fixture`): the package
    // `user` includes `login.frm.rs`, calls a macro of the path dependency `macros` directly and
    // through its own `wrap!`, and has a local macro with an error and an unused variable.
    #[test]
    fn an_included_file_is_the_place_with_its_own_line_and_column() {
        let all = diagnostics(MACROS, "/fixture/user");
        let first = &all[0];
        assert_eq!(
            place(first),
            ("/fixture/user/src/login.frm.rs".into(), 7, 5)
        );
        assert_eq!(first.diagnostic.severity, Severity::Error);
        assert_eq!(first.diagnostic.message, "mismatched types");
        assert_eq!(first.code.as_deref(), Some("E0308"));
        assert_eq!(
            (first.target_kind.as_str(), first.target_name.as_str()),
            ("lib", "user")
        );
        assert_eq!(first.manifest_path, Path::new("/fixture/user/Cargo.toml"));
    }

    #[test]
    fn a_span_in_a_dependency_macro_maps_to_the_outermost_call_in_the_package() {
        let all = diagnostics(MACROS, "/fixture/user");
        // `wrap!()` at line 10 calls `macros::bad!()` at line 5: the outermost call is line 10.
        assert_eq!(place(&all[1]), ("/fixture/user/src/lib.rs".into(), 10, 5));
        // Called directly at line 14.
        assert_eq!(place(&all[2]), ("/fixture/user/src/lib.rs".into(), 14, 5));
        // The raw span is still there, absolute in the dependency.
        assert!(all[1].spans.iter().any(|s| s.primary
            && s.file == Path::new("/fixture/macros/src/lib.rs")
            && s.line == 4));
    }

    #[test]
    fn a_span_in_a_package_macro_stays_in_the_macro() {
        let all = diagnostics(MACROS, "/fixture/user");
        // `local!` is defined at line 23 and called at line 28; the span is in the package, so it stays.
        assert_eq!(place(&all[3]), ("/fixture/user/src/lib.rs".into(), 23, 22));
    }

    #[test]
    fn a_warning_keeps_its_rendered_text_with_notes_and_help() {
        let all = diagnostics(MACROS, "/fixture/user");
        let warning = &all[4];
        assert_eq!(warning.diagnostic.severity, Severity::Warning);
        assert_eq!(warning.code.as_deref(), Some("unused_variables"));
        assert_eq!(place(warning), ("/fixture/user/src/lib.rs".into(), 18, 9));
        assert!(
            warning
                .rendered
                .starts_with("warning: unused variable: `unused`"),
            "{}",
            warning.rendered
        );
        assert!(
            warning.rendered.contains("help: if this is intentional"),
            "{}",
            warning.rendered
        );
        assert!(
            warning
                .rendered
                .contains("note: `#[warn(unused_variables)]`"),
            "{}",
            warning.rendered
        );
    }

    #[test]
    fn a_failure_note_has_no_place() {
        let all = diagnostics(MACROS, "/fixture/user");
        let note = all.last().unwrap();
        assert_eq!(note.level, "failure-note");
        assert_eq!(note.diagnostic.severity, Severity::Note);
        assert_eq!(note.diagnostic.file, PathBuf::new());
        assert_eq!((note.diagnostic.line, note.diagnostic.col), (0, 0));
        assert!(note.spans.is_empty());
    }

    #[test]
    fn multiple_spans_are_all_kept_and_the_primary_one_is_the_place() {
        let all = diagnostics(MACROS, "/fixture/user");
        let spans = &all[0].spans;
        assert_eq!(spans.len(), 2);
        assert!(spans[0].primary && !spans[1].primary);
        assert_eq!((spans[1].line, spans[1].col, spans[1].end_col), (6, 19, 22));
        assert_eq!(
            spans[1].label.as_deref(),
            Some("expected `u32` because of return type")
        );
        assert_eq!((spans[0].end_line, spans[0].end_col), (7, 19));
    }

    // Real output (`fixtures/targets.jsonl`) of `cargo check --all-targets`: a warning in an example,
    // an error in a test and an error in a form under `src/bin/forms`, reported twice (bin and bin test).
    #[test]
    fn every_target_kind_reports_its_place() {
        let all = diagnostics(TARGETS, "/fixture/targets");
        let found: Vec<(String, String, (String, u32, u32))> = all
            .iter()
            .filter(|d| d.level != "failure-note")
            .map(|d| (d.target_kind.clone(), d.level.clone(), place(d)))
            .collect();
        assert_eq!(
            found,
            vec![
                (
                    "example".into(),
                    "warning".into(),
                    ("/fixture/targets/examples/demo.rs".into(), 2, 9)
                ),
                (
                    "test".into(),
                    "error".into(),
                    ("/fixture/targets/tests/adds.rs".into(), 4, 23)
                ),
                (
                    "bin".into(),
                    "error".into(),
                    ("/fixture/targets/src/bin/forms/tool.frm.rs".into(), 8, 9)
                ),
                (
                    "bin".into(),
                    "error".into(),
                    ("/fixture/targets/src/bin/forms/tool.frm.rs".into(), 8, 9)
                ),
            ]
        );
    }

    #[test]
    fn artifacts_and_build_finished_are_read_and_other_lines_ignored() {
        let messages: Vec<Message> = TARGETS
            .lines()
            .filter_map(|l| parse_message(l, Path::new("/fixture/targets")))
            .collect();
        assert_eq!(messages.len(), TARGETS.lines().count());
        assert!(matches!(
            messages[0],
            Message::Artifact { fresh: false, .. }
        ));
        assert_eq!(
            messages.last(),
            Some(&Message::BuildFinished { success: false })
        );
        for line in [
            "",
            "plain program output",
            "{\"reason\":\"hi\"}",
            "{\"no\":\"reason\"}",
            "[1,2]",
            "{\"reason\":\"build-script-executed\",\"package_id\":\"x\"}",
        ] {
            assert_eq!(parse_message(line, Path::new("/")), None, "{line}");
        }
    }

    #[test]
    fn truncated_or_invalid_lines_never_panic() {
        let line = MACROS.lines().nth(2).unwrap();
        for end in 0..line.len() {
            if line.is_char_boundary(end) {
                assert_eq!(
                    parse_message(&line[..end], Path::new("/fixture/user")),
                    None
                );
            }
        }
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert_eq!(parse_message(&deep, Path::new("/")), None);
        // Wrong types everywhere: a message, but nothing usable in it.
        let odd = r#"{"reason":"compiler-message","manifest_path":7,"target":3,"message":{"level":"error","message":5,"spans":[{"file_name":"a.rs","line_start":-1,"column_start":"x","is_primary":true}],"rendered":null}}"#;
        let Some(Message::Diagnostic(d)) = parse_message(odd, Path::new("/r")) else {
            panic!("a message")
        };
        assert_eq!(
            (
                d.diagnostic.file.clone(),
                d.diagnostic.line,
                d.diagnostic.message.as_str()
            ),
            (PathBuf::new(), 0, "")
        );
        assert!(d.spans.is_empty());
        // No level: not a diagnostic.
        assert_eq!(
            parse_message(
                r#"{"reason":"compiler-message","message":{}}"#,
                Path::new("/")
            ),
            None
        );
    }

    fn hand_written(level: &str, spans: &str) -> CargoDiagnostic {
        let line = format!(
            r#"{{"reason":"compiler-message","manifest_path":"/ws/app/Cargo.toml","target":{{"kind":["bin"],"name":"app"}},"message":{{"level":"{level}","message":"m","code":null,"spans":[{spans}],"children":[],"rendered":"\u001b[1m\u001b[91m{level}\u001b[0m: m\n"}}}}"#
        );
        match parse_message(&line, Path::new("/ws")) {
            Some(Message::Diagnostic(d)) => *d,
            other => panic!("{other:?}"),
        }
    }

    fn span_json(file: &str, line: u32, primary: bool, expansion: Option<String>) -> String {
        let expansion = expansion.map_or("null".to_string(), |e| {
            format!(r#"{{"span":{e},"macro_decl_name":"m!"}}"#)
        });
        format!(
            r#"{{"file_name":"{file}","line_start":{line},"line_end":{line},"column_start":3,"column_end":4,"is_primary":{primary},"label":null,"expansion":{expansion}}}"#
        )
    }

    #[test]
    fn every_level_has_its_severity() {
        for (level, severity) in [
            ("error", Severity::Error),
            ("warning", Severity::Warning),
            ("note", Severity::Note),
            ("help", Severity::Hint),
            ("failure-note", Severity::Note),
            ("error: internal compiler error", Severity::Fatal),
            ("something new", Severity::Note),
        ] {
            let d = hand_written(level, "");
            assert_eq!(d.diagnostic.severity, severity, "{level}");
            assert_eq!(d.level, level);
            assert_eq!(d.rendered, format!("{level}: m\n"), "ANSI is stripped");
        }
    }

    #[test]
    fn member_paths_are_relative_to_the_workspace_root() {
        let d = hand_written("error", &span_json("app/src/main.rs", 3, true, None));
        assert_eq!(place(&d), ("/ws/app/src/main.rs".into(), 3, 3));
        let d = hand_written(
            "error",
            &span_json("app/src/../src/./main.rs", 3, true, None),
        );
        assert_eq!(d.diagnostic.file, Path::new("/ws/app/src/main.rs"));
    }

    #[test]
    fn the_first_primary_span_is_the_place_when_there_are_several() {
        let spans = [
            span_json("app/src/a.rs", 1, false, None),
            span_json("app/src/b.rs", 2, true, None),
            span_json("app/src/c.rs", 3, true, None),
        ];
        let d = hand_written("error", &spans.join(","));
        assert_eq!(place(&d), ("/ws/app/src/b.rs".into(), 2, 3));
        assert_eq!(d.spans.iter().filter(|s| s.primary).count(), 2);
    }

    #[test]
    fn std_macros_map_to_the_outermost_package_call_and_a_chain_without_one_stays() {
        // The span is in the standard library, called from a package macro (line 5), called at line 9.
        let outer = span_json("app/src/main.rs", 9, false, None);
        let inner = span_json("app/src/main.rs", 5, false, Some(outer));
        let d = hand_written(
            "error",
            &span_json(
                "/rustc/abc/library/core/src/macros/mod.rs",
                40,
                true,
                Some(inner),
            ),
        );
        assert_eq!(place(&d), ("/ws/app/src/main.rs".into(), 9, 3));
        // A pseudo file and a call site in another package of the workspace are not the package.
        let other = span_json("lib2/src/lib.rs", 7, false, None);
        let d = hand_written("error", &span_json("<anon>", 1, true, Some(other)));
        assert_eq!(place(&d), ("<anon>".into(), 1, 3));
    }

    #[test]
    fn ansi_sequences_are_stripped() {
        assert_eq!(
            strip_ansi("\u{1b}[1m\u{1b}[38;5;9merror\u{1b}[0m: x"),
            "error: x"
        );
        assert_eq!(
            strip_ansi("a\u{1b}]8;;http://x\u{7}link\u{1b}]8;;\u{1b}\\b"),
            "alinkb"
        );
        assert_eq!(strip_ansi("plain é ✓"), "plain é ✓");
        assert_eq!(strip_ansi("cut \u{1b}["), "cut ");
        assert_eq!(strip_ansi("\u{1b}"), "");
    }
}
