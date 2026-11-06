//! Format Code: rustfmt, through rust-analyzer's `textDocument/formatting`, formats a Rust file
//! (`.rs` and `.frm.rs`).
//!
//! The client applies the edits to the file ON DISK, so the IDE saves the tab first, asks, and
//! reads the file again when the server answers (`finish_format`). Format on save (Preferences) does
//! the same after Cmd+S.

use crate::{App, assistant, lsp};
use eframe::egui;
use std::path::{Path, PathBuf};

/// The files rustfmt formats (`login.frm.rs` has the extension `rs` too).
const EXTENSIONS: [&str; 1] = ["rs"];

pub(crate) fn can_format(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// A format under way: the file, and the text that was saved for it (what the editor still shows when
/// nothing was typed in the meantime).
pub(crate) struct Formatting {
    pub path: PathBuf,
    pub text: String,
}

impl App {
    /// File > Save: saves the active tab; with Format on save (Preferences) a Rust file is then
    /// formatted, which saves it again as formatted.
    pub(crate) fn save_active(&mut self) {
        self.save_doc(self.active);
        // In the designer, Save exports the form; the .rs is not formatted (nor written).
        if crate::prefs::format_on_save() && !self.form_showing() {
            self.format_active(false);
        }
    }

    /// Formats the active file. `explicit`: the user asked (Edit > Format Code), so problems are
    /// said in the console; otherwise (format on save) the file is skipped silently.
    pub(crate) fn format_active(&mut self, explicit: bool) {
        let Some(path) = self.docs.get(self.active).map(|d| d.path.clone()) else {
            return;
        };
        if !can_format(&path) {
            if explicit {
                self.console
                    .push_str("Format Code: only Rust sources (.rs, .frm.rs) are formatted\n");
            }
            return;
        }
        if self.formatting.is_some() {
            return;
        }
        if self.lsp.is_none() || !self.lsp_ready {
            if explicit {
                self.console
                    .push_str("Format Code: the language server is off\n");
            }
            return;
        }
        // The server reads the file from disk, so what is open must be there.
        self.save_doc(self.active);
        let doc = &self.docs[self.active];
        if doc.text != doc.saved {
            self.console
                .push_str("Format Code: the file could not be saved first\n");
            return;
        }
        self.formatting = Some(Formatting {
            path: path.clone(),
            text: doc.text.clone(),
        });
        if let Some(lsp) = &self.lsp {
            let _ = lsp.requests.send(lsp::Req::Format { path });
        }
    }

    /// The server is done with the file (or failed): read the formatted file into its tab.
    pub(crate) fn finish_format(&mut self, ctx: &egui::Context, error: Option<String>) {
        let Some(sent) = self.formatting.take() else {
            return;
        };
        let name = sent
            .path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if let Some(e) = error {
            self.console.push_str(&format!("Format Code: {e}\n"));
            return;
        }
        let Ok(formatted) = std::fs::read_to_string(&sent.path) else {
            self.console
                .push_str(&format!("Format Code: could not read {name} again\n"));
            return;
        };
        let Some(i) = self.docs.iter().position(|d| d.path == sent.path) else {
            return; // the tab was closed: the file on disk is formatted, which is what was asked
        };
        if self.docs[i].text != sent.text {
            // Typed while the server worked. Their text wins: the saved text goes back on disk, so the
            // file and the editor still agree on what was saved.
            let _ = std::fs::write(&sent.path, &sent.text);
            self.console.push_str(&format!(
                "Format Code: {name} was edited meanwhile, so it was left as it was\n"
            ));
            return;
        }
        if formatted == sent.text {
            self.console
                .push_str(&format!("{name} is already formatted\n"));
            return;
        }
        assistant::set_doc_text(&mut self.docs[i], formatted, true, Some(ctx));
        self.console.push_str(&format!("Formatted {name}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSY: &str = "fn   foo( a:i32,b : &str )->i32{\nif a>0 {println!(\"{}\",b);}\n a}\n";
    /// What rustfmt makes of `MESSY`.
    const NEAT: &str = "fn foo(a: i32, b: &str) -> i32 {\n    if a > 0 {\n        println!(\"{}\", b);\n    }\n    a\n}\n";

    /// An app with `u.rs` open in a scratch folder, holding `text` on disk and in the tab.
    fn app_with(name: &str, text: &str) -> (App, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("formust-format-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("u.rs");
        std::fs::write(&file, text).unwrap();
        let app = App::bare(Some(file.clone()));
        (app, file.canonicalize().unwrap())
    }

    /// What the language server leaves on disk is read into the tab, which is then clean.
    #[test]
    fn the_formatted_file_replaces_the_tab_text_and_leaves_it_saved() {
        let (mut app, file) = app_with("applies", MESSY);
        let ctx = egui::Context::default();
        app.formatting = Some(Formatting {
            path: file.clone(),
            text: MESSY.into(),
        });
        std::fs::write(&file, NEAT).unwrap(); // the server did its work
        app.finish_format(&ctx, None);
        assert_eq!(app.docs[0].text, NEAT);
        assert_eq!(app.docs[0].saved, NEAT, "clean: the disk has the same text");
        assert!(!app.is_dirty(0));
        assert!(app.formatting.is_none());
        assert!(app.console.contains("Formatted u.rs"), "{}", app.console);
    }

    /// Typing while the server works keeps the user's text; the saved text goes back on disk.
    #[test]
    fn text_typed_during_the_format_is_kept_and_the_disk_goes_back() {
        let (mut app, file) = app_with("typed", MESSY);
        let ctx = egui::Context::default();
        app.formatting = Some(Formatting {
            path: file.clone(),
            text: MESSY.into(),
        });
        std::fs::write(&file, NEAT).unwrap();
        app.docs[0].text.push_str("// typed\n");
        app.finish_format(&ctx, None);
        assert!(
            app.docs[0].text.ends_with("// typed\n") && app.docs[0].text.starts_with("fn   foo(")
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            MESSY,
            "the file and the editor agree on what was saved"
        );
        assert!(app.console.contains("edited meanwhile"), "{}", app.console);
    }

    #[test]
    fn already_formatted_code_and_server_errors_change_nothing() {
        let (mut app, file) = app_with("same", NEAT);
        let ctx = egui::Context::default();
        app.formatting = Some(Formatting {
            path: file.clone(),
            text: NEAT.into(),
        });
        app.finish_format(&ctx, None);
        assert!(
            app.console.contains("u.rs is already formatted"),
            "{}",
            app.console
        );
        app.console.clear();
        app.formatting = Some(Formatting {
            path: file,
            text: NEAT.into(),
        });
        app.finish_format(&ctx, Some("textDocument/formatting: boom".into()));
        assert_eq!(app.docs[0].text, NEAT);
        assert!(
            app.console
                .contains("Format Code: textDocument/formatting: boom"),
            "{}",
            app.console
        );
        assert!(app.formatting.is_none(), "the next format can start");
    }

    #[test]
    fn format_code_says_why_it_does_nothing() {
        let (mut app, _) = app_with("off", MESSY);
        app.format_active(true);
        assert!(
            app.console.contains("language server is off"),
            "{}",
            app.console
        );
        assert!(app.formatting.is_none());
        // Format on save stays quiet.
        app.console.clear();
        app.format_active(false);
        assert!(app.console.is_empty(), "{}", app.console);
        // Not a Rust source.
        let dir = std::env::temp_dir().join(format!("formust-format-md-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.md"), "# x\n").unwrap();
        let mut md = App::bare(Some(dir.join("notes.md")));
        md.format_active(true);
        assert!(md.console.contains("only Rust sources"), "{}", md.console);
    }

    /// A plain Save formats the file only when Format on save is on.
    #[test]
    fn a_save_formats_only_with_format_on_save() {
        let _apply = crate::prefs::tests::APPLY
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let ctx = egui::Context::default();
        let run = |on: bool, name: &str| -> (String, String) {
            let (mut app, file) = app_with(name, MESSY);
            crate::prefs::Prefs {
                format_on_save: on,
                ..crate::prefs::Prefs::default()
            }
            .apply();
            app.poll_lsp(&ctx);
            let have_server = app.lsp.is_some();
            for _ in 0..400 {
                if app.lsp_ready || !have_server {
                    break;
                }
                app.poll_lsp(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            app.save_active();
            for _ in 0..400 {
                if app.formatting.is_none() {
                    break;
                }
                app.poll_lsp(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            (
                app.docs[0].text.clone(),
                std::fs::read_to_string(&file).unwrap(),
            )
        };
        let (off_tab, off_disk) = run(false, "save-off");
        assert_eq!(
            (off_tab.as_str(), off_disk.as_str()),
            (MESSY, MESSY),
            "saved as it is"
        );
        let (on_tab, on_disk) = run(true, "save-on");
        crate::prefs::Prefs::default().apply();
        if on_disk == MESSY {
            eprintln!("skipped the formatted half: rust-analyzer or rustfmt missing");
            return;
        }
        assert_eq!(on_tab, on_disk, "the tab shows what is on disk");
        assert_eq!(on_tab, NEAT);
    }

    /// The whole path with the real language server: Format Code saves the tab, rust-analyzer's rustfmt
    /// rewrites the file, and the tab shows the formatted text. Skipped without rust-analyzer or rustfmt.
    #[test]
    fn format_code_formats_the_open_unit_through_the_real_server() {
        let (mut app, file) = app_with("real", MESSY);
        let ctx = egui::Context::default();
        let wait = |app: &mut App, what: &str, done: &dyn Fn(&App) -> bool| {
            for _ in 0..400 {
                app.poll_lsp(&ctx);
                if done(app) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            eprintln!("gave up waiting for {what}");
            false
        };
        app.poll_lsp(&ctx); // starts the server for the project
        if app.lsp.is_none() {
            eprintln!("skipped: rust-analyzer not found");
            return;
        }
        assert!(
            wait(&mut app, "the server", &|a| a.lsp_ready),
            "the server starts"
        );
        app.docs[0].text.push_str("\n"); // an unsaved edit: Format Code saves it first
        app.format_active(true);
        assert!(app.formatting.is_some(), "waiting for the server");
        assert!(
            wait(&mut app, "the format", &|a| a.formatting.is_none()),
            "the server answers"
        );
        assert_eq!(app.docs[0].text, NEAT);
        assert_eq!(
            app.docs[0].text,
            std::fs::read_to_string(&file).unwrap(),
            "the tab shows what is on disk"
        );
        assert_eq!(app.docs[0].saved, app.docs[0].text);
    }

    #[test]
    fn only_rust_sources_are_formatted() {
        for ok in [
            "a.rs",
            "A.RS",
            "main.rs",
            "forms/login.frm.rs",
            "/x/y/lib.rs",
        ] {
            assert!(can_format(Path::new(ok)), "{ok}");
        }
        for no in [
            "a.txt",
            "notes.md",
            "Cargo.toml",
            "Makefile",
            "a.rs.bak",
            "a.flr",
        ] {
            assert!(!can_format(Path::new(no)), "{no}");
        }
    }
}
