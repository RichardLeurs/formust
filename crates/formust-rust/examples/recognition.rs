//! The recognised-node ratio over real egui code (`docs/specs.md`, Risk 2).
//!
//! ```text
//! cargo run --offline -p formust-rust --example recognition -- [--methods-only] [--extended] [--top N] [--files] [--sample REASON] NAME=PATH...
//! ```
//!
//! Each `NAME=PATH` is a corpus: a `.rs` file or a folder searched recursively. Files are parsed by
//! `syn` and nothing else: never built or run. A file that doesn't parse is reported, not hidden.

use std::path::{Path, PathBuf};

use formust_rust::stats::Tally;
use formust_rust::{ImportOptions, emit_body, import_with};

fn main() {
    let mut methods_only = false;
    let mut extended = false;
    let mut show_files = false;
    let mut top = 25;
    let mut sample: Option<String> = None;
    let mut corpora = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--methods-only" => methods_only = true,
            "--files" => show_files = true,
            "--extended" => extended = true,
            "--sample" => sample = args.next(),
            "--top" => {
                top = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .expect("--top takes a number")
            }
            other => match other.split_once('=') {
                Some((name, path)) => corpora.push((name.to_string(), PathBuf::from(path))),
                None => panic!("expected NAME=PATH, got {other}"),
            },
        }
    }
    assert!(!corpora.is_empty(), "give at least one NAME=PATH");

    let options = ImportOptions {
        free_functions: !methods_only,
        extended,
        ..ImportOptions::default()
    };
    let mut overall = Tally::default();
    for (name, path) in &corpora {
        let mut files = Vec::new();
        collect(path, &mut files);
        files.sort();
        let mut corpus = Tally::default();
        let (mut failed, mut scanned, mut with_forms, mut lost, mut form_files) = (0, 0, 0, 0, 0);
        for file in &files {
            // A form file holds forms already counted from their source.
            if formust_rust::file_kind(file) == formust_rust::FileKind::Form {
                form_files += 1;
                continue;
            }
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            scanned += 1;
            let imported = match import_with(&text, options) {
                Ok(imported) => imported,
                Err(error) => {
                    failed += 1;
                    println!("  PARSE FAILED {}: {error}", file.display());
                    continue;
                }
            };
            for form in &imported.forms {
                if let Some(wanted) = &sample {
                    for custom in form
                        .custom
                        .iter()
                        .filter(|c| c.reason.contains(wanted.as_str()))
                    {
                        if let Some(r) = form.ranges.get(&custom.node) {
                            let text: String = text[r.clone()].chars().take(200).collect();
                            println!("[{}] {}", custom.reason, text.replace('\n', " "));
                        }
                    }
                }
                // The measurement is only worth reading if nothing was lost on the way.
                if form.form.legacy_ctx.is_none()
                    && emit_body(&form.form) != text[form.body.clone()]
                {
                    lost += 1;
                    println!("  NOT VERBATIM {} {}", file.display(), form.form.id());
                }
            }
            let mut tally = Tally::default();
            tally.add_file(&imported);
            if tally.forms > 0 {
                with_forms += 1;
                if show_files {
                    println!("  {:<58} {}", short(file, path), line(&tally));
                }
            }
            corpus.add(&tally);
        }
        println!(
            "\n== {name}: {scanned} files, {with_forms} with forms, {failed} failed to parse, {lost} not verbatim, {form_files} .frm.rs skipped"
        );
        println!("{}", line(&corpus));
        overall.add(&corpus);
    }
    println!("\n== overall");
    println!("{}", line(&overall));
    println!(
        "palette calls hidden inside custom blocks: {}",
        overall.hidden_calls
    );
    let mut reasons: Vec<_> = overall.reasons.iter().collect();
    reasons.sort_by_key(|(_, t)| std::cmp::Reverse(t.bytes));
    println!(
        "\ntop custom reasons, by bytes (count, bytes, share of custom bytes, hidden palette calls):"
    );
    for (reason, tally) in reasons.iter().take(top) {
        let share = 100.0 * tally.bytes as f64 / overall.custom_bytes.max(1) as f64;
        println!(
            "  {:>5} {:>8} {:>5.1}% {:>5}  {reason}",
            tally.count, tally.bytes, share, tally.hidden_calls
        );
    }
    let mut by_count: Vec<_> = overall.reasons.iter().collect();
    by_count.sort_by_key(|(_, t)| std::cmp::Reverse(t.count));
    println!("\ntop custom reasons, by count:");
    for (reason, tally) in by_count.iter().take(top) {
        println!(
            "  {:>5} {:>8} {:>5}  {reason}",
            tally.count, tally.bytes, tally.hidden_calls
        );
    }
}

fn line(t: &Tally) -> String {
    format!(
        "forms {:>4} | widgets {:>5} containers {:>4} custom {:>5} flow {:>4} | by count {:>5.1}% | bytes w {:>7} c {:>6} custom {:>7} flow {:>6} | by bytes {:>5.1}% | ui calls {:>5.1}%",
        t.forms,
        t.widgets,
        t.containers,
        t.customs,
        t.flows,
        100.0 * t.count_ratio(),
        t.widget_bytes,
        t.container_bytes,
        t.custom_bytes,
        t.flow_bytes,
        100.0 * t.byte_ratio(),
        100.0 * t.ui_call_ratio()
    )
}

fn short(file: &Path, root: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        files.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "target") {
                collect(&p, files);
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            files.push(p);
        }
    }
}
