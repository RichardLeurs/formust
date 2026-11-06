//! Draws the whole IDE offscreen and saves PNGs, so the look can be checked without a screen.
//! Output: `<target dir>/tmp/ui-*.png`.

use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn out_dir() -> PathBuf {
    crate::target_tmp()
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/editor/debug.rs")
}

/// Renders `app` at `size` points and saves `<name>.png`; returns the path.
fn render(app: App, dark: bool, size: (f32, f32), name: &str) -> PathBuf {
    // The fonts must be in place before the first frame (menu shortcuts use SF Pro's ⌘).
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .with_pixels_per_point(1.0)
        .with_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        })
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    // A fixed number of frames: the build bar repaints forever, so `run()` would never settle.
    harness.run_steps(3);
    let image = harness.render().expect("offscreen render");
    let path = out_dir().join(format!("{name}.png"));
    image.save(&path).unwrap();
    path
}

fn editing_app() -> App {
    let mut app = App::bare(Some(fixture()));
    app.console = "   Compiling debug v0.1.0\n    Finished `dev` profile in 0.3s\n".into();
    app.lsp_ready = true;
    app
}

/// A terminal running `script` under `/bin/sh`, waited for until `marker` is on its screen.
fn terminal_with(script: &str, marker: &str) -> crate::terminal::Terminal {
    let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
    cmd.arg("-c");
    cmd.arg(script);
    let t = crate::terminal::Terminal::spawn(cmd, 12, 100, || {}).expect("spawn");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !t.screen_text().contains(marker) && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        t.screen_text().contains(marker),
        "no output: {}",
        t.screen_text()
    );
    t
}

/// The four tabs of the bottom pane, in both themes, with the pane tall enough to read.
#[test]
fn renders_the_references_in_the_search_tab() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.bottom_height = 260.0;
    app.nav_tab = shell::NavTab::Search;
    app.show_navigator = true;
    let f = fixture();
    app.project_bar.title = "References to total".into();
    app.show_references(vec![
        formust_lsp::Location {
            path: f.clone(),
            line: 2,
            col: 4,
        },
        formust_lsp::Location {
            path: f.clone(),
            line: 8,
            col: 4,
        },
        formust_lsp::Location {
            path: f.clone(),
            line: 8,
            col: 13,
        },
    ]);
    render(app, false, (1100.0, 640.0), "ui-references");
}

#[test]
fn renders_the_document_outline() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    // A stub server, so the outline is shown as read for the current text (nothing is asked).
    let (requests, _asked) = std::sync::mpsc::channel();
    let (_answers, responses) = std::sync::mpsc::channel();
    app.lsp = Some(lsp::Handle {
        requests,
        responses,
    });
    let path = app.docs[0].path.clone();
    let key = shell::text_key(&app.docs[0].text);
    app.outline.shown = Some((path.clone(), key));
    app.outline.symbols = vec![
        formust_lsp::Symbol {
            name: "TForm1".into(),
            kind: 5,
            line: 4,
            col: 2,
            depth: 0,
        },
        formust_lsp::Symbol {
            name: "Button1".into(),
            kind: 8,
            line: 5,
            col: 4,
            depth: 1,
        },
        formust_lsp::Symbol {
            name: "Button1Click".into(),
            kind: 6,
            line: 6,
            col: 4,
            depth: 1,
        },
        formust_lsp::Symbol {
            name: "Form1".into(),
            kind: 13,
            line: 10,
            col: 2,
            depth: 0,
        },
    ];
    app.nav_tab = shell::NavTab::Outline;
    render(app, false, (1100.0, 640.0), "ui-outline");
}

#[test]
fn renders_the_editor_with_a_font_and_background_from_the_preferences() {
    let _guard = crate::prefs::tests::APPLY
        .write()
        .unwrap_or_else(|e| e.into_inner());
    let app = editing_app();
    let mut p = crate::prefs::Prefs {
        font: 2,
        ..crate::prefs::Prefs::default()
    };
    p.set_background(false, Some([0xFB, 0xF6, 0xE9]));
    p.apply();
    let png = render(app, false, (1100.0, 640.0), "ui-editor-courier-background");
    crate::prefs::Prefs::default().apply();
    assert!(png.exists());
}

#[test]
fn renders_the_editor_with_word_wrap() {
    let _guard = crate::prefs::tests::APPLY
        .write()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.docs[0]
        .text
        .push_str(&format!("\n// {}\n", "wrapped words ".repeat(30)));
    crate::prefs::Prefs {
        word_wrap: true,
        ..crate::prefs::Prefs::default()
    }
    .apply();
    let png = render(app, false, (1100.0, 640.0), "ui-editor-wrap");
    crate::prefs::Prefs::default().apply();
    assert!(png.exists());
}

#[test]
fn renders_the_preferences_window() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.preferences_open = true;
    render(app, false, (1100.0, 640.0), "ui-preferences");
}

#[test]
fn renders_the_preferences_window_dark() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.preferences_open = true;
    render(app, true, (1100.0, 640.0), "ui-preferences-dark");
}

#[test]
fn renders_the_about_window() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = App::bare(None);
    app.about_open = true;
    render(app, false, (900.0, 760.0), "ui-about");
}

#[test]
fn renders_the_about_window_dark() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = App::bare(None);
    app.about_open = true;
    render(app, true, (900.0, 760.0), "ui-about-dark");
}

#[test]
fn renders_the_signature_hint_over_a_call() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.docs[0].text = "fn main() {\n    add(1, 2);\n}\n".into();
    let at = app.docs[0].text.find("2)").unwrap();
    app.signature = Some(shell::SigHint {
        key: shell::text_key(&app.docs[0].text),
        at,
        sig: formust_lsp::Signature {
            label: "Add(a: Integer; b: Integer):Integer".into(),
            parameters: vec!["a: Integer".into(), "b: Integer".into()],
            active: Some(1),
        },
    });
    render(app, false, (1100.0, 640.0), "ui-signature-hint");
}

#[test]
fn renders_a_go_to_scrolled_to_a_far_line() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.docs[0].text = (1..=120).map(|i| format!("  line {i} = {i};\n")).collect();
    let offset = app.docs[0].text.find("line 90 ").unwrap();
    app.reveal = Some(offset..offset + 1);
    render(app, false, (1100.0, 640.0), "ui-reveal-line-90");
}

/// A click on the message of a problem (not on its red marker) goes to its line in the editor.
#[test]
fn clicking_a_problem_message_goes_to_its_line() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.bottom_height = 250.0;
    app.bottom.tab = bottom::BottomTab::Problems;
    app.diagnostics = vec![Diagnostic {
        file: app.docs[0].path.clone(),
        line: 8,
        col: 5,
        severity: Severity::Error,
        message: "Identifier not found \"Foo\"".into(),
    }];
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 640.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    theme::apply(&harness.ctx);
    harness.run_steps(2);
    harness
        .get_by_label("Error: Identifier not found \"Foo\"")
        .click();
    harness.run_steps(6);
    // The caret is on the problem's line and column (the editor scrolls to it the frame after).
    assert_eq!(harness.state().cursor_pos, (8, 5), "line 8, column 5");
}

/// A long search result is cut to one line that fits the row, ending with "...". Short ones are kept.
#[test]
fn a_search_result_is_cut_to_one_line_with_dots() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(300.0, 120.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(
            |ui, out: &mut Vec<String>| {
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let long = "   10:5    println!(\"a very long line of text that will not fit in the sidebar at all\");";
                out.push(shell::fit_one_line(ui, long, &font, 200.0));
                out.push(shell::fit_one_line(ui, "short", &font, 200.0));
            },
            Vec::new(),
        );
    harness.run_steps(2);
    let out = harness.state().clone();
    assert!(out[0].ends_with("..."), "{:?}", out[0]);
    assert!(out[0].chars().count() < 80, "cut: {:?}", out[0]);
    assert_eq!(out[1], "short", "text that fits is kept");
}

/// The Search tab in the sidebar with a result too long for its row.
#[test]
fn renders_the_search_results_on_one_line_each() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.nav_tab = shell::NavTab::Search;
    app.show_navigator = true;
    app.project_bar.query = "Write".into();
    app.project_bar.results = vec![find::FileHits {
        path: app.docs[0].path.clone(),
        hits: vec![find::Hit {
            line: 10,
            col: 3,
            text: "  println!(\"a line of text far longer than the sidebar can show, so it must be cut\");\n".into(),
            range: 0..0,
        }],
    }];
    app.project_bar.searched = true;
    render(app, false, (1100.0, 640.0), "ui-search-one-line");
}

#[test]
fn renders_the_go_to_line_box() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.go_to.open = true;
    app.go_to.text = "99".into();
    app.go_to.error = "Line 99 is not in the file (it has 12 lines)".into();
    render(app, false, (1100.0, 640.0), "ui-go-to-line");
}

#[test]
fn renders_the_find_bar_and_the_project_search() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.find.show(true);
    app.find.query = "end".into();
    app.find.with = "stop".into();
    app.find.current = Some(0);
    render(app, false, (1100.0, 640.0), "ui-find-bar");

    let mut app = editing_app();
    app.bottom_height = 260.0;
    app.nav_tab = shell::NavTab::Search;
    app.show_navigator = true;
    app.project_bar.query = "Write".into();
    app.run_project_search();
    app.finish_project_search();
    app.project_bar.with = "Print".into();
    render(app, false, (1100.0, 640.0), "ui-project-search");
}

#[test]
fn renders_the_bottom_tabs() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use bottom::BottomTab::*;
    let diagnostics = || {
        vec![
            Diagnostic {
                file: fixture(),
                line: 8,
                col: 5,
                severity: Severity::Error,
                message: "Identifier not found \"Foo\"".into(),
            },
            Diagnostic {
                file: fixture(),
                line: 3,
                col: 1,
                severity: Severity::Warning,
                message: "Local variable \"i\" does not seem to be initialized".into(),
            },
        ]
    };
    for dark in [false, true] {
        let suffix = if dark { "-dark" } else { "" };
        let size = (1100.0, 640.0);
        let mut app = editing_app();
        app.bottom_height = 250.0;
        app.diagnostics = diagnostics();
        app.bottom.tab = Problems;
        render(app, dark, size, &format!("ui-bottom-problems{suffix}"));

        let mut app = editing_app();
        app.bottom_height = 250.0;
        app.bottom.tab = Output;
        app.console.push_str(
            "Debug build, 1 warning(s)\n\n--- program output ---\nHello from Formust\nsum = 42\n",
        );
        render(app, dark, size, &format!("ui-bottom-output{suffix}"));

        let mut app = editing_app();
        app.bottom_height = 250.0;
        app.bottom.tab = Terminal;
        app.bottom.view_shown = true;
        app.bottom.terminal = Some(terminal_with(
            "printf '\\033[1;32muser@mac\\033[0m \\033[1;34m~/proj\\033[0m %% ls\\n'; \
             printf '\\033[1;34mexamples\\033[0m  \\033[32mbuild.sh\\033[0m  main.rs  \\033[4munderlined\\033[0m  \\033[7minverse\\033[0m\\n'; \
             printf '\\033[31mred\\033[0m \\033[33myellow\\033[0m \\033[36mcyan\\033[0m \\033[38;5;208morange256\\033[0m \\033[38;2;120;80;255mtruecolor\\033[0m \\033[48;5;236m bg \\033[0m\\n'; \
             printf '\\033[1;32muser@mac\\033[0m \\033[1;34m~/proj\\033[0m %% '; sleep 30",
            "~/proj",
        ));
        render(app, dark, size, &format!("ui-bottom-terminal{suffix}"));
    }
}

#[test]
fn renders_the_ide() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let a = render(editing_app(), false, (1368.0, 800.0), "ui-light");
    println!("{}", a.display());
}

/// Squiggles, gutter dots and the hover popup, with a build error and a warning in the unit.
#[test]
fn renders_diagnostics_and_hover() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use egui_kittest::kittest::Queryable;
    use hints::{FileMarks, Mark};
    let mut app = editing_app();
    let mark = |line, col, severity, message: &str| Mark {
        line,
        col,
        severity,
        message: message.into(),
    };
    app.marks = vec![FileMarks {
        path: app.docs[0].path.clone(),
        text: app.docs[0].text.clone(),
        items: vec![
            mark(6, 2, Severity::Error, "Identifier not found \"totl\""),
            mark(
                8,
                4,
                Severity::Warning,
                "Variable \"i\" does not seem to be initialized",
            ),
        ],
    }];
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 640.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    theme::apply(&harness.ctx);
    harness.run();
    // Rest the pointer on the second mark (`total`, line 9), then on `Formust`'s identifier.
    let editor = harness.get_by_label("Code editor").rect();
    let at = |line: f32, col: f32| {
        egui::pos2(editor.min.x + 7.9 * col, editor.min.y + 18.0 * (line + 0.5))
    };
    harness.hover_at(at(8.0, 6.0));
    for _ in 0..8 {
        harness.step();
    }
    assert!(
        harness
            .state()
            .hover
            .popup
            .as_ref()
            .is_some_and(|p| p.text.starts_with("Warning")),
        "no popup for the mark"
    );
    let image = harness.render().expect("offscreen render");
    let path = out_dir().join("ui-diagnostics-hover.png");
    image.save(&path).unwrap();
    println!("{}", path.display());
}

#[test]
fn renders_build_progress() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.building = Some(crate::cargo::Session::for_test());
    app.build_progress = Some(formust_build::Progress {
        label: "Compiling formust-build v0.1.0".into(),
        fraction: Some(0.4),
        done: 3,
    });
    let path = render(app, false, (1100.0, 300.0), "ui-build-progress");
    assert!(path.exists());
}

// ---- the assistant panel ------------------------------------------------------------------

fn assistant_app() -> App {
    let mut app = editing_app();
    app.assistant.open = true;
    app.assistant.use_key("not-a-real-key");
    app.assistant.usage = formust_ai::Usage {
        input_tokens: 5230,
        output_tokens: 412,
        cache_read_input_tokens: 4800,
        ..Default::default()
    };
    app
}

#[test]
fn renders_the_assistant_key_card() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.assistant.open = true;
    app.assistant.key = assistant::KeyState::Missing;
    let p = render(app, false, (1368.0, 800.0), "ui-assistant-key");
    println!("{}", p.display());
}

#[test]
fn renders_the_empty_assistant() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let p = render(
        assistant_app(),
        false,
        (1368.0, 800.0),
        "ui-assistant-empty",
    );
    println!("{}", p.display());
}

fn chat_items() -> Vec<assistant::Item> {
    use assistant::*;
    let md = "I looked at the program. The problem is the **missing semicolon** after `total = 0`.\n\n\
## Fix\n\n\
1. Add the semicolon\n\
2. Rebuild with `Cmd+R`\n\n\
- `println!` needs no import for plain strings\n\
- keep the *mode* line first\n\n\
```rust\nfn main() {\n    let mut total = 0; // reset\n    for i in 0..3 {\n        total = total + i;\n    }\n    println!(\"{total}\");\n}\n```\n\n\
```rust\ntotal += 1;\n```\n\n\
Want me to apply it?";
    vec![
        Item::User {
            text: "Why does it not compile?".into(),
            sent: "[Context from the IDE]\nActive file: debug.rs (caret at line 8)\n[End of context]\n\nWhy does it not compile?".into(),
            chips: vec!["debug.rs".into(), "1 error".into()],
        },
        Item::Tool(ToolRow {
            id: "1".into(),
            label: "Read debug.rs".into(),
            state: ToolState::Done,
            output: "fn main() {}".into(),
        }),
        Item::Tool(ToolRow {
            id: "2".into(),
            label: "Get build errors".into(),
            state: ToolState::Done,
            output: String::new(),
        }),
        Item::Tool(ToolRow {
            id: "3".into(),
            label: "Build".into(),
            state: ToolState::Failed("The user rejected this change.".into()),
            output: String::new(),
        }),
        Item::Assistant(md.into()),
    ]
}

#[test]
fn renders_the_assistant_chat() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = assistant_app();
    app.assistant.items = chat_items();
    app.assistant
        .items
        .push(assistant::Item::Note("Stopped.".into()));
    let p = render(app, false, (1368.0, 1300.0), "ui-assistant-chat");
    println!("{}", p.display());
    let mut app = assistant_app();
    app.assistant.items = chat_items();
    let p = render(app, true, (1368.0, 1300.0), "ui-assistant-chat-dark");
    println!("{}", p.display());
}

#[test]
fn renders_a_pending_proposal() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use assistant::*;
    let mut app = assistant_app();
    let abs = fixture().canonicalize().unwrap();
    let old = app.docs[0].text.clone();
    let change = formust_ai::Change::Edit {
        old_text: "    total = total + i;".into(),
        new_text: "    {\n      total = total + i;\n      total += 1;\n    }".into(),
    };
    let p = formust_ai::Proposal {
        id: "e1".into(),
        path: "debug.rs".into(),
        abs_path: abs.clone(),
        change: change.clone(),
    };
    let preview = preview_of(&p, Some(old));
    assert!(preview.is_ok());
    app.assistant.items = chat_items()[..2].to_vec();
    app.assistant.items.push(Item::Assistant(
        "The loop never counts the last step. This edit fixes it:".into(),
    ));
    app.assistant.items.push(Item::Tool(ToolRow {
        id: "e1".into(),
        label: "Edit debug.rs".into(),
        state: ToolState::Running,
        output: String::new(),
    }));
    app.assistant.items.push(Item::Card(Card {
        id: "e1".into(),
        path: "debug.rs".into(),
        abs,
        change,
        state: CardState::Pending,
        auto: false,
        undo: None,
        local: false,
        preview,
    }));
    app.assistant.items.push(Item::Card(Card {
        id: "b1".into(),
        path: String::new(),
        abs: PathBuf::new(),
        change: formust_ai::Change::Build,
        state: CardState::Pending,
        auto: false,
        undo: None,
        local: false,
        preview: Ok((vec![], (0, 0))),
    }));
    let path = render(app, false, (1368.0, 1200.0), "ui-assistant-diff");
    println!("{}", path.display());
}

/// Typing into the real input field: Enter sends, Shift+Enter adds a line.
#[test]
fn enter_sends_and_shift_enter_adds_a_line() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    // The mock answers with one text reply; nothing leaves this machine.
    let (url, _) = assistant::mock_server(vec![assistant::text_reply("Hi there")]);
    let mut app = assistant_app();
    app.assistant.base_url = Some(url);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1368.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(2);
    harness.get_by_label("Message").focus();
    harness.run_steps(2);
    harness.get_by_label("Message").type_text("first");
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    harness.run_steps(2);
    harness.get_by_label("Message").type_text("second");
    harness.run_steps(2);
    assert_eq!(harness.state().assistant.input, "first\nsecond");
    assert!(harness.state().assistant.items.is_empty());
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    let a = &harness.state().assistant;
    assert!(a.input.is_empty(), "Enter sent the message: {:?}", a.input);
    assert!(matches!(&a.items[0], assistant::Item::User { text, .. } if text == "first\nsecond"));
    // The answer streams in through the app's poll.
    for _ in 0..400 {
        harness.run_steps(1);
        if !harness.state().assistant.streaming() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let a = &harness.state().assistant;
    assert!(!a.streaming(), "{:?}", a.items);
    assert!(matches!(a.items.last(), Some(assistant::Item::Assistant(t)) if t == "Hi there"));
}

/// Clicking Apply / Reject on a card in the real panel.
#[test]
fn card_buttons_apply_and_reject() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use assistant::*;
    let mut app = assistant_app();
    let abs = app.docs[0].path.clone();
    let old = app.docs[0].text.clone();
    for (id, new) in [("1", "\"One\""), ("2", "\"Two\"")] {
        let change = formust_ai::Change::Edit {
            old_text: "\"Formust\"".into(),
            new_text: new.into(),
        };
        let p = formust_ai::Proposal {
            id: id.into(),
            path: "debug.rs".into(),
            abs_path: abs.clone(),
            change: change.clone(),
        };
        app.assistant.items.push(Item::Card(Card {
            id: id.into(),
            path: "debug.rs".into(),
            abs: abs.clone(),
            preview: preview_of(&p, Some(old.clone())),
            change,
            state: CardState::Pending,
            auto: false,
            undo: None,
            local: true,
        }));
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1368.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(2);
    // Two cards, two Dismiss buttons: the first one is for card "1"
    harness.get_all_by_label("Dismiss").next().unwrap().click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().docs[0].text,
        old,
        "dismissing changes nothing"
    );
    harness.get_by_label("Apply").click();
    harness.run_steps(3);
    let a = &harness.state();
    assert!(
        a.docs[0].text.contains("let name = \"Two\";"),
        "{}",
        a.docs[0].text
    );
    assert!(matches!(&a.assistant.items[0], Item::Card(c) if c.state == CardState::Rejected));
    assert!(matches!(&a.assistant.items[1], Item::Card(c) if c.state == CardState::Applied));
}

// ---- the Claude Code (subscription) provider ----------------------------------------------------

fn cli_app(cli: assistant::CliState) -> App {
    let mut app = editing_app();
    app.assistant.open = true;
    app.assistant.provider = assistant::Provider::ClaudeCode;
    app.assistant.cli = cli;
    app
}

fn cli_found(logged_in: Option<bool>) -> assistant::CliState {
    assistant::CliState::Found(formust_ai::claude_code::CliStatus {
        binary: "/Users/me/.local/bin/claude".into(),
        version: Some("2.1.312".into()),
        logged_in,
    })
}

#[test]
fn renders_the_claude_code_status_cards() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    for (cli, name) in [
        (assistant::CliState::Missing, "ui-assistant-cli-missing"),
        (cli_found(Some(false)), "ui-assistant-cli-login"),
        (assistant::CliState::Checking, "ui-assistant-cli-checking"),
    ] {
        let p = render(cli_app(cli), false, (1368.0, 800.0), name);
        println!("{}", p.display());
    }
    let p = render(
        cli_app(assistant::CliState::Missing),
        true,
        (1368.0, 800.0),
        "ui-assistant-cli-missing-dark",
    );
    println!("{}", p.display());
}

#[test]
fn renders_claude_code_settings_and_chat() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = cli_app(cli_found(Some(true)));
    app.assistant.show_settings = true;
    app.assistant.items = chat_items();
    let p = render(app, false, (1368.0, 1300.0), "ui-assistant-cli-settings");
    println!("{}", p.display());
    // the API provider's settings with the switch
    let mut app = assistant_app();
    app.assistant.show_settings = true;
    let p = render(app, false, (1368.0, 800.0), "ui-assistant-api-settings");
    println!("{}", p.display());
    let mut app = cli_app(cli_found(Some(true)));
    app.assistant.items = chat_items();
    let p = render(app, true, (1368.0, 1300.0), "ui-assistant-cli-chat-dark");
    println!("{}", p.display());
}

/// Clicking the provider switch in the real panel.
#[test]
fn the_provider_switch_changes_the_card() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let _keychain = assistant::KEYCHAIN
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.assistant.open = true;
    app.assistant.key = assistant::KeyState::Missing;
    // The CLI check is not started here: a state with a result already.
    app.assistant.cli = assistant::CliState::Missing;
    app.assistant.show_settings = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1368.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(2);
    assert_eq!(harness.state().assistant.provider, assistant::Provider::Api);
    harness.get_by_label("Claude Code CLI").click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().assistant.provider,
        assistant::Provider::ClaudeCode
    );
    // the CLI card is up: it says so and the key field is gone
    assert!(harness.query_by_label("Check again").is_some());
    assert!(harness.query_by_label("API key").is_none());
    harness.get_by_label("Claude Console API").click();
    harness.run_steps(3);
    assert_eq!(harness.state().assistant.provider, assistant::Provider::Api);
    assert!(harness.query_by_label("API key").is_some());
}

// ---- the OpenAI provider ------------------------------------------------------------------

fn openai_app() -> App {
    let mut app = editing_app();
    app.assistant.open = true;
    app.assistant.provider = assistant::Provider::OpenAi;
    app.assistant.openai_key_state = assistant::KeyState::Missing; // no Keychain lookup
    app
}

fn openai_chat_app() -> App {
    let mut app = openai_app();
    app.assistant.use_openai_key("not-a-real-key");
    app.assistant.openai.model = "my-model".into();
    app.assistant.usage = formust_ai::Usage {
        input_tokens: 2100,
        output_tokens: 380,
        cache_read_input_tokens: 1800,
        ..Default::default()
    };
    app
}

#[test]
fn renders_the_openai_key_card() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let p = render(
        openai_app(),
        false,
        (1368.0, 900.0),
        "ui-assistant-openai-key",
    );
    println!("{}", p.display());
    // A local server typed in, the message after a rejected key, dark mode
    let mut app = openai_app();
    app.assistant.openai_edit.base_url = "http://localhost:11434/v1".into();
    app.assistant.openai_edit.model = "llama3.1".into();
    app.assistant.key_message = Some(
        "The API key was rejected (Incorrect API key provided). Check the key in Settings.".into(),
    );
    let p = render(
        app,
        true,
        (1368.0, 900.0),
        "ui-assistant-openai-key-local-dark",
    );
    println!("{}", p.display());
}

#[test]
fn renders_the_openai_chat_and_settings() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = openai_chat_app();
    app.assistant.items = chat_items();
    let p = render(app, false, (1368.0, 1300.0), "ui-assistant-openai-chat");
    println!("{}", p.display());
    let mut app = openai_chat_app();
    app.assistant.items = chat_items();
    let p = render(app, true, (1368.0, 1300.0), "ui-assistant-openai-chat-dark");
    println!("{}", p.display());
    let mut app = openai_chat_app();
    app.assistant.show_settings = true;
    app.assistant.openai_edit = app.assistant.openai.clone();
    let p = render(app, false, (1368.0, 1000.0), "ui-assistant-openai-settings");
    println!("{}", p.display());
    // no model yet: the field shows its hint
    let mut app = openai_chat_app();
    app.assistant.openai.model.clear();
    let p = render(app, false, (1368.0, 800.0), "ui-assistant-openai-nomodel");
    println!("{}", p.display());
}

/// Clicking through the OpenAI card: choose the provider, point at a local server, save.
#[test]
fn the_openai_card_has_only_the_key() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.assistant.open = true;
    app.assistant.key = assistant::KeyState::Missing;
    app.assistant.openai_key_state = assistant::KeyState::Missing; // never read the real Keychain
    app.assistant.show_settings = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1368.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(2);
    harness.get_by_label("OpenAI API").click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().assistant.provider,
        assistant::Provider::OpenAi
    );
    assert!(!harness.state().assistant.ready(), "the key is needed");
    // The endpoint is fixed and the model is picked under the message box: no other fields here.
    assert!(harness.query_by_label("OpenAI API key").is_some());
    assert!(harness.query_by_label("OpenAI base URL").is_none());
    assert!(harness.query_by_label("OpenAI model name").is_none());
    assert!(harness.query_by_label("Server address").is_none());
}

// ---- "Allow edits this session" and "save first" ----------------------------------------------

/// A card for an edit of the first open tab.
fn edit_card(
    app: &App,
    id: &str,
    old: &str,
    new: &str,
    state: assistant::CardState,
) -> assistant::Card {
    let abs = app.docs[0].path.clone();
    let change = formust_ai::Change::Edit {
        old_text: old.into(),
        new_text: new.into(),
    };
    let p = formust_ai::Proposal {
        id: id.into(),
        path: "debug.rs".into(),
        abs_path: abs.clone(),
        change: change.clone(),
    };
    let preview = assistant::preview_of(&p, Some(app.docs[0].text.clone()));
    assert!(preview.is_ok(), "{preview:?}");
    assistant::Card {
        id: id.into(),
        path: "debug.rs".into(),
        abs,
        change,
        state,
        local: false,
        preview,
        auto: false,
        undo: None,
    }
}

#[test]
fn renders_auto_mode_with_applied_cards_and_revert() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use assistant::*;
    let mut app = assistant_app();
    app.assistant.set_auto_edits(true);
    app.assistant.items = chat_items()[..2].to_vec();
    app.assistant.items.push(Item::Assistant(
        "I fixed the loop and added a counter.".into(),
    ));
    // 1: applied, the file still has the text the edit left: Revert is on
    let mut c1 = edit_card(
        &app,
        "e1",
        "    total = total + i;",
        "    {\n      total = total + i;\n      total += 1;\n    }",
        CardState::Applied,
    );
    c1.auto = true;
    c1.undo = Some(Box::new(Undo {
        before: Some(app.docs[0].text.clone()),
        after: app.docs[0].text.clone(),
        unit: None,
        on_disk: false,
    }));
    // 2: applied, but the file changed since: Revert is off
    let mut c2 = edit_card(&app, "e2", "total = 0;", "total = 1;", CardState::Applied);
    c2.auto = true;
    c2.undo = Some(Box::new(Undo {
        before: Some(app.docs[0].text.clone()),
        after: "something else".into(),
        unit: None,
        on_disk: false,
    }));
    // 3: taken back
    let mut c3 = edit_card(&app, "e3", "total = 0;", "total = 2;", CardState::Reverted);
    c3.auto = true;
    app.assistant.items.push(Item::Card(c1));
    app.assistant.items.push(Item::Card(c2));
    app.assistant.items.push(Item::Card(c3));
    let p = render(app, false, (1368.0, 1300.0), "ui-assistant-auto-on");
    println!("{}", p.display());
    // dark mode, second time on (no warning line any more)
    let mut app = assistant_app();
    app.assistant.set_auto_edits(true);
    app.assistant.set_auto_edits(false);
    app.assistant.set_auto_edits(true);
    app.assistant.items = chat_items()[..2].to_vec();
    let p = render(app, true, (1368.0, 800.0), "ui-assistant-auto-on-dark");
    println!("{}", p.display());
}

#[test]
fn renders_the_save_first_card_for_claude_code() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use assistant::*;
    let mut app = cli_app(cli_found(Some(true)));
    app.assistant.backend_writes = true;
    app.assistant.items = chat_items()[..2].to_vec();
    app.docs[0].text.push_str("// typed, not saved\n");
    let c = edit_card(
        &app,
        "e1",
        "    total = total + i;",
        "    {\n      total = total + i;\n      total += 1;\n    }",
        CardState::Pending,
    );
    app.assistant.items.push(Item::Card(c));
    let p = render(app, false, (1368.0, 1000.0), "ui-assistant-save-first");
    println!("{}", p.display());
}

/// The switch in the real panel: a click turns it on (badge, warning), a click on the badge
/// turns it off. A card for a dirty file offers "Save and apply". Works on a copy: the
/// fixture itself is never written.
#[test]
fn the_edit_switch_and_the_save_first_button_work_with_clicks() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let dir = crate::target_tmp().join("render-save-first");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(fixture(), dir.join("debug.rs")).unwrap();
    let mut app = App::bare(Some(dir.join("debug.rs")));
    app.assistant.open = true;
    app.assistant.provider = assistant::Provider::ClaudeCode;
    app.assistant.cli = cli_found(Some(true));
    app.assistant.backend_writes = true;
    app.docs[0].text.push_str("// typed, not saved\n");
    let c = edit_card(
        &app,
        "e1",
        "    total = total + i;",
        "    total += 1;",
        assistant::CardState::Pending,
    );
    app.assistant.items.push(assistant::Item::Card(c));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1368.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(2);
    assert!(!harness.state().assistant.auto_edits);
    harness.get_by_label("Allow edits this session").click();
    harness.run_steps(3);
    assert!(harness.state().assistant.auto_edits);
    harness.get_by_label("Allow edits this session").click();
    harness.run_steps(3);
    assert!(!harness.state().assistant.auto_edits);
    // the dirty file: "Save and apply" saves the tab, then answers
    assert!(harness.query_by_label("Apply").is_none());
    harness.get_by_label("Save and apply").click();
    harness.run_steps(3);
    let st = harness.state();
    assert!(!st.docs[0].dirty(), "the tab was saved");
    assert!(
        std::fs::read_to_string(dir.join("debug.rs"))
            .unwrap()
            .contains("typed, not saved")
    );
    assert!(matches!(st.assistant.items.last(),
        Some(assistant::Item::Card(c)) if c.state == assistant::CardState::Writing));
}

/// Typing into the terminal tab reaches a real shell and nothing else: Tab stays in the
/// terminal, Ctrl+C interrupts, Cmd+K clears, and the editor's text is untouched.
/// Dragging a file row onto a folder row moves the file, through the real pointer events.
#[test]
fn dragging_a_file_onto_a_folder_moves_it() {
    let base = std::env::temp_dir().join(format!("formust-drag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sub")).unwrap();
    std::fs::write(base.join("debug.rs"), "fn main() {}\n").unwrap();
    std::fs::write(base.join("notes.txt"), "hello\n").unwrap();
    // notes.txt is the open tab, so the move has to carry the tab along as well.
    let app = App::bare(Some(base.join("notes.txt")));
    let root = app.root.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1368.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    h.run_steps(3);
    let from = h.get_by_label("notes.txt").rect().center();
    let to = h.get_by_label("sub").rect().center();
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(2);
    }
    h.drop_at(to);
    h.run_steps(4);
    let moved = root.join("sub").join("notes.txt").exists();
    let notice = h.state().notice().map(|(m, _)| m.to_string());
    let tab = h.state().docs[0].path.to_string_lossy().into_owned();
    let banners = h.state().external.banners.len();
    let _ = std::fs::remove_dir_all(&base);
    assert!(moved, "the file should move into sub; note: {notice:?}");
    assert_eq!(
        tab,
        root.join("sub").join("notes.txt").to_string_lossy(),
        "the open tab follows the file"
    );
    assert_eq!(banners, 0, "no read error for the tab");
}

/// The Move to Trash prompt: Cancel and Move to Trash are the same height, Cancel on the left, Trash red.
#[test]
fn renders_the_move_to_trash_prompt() {
    let mut app = editing_app();
    app.tree_delete = Some(fixture());
    let path = render(app, false, (1368.0, 800.0), "ui-delete-prompt");
    assert!(path.exists());
}

/// Edit > Set Bookmark takes the next free number; Previous and Next walk the bookmarks and wrap around.
#[test]
fn the_edit_menu_sets_and_steps_through_bookmarks() {
    let mut app = editing_app();
    let ctx = egui::Context::default();
    caret_at_line_start(&ctx, &app, 3);
    app.run_menu_list(&ctx, vec![MenuAction::SetBookmark]);
    caret_at_line_start(&ctx, &app, 8);
    app.run_menu_list(&ctx, vec![MenuAction::SetBookmark]);
    assert_eq!(
        app.bookmarks.get(0).map(|(_, l)| l),
        Some(3),
        "the first free number"
    );
    assert_eq!(
        app.bookmarks.get(1).map(|(_, l)| l),
        Some(8),
        "the next free number"
    );

    let line = |app: &App, ctx: &egui::Context| {
        let index = app.cursor_index(ctx).expect("a caret");
        line_col(&app.docs[0].text, index).0 + 1
    };
    caret_at_line_start(&ctx, &app, 5);
    app.run_menu_list(&ctx, vec![MenuAction::NextBookmark]);
    assert_eq!(
        line(&app, &ctx),
        8,
        "next goes to the bookmark after the caret"
    );
    app.run_menu_list(&ctx, vec![MenuAction::NextBookmark]);
    assert_eq!(line(&app, &ctx), 3, "next wraps to the first");
    app.run_menu_list(&ctx, vec![MenuAction::PreviousBookmark]);
    assert_eq!(line(&app, &ctx), 8, "previous wraps to the last");
    app.run_menu_list(&ctx, vec![MenuAction::PreviousBookmark]);
    assert_eq!(line(&app, &ctx), 3, "previous goes back");

    // Unset clears the bookmark on the caret's line; a line without one is left alone.
    caret_at_line_start(&ctx, &app, 2);
    app.run_menu_list(&ctx, vec![MenuAction::UnsetBookmark]);
    assert_eq!(
        app.bookmarks.get(0).map(|(_, l)| l),
        Some(3),
        "no bookmark on line 2: nothing changes"
    );
    caret_at_line_start(&ctx, &app, 3);
    app.run_menu_list(&ctx, vec![MenuAction::UnsetBookmark]);
    assert_eq!(app.bookmarks.get(0), None, "line 3's bookmark is removed");
    assert_eq!(
        app.bookmarks.get(1).map(|(_, l)| l),
        Some(8),
        "the others stay"
    );
}

/// Clicking a bookmark's green chip in the gutter unsets it; clicking the line's number leaves it.
#[test]
fn clicking_a_bookmark_chip_unsets_it() {
    let mut app = editing_app();
    let path = app.docs[0].path.clone();
    app.bookmarks.toggle(2, &path, 8);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1368.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    h.run_steps(3);
    let gutter = h.get_by_label("Line numbers").rect();
    // Line 8 is the eighth row of 18 points (the default line height).
    let row_y = gutter.top() + 7.0 * 18.0 + 9.0;
    let click = |h: &mut Harness<'_, App>, p: egui::Pos2| {
        h.hover_at(p);
        h.run_steps(2);
        h.drag_at(p);
        h.run_steps(2);
        h.drop_at(p);
        h.run_steps(3);
    };
    // Right of the chip, on the number: nothing happens to the bookmark.
    click(&mut h, egui::pos2(gutter.right() - 12.0, row_y));
    assert_eq!(
        h.state().bookmarks.get(2).map(|(_, l)| l),
        Some(8),
        "clicking the number leaves it"
    );
    // On the chip: it is unset.
    click(&mut h, egui::pos2(gutter.left() + 6.0, row_y));
    assert_eq!(
        h.state().bookmarks.get(2),
        None,
        "clicking the chip unsets the bookmark"
    );
}

/// Ctrl+Shift+3 sets bookmark 3 at the caret's line; Ctrl+3 from elsewhere jumps back to it.
#[test]
fn bookmarks_are_set_from_the_keyboard_and_jumped_to() {
    use egui::{Event, Key, Modifiers};
    let mut h = Harness::builder()
        .with_size(egui::vec2(1368.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), editing_app());
    h.run_steps(3);
    let key = |h: &Harness<'_, App>, k: Key, m: Modifiers| {
        h.event_modifiers(
            Event::Key {
                key: k,
                physical_key: Some(k),
                pressed: true,
                repeat: false,
                modifiers: m,
            },
            m,
        );
    };
    let path = h.state().docs[0].path.clone();
    caret_at_line_start(&h.ctx, h.state(), 8);
    h.run_steps(2);
    key(&h, Key::Num3, Modifiers::CTRL | Modifiers::SHIFT);
    h.run_steps(2);
    assert_eq!(
        h.state().bookmarks.get(3),
        Some((path.as_path(), 8)),
        "set on the caret's line"
    );

    // Move the caret away, then jump back with Ctrl+3.
    caret_at_line_start(&h.ctx, h.state(), 2);
    h.run_steps(2);
    key(&h, Key::Num3, Modifiers::CTRL);
    h.run_steps(3);
    let index = h.state().cursor_index(&h.ctx).expect("a caret");
    let (line, _) = line_col(&h.state().docs[0].text, index);
    assert_eq!(line + 1, 8, "the caret moved to the bookmark's line");

    let image = h.render().expect("offscreen render");
    image.save(out_dir().join("ui-bookmarks.png")).unwrap();
}

/// A TOML file in the editor is coloured as configuration (keys, strings, comments), not as Rust.
#[test]
fn renders_a_toml_file_with_its_own_highlighting() {
    let toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let app = App::bare(Some(toml));
    let path = render(app, false, (1368.0, 800.0), "ui-highlight-toml");
    assert!(path.exists());
}

/// Dropping an item onto a file row (between the files of a folder) moves it into that file's folder.
#[test]
fn dropping_onto_a_file_moves_it_into_that_files_folder() {
    let base = std::env::temp_dir().join(format!("formust-drop-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sub")).unwrap();
    std::fs::write(base.join("debug.rs"), "fn main() {}\n").unwrap();
    std::fs::write(base.join("notes.txt"), "hello\n").unwrap();
    std::fs::write(base.join("sub").join("inner.txt"), "inner\n").unwrap();
    let app = App::bare(Some(base.join("debug.rs")));
    let root = app.root.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1368.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    h.run_steps(3);
    // Expand sub, so its files are rows.
    h.get_by_label("sub").click();
    h.run_steps(3);
    let from = h.get_by_label("notes.txt").rect().center();
    let to = h.get_by_label("inner.txt").rect().center();
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(2);
    }
    // Still dragging, over inner.txt: the frame shows where the item would land.
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-drop-file-hover.png"))
        .unwrap();
    h.drop_at(to);
    h.run_steps(4);
    let moved = root.join("sub").join("notes.txt").exists();
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        moved,
        "dropping onto inner.txt should move notes.txt into sub"
    );
}

/// Puts the editor's caret at the start of 1-based `line` (what a click there leaves behind).
fn caret_at_line_start(ctx: &egui::Context, app: &App, line: usize) {
    let doc = &app.docs[0];
    let at: usize = doc
        .text
        .split_inclusive('\n')
        .take(line - 1)
        .map(|l| l.chars().count())
        .sum();
    let mut state = egui::TextEdit::load_state(ctx, doc.editor_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(at),
        )));
    state.store(ctx, doc.editor_id);
}

#[test]
fn the_terminal_tab_takes_the_keyboard() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use egui::{Event, Key, Modifiers};
    let mut app = editing_app();
    app.bottom.tab = bottom::BottomTab::Terminal;
    let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
    cmd.env("PS1", "$ ");
    app.bottom.terminal =
        Some(crate::terminal::Terminal::spawn(cmd, 10, 80, || {}).expect("spawn"));
    let before = app.docs[0].text.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    h.run_steps(3);
    assert!(
        h.state().bottom.terminal_focused,
        "the terminal asks for focus when shown"
    );

    let key = |h: &Harness<'_, App>, k: Key, m: Modifiers| {
        h.event_modifiers(
            Event::Key {
                key: k,
                physical_key: Some(k),
                pressed: true,
                repeat: false,
                modifiers: m,
            },
            m,
        );
    };
    let screen = |h: &Harness<'_, App>| h.state().bottom.terminal.as_ref().unwrap().screen_text();
    let wait = |h: &mut Harness<'_, App>, what: &str| {
        for _ in 0..200 {
            h.run_steps(1);
            if screen(h).contains(what) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("`{what}` never appeared:\n{}", screen(h));
    };

    h.event(Event::Text("echo hello-$((6*7))".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    wait(&mut h, "hello-42");

    // Tab is sent to the shell (it completes a name) and does not move the focus away.
    h.event(Event::Text("ech".into()));
    key(&h, Key::Tab, Modifiers::NONE);
    h.run_steps(3);
    assert!(h.state().bottom.terminal_focused);

    // Ctrl+C interrupts a running command.
    key(&h, Key::C, Modifiers::CTRL);
    h.event(Event::Text("sleep 30".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run_steps(3);
    std::thread::sleep(std::time::Duration::from_millis(300));
    key(&h, Key::C, Modifiers::CTRL);
    h.event(Event::Text("echo after-$((1+1))".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    wait(&mut h, "after-2");

    // Cmd+K clears the screen.
    key(&h, Key::K, Modifiers::COMMAND);
    h.run_steps(3);
    std::thread::sleep(std::time::Duration::from_millis(200));
    h.run_steps(2);
    assert!(!screen(&h).contains("hello-42"), "{}", screen(&h));

    assert!(h.state().bottom.terminal_focused);
    assert_eq!(h.state().docs[0].text, before, "the editor saw keystrokes");
}

#[test]
fn the_bottom_pane_switches_by_itself_unless_the_user_just_chose() {
    use bottom::BottomTab::*;
    let ctx = egui::Context::default();
    let mut app = editing_app();
    app.auto_tab(&ctx, Output);
    assert_eq!(app.bottom.tab, Output);
    app.auto_tab(&ctx, Terminal);
    assert_eq!(app.bottom.tab, Terminal);
    app.bottom.choose(Problems, ctx.input(|i| i.time));
    app.auto_tab(&ctx, Output);
    assert_eq!(
        app.bottom.tab, Problems,
        "the user's click wins for a moment"
    );
}

#[test]
fn renders_the_context_chips_and_what_was_sent() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use assistant::*;
    // A scratch copy of the notes example; the opened unit has an unsaved edit and a second
    // tab is open, so every chip shows.
    let dir = assistant::overview::tests::notes_copy("render-context");
    let mut app = App::bare(Some(dir.join("src/main.rs")));
    app.open_path(dir.join("src/about.rs"));
    app.active = 0;
    app.docs[1].text.push_str("// unsaved edit\n");
    app.assistant.open = true;
    app.assistant.use_key("not-a-real-key");
    app.assistant
        .project
        .set_index(assistant::overview::Index::scan(&app.root));
    app.assistant.chips.tabs = true;
    let ctx = egui::Context::default();
    let snap = app.assistant_snapshot(&ctx);
    let (sent, chips) = compose(
        app.assistant.chips,
        &snap,
        "Which units use the settings form?",
    );
    app.assistant.items = vec![
        Item::User {
            text: "Which units use the settings form?".into(),
            sent,
            chips,
        },
        Item::Assistant("Let me read `src/settings.rs` and search for its uses.".into()),
    ];
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1368.0, 1000.0))
        .with_pixels_per_point(1.0)
        .with_theme(egui::Theme::Light)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(3);
    harness.get_by_label("What was sent").click();
    harness.run_steps(3);
    let path = out_dir().join("ui-assistant-context.png");
    harness
        .render()
        .expect("offscreen render")
        .save(&path)
        .unwrap();
    println!("{}", path.display());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opening a file from another folder moves the terminal there: a shell nobody has typed in is
/// started again in the new folder; one in use stays and offers a `cd` button.
#[test]
fn the_terminal_follows_the_project_folder() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let a = crate::target_tmp().join("term-follow-a");
    let b = crate::target_tmp().join("term-follow-b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (a, b) = (a.canonicalize().unwrap(), b.canonicalize().unwrap());
    let make = |app: App| {
        Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .with_pixels_per_point(1.0)
            .with_context_setup(theme::apply)
            .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app)
    };

    // Untouched shell: restarted in the new folder.
    let mut app = editing_app();
    app.bottom.tab = bottom::BottomTab::Terminal;
    app.root = a.clone();
    let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
    cmd.cwd(&a);
    app.bottom.terminal =
        Some(crate::terminal::Terminal::spawn(cmd, 10, 80, || {}).expect("spawn"));
    app.bottom.terminal_root = Some(a.clone());
    app.root = b.clone();
    let mut h = make(app);
    h.run_steps(3);
    assert_eq!(h.state().bottom.terminal_root.as_deref(), Some(b.as_path()));
    assert!(h.state().bottom.terminal.is_some());
    drop(h);

    // A shell in use: left alone, with a button that sends `cd`.
    let mut app = editing_app();
    app.bottom.tab = bottom::BottomTab::Terminal;
    let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
    cmd.env("PS1", "$ ");
    cmd.cwd(&a);
    let term = crate::terminal::Terminal::spawn(cmd, 10, 80, || {}).expect("spawn");
    term.write(b"echo busy\n");
    assert!(term.typed());
    app.bottom.terminal = Some(term);
    app.bottom.terminal_root = Some(a.clone());
    app.root = b.clone();
    let mut h = make(app);
    h.run_steps(3);
    assert_eq!(h.state().bottom.terminal_root.as_deref(), Some(a.as_path()));
    h.get_by_label("cd to project folder").click();
    h.run_steps(3);
    assert_eq!(h.state().bottom.terminal_root.as_deref(), Some(b.as_path()));
    let want = b.display().to_string();
    for _ in 0..200 {
        h.run_steps(1);
        if h.state()
            .bottom
            .terminal
            .as_ref()
            .unwrap()
            .screen_text()
            .contains(&want)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!(
        "the cd never showed:\n{}",
        h.state().bottom.terminal.as_ref().unwrap().screen_text()
    );
}

#[test]
fn the_terminal_wheel_scrolls_the_history() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use egui::{Event, MouseWheelUnit};
    let mut app = editing_app();
    app.bottom.tab = bottom::BottomTab::Terminal;
    let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
    cmd.env("PS1", "$ ");
    let term = crate::terminal::Terminal::spawn(cmd, 10, 80, || {}).expect("spawn");
    term.write(b"seq 1 300\n");
    app.bottom.terminal = Some(term);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    for _ in 0..200 {
        h.run_steps(1);
        let t = h.state().bottom.terminal.as_ref().unwrap();
        if t.screen_text().contains("300") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let scrollback = |h: &Harness<'_, App>| {
        h.state()
            .bottom
            .terminal
            .as_ref()
            .unwrap()
            .parser()
            .screen()
            .scrollback()
    };
    assert_eq!(scrollback(&h), 0);
    // The pointer over the terminal pane, then the wheel (positive y = towards older output).
    h.hover_at(egui::pos2(400.0, 480.0));
    h.run_steps(1);
    h.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 120.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(scrollback(&h) > 0, "the wheel should scroll the history");
    h.run_steps(2);
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-terminal-scrollbar.png"))
        .unwrap();
}

// ---- unsaved changes ---------------------------------------------------------------------------

#[test]
fn a_changed_tab_is_red_and_closing_it_asks() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    for dark in [false, true] {
        let mut app = editing_app();
        app.docs[0].text.push_str("// typed, not saved\n");
        render(
            app,
            dark,
            (900.0, 560.0),
            &format!("ui-tab-changed{}", if dark { "-dark" } else { "" }),
        );
    }
    let mut app = editing_app();
    app.docs[0].text.push_str("// typed, not saved\n");
    app.request_close(0);
    render(app, false, (900.0, 560.0), "ui-close-asks");

    // Don't Save closes the tab and leaves the file alone; Cancel keeps the tab
    let on_disk = std::fs::read_to_string(fixture()).unwrap();
    let mut app = editing_app();
    app.docs[0].text.push_str("// typed, not saved\n");
    app.request_close(0);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 560.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(2);
    harness.get_by_label("Cancel").click();
    harness.run_steps(2);
    assert_eq!(harness.state().docs.len(), 1);
    assert!(harness.state().confirm.is_none());
    harness.state_mut().request_close(0);
    harness.run_steps(2);
    harness.get_by_label("Don't Save").click();
    harness.run_steps(2);
    assert!(harness.state().docs.is_empty());
    assert_eq!(std::fs::read_to_string(fixture()).unwrap(), on_disk);
}

/// A full-screen program that asked for mouse reports (Codex, vim, htop) gets the wheel as a
/// mouse report, not as arrow keys; a plain shell keeps the IDE's own scrolling.
#[test]
fn the_wheel_reaches_a_program_that_asked_for_the_mouse() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use egui::Event;
    let mut app = editing_app();
    app.bottom.tab = bottom::BottomTab::Terminal;
    let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
    cmd.args([
        "-c",
        "printf '\\033[?1049h\\033[?1000h\\033[?1006h'; stty -icanon -echo; cat -v",
    ]);
    app.bottom.terminal =
        Some(crate::terminal::Terminal::spawn(cmd, 10, 80, || {}).expect("spawn"));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    let screen = |h: &Harness<'_, App>| h.state().bottom.terminal.as_ref().unwrap().screen_text();
    for _ in 0..50 {
        h.run_steps(1);
        if h.state().bottom.terminal.as_ref().unwrap().wants_mouse() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(h.state().bottom.terminal.as_ref().unwrap().wants_mouse());
    // over the terminal, which fills the bottom pane
    let pos = egui::pos2(300.0, 520.0);
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 3.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    for _ in 0..100 {
        h.run_steps(1);
        if screen(&h).contains("^[[<64;") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("no wheel report arrived:\n{}", screen(&h));
}

/// The inline name field under the project, and the delete prompt above the file list.
#[test]
fn renders_the_project_tree_actions() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("formust-tree-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("debug.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("sub/inner.rs"), "mod inner;\n").unwrap();
    let mut app = App::bare(Some(root.join("debug.rs")));
    // The app's root is canonical (`/private/var` on macOS); the new row must match it.
    let canonical = app.root.clone();
    app.tree_new = Some(shell::TreeNew::new(canonical, false));
    render(app, false, (1368.0, 800.0), "ui-tree-new-file");

    let mut app = App::bare(Some(root.join("debug.rs")));
    app.tree_delete = Some(root.join("sub"));
    render(app, true, (1368.0, 800.0), "ui-tree-delete-dark");
    let _ = std::fs::remove_dir_all(&root);
}

/// The name field that replaces a row while it is renamed.
#[test]
fn renders_the_rename_field() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("formust-rename-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("debug.rs"), "fn main() {}\n").unwrap();
    let mut app = App::bare(Some(root.join("debug.rs")));
    let canonical = app.root.clone();
    app.tree_rename = Some(shell::TreeRename::new(
        canonical.join("debug.rs"),
        "debug.rs",
    ));
    render(app, false, (1368.0, 800.0), "ui-tree-rename");
    let _ = std::fs::remove_dir_all(&root);
}

/// Tab in the editor inserts spaces up to the next tab stop and keeps the focus there.
#[test]
fn tab_in_the_editor_inserts_spaces_to_the_next_stop() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    use egui::text::{CCursor, CCursorRange};
    use egui::{Event, Key, Modifiers};
    let mut app = editing_app();
    app.docs[0].text = "ab".into();
    let id = app.docs[0].editor_id;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 640.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    theme::apply(&harness.ctx);
    harness.run_steps(2);
    harness.ctx.memory_mut(|m| m.request_focus(id));
    let mut state = egui::TextEdit::load_state(&harness.ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(2))));
    state.store(&harness.ctx, id);
    harness.run_steps(2);
    assert!(
        harness.ctx.memory(|m| m.has_focus(id)),
        "the editor has the focus"
    );

    let tab = |h: &Harness<'_, App>| {
        h.event_modifiers(
            Event::Key {
                key: Key::Tab,
                physical_key: Some(Key::Tab),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            Modifiers::NONE,
        );
    };
    tab(&harness);
    harness.run_steps(3);
    // Column 2 with 4-wide tab stops: two spaces.
    assert_eq!(harness.state().docs[0].text, "ab  ");
    assert!(
        harness.ctx.memory(|m| m.has_focus(id)),
        "Tab keeps the focus in the editor"
    );
}

/// The OpenAI Compatible settings page for Ollama: the server's address, the optional key and the
/// privacy note.
#[test]
fn renders_the_ollama_settings() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.assistant.open = true;
    app.assistant.provider = assistant::Provider::Ollama;
    app.assistant.compat = formust_ai::openai::OpenAiSettings {
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: "qwen3:4b".into(),
    };
    app.assistant.compat_edit = app.assistant.compat.clone();
    app.assistant.compat_key_state = assistant::KeyState::Missing; // never read the real Keychain
    app.assistant.show_settings = true;
    let a = render(app, false, (1368.0, 900.0), "ui-assistant-ollama-settings");
    println!("{}", a.display());
}

#[test]
fn renders_the_configure_tools_window_and_add_adds_a_tool() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let tool = formust_build::tools::Tool {
        name: "Echo".into(),
        command: "/bin/echo".into(),
        args: "$(File)".into(),
        workdir: String::new(),
    };
    let mut app = editing_app();
    app.tools_editor = Some(crate::tools_editor::Editor::new(vec![tool.clone()]));
    let png = render(app, false, (1100.0, 640.0), "ui-tools-editor");
    assert!(png.exists());

    let mut app = editing_app();
    app.tools_editor = Some(crate::tools_editor::Editor::new(vec![tool]));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 640.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(3);
    harness.get_by_label("Add").click();
    harness.run_steps(3);
    let editor = harness
        .state()
        .tools_editor
        .as_ref()
        .expect("the window stays open");
    assert_eq!(editor.tools.len(), 2, "Add adds a tool");
    assert_eq!(editor.selected, 1, "and selects it");
}

#[test]
fn the_terminal_pane_opens_a_new_shell_and_switches_between_shells() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = editing_app();
    app.bottom.tab = bottom::BottomTab::Terminal;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    h.run_steps(3);
    assert_eq!(h.state().bottom.shells(), [(1, true)], "the first shell");

    h.get_by_label("+ New Shell").click();
    h.run_steps(3);
    assert_eq!(
        h.state().bottom.shells(),
        [(1, false), (2, true)],
        "a second shell is shown"
    );
    assert!(h.state().bottom.terminal.is_some(), "and started");

    h.get_by_label("Shell 1").click();
    h.run_steps(3);
    assert_eq!(
        h.state().bottom.shells(),
        [(1, true), (2, false)],
        "back to the first"
    );
}

// ---- The form designer (phase 6: read-only) ----------------------------------------------------

use egui_kittest::kittest::NodeT as _;

/// Every first-palette widget and container, a custom block, and a second form of panels.
fn designer_sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/designer/sample.rs")
}

/// The IDE showing the sample's designer, with the navigator and the bottom pane hidden so the
/// whole form fits on the canvas. It works on a copy of the sample: the app's project is the copy's
/// folder, so a preview action that writes (New Rust Form, New File, Save) never touches the fixture.
fn designer_harness(dark: bool) -> Harness<'static, App> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    designer_harness_for(designer_copy(&format!("view-{n}")), dark)
}

fn designer_harness_for(path: PathBuf, dark: bool) -> Harness<'static, App> {
    let mut app = App::bare(Some(path));
    app.lsp_ready = true;
    app.show_navigator = false;
    app.show_bottom = false;
    app.show_form();
    assert!(
        app.form_showing(),
        "the sample's forms show in the designer"
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1300.0))
        .with_pixels_per_point(1.0)
        .with_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        })
        .with_context_setup(theme::apply)
        // The preview shows the user's own form, whose text fields need not have labels.
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(4);
    harness
}

/// The one node of the shown form whose outline title contains `needle`.
fn designer_node(app: &App, needle: &str) -> formust_model::NodeId {
    let (mut found, mut titles) = (Vec::new(), Vec::new());
    app.designer
        .current()
        .expect("a form is shown")
        .form()
        .root
        .walk(&mut |n| {
            let title = designer::node_title(n);
            if title.contains(needle) {
                found.push(n.id);
            }
            titles.push(title);
        });
    assert_eq!(
        found.len(),
        1,
        "one node titled like {needle:?} in {titles:?}"
    );
    found[0]
}

/// One frame, then the checks that hold on every frame: no preview widget reacted to anything and
/// none holds the keyboard focus.
fn designer_step(h: &mut Harness<'static, App>) {
    h.step();
    let d = &h.state().designer;
    assert!(
        d.reacted.is_empty(),
        "preview widgets reacted: {:?}",
        d.reacted
    );
    let layer = d.preview_layer.expect("the preview was drawn");
    if let Some(id) = h.ctx.memory(|m| m.focused()) {
        assert!(
            h.ctx.read_response(id).is_none_or(|r| r.layer_id != layer),
            "a preview widget has the focus"
        );
    }
}

fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn designer_click(h: &mut Harness<'static, App>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    designer_step(h);
    h.event(pointer_button(pos, true));
    designer_step(h);
    h.event(pointer_button(pos, false));
    designer_step(h);
}

fn designer_rect(h: &Harness<'static, App>, id: formust_model::NodeId) -> egui::Rect {
    *h.state()
        .designer
        .rects
        .get(&id)
        .unwrap_or_else(|| panic!("node {id:?} has no rect"))
}

/// The sample renders: every first-palette widget is in the form, and every one that isn't inside
/// a closed section or a grid's end of row has a place on the canvas. Light and dark PNGs.
#[test]
fn the_designer_renders_every_first_palette_widget() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    for dark in [false, true] {
        let mut h = designer_harness(dark);
        let app = h.state();
        let form = app.designer.current().unwrap().form().clone();
        assert_eq!(form.id(), "Sample::ui");
        let mut seen = std::collections::BTreeSet::new();
        let mut missing = Vec::new();
        form.root.walk(&mut |n| {
            if let formust_model::NodeKind::Widget { def, .. } = &n.kind {
                seen.insert(*def);
                let placeless = matches!(
                    def,
                    formust_model::WidgetId::AddSpace | formust_model::WidgetId::EndRow
                );
                let hidden = designer::node_title(n).contains("Hidden inside");
                if !placeless && !hidden && !app.designer.rects.contains_key(&n.id) {
                    missing.push(designer::node_title(n));
                }
            }
        });
        // Batch D's widgets are in their own form (`batch_d_widgets_preview_and_edit`).
        for w in formust_model::ALL_WIDGETS
            .iter()
            .filter(|w| !BATCH_D_WIDGETS.contains(w))
        {
            assert!(seen.contains(w), "the sample has no {w:?}");
        }
        assert!(missing.is_empty(), "drawn without a rect: {missing:?}");
        assert!(app.designer.rects.contains_key(&designer_node(
            app,
            "Collapsing header \"Advanced section\""
        )));
        // The real widgets are there, reachable by assistive technology like any egui widget.
        h.get_by_label("Remember me");
        h.get_by_label("Sign in");
        h.get_by_label_contains("Custom code (locked)");
        let image = h.render().expect("offscreen render");
        let path = out_dir().join(if dark {
            "ui-designer-dark.png"
        } else {
            "ui-designer-light.png"
        });
        image.save(&path).unwrap();
        println!("{}", path.display());
    }
}

/// A form of panels (`impl eframe::App`, `fn ui`) fills its window.
#[test]
fn the_designer_renders_a_form_of_panels() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness(false);
    assert_eq!(
        h.state().designer.forms().len(),
        2,
        "both forms of the file can be picked"
    );
    h.state_mut().designer.choose_form("Panels::ui".into());
    h.run_steps(3);
    let app = h.state();
    let panels: Vec<_> = app
        .designer
        .current()
        .unwrap()
        .form()
        .root
        .children()
        .iter()
        .map(|n| n.id)
        .collect();
    let (top, central) = (panels[0], panels[1]);
    assert_eq!(central, designer_node(app, "Central panel"));
    let (top, central) = (designer_rect(&h, top), designer_rect(&h, central));
    assert!(
        top.bottom() <= central.top() + 1.0,
        "the top panel is above the central one"
    );
    assert!(
        (top.width() - central.width()).abs() < 1.0,
        "both span the window"
    );
    h.get_by_label("Top panel heading");
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-panels.png"))
        .unwrap();
}

/// A click on a preview widget selects its node and does nothing else: the checkbox stays
/// unticked, the collapsing header closed, no combo box popup opens, a drag doesn't move the slider,
/// and no frame ever has a preview widget reacting or focused (`designer_step`).
#[test]
fn clicking_the_preview_selects_and_never_operates_a_widget() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness(false);
    let before = h.state().designer.rects.clone();
    let checkbox_state =
        |h: &Harness<'static, App>| h.get_by_label("Remember me").accesskit_node().toggled();
    let unticked = checkbox_state(&h);
    for needle in [
        "Checkbox \"Remember me\"",
        "Toggle \"Dark toggle\"",
        "Radio button \"Small radio\"",
        "Selectable value \"Large choice\"",
        "Button \"Sign in\"",
        "Link \"A link\"",
        "Hyperlink \"Egui site\"",
        "Slider \"Volume\"",
        "Drag value",
        "Combo box \"Size combo\"",
        "Collapsing header \"Advanced section\"",
        "Text edit &mut self.name",
        "Multiline text edit",
        "Label \"Name label\"",
    ] {
        let id = designer_node(h.state(), needle);
        let at = designer_rect(&h, id).center();
        designer_click(&mut h, at);
        assert_eq!(
            h.state().designer.selected,
            Some(id),
            "a click selects {needle}"
        );
        assert!(
            !egui::Popup::is_any_open(&h.ctx),
            "no popup opened for {needle}"
        );
    }
    // Drag across the slider: its value must not move (its rect and everything else stay put).
    let slider = designer_rect(&h, designer_node(h.state(), "Slider \"Volume\""));
    let (from, to) = (
        slider.left_center() + egui::vec2(10.0, 0.0),
        slider.right_center() - egui::vec2(10.0, 0.0),
    );
    h.event(egui::Event::PointerMoved(from));
    designer_step(&mut h);
    h.event(pointer_button(from, true));
    designer_step(&mut h);
    for i in 1..=5 {
        h.event(egui::Event::PointerMoved(
            from + (to - from) * (i as f32 / 5.0),
        ));
        designer_step(&mut h);
    }
    h.event(pointer_button(to, false));
    designer_step(&mut h);
    // The wheel over the canvas doesn't scroll the form's scroll area (it is inert).
    let scrolled = designer_rect(&h, designer_node(h.state(), "Scrolled label"));
    h.event(egui::Event::PointerMoved(scrolled.center()));
    // A trackpad swipe, then its momentum tail (what macOS sends after the fingers lift).
    for source in [
        egui::MouseWheelSource::Trackpad,
        egui::MouseWheelSource::Momentum,
        egui::MouseWheelSource::Wheel,
    ] {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -30.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source,
        });
        designer_step(&mut h);
    }
    h.run_steps(2);
    assert_eq!(checkbox_state(&h), unticked, "the checkbox wasn't ticked");
    let after = &h.state().designer.rects;
    let scroll_y = h.state().designer.scroll.y;
    let moved: Vec<_> = before
        .iter()
        .filter(|(id, r)| {
            after.get(id).is_none_or(|a| {
                (a.min.y + scroll_y - r.min.y).abs() > 0.5 || (a.min.x - r.min.x).abs() > 0.5
            })
        })
        .map(|(id, _)| *id)
        .collect();
    assert!(
        moved.is_empty(),
        "nodes moved or appeared/disappeared: {moved:?}"
    );
    assert!(
        !after.contains_key(&designer_node(h.state(), "Hidden inside")),
        "the collapsing header stayed closed"
    );
}

/// Tab, Shift+Tab, Space, Enter and typing never give a preview widget the focus or change it, and
/// Tab still reaches the IDE's own widgets.
#[test]
fn the_keyboard_never_reaches_a_preview_widget() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness(false);
    let id = designer_node(h.state(), "Text edit &mut self.name");
    let at = designer_rect(&h, id).center();
    designer_click(&mut h, at);
    let mut ide_focus = 0;
    for _round in 0..3 {
        for (key, mods) in [
            (egui::Key::Tab, egui::Modifiers::NONE),
            (egui::Key::Tab, egui::Modifiers::SHIFT),
            (egui::Key::Space, egui::Modifiers::NONE),
            (egui::Key::Enter, egui::Modifiers::NONE),
            (egui::Key::ArrowDown, egui::Modifiers::NONE),
        ] {
            h.key_press_modifiers(mods, key);
            designer_step(&mut h);
            h.event(egui::Event::Text("X".into()));
            designer_step(&mut h);
            ide_focus += usize::from(h.ctx.memory(|m| m.focused()).is_some());
        }
        for _ in 0..12 {
            h.key_press(egui::Key::Tab);
            designer_step(&mut h);
            ide_focus += usize::from(h.ctx.memory(|m| m.focused()).is_some());
        }
    }
    assert!(ide_focus > 0, "Tab still moves through the IDE's widgets");
    assert!(
        !h.state().docs.iter().any(|d| d.dirty()),
        "nothing typed reached the source"
    );
}

/// A custom block is a locked box showing its source text; it can be selected, and the property
/// list says why it is custom.
#[test]
fn a_custom_block_is_a_locked_placeholder() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness(false);
    let id = designer_node(h.state(), "Custom code: if self.busy");
    let placeholder = h.get_by_label_contains("Custom code (locked)").rect();
    assert!(designer_rect(&h, id).contains(placeholder.center()));
    let at = designer_rect(&h, id).center();
    designer_click(&mut h, at);
    assert_eq!(h.state().designer.selected, Some(id));
    h.run_steps(2);
    h.get_by_label("Why: if (not a widget event)");
    h.get_by_label("LOCKED CUSTOM CODE");
}

/// Picking a row in the outline selects that node on the canvas, and a click on the canvas selects
/// the row.
#[test]
fn the_outline_and_the_canvas_share_the_selection() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness(false);
    let checkbox = designer_node(h.state(), "Checkbox \"Remember me\"");
    h.get_by_role_and_label(egui::accesskit::Role::TreeItem, "Checkbox \"Remember me\"")
        .click();
    h.run_steps(3);
    assert_eq!(
        h.state().designer.selected,
        Some(checkbox),
        "the outline row selects the node"
    );
    let row_selected = |h: &Harness<'static, App>, label: &str| {
        // egui reports a selected tree item as toggled.
        h.get_by_role_and_label(egui::accesskit::Role::TreeItem, label)
            .accesskit_node()
            .toggled()
            == Some(egui::accesskit::Toggled::True)
    };
    assert!(row_selected(&h, "Checkbox \"Remember me\""));

    let button = designer_node(h.state(), "Button \"Sign in\"");
    let at = designer_rect(&h, button).center();
    designer_click(&mut h, at);
    h.run_steps(2);
    assert_eq!(h.state().designer.selected, Some(button));
    assert!(
        row_selected(&h, "Button \"Sign in\" · clicked"),
        "the canvas click selects the row"
    );
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-selected.png"))
        .unwrap();
    assert!(!row_selected(&h, "Checkbox \"Remember me\""));
    // The inspector shows the selected node: its text property (an editable field) and its handler.
    assert!(
        h.query_all_by_value("Sign in")
            .any(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput),
        "the Text field holds the button's text"
    );
    h.get_by_label("clicked");
    // Escape clears the selection.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().designer.selected, None);
}

/// The preview follows the IDE's mode until Light or Dark is picked; the form's background is
/// egui's own panel colour for that mode, as at runtime.
#[test]
fn the_preview_shows_light_and_dark() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let background = |h: &mut Harness<'static, App>| {
        let client = h.state().designer.client.expect("drawn");
        let image = h.render().expect("offscreen render");
        let p = client.right_bottom() - egui::vec2(3.0, 3.0);
        let px = image.get_pixel(p.x as u32, p.y as u32).0;
        egui::Color32::from_rgb(px[0], px[1], px[2])
    };
    let near = |a: egui::Color32, b: egui::Color32| {
        (a.r() as i32 - b.r() as i32).abs() <= 2
            && (a.g() as i32 - b.g() as i32).abs() <= 2
            && (a.b() as i32 - b.b() as i32).abs() <= 2
    };
    let (light, dark) = (
        egui::Visuals::light().panel_fill,
        egui::Visuals::dark().panel_fill,
    );
    let mut h = designer_harness(false);
    assert!(near(background(&mut h), light), "follows the light IDE");
    h.get_by_label("Dark").click();
    h.run_steps(3);
    assert_eq!(h.state().designer.dark, Some(true));
    assert!(near(background(&mut h), dark), "Dark picked");
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-light-ide-dark-form.png"))
        .unwrap();

    let mut h = designer_harness(true);
    assert!(near(background(&mut h), dark), "follows the dark IDE");
    h.get_by_label("Light").click();
    h.run_steps(3);
    assert!(near(background(&mut h), light), "Light picked");
}

/// AccessKit actions (what VoiceOver sends) bypass the pointer and the shield; the preview holds
/// them back for its pass. The control run lets them through and the widgets do react, which shows
/// the actions really arrive.
#[test]
fn accesskit_actions_never_reach_the_preview() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness(false);
    let hidden = designer_node(h.state(), "Hidden inside");
    for label in ["Sign in", "Remember me", "Advanced section", "Dark toggle"] {
        h.get_by_label(label).click_accesskit();
        designer_step(&mut h);
        designer_step(&mut h);
    }
    h.get_by_role_and_label(egui::accesskit::Role::Slider, "Volume")
        .increment_accesskit();
    designer_step(&mut h);
    designer_step(&mut h);
    // Focus is the one action egui handles before the pass; the focus guard takes it back.
    h.get_by_label("Sign in").focus();
    designer_step(&mut h);
    designer_step(&mut h);
    // The preview's fields only: the IDE's own (the palette filter) take text as usual.
    let client = h.state().designer.client.expect("drawn");
    for input in h.get_all_by_role(egui::accesskit::Role::TextInput) {
        if !client.contains(input.rect().center()) {
            continue;
        }
        input.focus();
        input.set_text_value_accesskit("typed by VoiceOver");
    }
    designer_step(&mut h);
    designer_step(&mut h);
    assert!(
        h.query_by_value("typed by VoiceOver").is_none(),
        "no text reached a preview field"
    );
    assert!(
        !h.state().designer.rects.contains_key(&hidden),
        "the collapsing header stayed closed"
    );
    assert_eq!(
        h.state().designer.selected,
        None,
        "actions don't select either"
    );

    // Control: without the stash the same action opens the header.
    h.state_mut().designer.open_accesskit = true;
    h.get_by_label("Advanced section").click_accesskit();
    h.run_steps(4);
    assert!(
        h.state().designer.rects.contains_key(&hidden),
        "without the stash the action opens the header"
    );
}

/// A dialog (`fn show(&mut self, ctx: &egui::Context)` showing a Window) is previewed with its
/// Window drawn in place on the canvas, its widgets inside it; a widget added goes into it.
#[test]
fn the_designer_previews_a_dialog_window_in_place() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let dir = crate::target_tmp().join("designer-dialog");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("about.rs");
    std::fs::write(
        &rs,
        "pub struct About { name: String, open: bool }\nimpl About {\n    pub fn show(&mut self, ctx: &egui::Context) {\n        let mut open = self.open;\n        egui::Window::new(\"About\").open(&mut open).show(ctx, |ui| {\n            ui.label(\"Hello dialog\");\n            ui.text_edit_singleline(&mut self.name);\n        });\n        self.open = open;\n    }\n}\n",
    )
    .unwrap();
    let mut h = designer_harness_for(rs, false);
    assert_eq!(h.state().designer.forms().len(), 1);
    let app = h.state();
    assert_eq!(app.designer.current().unwrap().form().id(), "About::show");
    let window = designer_node(app, "Window \"About\"");
    let label = designer_node(app, "Label \"Hello dialog\"");
    let (w, l) = (designer_rect(&h, window), designer_rect(&h, label));
    assert!(
        w.contains_rect(l),
        "the label is inside the window: {w:?} {l:?}"
    );
    let preview = h.get_by_label("Hello dialog").rect();
    assert!(w.contains_rect(preview), "a real label, drawn in place");
    designer_step(&mut h);

    let added = h
        .state_mut()
        .designer
        .add(
            designer::edit::Item::Widget(formust_model::WidgetId::Button),
            None,
        )
        .unwrap();
    h.run_steps(3);
    let form = shown_form(&h);
    assert_eq!(form.parent_of(added).unwrap().0, window);
    let b = designer_rect(&h, added);
    assert!(designer_rect(&h, window).contains_rect(b));
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-dialog.png"))
        .unwrap();
}

/// A dialog shown through a helper (`crate::style::dialog(ctx, "About", .., |ui| ..)`, as the app's own
/// About window does) is previewed like a Window: framed in place, titled by its first string
/// literal, its widgets inside it; a widget added goes into it.
#[test]
fn the_designer_previews_a_dialog_helper_call_in_place() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let dir = crate::target_tmp().join("designer-dialog-helper");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("about.rs");
    std::fs::write(
        &rs,
        "pub struct About { name: String }\nimpl About {\n    pub fn show(&mut self, ctx: &egui::Context) -> bool {\n        let mut done = false;\n        crate::style::dialog(ctx, \"Helper title\", None, 540.0, |ui| {\n            ui.label(\"Hello helper\");\n            ui.text_edit_singleline(&mut self.name);\n        });\n        done\n    }\n}\n",
    )
    .unwrap();
    let mut h = designer_harness_for(rs, false);
    assert_eq!(h.state().designer.forms().len(), 1);
    let app = h.state();
    assert_eq!(app.designer.current().unwrap().form().id(), "About::show");
    let dialog = designer_node(app, "Dialog crate::style::dialog(ctx, \"Helper title\"");
    let label = designer_node(app, "Label \"Hello helper\"");
    let (d, l) = (designer_rect(&h, dialog), designer_rect(&h, label));
    assert!(
        d.contains_rect(l),
        "the label is inside the dialog: {d:?} {l:?}"
    );
    let preview = h.get_by_label("Hello helper").rect();
    assert!(d.contains_rect(preview), "a real label, drawn in place");
    let title = h.get_by_label("Helper title").rect();
    assert!(d.contains_rect(title), "titled by its first string literal");
    designer_step(&mut h);

    let added = h
        .state_mut()
        .designer
        .add(
            designer::edit::Item::Widget(formust_model::WidgetId::Button),
            None,
        )
        .unwrap();
    h.run_steps(3);
    let form = shown_form(&h);
    assert_eq!(form.parent_of(added).unwrap().0, dialog);
    let b = designer_rect(&h, added);
    assert!(designer_rect(&h, dialog).contains_rect(b));
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-dialog-helper.png"))
        .unwrap();
}

// ---- The form designer, editing (phase 7) --------------------------------------------------------

/// The sample copied to a folder of its own, so a test can export next to it.
fn designer_copy(name: &str) -> PathBuf {
    let dir = crate::target_tmp().join("designer-edit").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("sample.rs");
    std::fs::copy(designer_sample(), &rs).unwrap();
    rs
}

/// The one node titled exactly `title`.
fn exact_node(app: &App, title: &str) -> formust_model::NodeId {
    let mut found = Vec::new();
    app.designer
        .current()
        .expect("a form is shown")
        .form()
        .root
        .walk(&mut |n| {
            if designer::node_title(n) == title {
                found.push(n.id);
            }
        });
    assert_eq!(found.len(), 1, "one node titled {title:?}");
    found[0]
}

fn shown_form(h: &Harness<'static, App>) -> formust_model::Form {
    h.state().designer.current().unwrap().form().clone()
}

fn child_titles(h: &Harness<'static, App>, parent: formust_model::NodeId) -> Vec<String> {
    shown_form(h)
        .find(parent)
        .unwrap()
        .children()
        .iter()
        .map(designer::node_title)
        .collect()
}

/// Press at `from`, move to `to` in steps, and stay pressed.
fn designer_drag(h: &mut Harness<'static, App>, from: egui::Pos2, to: egui::Pos2) {
    h.event(egui::Event::PointerMoved(from));
    designer_step(h);
    h.event(pointer_button(from, true));
    designer_step(h);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(
            from + (to - from) * (i as f32 / 8.0),
        ));
        designer_step(h);
    }
}

fn designer_release(h: &mut Harness<'static, App>, at: egui::Pos2) {
    h.event(pointer_button(at, false));
    designer_step(h);
    designer_step(h);
}

fn outline_row(h: &Harness<'static, App>, id: formust_model::NodeId) -> egui::Rect {
    h.state()
        .designer
        .rows
        .iter()
        .find(|r| r.id == id)
        .expect("an outline row")
        .rect
}

fn palette_button(h: &Harness<'static, App>, label: &str) -> egui::Rect {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label)
        .rect()
}

/// A click on the palette adds into the selected container (at its end) or after the selected
/// widget, and selects what it added; a stateful widget gets a field of the generated struct.
#[test]
fn a_palette_click_adds_into_the_selected_container_or_after_the_widget() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("palette-click"), false);
    let row = exact_node(h.state(), "Horizontal");
    h.get_by_role_and_label(egui::accesskit::Role::TreeItem, "Horizontal")
        .click();
    h.run_steps(2);
    // The filter brings an item into view in the scrolling palette.
    h.state_mut().designer.palette_filter = "separ".to_string();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Separator")
        .click();
    h.run_steps(3);
    assert_eq!(
        child_titles(&h, row),
        [
            "Label \"Name label\"",
            "Text edit &mut self.name",
            "Separator"
        ]
    );
    let added = shown_form(&h).find(row).unwrap().children()[2].id;
    assert_eq!(h.state().designer.selected, Some(added));
    assert!(
        h.state().designer.rects.contains_key(&added),
        "the new widget is drawn"
    );
    // With a widget selected, the next one goes after it.
    let name = exact_node(h.state(), "Label \"Name label\"");
    h.state_mut().designer.select(Some(name), false);
    h.state_mut().designer.palette_filter = "Checkbox".to_string();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Checkbox")
        .click();
    h.run_steps(3);
    assert_eq!(
        child_titles(&h, row)[1],
        "Checkbox \"Check\" &mut self.form.checked".replace(" &mut self.form.checked", "")
    );
    assert_eq!(
        shown_form(&h)
            .state
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["checked"]
    );
    assert!(h.state().is_dirty(0), "designer edits make the tab unsaved");
}

/// A drag from the palette shows the indicator line where the drop will land, and drops there.
#[test]
fn a_palette_drag_shows_the_indicator_and_drops_there() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("palette-drag"), false);
    let row = exact_node(h.state(), "Horizontal");
    let label = designer_rect(&h, exact_node(h.state(), "Label \"Name label\""));
    let from = palette_button(&h, "Checkbox").center();
    let to = label.center() + egui::vec2(label.width() / 4.0, 0.0);
    designer_drag(&mut h, from, to);
    let d = &h.state().designer;
    assert_eq!(
        d.drag,
        Some(designer::edit::Payload::New(designer::edit::Item::Widget(
            formust_model::WidgetId::Checkbox
        )))
    );
    let drop = d.drop.expect("an indicator over the canvas");
    assert_eq!(
        (drop.parent, drop.index),
        (row, 1),
        "between the label and the text edit"
    );
    assert!(
        drop.line[0].x > label.center().x && drop.line[0].x == drop.line[1].x,
        "a vertical line after the label"
    );
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-drop-indicator.png"))
        .unwrap();
    designer_release(&mut h, to);
    assert_eq!(
        child_titles(&h, row),
        [
            "Label \"Name label\"",
            "Checkbox \"Check\"",
            "Text edit &mut self.name"
        ]
    );
    assert_eq!(h.state().designer.drag, None);
    // Dropping outside any target adds nothing.
    let before = shown_form(&h);
    designer_drag(&mut h, from, from + egui::vec2(0.0, 30.0));
    designer_release(&mut h, from + egui::vec2(0.0, 30.0));
    assert_eq!(shown_form(&h).root, before.root);
}

/// Dragging a node on the canvas reorders it among its siblings; dragging outline rows reorders
/// and reparents.
#[test]
fn dragging_on_the_canvas_and_in_the_outline_reorders_and_reparents() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("reorder"), false);
    let wrapped = exact_node(h.state(), "Horizontal (wrapped)");
    let weak = exact_node(h.state(), "Weak text \"Weak text\"");
    let small = designer_rect(&h, exact_node(h.state(), "Small text \"Small text\""));
    let from = designer_rect(&h, weak).center();
    let to = small.left_center() + egui::vec2(small.width() / 4.0, 0.0);
    designer_drag(&mut h, from, to);
    assert_eq!(
        h.state().designer.drag,
        Some(designer::edit::Payload::Move(weak))
    );
    assert_eq!(
        h.state().designer.drop.map(|d| (d.parent, d.index)),
        Some((wrapped, 0))
    );
    designer_release(&mut h, to);
    assert_eq!(
        child_titles(&h, wrapped),
        [
            "Weak text \"Weak text\"",
            "Small text \"Small text\"",
            "Strong text \"Strong text\"",
            "Monospace text \"Mono text\""
        ]
    );
    assert_eq!(
        h.state().designer.selected,
        Some(weak),
        "the moved node is selected"
    );

    // In the outline: the checkbox row onto the top of the heading row moves it first.
    let root = shown_form(&h).root.id;
    let checkbox = exact_node(h.state(), "Checkbox \"Remember me\"");
    let heading = outline_row(&h, exact_node(h.state(), "Heading \"Sample heading\""));
    let from = outline_row(&h, checkbox).center();
    let to = egui::pos2(heading.center().x, heading.top() + 3.0);
    designer_drag(&mut h, from, to);
    assert_eq!(
        h.state().designer.drop.map(|d| (d.parent, d.index)),
        Some((root, 0))
    );
    designer_release(&mut h, to);
    assert_eq!(shown_form(&h).root.children()[0].id, checkbox);

    // The middle of a container's row: into it, at the end.
    let group = exact_node(h.state(), "Group");
    let link = exact_node(h.state(), "Link \"A link\"");
    let from = outline_row(&h, link).center();
    let to = outline_row(&h, group).center();
    designer_drag(&mut h, from, to);
    designer_release(&mut h, to);
    assert_eq!(shown_form(&h).parent_of(link), Some((group, 1)));
    // One undo step per move.
    for _ in 0..3 {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    }
    assert_eq!(child_titles(&h, wrapped)[0], "Small text \"Small text\"");
    assert!(
        !h.state().designer.is_dirty(),
        "three undos bring back the file as read"
    );
}

/// Typing in the inspector edits the property as one undo step; undo and redo keep the selection,
/// because node ids survive.
#[test]
fn the_inspector_edits_and_undo_redo_keep_the_selection() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("inspector"), false);
    let button = exact_node(h.state(), "Button \"Sign in\" · clicked");
    h.get_by_role_and_label(
        egui::accesskit::Role::TreeItem,
        "Button \"Sign in\" · clicked",
    )
    .click();
    h.run_steps(2);
    let field = |h: &Harness<'static, App>, value: &str| {
        h.query_all_by_value(value)
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
            .map(|n| n.value().unwrap())
    };
    let input = |h: &Harness<'static, App>, value: &str| {
        h.query_all_by_value(value)
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
            .unwrap()
            .focus()
    };
    input(&h, "Sign in");
    h.run_steps(2);
    h.event(egui::Event::Text("!".into()));
    h.run_steps(2);
    let typed = field(&h, "Sign in!")
        .or_else(|| field(&h, "!Sign in"))
        .expect("the text took the typing");
    assert!(!typed.is_empty());
    h.event(egui::Event::Text("?".into()));
    h.run_steps(2);
    let title = designer::node_title(shown_form(&h).find(button).unwrap());
    assert!(title.contains('!') && title.contains('?'), "{title}");
    assert_eq!(
        h.state().designer.loaded.as_ref().unwrap().history.steps(),
        1,
        "typing in one field is one step"
    );
    if let Some(id) = h.ctx.memory(|m| m.focused()) {
        h.ctx.memory_mut(|m| m.surrender_focus(id));
    }
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(
        designer::node_title(shown_form(&h).find(button).unwrap()),
        "Button \"Sign in\" · clicked"
    );
    assert_eq!(
        h.state().designer.selected,
        Some(button),
        "undo keeps the selection"
    );
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    h.run_steps(2);
    assert_eq!(
        designer::node_title(shown_form(&h).find(button).unwrap()),
        title
    );
    assert_eq!(h.state().designer.selected, Some(button));
    // A flag is a checkbox: on writes `.small()`, off removes it again (a default isn't stored).
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Small")
        .click();
    h.run_steps(2);
    let small = |h: &Harness<'static, App>| matches!(&shown_form(h).find(button).unwrap().kind, formust_model::NodeKind::Widget { props, .. } if props.contains_key("small"));
    assert!(small(&h));
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Small")
        .click();
    h.run_steps(2);
    assert!(!small(&h));
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-inspector.png"))
        .unwrap();
}

/// Delete, Copy, Paste, Cut and Cmd+D on the selection, from the keyboard and the Edit menu events.
#[test]
fn delete_copy_paste_cut_and_duplicate() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("clipboard"), false);
    let links = |h: &Harness<'static, App>| {
        shown_form(h)
            .root
            .children()
            .iter()
            .filter(|n| designer::node_title(n) == "Link \"A link\"")
            .count()
    };
    let link = exact_node(h.state(), "Link \"A link\"");
    h.get_by_role_and_label(egui::accesskit::Role::TreeItem, "Link \"A link\"")
        .click();
    h.run_steps(2);
    h.event(egui::Event::Copy);
    h.run_steps(2);
    assert_eq!(
        h.state().designer.clipboard.as_deref(),
        Some("ui.link(\"A link\");")
    );
    h.event(egui::Event::Paste("ui.link(\"A link\");".into()));
    h.run_steps(2);
    assert_eq!(links(&h), 2);
    let pasted = h.state().designer.selected.unwrap();
    assert_ne!(pasted, link);
    assert_eq!(
        shown_form(&h).parent_of(pasted).unwrap().1,
        shown_form(&h).parent_of(link).unwrap().1 + 1,
        "right after the original"
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(2);
    assert_eq!(links(&h), 3);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(links(&h), 2);
    h.state_mut().designer.select(Some(link), false);
    h.event(egui::Event::Cut);
    h.run_steps(2);
    assert_eq!(links(&h), 1);
    assert!(shown_form(&h).find(link).is_none());
    // Undo brings the cut node back under its own id.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert!(shown_form(&h).find(link).is_some());
    // The arrow keys move the selection among its siblings.
    h.state_mut().designer.select(Some(link), false);
    let at = shown_form(&h).parent_of(link).unwrap().1;
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(shown_form(&h).parent_of(link).unwrap().1, at - 1);
    // Locked custom code is copied as written.
    let custom = designer_node(h.state(), "Custom code: if self.busy");
    h.state_mut().designer.select(Some(custom), false);
    h.event(egui::Event::Copy);
    h.run_steps(2);
    assert!(
        h.state()
            .designer
            .clipboard
            .as_deref()
            .unwrap()
            .starts_with("if self.busy {")
    );
}

/// Cmd+S in the designer writes `sample.frm.rs` beside `sample.rs` (which stays as it was): a
/// standalone copy of the source with the edited form and its new state. Reopening reads the same
/// form back, and a second save has nothing to write.
#[test]
fn saving_exports_the_form_file_and_reopening_gives_the_same_form() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let rs = designer_copy("export");
    let original = std::fs::read_to_string(&rs).unwrap();
    let mut h = designer_harness_for(rs.clone(), false);
    h.state_mut().designer.palette_filter = "Text edit".to_string();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Text edit")
        .click();
    h.run_steps(2);
    let heading = exact_node(h.state(), "Heading \"Sample heading\"");
    h.state_mut().designer.try_edit(None, |f| {
        f.set_prop(
            heading,
            "text",
            formust_model::PropValue::Text("Edited heading".into()),
        )
    });
    h.run_steps(1);
    assert!(h.state().is_dirty(0));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(3);
    let frm = rs.with_file_name("sample.frm.rs");
    let text = std::fs::read_to_string(&frm).expect("the form file was written");
    assert!(
        text.starts_with("// formust 1 | sample.rs | Sample::ui | src-hash "),
        "{text}"
    );
    assert!(
        text.contains("#[derive(Default)] pub struct SampleForm { pub text: String }"),
        "{text}"
    );
    assert!(
        text.contains("    busy: bool,\n    form: SampleForm,\n}"),
        "the state field is in the copy's struct: {text}"
    );
    assert!(
        text.contains("ui.text_edit_singleline(&mut self.form.text);")
            && text.contains("ui.heading(\"Edited heading\");"),
        "{text}"
    );
    assert!(
        text.contains("impl eframe::App for Panels")
            && text.contains("// formust 1 | sample.rs | Panels::ui | src-hash "),
        "the copy is the whole source, the unedited form too"
    );
    assert!(!text.contains("include!"), "{text}");
    assert!(
        text.contains("if self.busy {"),
        "custom code is exported verbatim"
    );
    assert_eq!(
        std::fs::read_to_string(&rs).unwrap(),
        original,
        "the .rs is never written"
    );
    assert!(!h.state().is_dirty(0));
    assert_eq!(
        h.state().designer.current().unwrap().origin,
        Some(formust_rust::Origin::Frm)
    );
    // Idempotent: the file reads back to itself, and another save writes nothing.
    assert_eq!(formust_rust::parse_frm(&text).unwrap().emit(), text);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(2);
    assert_eq!(std::fs::read_to_string(&frm).unwrap(), text);
    // Reopened: the same form, from the form file.
    let titles = |f: &formust_model::Form| {
        let mut t = Vec::new();
        f.root.walk(&mut |n| t.push(designer::node_title(n)));
        t
    };
    let edited = titles(&shown_form(&h));
    let reopened = designer_harness_for(rs.clone(), false);
    assert_eq!(titles(&shown_form(&reopened)), edited);
    assert_eq!(
        reopened.state().designer.current().unwrap().conflict,
        Some(formust_rust::Conflict::Unchanged)
    );
    assert!(!reopened.state().is_dirty(0));
}

/// A source that changed under the form, and a form file changed outside, both stop the save
/// with the prompt; a form file that doesn't parse is never overwritten.
#[test]
fn a_changed_source_or_form_file_brings_up_the_conflict_prompt() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let rs = designer_copy("conflict");
    let frm = rs.with_file_name("sample.frm.rs");
    let mut h = designer_harness_for(rs.clone(), false);
    let add = |h: &mut Harness<'static, App>| {
        h.state_mut().designer.select(None, false);
        // The filter brings the item into view in the scrolling palette.
        h.state_mut().designer.palette_filter = "Spinner".to_string();
        h.run_steps(2);
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Spinner")
            .click();
        h.run_steps(2);
    };
    let save = |h: &mut Harness<'static, App>| {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        h.run_steps(3);
    };
    add(&mut h);
    save(&mut h);
    assert!(frm.exists());
    // The source changes in its tab while the designer has edits.
    add(&mut h);
    let changed = h.state().docs[0]
        .text
        .replace("\"Sample heading\"", "\"Sample title\"");
    h.state_mut().docs[0].text = changed.clone();
    h.run_steps(2);
    assert!(
        h.state().designer.loaded.as_ref().unwrap().stale,
        "the edits are kept"
    );
    save(&mut h);
    assert_eq!(
        h.state().designer.prompt,
        Some(designer::export::Prompt::SourceChanged {
            form: "Sample::ui".into()
        })
    );
    h.get_by_label("Show diff").click();
    h.run_steps(3);
    let diff = h
        .state()
        .designer
        .diff
        .clone()
        .expect("the difference is shown");
    assert!(
        diff.old.contains("Sample title") && diff.new.contains("Sample heading"),
        "{diff:?}"
    );
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-conflict.png"))
        .unwrap();
    h.state_mut().designer.diff = None;
    h.get_by_label("Keep .frm.rs").click();
    h.run_steps(3);
    assert_eq!(h.state().designer.prompt, None);
    let text = std::fs::read_to_string(&frm).unwrap();
    let hash = formust_rust::import(&changed)
        .unwrap()
        .form("Sample::ui")
        .unwrap()
        .src_hash;
    assert!(
        text.starts_with(&format!(
            "// formust 1 | sample.rs | Sample::ui | src-hash {hash:08x}"
        )),
        "the header takes the new hash: {text}"
    );
    assert!(
        text.contains("ui.heading(\"Sample heading\");"),
        "the form file's version is kept"
    );

    // The form file is edited outside: the save asks, and Overwrite keeps the hand-written comment.
    add(&mut h);
    std::fs::write(&frm, format!("// a note by hand\n{text}")).unwrap();
    save(&mut h);
    assert!(matches!(
        h.state().designer.prompt,
        Some(designer::export::Prompt::FormFileChanged { .. })
    ));
    h.get_by_label("Overwrite").click();
    h.run_steps(3);
    let text = std::fs::read_to_string(&frm).unwrap();
    assert!(
        text.contains("// a note by hand") && text.matches("ui.spinner();").count() >= 3,
        "{text}"
    );

    // A form file that doesn't parse: Overwrite still refuses.
    add(&mut h);
    std::fs::write(&frm, "impl Sample { fn ui(").unwrap();
    save(&mut h);
    h.get_by_label("Overwrite").click();
    h.run_steps(3);
    assert_eq!(
        std::fs::read_to_string(&frm).unwrap(),
        "impl Sample { fn ui("
    );
    assert!(
        h.state()
            .designer
            .status
            .as_deref()
            .is_some_and(|s| s.contains("not overwritten")),
        "{:?}",
        h.state().designer.status
    );
    assert!(h.state().is_dirty(0), "the edits are still unsaved");
}

/// "Use in code" says the copy stands alone, lists the handler stubs the copy needs, what Save does
/// for the state, and how a program can build the copy in place of the source. No `include!`.
#[test]
fn the_use_in_code_panel_lists_the_lines_to_add() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("usage"), false);
    h.state_mut().designer.select(None, false);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Checkbox")
        .click();
    h.run_steps(2);
    let button = h
        .state_mut()
        .designer
        .add(
            designer::edit::Item::Widget(formust_model::WidgetId::Button),
            None,
        )
        .unwrap();
    h.state_mut().designer.try_edit(None, |f| {
        f.set_event(
            button,
            formust_model::EventId::Clicked,
            &formust_model::Form::default_handler("Button", formust_model::EventId::Clicked),
        )
    });
    h.get_by_label("Use in code").click();
    h.run_steps(3);
    h.get_by_label_contains("standalone copy of sample.rs");
    h.get_by_label_contains("`form: SampleForm` field in struct Sample");
    h.get_by_label("impl Sample {\n    fn on_button_clicked(&mut self) {}\n}");
    h.get_by_label("#[path = \"sample.frm.rs\"]\nmod sample;");
    assert!(
        h.query_by_label_contains("include!(\"").is_none(),
        "the source includes nothing"
    );
    h.get_all_by_label("Copy").next().unwrap().click();
    h.run_steps(2);
    assert_eq!(
        h.state().designer.clipboard.as_deref(),
        Some("impl Sample {\n    fn on_button_clicked(&mut self) {}\n}")
    );
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-use-in-code.png"))
        .unwrap();
}

/// File > New Rust Form makes `src/form1.rs` with an egui form and opens it in the designer.
#[test]
fn new_rust_form_creates_a_sample_and_opens_the_designer() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let dir = crate::target_tmp().join("designer-edit").join("new-form");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\negui = \"0.37\"\n").unwrap();
    let mut app = App::in_project(dir.clone(), None);
    let ctx = egui::Context::default();
    app.run_menu_list(&ctx, vec![MenuAction::NewForm]);
    let path = dir.canonicalize().unwrap().join("src/form1.rs");
    assert!(path.is_file());
    assert!(
        !std::fs::read_to_string(&path).unwrap().contains("eframe"),
        "the manifest names egui only"
    );
    assert!(app.form_showing());
    assert_eq!(app.designer.current().unwrap().form().id(), "Form1::ui");
    app.run_menu_list(&ctx, vec![MenuAction::NewForm]);
    assert!(
        path.with_file_name("form2.rs").is_file(),
        "the next one gets the next name"
    );
}

/// A source with a form opened from the tree (the sample) shows its designer in the window, not its
/// code; a source without one (the editor fixture) keeps its code editor.
#[test]
fn a_source_with_a_form_opens_in_the_designer_window() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut app = App::bare(Some(fixture()));
    app.lsp_ready = true;
    app.show_navigator = false;
    app.show_bottom = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1300.0))
        .with_pixels_per_point(1.0)
        .with_theme(egui::Theme::Light)
        .with_context_setup(theme::apply)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut App| app.draw(ui, None), app);
    harness.run_steps(2);
    assert!(
        harness.query_by_label("Code editor").is_some(),
        "the fixture has no form, so it shows its code"
    );

    harness
        .state_mut()
        .select_in_tree(designer_copy("select-in-tree"));
    harness.run_steps(4);
    assert!(
        harness.state().form_showing(),
        "the sample opens in the designer"
    );
    assert!(
        harness.query_by_label("Code editor").is_none(),
        "the designer replaces the code"
    );
}

// ---- The form designer, layout: resize handles, Fill, anchors, wrap -----------------------------

fn widget_prop(
    h: &Harness<'static, App>,
    id: formust_model::NodeId,
    name: &str,
) -> Option<formust_model::PropValue> {
    match &shown_form(h).find(id).unwrap().kind {
        formust_model::NodeKind::Widget { props, .. } => props.get(name).cloned(),
        _ => None,
    }
}

fn undo_steps(h: &Harness<'static, App>) -> usize {
    h.state().designer.loaded.as_ref().unwrap().history.steps()
}

fn select_node(h: &mut Harness<'static, App>, id: formust_model::NodeId) {
    h.state_mut().designer.select(Some(id), false);
    designer_step(h);
    designer_step(h);
}

/// Dragging the right handle of a selected text edit sets `desired_width` live through real
/// pointer events, as one undo step; Escape in the middle of a drag puts it back.
#[test]
fn dragging_the_right_handle_sets_the_desired_width() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("resize-width"), false);
    let field = exact_node(h.state(), "Text edit &mut self.name");
    select_node(&mut h, field);
    let rect = designer_rect(&h, field);
    let from = rect.right_center();
    let to = from + egui::vec2(60.0, 7.0);
    designer_drag(&mut h, from, to);
    assert!(
        h.state().designer.resize.is_some(),
        "the handle takes the drag, not the shield"
    );
    assert_eq!(h.state().designer.drag, None, "no move starts");
    let want = (rect.width() + 60.0).round() as f64;
    assert_eq!(
        widget_prop(&h, field, "desired_width"),
        Some(formust_model::PropValue::Float(want)),
        "live while dragging"
    );
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-resize.png"))
        .unwrap();
    designer_release(&mut h, to);
    assert!(h.state().designer.resize.is_none());
    assert_eq!(undo_steps(&h), 1, "one drag, one step");
    let now = designer_rect(&h, field);
    assert!(
        (now.width() - want as f32).abs() < 1.5,
        "the preview follows: {now:?}"
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains(&format!(
            "ui.add(egui::TextEdit::singleline(&mut self.name).desired_width({}));",
            formust_model::num(want)
        )),
        "{text}"
    );

    // Escape during a drag: back to where it was, no step.
    designer_drag(
        &mut h,
        now.right_center(),
        now.right_center() + egui::vec2(40.0, 0.0),
    );
    assert!(h.state().designer.resize.is_some());
    h.key_press(egui::Key::Escape);
    designer_step(&mut h);
    designer_release(&mut h, now.right_center() + egui::vec2(40.0, 0.0));
    assert_eq!(
        widget_prop(&h, field, "desired_width"),
        Some(formust_model::PropValue::Float(want))
    );
    assert_eq!(undo_steps(&h), 1);
    assert_eq!(
        h.state().designer.selected,
        Some(field),
        "Escape kept the selection"
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(widget_prop(&h, field, "desired_width"), None);
}

/// A label has no size property of its own: the bottom handle gives it `ui.add_sized([w, h], ..)`,
/// and so does the right handle of an anchored checkbox, inside its wrapper.
#[test]
fn a_widget_without_size_properties_is_resized_with_add_sized() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("resize-sized"), false);
    let label = exact_node(h.state(), "Label \"Name label\"");
    select_node(&mut h, label);
    let rect = designer_rect(&h, label);
    let from = rect.center_bottom();
    let to = from + egui::vec2(0.0, 20.0);
    designer_drag(&mut h, from, to);
    designer_release(&mut h, to);
    let size = format!(
        "[{}, {}]",
        formust_model::num(rect.width().round() as f64),
        formust_model::num((rect.height() + 20.0).round() as f64)
    );
    assert_eq!(
        widget_prop(&h, label, "add_sized"),
        Some(formust_model::PropValue::Expr(size.clone()))
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains(&format!(
            "ui.add_sized({size}, egui::Label::new(\"Name label\"));"
        )),
        "{text}"
    );

    // Anchored right, then resized inside its wrapper.
    let checkbox = exact_node(h.state(), "Checkbox \"Remember me\"");
    select_node(&mut h, checkbox);
    assert!(
        h.state_mut()
            .designer
            .set_anchor(Some(formust_model::Anchor::Right))
    );
    designer_step(&mut h);
    designer_step(&mut h);
    let rect = designer_rect(&h, checkbox);
    let client = h.state().designer.client.unwrap();
    assert!(
        rect.right() > client.right() - 20.0,
        "anchored right: {rect:?} in {client:?}"
    );
    let from = rect.right_center();
    // Right to left: the handle drags its right edge, and the box grows leftwards.
    let to = from + egui::vec2(-30.0, 0.0);
    designer_drag(&mut h, from, to);
    designer_release(&mut h, to);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains(&format!(
            "ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {{\n            ui.add_sized([{}, {}], egui::Checkbox::new(&mut self.remember, \"Remember me\"));",
            formust_model::num((rect.width() - 30.0).round() as f64),
            formust_model::num(rect.height().round() as f64),
        )),
        "{text}"
    );
    assert_eq!(h.state().designer.selected, Some(checkbox));
}

/// Cmd+E fills: the text edit in the Name row stretches to the end of the row, and the export says
/// `.desired_width(f32::INFINITY)`. A click on the anchored node's empty row selects the node.
#[test]
fn fill_stretches_a_text_edit_in_its_row() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("fill"), false);
    let field = exact_node(h.state(), "Text edit &mut self.name");
    select_node(&mut h, field);
    let before = designer_rect(&h, field);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::E);
    designer_step(&mut h);
    designer_step(&mut h);
    assert_eq!(h.state().designer.selected_fill(), Some(true));
    let after = designer_rect(&h, field);
    let client = h.state().designer.client.unwrap();
    assert_eq!(after.left(), before.left(), "still after its label");
    assert!(
        (after.right() - (client.right() - designer::preview::MARGIN)).abs() < 1.5,
        "to the end of the row: {after:?} in {client:?}"
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains(
            "ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));"
        ),
        "{text}"
    );
    let image = h.render().expect("offscreen render");
    image.save(out_dir().join("ui-designer-fill.png")).unwrap();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::E);
    designer_step(&mut h);
    assert_eq!(h.state().designer.selected_fill(), Some(false));

    // An anchored link: at the right, and a click on it selects the link, never its wrapper.
    let link = exact_node(h.state(), "Link \"A link\"");
    select_node(&mut h, link);
    assert!(
        h.state_mut()
            .designer
            .set_anchor(Some(formust_model::Anchor::Right))
    );
    designer_step(&mut h);
    let wrapper = h.state().designer.host(link);
    assert_ne!(wrapper, link);
    let at = designer_rect(&h, link);
    assert!(at.right() > client.right() - 20.0, "{at:?}");
    h.state_mut().designer.select(None, true);
    designer_click(&mut h, at.center());
    assert_eq!(h.state().designer.selected, Some(link));
    assert!(
        h.state().designer.rows.iter().all(|r| r.id != wrapper),
        "the outline has no row for the wrapper"
    );
    assert!(h.query_by_label("Link \"A link\" · anchor right").is_some());
}

/// A right-click on the canvas opens the node's menu; "Wrap in row" puts it in a new Horizontal
/// as one step, and Cmd+Shift+J unwraps it again.
#[test]
fn the_canvas_menu_wraps_in_a_row_and_the_shortcut_unwraps() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(designer_copy("wrap"), false);
    let root = shown_form(&h).root.id;
    let checkbox = exact_node(h.state(), "Checkbox \"Remember me\"");
    let at = designer_rect(&h, checkbox).center();
    let secondary = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(at));
    designer_step(&mut h);
    h.event(secondary(true));
    designer_step(&mut h);
    h.event(secondary(false));
    designer_step(&mut h);
    designer_step(&mut h);
    assert_eq!(h.state().designer.selected, Some(checkbox));
    let item = h
        .query_all_by_label("Wrap in row")
        .filter(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
        .min_by(|a, b| {
            let d = |n: &egui_kittest::Node<'_>| (n.rect().center() - at).length();
            d(a).total_cmp(&d(b))
        })
        .expect("the menu's item");
    item.click();
    h.run_steps(3);
    let (row, _) = shown_form(&h).parent_of(checkbox).unwrap();
    assert_ne!(row, root);
    assert_eq!(
        designer::node_title(shown_form(&h).find(row).unwrap()),
        "Horizontal"
    );
    assert_eq!(undo_steps(&h), 1);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("        ui.horizontal(|ui| {\n            ui.checkbox(&mut self.remember, \"Remember me\");\n        });"),
        "{text}"
    );
    select_node(&mut h, row);
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::J,
    );
    h.run_steps(2);
    assert_eq!(shown_form(&h).parent_of(checkbox).unwrap().0, root);
    assert!(shown_form(&h).find(row).is_none());
    assert_eq!(undo_steps(&h), 2);
    // Cmd+J wraps the selected node again.
    select_node(&mut h, checkbox);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::J);
    h.run_steps(2);
    assert_ne!(shown_form(&h).parent_of(checkbox).unwrap().0, root);
}

// ---- The form designer, batch B: Frame, spacing and size, tooltip/enabled/visible, text style, events

const LOOK: &str = r#"#[derive(Default)]
pub struct Look {
    name: String,
    ready: bool,
}

impl Look {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label("First label");
        ui.vertical(|ui| {
            ui.label("Top line");
            ui.label("Bottom line");
        });
        ui.horizontal(|ui| {
            ui.label("In a row");
        });
        if ui.button("Store values").clicked() {
            self.ready = true;
        }
        ui.label("Plain text");
        ui.heading("Big title");
        ui.text_edit_singleline(&mut self.name);
    }
}
"#;

/// The designer on `LOOK`, tall enough that a widget's whole inspector fits without scrolling.
fn look_harness(name: &str) -> Harness<'static, App> {
    let mut h = designer_harness_for(look_copy(name), false);
    h.set_size(egui::vec2(1500.0, 1900.0));
    h.run_steps(3);
    h
}

/// `LOOK` in a folder of its own under the target dir, never in the fixtures.
fn look_copy(name: &str) -> PathBuf {
    let dir = crate::target_tmp().join("designer-look").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("look.rs");
    std::fs::write(&rs, LOOK).unwrap();
    rs
}

fn node_props(h: &Harness<'static, App>, id: formust_model::NodeId) -> formust_model::Props {
    match &shown_form(h).find(id).unwrap().kind {
        formust_model::NodeKind::Widget { props, .. }
        | formust_model::NodeKind::Container { props, .. } => props.clone(),
        formust_model::NodeKind::Custom { .. } => formust_model::Props::new(),
    }
}

fn set_prop(
    h: &mut Harness<'static, App>,
    id: formust_model::NodeId,
    name: &'static str,
    value: formust_model::PropValue,
) {
    assert!(
        h.state_mut()
            .designer
            .try_edit(None, |f| f.set_prop(id, name, value)),
        "{:?}",
        h.state().designer.status
    );
    designer_step(h);
    designer_step(h);
}

/// The one combo box showing `value`.
fn combo<'h>(h: &'h Harness<'static, App>, value: &'h str) -> egui_kittest::Node<'h> {
    let mut found: Vec<_> = h
        .query_all_by_value(value)
        .filter(|n| n.accesskit_node().role() == egui::accesskit::Role::ComboBox)
        .collect();
    assert_eq!(found.len(), 1, "one combo box showing {value:?}");
    found.remove(0)
}

/// The colour of one pixel of the last render, in points (pixels_per_point is 1).
fn pixel(image: &image::RgbaImage, at: egui::Pos2) -> [u8; 4] {
    image.get_pixel(at.x as u32, at.y as u32).0
}

/// A Frame from the palette: a margin, rounded corners and a line that suits both modes. Its fill
/// set as a light and dark pair shows the light colour in light mode and the dark one in dark mode,
/// and the export is the `egui::Frame::new()..show(ui, |ui| ..)` chain.
#[test]
fn a_frame_from_the_palette_is_light_and_dark_aware() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(look_copy("frame"), false);
    let first = exact_node(h.state(), "Label \"First label\"");
    select_node(&mut h, first);
    // The palette offers it (its button may be scrolled out of the panel here).
    assert!(
        designer::edit::palette().contains(&designer::edit::Item::Container(
            formust_model::ContainerId::Frame
        ))
    );
    h.state_mut().designer.add(
        designer::edit::Item::Container(formust_model::ContainerId::Frame),
        None,
    );
    h.run_steps(3);
    let frame = h
        .state()
        .designer
        .selected
        .expect("the new frame is selected");
    assert_eq!(
        designer::node_title(shown_form(&h).find(frame).unwrap()),
        "Frame"
    );
    let plain = exact_node(h.state(), "Label \"Plain text\"");
    assert!(h.state_mut().designer.move_to(plain, frame, 0));
    designer_step(&mut h);
    set_prop(
        &mut h,
        frame,
        "fill",
        formust_model::PropValue::color_pair([250, 240, 200, 255], [20, 40, 60, 255]),
    );
    select_node(&mut h, frame);
    // The inspector has the Frame's own editors, the stroke with its width and colours.
    for label in [
        "Fill",
        "Stroke",
        "Corner radius",
        "Inner margin",
        "Item spacing",
        "Min width",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    let rect = designer_rect(&h, frame);
    let inner = designer_rect(&h, plain);
    assert!(
        inner.left() - rect.left() >= 8.0 && inner.top() - rect.top() >= 8.0,
        "the inner margin: {inner:?} in {rect:?}"
    );
    let inside = egui::pos2(rect.left() + 4.0, rect.center().y);
    let light = h.render().expect("offscreen render");
    assert_eq!(pixel(&light, inside), [250, 240, 200, 255]);
    light
        .save(out_dir().join("ui-designer-frame-light.png"))
        .unwrap();
    h.state_mut().designer.dark = Some(true);
    designer_step(&mut h);
    designer_step(&mut h);
    let dark = h.render().expect("offscreen render");
    assert_eq!(pixel(&dark, inside), [20, 40, 60, 255]);
    dark.save(out_dir().join("ui-designer-frame-dark.png"))
        .unwrap();

    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("        egui::Frame::new().fill(if ui.visuals().dark_mode { egui::Color32::from_rgb(20, 40, 60) } else { egui::Color32::from_rgb(250, 240, 200) }).stroke(egui::Stroke::new(1.0, if ui.visuals().dark_mode { egui::Color32::from_rgb(90, 90, 90) } else { egui::Color32::from_rgb(180, 180, 180) })).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {\n            ui.label(\"Plain text\");\n        });"),
        "{text}"
    );
    let again = formust_rust::import(&text).unwrap();
    assert!(
        again.forms[0].custom.is_empty(),
        "{:?}",
        again.forms[0].custom
    );
}

/// Item spacing on a Vertical spreads its children in the preview as at runtime, and dragging a
/// row's right handle writes `ui.set_min_width(..)` at the start of its body, as one undo step.
#[test]
fn spacing_and_a_min_width_change_the_preview() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(look_copy("spacing"), false);
    let top = exact_node(h.state(), "Label \"Top line\"");
    let bottom = exact_node(h.state(), "Label \"Bottom line\"");
    let column = shown_form(&h).parent_of(top).unwrap().0;
    let gap =
        |h: &Harness<'static, App>| designer_rect(h, bottom).top() - designer_rect(h, top).bottom();
    let before = gap(&h);
    set_prop(
        &mut h,
        column,
        "item_spacing",
        formust_model::PropValue::Expr("egui::vec2(8.0, 24.0)".into()),
    );
    assert!((gap(&h) - 24.0).abs() < 1.0, "{before} to {}", gap(&h));
    select_node(&mut h, column);
    assert!(h.query_by_label("SPACING AND PADDING").is_some());

    let label = exact_node(h.state(), "Label \"In a row\"");
    let row = shown_form(&h).parent_of(label).unwrap().0;
    select_node(&mut h, row);
    let rect = designer_rect(&h, row);
    let from = rect.right_center();
    let to = from + egui::vec2(120.0, 0.0);
    designer_drag(&mut h, from, to);
    designer_release(&mut h, to);
    let want = (rect.width() + 120.0).round() as f64;
    assert_eq!(
        node_props(&h, row).get("set_min_width"),
        Some(&formust_model::PropValue::Float(want))
    );
    assert_eq!(undo_steps(&h), 2, "the spacing, then one drag");
    assert!((designer_rect(&h, row).width() - want as f32).abs() < 1.5);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains(&format!(
            "        ui.horizontal(|ui| {{\n            ui.set_min_width({});\n            ui.label(\"In a row\");\n        }});",
            formust_model::num(want)
        )),
        "{text}"
    );
    assert!(text.contains("            ui.spacing_mut().item_spacing = egui::vec2(8.0, 24.0);\n            ui.label(\"Top line\");"), "{text}");
}

/// A tooltip typed in the inspector goes onto the response; a button set disabled shows disabled
/// (AccessKit says so too); a heading set invisible stays on the canvas, dimmed and selectable; and
/// "Expression…" lets enabled take `self.ready`.
#[test]
fn tooltip_enabled_and_visible_show_in_the_designer() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = look_harness("behaviour");
    let button = exact_node(h.state(), "Button \"Store values\" · clicked");
    select_node(&mut h, button);
    assert!(h.query_by_label("BEHAVIOUR").is_some());
    h.query_all_by_label("Tooltip")
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::TextInput)
        .expect("the tooltip field")
        .focus();
    h.run_steps(2);
    h.event(egui::Event::Text("Keeps it".into()));
    h.run_steps(2);
    assert_eq!(
        node_props(&h, button).get("tooltip"),
        Some(&formust_model::PropValue::Text("Keeps it".into()))
    );
    if let Some(id) = h.ctx.memory(|m| m.focused()) {
        h.ctx.memory_mut(|m| m.surrender_focus(id));
    }
    h.run_steps(1);
    assert!(
        !h.get_by_label("Store values")
            .accesskit_node()
            .is_disabled()
    );
    set_prop(
        &mut h,
        button,
        "enabled",
        formust_model::PropValue::Bool(false),
    );
    assert!(
        h.get_by_label("Store values")
            .accesskit_node()
            .is_disabled(),
        "the preview shows it disabled"
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("if ui.add_enabled(false, egui::Button::new(\"Store values\")).on_hover_text(\"Keeps it\").clicked() {"),
        "{text}"
    );

    // Invisible: dimmed, still drawn where it would be and selectable.
    let heading = exact_node(h.state(), "Heading \"Big title\"");
    let shown = designer_rect(&h, heading);
    set_prop(
        &mut h,
        heading,
        "visible",
        formust_model::PropValue::Bool(false),
    );
    assert_eq!(designer_rect(&h, heading), shown, "it keeps its place");
    h.state_mut().designer.select(None, true);
    designer_click(&mut h, shown.center());
    assert_eq!(h.state().designer.selected, Some(heading));
    let image = h.render().expect("offscreen render");
    image
        .save(out_dir().join("ui-designer-behaviour.png"))
        .unwrap();

    // Enabled as an expression, picked from the bool editor and typed.
    select_node(&mut h, heading);
    combo(&h, "(not set)").click();
    h.run_steps(2);
    h.get_by_label("Expression…").click();
    h.run_steps(3);
    assert_eq!(
        node_props(&h, heading).get("enabled"),
        Some(&formust_model::PropValue::Expr("true".into()))
    );
    set_prop(
        &mut h,
        heading,
        "visible",
        formust_model::PropValue::Expr("self.ready".into()),
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("ui.add_enabled_ui(true, |ui| ui.scope(|ui| { if !(self.ready) { ui.set_invisible(); } ui.heading(\"Big title\") }).inner).inner;"),
        "{text}"
    );
    let again = formust_rust::import(&text).unwrap();
    assert!(
        again.forms[0].custom.is_empty(),
        "{:?}",
        again.forms[0].custom
    );
}

/// Font size, bold and italic on a label: the preview grows the text, the inspector groups them
/// under TEXT STYLE, and the export is an `egui::RichText` chain that reads back.
#[test]
fn text_styling_changes_the_label() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(look_copy("text-style"), false);
    let label = exact_node(h.state(), "Label \"Plain text\"");
    let before = designer_rect(&h, label);
    select_node(&mut h, label);
    assert!(h.query_by_label("TEXT STYLE").is_some());
    let bold = h
        .query_all_by_label("Bold")
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::CheckBox)
        .expect("the bold checkbox");
    bold.click();
    h.run_steps(2);
    set_prop(
        &mut h,
        label,
        "font_size",
        formust_model::PropValue::Float(28.0),
    );
    set_prop(
        &mut h,
        label,
        "italic",
        formust_model::PropValue::Bool(true),
    );
    let after = designer_rect(&h, label);
    assert!(
        after.height() > before.height() + 10.0 && after.width() > before.width(),
        "{before:?} to {after:?}"
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains(
            "ui.label(egui::RichText::new(\"Plain text\").size(28.0).strong().italics());"
        ),
        "{text}"
    );
    let again = formust_rust::import(&text).unwrap();
    let props = widget_props_of(&again.forms[0].form, "Plain text");
    assert_eq!(
        props.get("bold"),
        Some(&formust_model::PropValue::Bool(true))
    );
    assert_eq!(
        props.get("font_size"),
        Some(&formust_model::PropValue::Float(28.0))
    );
}

/// The props of the widget whose text is `text`.
fn widget_props_of(form: &formust_model::Form, text: &str) -> formust_model::Props {
    let mut found = None;
    form.root.walk(&mut |n| {
        if let formust_model::NodeKind::Widget { props, .. } = &n.kind
            && props.get("text") == Some(&formust_model::PropValue::Text(text.into()))
        {
            found = Some(props.clone());
        }
    });
    found.expect("the widget")
}

/// "Add handler…" lists the widget's other events; picking double_clicked adds its handler stub
/// next to clicked, written with one `response`, and "Use in code" lists the new stub.
#[test]
fn a_double_click_handler_is_added_from_the_inspector() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = look_harness("events");
    let button = exact_node(h.state(), "Button \"Store values\" · clicked");
    select_node(&mut h, button);
    combo(&h, "Add handler…").click();
    h.run_steps(2);
    for event in ["double_clicked", "secondary_clicked", "hovered"] {
        assert!(h.query_by_label(event).is_some(), "{event} is offered");
    }
    h.get_by_label("double_clicked").click();
    h.run_steps(3);
    let events: Vec<formust_model::EventId> = match &shown_form(&h).find(button).unwrap().kind {
        formust_model::NodeKind::Widget { events, .. } => events.iter().map(|(e, _)| *e).collect(),
        _ => Vec::new(),
    };
    assert_eq!(
        events,
        [
            formust_model::EventId::Clicked,
            formust_model::EventId::DoubleClicked
        ]
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("{ let response = ui.button(\"Store values\"); if response.clicked() {\n            self.ready = true;\n        } if response.double_clicked() { self.on_store_values_double_clicked(); } }"),
        "{text}"
    );
    let usage = formust_rust::use_in_code(&shown_form(&h), "look.frm.rs", "look.rs", None);
    assert!(
        usage
            .stubs
            .as_deref()
            .is_some_and(|s| s.contains("fn on_store_values_double_clicked(&mut self)")),
        "{:?}",
        usage.stubs
    );
}

// ---- The form designer, batch C: palette, multi-select, align, Columns and Sides, overlays, tab order

/// A click on the canvas with modifiers held over several frames, as a real keyboard does.
fn designer_click_with(h: &mut Harness<'static, App>, pos: egui::Pos2, mods: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(pos));
    designer_step(h);
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: mods,
    });
    designer_step(h);
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: mods,
    });
    designer_step(h);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    designer_step(h);
}

/// The palette scrolls, filters and folds its groups: every item can be brought into view, a
/// closed group hides its items, and the outline keeps its rows.
#[test]
fn the_palette_scrolls_filters_and_folds_its_groups() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = designer_harness_for(look_copy("palette"), false);
    // A short window: the palette is taller than its room and scrolls.
    h.set_size(egui::vec2(1500.0, 700.0));
    h.run_steps(3);
    let outline_top = h.get_by_label("OUTLINE").rect().top();
    for item in designer::edit::palette() {
        h.state_mut().designer.palette_filter = item.label().to_string();
        h.run_steps(2);
        let button = h
            .query_all_by_label(item.label())
            .filter(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .unwrap_or_else(|| panic!("{} is in the palette", item.label()));
        assert!(
            button.rect().bottom() < outline_top + 1.0,
            "{} is in view above the outline",
            item.label()
        );
    }
    h.state_mut().designer.palette_filter.clear();
    h.run_steps(2);
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-palette.png"))
        .unwrap();
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, "Heading")
            .is_some()
    );
    h.get_by_label("▾ Text").click();
    h.run_steps(2);
    assert!(h.state().designer.palette_closed.contains("Text"));
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, "Heading")
            .is_none(),
        "a closed group hides its items"
    );
    h.get_by_label("▸ Text").click();
    h.run_steps(2);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, "Heading")
            .is_some()
    );
}

/// Cmd-click on the canvas and in the outline builds a selection; the inspector shows what they
/// have in common and a change there sets it on all of them as one undo step; copy, duplicate,
/// arrows and delete act on the set; a rubber band selects what it encloses.
#[test]
fn several_nodes_are_selected_and_edited_together() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = look_harness("multi");
    let first = exact_node(h.state(), "Label \"First label\"");
    let plain = exact_node(h.state(), "Label \"Plain text\"");
    let title = exact_node(h.state(), "Heading \"Big title\"");
    let at = designer_rect(&h, first).center();
    designer_click(&mut h, at);
    let at = designer_rect(&h, plain).center();
    designer_click_with(&mut h, at, egui::Modifiers::COMMAND);
    assert_eq!(h.state().designer.selection_len(), 2);
    h.get_by_role_and_label(egui::accesskit::Role::TreeItem, "Heading \"Big title\"")
        .click_modifiers(egui::Modifiers::COMMAND);
    h.run_steps(2);
    // The outline click arrives with Cmd in the same frame: hold it over frames instead.
    if h.state().designer.selection_len() != 3 {
        let row = outline_row(&h, title).center();
        designer_click_with(&mut h, row, egui::Modifiers::COMMAND);
    }
    assert_eq!(h.state().designer.selection_len(), 3);
    assert!(h.query_by_label("3 selected").is_some());
    assert!(h.query_by_label("COMMON PROPERTIES").is_some());
    let steps = undo_steps(&h);
    let bold = h
        .query_all_by_label("Bold")
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::CheckBox)
        .expect("the common bold checkbox");
    bold.click();
    h.run_steps(2);
    for id in [first, plain, title] {
        assert_eq!(
            node_props(&h, id).get("bold"),
            Some(&formust_model::PropValue::Bool(true)),
            "bold on every one"
        );
    }
    assert_eq!(undo_steps(&h), steps + 1, "one undo step");
    // Cmd-click again takes one out.
    let at = designer_rect(&h, plain).center();
    designer_click_with(&mut h, at, egui::Modifiers::COMMAND);
    assert_eq!(h.state().designer.selection_len(), 2);
    // Copy holds both; duplicate copies both after themselves; delete removes the copies.
    let text = h.state_mut().designer.copy_selected().unwrap();
    assert!(
        text.contains("First label") && text.contains("Big title"),
        "{text}"
    );
    let count = shown_form(&h).node_ids().len();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(2);
    assert_eq!(shown_form(&h).node_ids().len(), count + 2);
    assert_eq!(
        h.state().designer.selection_len(),
        2,
        "the copies are selected"
    );
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(shown_form(&h).node_ids().len(), count);
    // A rubber band from empty space selects the nodes it encloses.
    let a = designer_rect(&h, plain);
    let b = designer_rect(&h, title);
    let client = h.state().designer.client.unwrap();
    let from = egui::pos2(client.right() - 6.0, a.top() - 2.0);
    let to = egui::pos2(a.left() - 3.0, b.bottom() + 2.0);
    designer_drag(&mut h, from, to);
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-rubber-band.png"))
        .unwrap();
    designer_release(&mut h, to);
    let mut picked = h.state().designer.selection_hosts();
    picked.sort();
    let mut want = vec![plain, title];
    want.sort();
    assert_eq!(picked, want);
    // The arrow keys move them together.
    let at = shown_form(&h).parent_of(plain).unwrap().1;
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(shown_form(&h).parent_of(plain).unwrap().1, at - 1);
    assert_eq!(shown_form(&h).parent_of(title).unwrap().1, at);
}

/// The Align menu: same width writes each node's size property (a heading has none and is
/// skipped), align right wraps each in a right-to-left layout, distribute evenly writes
/// `ui.add_space`, and make a column wraps them in `ui.vertical`. Each is one undo step.
#[test]
fn align_and_distribute_write_what_egui_has() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = look_harness("align");
    let first = exact_node(h.state(), "Label \"First label\"");
    let plain = exact_node(h.state(), "Label \"Plain text\"");
    let title = exact_node(h.state(), "Heading \"Big title\"");
    let edit = exact_node(h.state(), "Text edit &mut self.name");
    let select = |h: &mut Harness<'static, App>, ids: &[formust_model::NodeId]| {
        h.state_mut().designer.select_all_of(ids);
        designer_step(h);
        designer_step(h);
    };
    select(&mut h, &[first, plain, title]);
    // The widest selected node, the heading included (it is measured, though it is skipped).
    let wide = [first, plain, title]
        .iter()
        .map(|id| designer_rect(&h, *id).width())
        .fold(0.0, f32::max);
    // From the context menu's Align submenu.
    let at = designer_rect(&h, first).center();
    h.event(egui::Event::PointerMoved(at));
    designer_step(&mut h);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        designer_step(&mut h);
    }
    designer_step(&mut h);
    assert_eq!(
        h.state().designer.selection_len(),
        3,
        "the right-click keeps the set"
    );
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Align")
        .click();
    h.run_steps(2);
    // The menu's item, not the inspector's button of the same name.
    h.query_all_by_label("Same width")
        .filter(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
        .min_by(|a, b| {
            let d = |n: &egui_kittest::Node<'_>| (n.rect().center() - at).length();
            d(a).total_cmp(&d(b))
        })
        .expect("the menu's item")
        .click();
    h.run_steps(3);
    let size = |h: &Harness<'static, App>, id| node_props(h, id).get("add_sized").cloned();
    let want = format!("[{:.1}, ", wide.round());
    for id in [first, plain] {
        match size(&h, id) {
            Some(formust_model::PropValue::Expr(t)) => assert!(t.starts_with(&want), "{t}"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(size(&h, title), None, "a heading has no size property");
    assert!(
        h.state()
            .designer
            .status
            .as_deref()
            .is_some_and(|s| s.contains("1 skipped")),
        "{:?}",
        h.state().designer.status
    );
    let steps = undo_steps(&h);
    assert!(
        h.state_mut()
            .designer
            .align(designer::align::AlignAction::Right)
    );
    designer_step(&mut h);
    for id in [first, plain, title] {
        assert_eq!(
            shown_form(&h).anchor_of(id),
            Some(formust_model::Anchor::Right)
        );
    }
    assert_eq!(undo_steps(&h), steps + 1);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {"),
        "{text}"
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(shown_form(&h).anchor_of(plain), None);
    // Distribute: the gap after the first of three neighbours is made like the widest one.
    h.state_mut()
        .designer
        .try_edit(None, |f| f.set_space_after(plain, 20.0));
    designer_step(&mut h);
    designer_step(&mut h);
    select(&mut h, &[plain, title, edit]);
    assert!(
        h.state_mut()
            .designer
            .align(designer::align::AlignAction::Distribute)
    );
    designer_step(&mut h);
    designer_step(&mut h);
    let space = shown_form(&h)
        .space_after(title)
        .expect("a space after the heading");
    assert!((space - 20.0).abs() <= 1.0, "{space}");
    let gap =
        |h: &Harness<'static, App>, a, b| designer_rect(h, b).top() - designer_rect(h, a).bottom();
    assert!(
        (gap(&h, plain, title) - gap(&h, title, edit)).abs() <= 1.0,
        "even gaps"
    );
    // Make a column: one Vertical around them.
    assert!(
        h.state_mut()
            .designer
            .align(designer::align::AlignAction::MakeColumn)
    );
    designer_step(&mut h);
    let (column, _) = shown_form(&h).parent_of(plain).unwrap();
    assert_eq!(
        designer::node_title(shown_form(&h).find(column).unwrap()),
        "Vertical"
    );
    assert_eq!(shown_form(&h).parent_of(edit).unwrap().0, column);
    // What each action writes is in its hover text.
    let writes = designer::align::AlignAction::Distribute.writes("egui::");
    assert!(writes.contains("ui.add_space"), "{writes}");
}

const SPLIT: &str = r#"#[derive(Default)]
pub struct Split {
    name: String,
}

impl Split {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.columns(3, |cols| {
            cols[0].label("Left column");
            cols[2].label("Right column");
        });
        egui::Sides::new().show(ui, |ui| {
            ui.label("Side left");
        }, |ui| {
            ui.label("Side right");
        });
        ui.text_edit_singleline(&mut self.name);
    }
}
"#;

fn split_harness(name: &str) -> Harness<'static, App> {
    let dir = crate::target_tmp().join("designer-split").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("split.rs");
    std::fs::write(&rs, SPLIT).unwrap();
    designer_harness_for(rs, false)
}

/// Columns and Sides preview as at runtime: the columns side by side at equal widths, the sides at
/// the left and right edges. A palette drag drops into an empty column, and the code says `cols[1]`.
#[test]
fn columns_and_sides_preview_and_take_drops() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = split_harness("preview");
    let left = designer_rect(&h, exact_node(h.state(), "Label \"Left column\""));
    let right = designer_rect(&h, exact_node(h.state(), "Label \"Right column\""));
    let form = shown_form(&h);
    let columns = form.root.children()[0].id;
    let middle = form.find(columns).unwrap().children()[1].id;
    let gap = designer_rect(&h, middle);
    assert!(left.right() <= gap.left() + 1.0 && gap.right() <= right.left() + 1.0);
    assert!((gap.width() - (right.left() - gap.left() - 8.0)).abs() <= 2.0 || gap.width() > 50.0);
    let side_left = designer_rect(&h, exact_node(h.state(), "Label \"Side left\""));
    let side_right = designer_rect(&h, exact_node(h.state(), "Label \"Side right\""));
    let client = h.state().designer.client.unwrap();
    assert!(side_left.left() < client.left() + 20.0);
    assert!(
        side_right.right() > client.right() - 20.0,
        "{side_right:?} {client:?}"
    );
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-columns.png"))
        .unwrap();
    // A label dragged from the palette into the empty middle column.
    h.state_mut().designer.palette_filter = "Label".into();
    h.run_steps(2);
    let from = palette_button(&h, "Label").center();
    let to = gap.center();
    designer_drag(&mut h, from, to);
    assert_eq!(h.state().designer.drop.map(|d| d.parent), Some(middle));
    designer_release(&mut h, to);
    assert_eq!(shown_form(&h).find(middle).unwrap().children().len(), 1);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("            cols[1].label(\"Label\");\n"),
        "{text}"
    );
    // A column can't be deleted on its own.
    select_node(&mut h, middle);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(shown_form(&h).find(middle).is_some());
    // The count in the inspector: two columns, the third's label moves to the second.
    select_node(&mut h, columns);
    set_prop(&mut h, columns, "count", formust_model::PropValue::Int(2));
    assert_eq!(shown_form(&h).find(columns).unwrap().children().len(), 2);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("ui.columns(2, |cols| {")
            && text.contains("cols[1].label(\"Right column\");"),
        "{text}"
    );
}

/// The overlays: rulers, grid lines and distance guides show on the canvas and are never part of
/// the form. Preset widths set the preview's width, and the light/dark switch sits in the canvas bar.
#[test]
fn rulers_grid_guides_and_preset_widths() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = look_harness("overlays");
    let before = formust_rust::emit_form_item(&shown_form(&h));
    let plain = exact_node(h.state(), "Label \"Plain text\"");
    select_node(&mut h, plain);
    h.get_by_label("Rulers").click();
    h.run_steps(2);
    h.get_by_label("Grid lines").click();
    h.run_steps(2);
    h.get_by_label("16").click();
    h.run_steps(2);
    assert!(h.state().designer.rulers);
    assert_eq!(h.state().designer.grid, Some(16.0));
    // The guides of the selected label: to its neighbours above and below and the form's edges.
    let d = &h.state().designer;
    let guides = designer::ui::distance_guides(&shown_form(&h), &d.rects, plain, d.rects[&plain]);
    // Up to the label above, down to the heading, right to the form's edge; the label is flush
    // with the form's left edge, so there is no left guide.
    assert_eq!(guides.len(), 3, "{guides:?}");
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-overlays.png"))
        .unwrap();
    // A ruler is drawn in the chrome colour along the canvas's top-left corner.
    let client = h.state().designer.client.unwrap();
    assert_eq!(
        designer::ui::grid_lines(client.left(), client.left(), client.left() + 40.0, 16.0).len(),
        3
    );
    assert!(!h.state().is_dirty(0), "overlays are no edit");
    assert_eq!(formust_rust::emit_form_item(&shown_form(&h)), before);
    // Preset widths.
    h.get_by_label("Width").click();
    h.run_steps(2);
    h.get_by_label("480 pt").click();
    h.run_steps(3);
    assert_eq!(h.state().designer.preview_width, Some(480.0));
    let client = h.state().designer.client.unwrap();
    assert!((client.width() - 480.0).abs() < 1.0, "{client:?}");
    // The light/dark switch is in the canvas bar.
    h.get_by_label("Dark").click();
    h.run_steps(2);
    assert_eq!(h.state().designer.dark, Some(true));
    assert!(!h.state().is_dirty(0));
}

/// Tab order mode numbers the focusable widgets and lists them; dragging a row moves the widget in
/// the code (egui's focus follows creation order), and the star writes the initial focus.
#[test]
fn the_tab_order_is_shown_reordered_and_given_an_initial_focus() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = look_harness("tab-order");
    // The canvas bar's switch (the right panel's pill has one too).
    h.query_all_by_label("Tab order")
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()).reverse())
        .expect("the switch")
        .click();
    h.run_steps(3);
    assert!(h.state().designer.tab_order);
    let order = shown_form(&h).tab_order();
    assert_eq!(order.len(), 2, "the button and the text edit");
    let (button, text_edit) = (order[0], order[1]);
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-tab-order.png"))
        .unwrap();
    let row = |h: &Harness<'static, App>, n: usize| {
        h.query_all_by_role(egui::accesskit::Role::ListItem)
            .find(|r| {
                r.accesskit_node()
                    .label()
                    .is_some_and(|l| l.starts_with(&format!("{n}  ")))
            })
            .expect("a tab order row")
            .rect()
    };
    // The text edit's row onto the top of the button's: the text edit comes first now.
    let from = row(&h, 2).center();
    let to = egui::pos2(row(&h, 1).center().x, row(&h, 1).top() + 3.0);
    designer_drag(&mut h, from, to);
    designer_release(&mut h, to);
    assert_eq!(shown_form(&h).tab_order(), [text_edit, button]);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    let (a, b) = (
        text.find("ui.text_edit_singleline").unwrap(),
        text.find("ui.button(\"Store values\")").unwrap(),
    );
    assert!(a < b, "the text edit is created first now:\n{text}");
    // The star: initial focus on the text edit.
    h.query_all_by_label("☆").next().expect("a star").click();
    h.run_steps(3);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("{ let response = ui.text_edit_singleline(&mut self.name); if ui.memory(|m| m.focused().is_none()) { response.request_focus(); } }"),
        "{text}"
    );
    let back = formust_rust::import(&text).unwrap();
    assert!(back.forms[0].custom.is_empty());
}

// ---- Designer batch D ----------------------------------------------------------------------

/// The widgets batch D added: `the_designer_renders_every_first_palette_widget` leaves them to
/// `batch_d_widgets_preview_and_edit`, whose form has them all.
const BATCH_D_WIDGETS: &[formust_model::WidgetId] = &[
    formust_model::WidgetId::Image,
    formust_model::WidgetId::ColorEdit,
    formust_model::WidgetId::ColorEditRgb,
    formust_model::WidgetId::RangeSlider,
];

const BATCH_D: &str = r#"#[derive(Default)]
pub struct Batch {
    tab: usize,
    color: egui::Color32,
    rgb: [f32; 3],
    low: f32,
    high: f32,
    open: bool,
}

impl Batch {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open").clicked() {
                    ui.close();
                }
            });
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, 0, "Main");
            ui.selectable_value(&mut self.tab, 1, "Other");
        });
        match self.tab {
            0 => {
                ui.label("On main");
            }
            1 => {
                ui.label("On other");
            }
            _ => {}
        }
        ui.image("file://logo.png");
        ui.image("https://example.com/remote.png");
        ui.color_edit_button_srgba(&mut self.color);
        ui.color_edit_button_rgb(&mut self.rgb);
        ui.add(egui::RangeSlider::new(&mut self.low, &mut self.high, 0.0..=100.0).text("Range"));
    }

    pub fn settings(&mut self, ctx: &egui::Context) {
        egui::Window::new("Settings").pivot(egui::Align2::CENTER_CENTER).show(ctx, |ui| {
            ui.label("Inside");
        });
    }

    pub fn dialog(&mut self, ctx: &egui::Context) {
        egui::Modal::new(egui::Id::unique("sure")).show(ctx, |ui| {
            ui.label("In the modal");
            if ui.button("Close").clicked() {
                self.open = false;
            }
        });
    }
}
"#;

/// The batch D form in a scratch folder, with a 20 x 10 PNG beside it as `logo.png`.
fn batch_d_harness(name: &str) -> Harness<'static, App> {
    let dir = crate::target_tmp().join("designer-batch-d").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let png = image::RgbaImage::from_pixel(20, 10, image::Rgba([200, 40, 40, 255]));
    png.save(dir.join("logo.png")).unwrap();
    let rs = dir.join("batch.rs");
    std::fs::write(&rs, BATCH_D).unwrap();
    designer_harness_for(rs, false)
}

/// Tabs show one page and a click on a tab button shows another (a drop goes into it); Add tab
/// adds a page; a selected menu is drawn open; a local PNG is drawn and a URL is a placeholder; the
/// colour pickers and the range slider are real widgets; a modal is drawn over a dimmed form. The
/// shield holds throughout (`designer_step` checks that nothing reacted).
#[test]
fn batch_d_widgets_preview_and_edit() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = batch_d_harness("preview");
    let form = shown_form(&h);
    let mut seen = std::collections::BTreeSet::new();
    form.root.walk(&mut |n| {
        if let formust_model::NodeKind::Widget { def, .. } = &n.kind {
            seen.insert(*def);
        }
    });
    for w in BATCH_D_WIDGETS {
        assert!(seen.contains(w), "the batch D form has no {w:?}");
    }
    for title in [
        "Colour picker &mut self.color",
        "Colour picker (RGB) &mut self.rgb",
        "Range slider \"Range\"",
    ] {
        designer_rect(&h, exact_node(h.state(), title));
    }
    // The local PNG at its own size; the URL as a placeholder box.
    let images: Vec<_> = {
        let mut out = Vec::new();
        form.root.walk(&mut |n| {
            if matches!(
                n.kind,
                formust_model::NodeKind::Widget {
                    def: formust_model::WidgetId::Image,
                    ..
                }
            ) {
                out.push(n.id);
            }
        });
        out
    };
    let local = designer_rect(&h, images[0]);
    assert_eq!(
        (local.width().round(), local.height().round()),
        (20.0, 10.0)
    );
    let remote = designer_rect(&h, images[1]);
    assert_eq!(
        (remote.width().round(), remote.height().round()),
        (96.0, 64.0)
    );
    // The first page shows; the other has no place on the canvas.
    let tabs = form
        .root
        .children()
        .iter()
        .find(|c| {
            matches!(
                c.kind,
                formust_model::NodeKind::Container {
                    def: formust_model::ContainerId::Tabs,
                    ..
                }
            )
        })
        .expect("the bar and its match read as tabs")
        .id;
    let pages: Vec<_> = form
        .find(tabs)
        .unwrap()
        .children()
        .iter()
        .map(|p| p.id)
        .collect();
    let main = exact_node(h.state(), "Label \"On main\"");
    let other = exact_node(h.state(), "Label \"On other\"");
    assert!(h.state().designer.rects.contains_key(&main));
    assert!(!h.state().designer.rects.contains_key(&other));
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-batch-d.png"))
        .unwrap();
    // A click on the "Other" tab button shows its page and selects it.
    let button = h
        .state()
        .designer
        .tab_buttons
        .iter()
        .find(|(_, page)| *page == pages[1])
        .expect("a tab button for the second page")
        .0;
    designer_click(&mut h, button.center());
    designer_step(&mut h);
    assert_eq!(h.state().designer.selected, Some(pages[1]));
    assert!(h.state().designer.rects.contains_key(&other));
    assert!(!h.state().designer.rects.contains_key(&main));
    // A label dropped from the palette onto that page goes into it.
    h.state_mut().designer.palette_filter = "Label".into();
    h.run_steps(2);
    let from = palette_button(&h, "Label").center();
    let to = designer_rect(&h, pages[1]).center();
    designer_drag(&mut h, from, to);
    assert_eq!(h.state().designer.drop.map(|d| d.parent), Some(pages[1]));
    designer_release(&mut h, to);
    assert_eq!(shown_form(&h).find(pages[1]).unwrap().children().len(), 2);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("            1 => {\n                ui.label(\"On other\");\n                ui.label(\"Label\");\n            }"),
        "{text}"
    );
    // Add tab in the inspector: a third tab whose empty page shows.
    select_node(&mut h, tabs);
    h.get_by_label("Add tab").click();
    h.run_steps(3);
    let now = shown_form(&h);
    assert_eq!(now.find(tabs).unwrap().children().len(), 3);
    let third = now.find(tabs).unwrap().children()[2].id;
    assert_eq!(h.state().designer.tab_pages.get(&tabs), Some(&third));
    assert!(h.state().designer.rects.contains_key(&third));
    assert!(
        formust_rust::emit_form_item(&now)
            .contains("ui.selectable_value(&mut self.tab, 2, \"Tab 3\");"),
    );
    // A menu opens when it is selected, and closes when the selection leaves it.
    let open = exact_node(h.state(), "Button \"Open\" · clicked");
    assert!(!h.state().designer.rects.contains_key(&open));
    let file = exact_node(h.state(), "Menu \"File\"");
    select_node(&mut h, file);
    assert!(h.state().designer.rects.contains_key(&open));
    let menu = designer_rect(&h, file);
    assert!(menu.contains(designer_rect(&h, open).center()));
    let image = h.render().unwrap();
    image
        .save(out_dir().join("ui-designer-menu-open.png"))
        .unwrap();
    // A click on the open menu's item selects it, and the menu stays open around it.
    let at = designer_rect(&h, open).center();
    designer_click(&mut h, at);
    assert_eq!(h.state().designer.selected, Some(open));
    select_node(&mut h, main);
    assert!(!h.state().designer.rects.contains_key(&open));
    // The palette has the new items, in their groups.
    for label in [
        "Tabs (horizontal)",
        "Tabs (vertical)",
        "Menu bar",
        "Menu",
        "Modal",
        "Image",
        "Colour picker",
        "Range slider",
    ] {
        h.state_mut().designer.palette_filter = label.into();
        h.run_steps(2);
        palette_button(&h, label);
    }
    // A Window's options in the inspector: the pivot read back, the order written.
    h.state_mut().designer.choose_form("Batch::settings".into());
    h.run_steps(3);
    let window = shown_form(&h).root.children()[0].id;
    select_node(&mut h, window);
    h.get_by_label("Pivot");
    h.get_by_label("Order");
    set_prop(
        &mut h,
        window,
        "order",
        formust_model::PropValue::Enum("Foreground"),
    );
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains("egui::Window::new(\"Settings\").pivot(egui::Align2::CENTER_CENTER).order(egui::Order::Foreground).show(ctx, |ui| {"),
        "{text}"
    );
    // The modal of the dialog form: framed over the form, its content selectable.
    h.state_mut().designer.choose_form("Batch::dialog".into());
    h.run_steps(3);
    let modal_label = exact_node(h.state(), "Label \"In the modal\"");
    let inside = designer_rect(&h, modal_label);
    let modal = shown_form(&h).root.children()[0].id;
    assert!(designer_rect(&h, modal).contains(inside.center()));
    designer_click(&mut h, inside.center());
    assert_eq!(h.state().designer.selected, Some(modal_label));
    let image = h.render().unwrap();
    image.save(out_dir().join("ui-designer-modal.png")).unwrap();
}

/// New tabs from the palette: both directions bind a new `usize` field, and the vertical bar sits
/// to the left of its page.
#[test]
fn new_tabs_from_the_palette_bind_a_field() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut h = split_harness("tabs");
    for label in ["Tabs (horizontal)", "Tabs (vertical)"] {
        h.state_mut().designer.select(None, false);
        h.state_mut().designer.palette_filter = label.into();
        h.run_steps(2);
        let at = palette_button(&h, label).center();
        h.event(egui::Event::PointerMoved(at));
        designer_step(&mut h);
        h.event(pointer_button(at, true));
        designer_step(&mut h);
        h.event(pointer_button(at, false));
        designer_step(&mut h);
        designer_step(&mut h);
    }
    let form = shown_form(&h);
    assert_eq!(
        form.state
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["tab", "tab_2"]
    );
    let vertical = form.root.children().last().unwrap();
    let page = vertical.children()[0].id;
    let bar = h
        .state()
        .designer
        .tab_buttons
        .iter()
        .find(|(_, p)| *p == page)
        .expect("the vertical bar's first button")
        .0;
    let page_rect = designer_rect(&h, page);
    assert!(bar.right() < page_rect.left(), "{bar:?} {page_rect:?}");
    let text = formust_rust::emit_form_item(&form);
    assert!(
        text.contains("match self.form.tab_2 {") && text.contains("ui.separator();"),
        "{text}"
    );
    let back = formust_rust::import(&text).unwrap();
    assert!(
        back.forms[0].custom.is_empty(),
        "{:?}",
        back.forms[0].custom
    );
}

const TABLE_FORM: &str = r#"#[derive(Default)]
pub struct People {}

impl People {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui_extras::TableBuilder::new(ui)
            .striped(true)
            .column(egui_extras::Column::auto())
            .column(egui_extras::Column::remainder())
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.heading("Name");
                });
                header.col(|ui| {
                    ui.heading("Role");
                });
            })
            .body(|mut body| {
                body.row(18.0, |mut row| {
                    row.col(|ui| {
                        ui.label("Ada");
                    });
                    row.col(|ui| {});
                });
            });
    }
}
"#;

/// A table previews as a grid of its cells (the header row above the body), takes a palette drop
/// into an empty cell, says in the inspector that it needs egui_extras, and grows a column.
#[test]
fn a_table_previews_its_cells_and_takes_drops() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let dir = crate::target_tmp().join("designer-table");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("people.rs");
    std::fs::write(&rs, TABLE_FORM).unwrap();
    let mut h = designer_harness_for(rs, false);
    let form = shown_form(&h);
    let table = form.root.children()[0].id;
    assert_eq!(form.table_size(table), Some((2, 1, true)));
    let name = designer_rect(&h, exact_node(h.state(), "Heading \"Name\""));
    let role = designer_rect(&h, exact_node(h.state(), "Heading \"Role\""));
    let ada = designer_rect(&h, exact_node(h.state(), "Label \"Ada\""));
    assert!(name.right() <= role.left() + 1.0, "{name:?} {role:?}");
    assert!(ada.top() >= name.bottom() - 1.0, "{ada:?} {name:?}");
    let empty = form.find(table).unwrap().children()[1].children()[1].id;
    let empty_rect = designer_rect(&h, empty);
    assert!(empty_rect.left() >= role.left() - 1.0);
    let image = h.render().unwrap();
    image.save(out_dir().join("ui-designer-table.png")).unwrap();
    // A label from the palette into the empty cell.
    h.state_mut().designer.palette_filter = "Label".into();
    h.run_steps(2);
    let from = palette_button(&h, "Label").center();
    let to = designer_rect(&h, empty).center();
    designer_drag(&mut h, from, to);
    assert_eq!(h.state().designer.drop.map(|d| d.parent), Some(empty));
    designer_release(&mut h, to);
    let text = formust_rust::emit_form_item(&shown_form(&h));
    assert!(
        text.contains(
            "row.col(|ui| {\n                    ui.label(\"Label\");\n                });"
        ),
        "{text}"
    );
    assert!(
        formust_rust::import(&text).unwrap().forms[0]
            .custom
            .is_empty()
    );
    // The inspector says what a table needs; a third column appears on the canvas.
    select_node(&mut h, table);
    h.get_by_label_contains("Requires egui_extras");
    assert!(
        h.state_mut()
            .designer
            .try_edit(None, |f| f.set_table_columns(table, 3))
    );
    designer_step(&mut h);
    designer_step(&mut h);
    let third = shown_form(&h).find(table).unwrap().children()[1].children()[2].id;
    assert!(designer_rect(&h, third).left() > role.left());
}

// ---- Dialog row, Form toggle and extended import (2026-11-06) ----------------------------------

/// A fresh folder in the target dir with `name` holding `text`; returns the file's path.
fn designer_file(folder: &str, name: &str, text: &str) -> PathBuf {
    let dir = crate::target_tmp().join(folder);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.canonicalize().unwrap()
}

/// The FORM panel of a dialog has a "Dialog" row saying what it shows; a form drawn into a `Ui`
/// has none.
#[test]
fn the_form_panel_has_a_dialog_row_for_a_dialog() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let rs = designer_file(
        "designer-dialog-row",
        "about.rs",
        "pub struct About { name: String, open: bool }\nimpl About {\n    pub fn show(&mut self, ctx: &egui::Context) {\n        let mut open = self.open;\n        egui::Window::new(\"About\").open(&mut open).show(ctx, |ui| {\n            ui.label(\"Hello dialog\");\n        });\n        self.open = open;\n    }\n}\n",
    );
    let mut h = designer_harness_for(rs, false);
    designer_step(&mut h);
    assert_eq!(
        h.state().designer.selected,
        None,
        "the root's FORM panel shows"
    );
    let note = designer::ui::dialog_note(&shown_form(&h)).expect("a dialog");
    assert!(note.starts_with("shows Window \"About\""), "{note}");
    h.get_by_label("Owner");
    h.get_by_label("Dialog");
    h.get_by_label(&note);

    // Control: an ordinary `Ui` form has no Dialog row.
    let rs = designer_file(
        "designer-dialog-row-ui",
        "login.rs",
        "pub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Name\");\n    }\n}\n",
    );
    let mut h = designer_harness_for(rs, false);
    designer_step(&mut h);
    h.get_by_label("Owner");
    assert!(h.query_by_label("Dialog").is_none());
    assert_eq!(designer::ui::dialog_note(&shown_form(&h)), None);
}

/// A free dialog through a helper call: the row says it is a free function and names the call.
#[test]
fn the_dialog_row_names_a_free_dialog_helper() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let rs = designer_file(
        "designer-dialog-row-free",
        "about.rs",
        "pub fn show(ctx: &egui::Context) {\n    crate::style::dialog(ctx, \"About\", None, 540.0, |ui| {\n        ui.label(\"Hello\");\n    });\n}\n",
    );
    let mut h = designer_harness_for(rs, false);
    designer_step(&mut h);
    let note = designer::ui::dialog_note(&shown_form(&h)).expect("a dialog");
    assert!(note.starts_with("free fn, shows Dialog "), "{note}");
    h.get_by_label("Dialog");
    h.get_by_label(&note);
}

/// Switching to Code makes a file open in code; clicking Form again clears that, so the file opens
/// in the form once more.
#[test]
fn clicking_form_again_forgets_the_code_preference() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let rs = designer_file(
        "designer-prefer-code",
        "login.rs",
        "pub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Name\");\n    }\n}\n",
    );
    let plain = rs.with_file_name("plain.rs");
    std::fs::write(&plain, "fn helper() {}\n").unwrap();
    let mut h = designer_harness_for(rs.clone(), false);
    // The Form segment is the "Form" next to Code (the toolbar's New Rust Form is "Form" too).
    let code = h.get_by_label("Code").rect();
    let form_segment = h
        .query_all_by_label("Form")
        .map(|n| n.rect())
        .min_by(|a, b| {
            a.center()
                .distance(code.center())
                .total_cmp(&b.center().distance(code.center()))
        })
        .unwrap();
    h.get_by_label("Code").click();
    h.run_steps(3);
    assert!(!h.state().form_showing());
    assert!(h.state().code_preferred.contains(&rs));
    // Reopened, it stays in code (the preference holds).
    h.state_mut().select_in_tree(plain.clone());
    h.state_mut().select_in_tree(rs.clone());
    h.run_steps(3);
    assert!(
        !h.state().form_showing(),
        "switched to Code, it opens in code"
    );

    designer_click(&mut h, form_segment.center());
    assert!(h.state().form_showing());
    assert!(
        !h.state().code_preferred.contains(&rs),
        "the toggle back to Form clears the preference"
    );
    h.state_mut().select_in_tree(plain);
    h.run_steps(2);
    assert!(!h.state().form_showing());
    h.state_mut().select_in_tree(rs);
    h.run_steps(3);
    assert!(h.state().form_showing(), "it opens in the form again");
}

/// The form shown has a control-flow container (`ImportOptions::extended`).
fn has_flow(form: &formust_model::Form) -> bool {
    let mut found = false;
    form.root.walk(&mut |n| {
        if let formust_model::NodeKind::Container { def, .. } = &n.kind
            && def.is_flow()
        {
            found = true;
        }
    });
    found
}

fn has_custom(form: &formust_model::Form) -> bool {
    let mut found = false;
    form.root.walk(&mut |n| {
        if matches!(n.kind, formust_model::NodeKind::Custom { .. }) {
            found = true;
        }
    });
    found
}

/// The "Extended import" switch is off by default; turning it on reads the file again with
/// control flow as nodes (drawn by the preview), and off again brings back the locked code. With
/// unsaved edits the switch is disabled.
#[test]
fn the_extended_import_switch_reloads_the_form() {
    let _ui = crate::prefs::tests::APPLY
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let rs = designer_file(
        "designer-extended",
        "login.rs",
        "pub struct Login { on: bool }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Before\");\n        if self.on {\n            ui.label(\"Inside\");\n        }\n    }\n}\n",
    );
    let mut h = designer_harness_for(rs, false);
    designer_step(&mut h);
    assert!(!h.state().designer.extended, "off by default");
    let form = shown_form(&h);
    assert!(
        !has_flow(&form) && has_custom(&form),
        "the if is locked code"
    );

    h.get_by_label("Extended import").click();
    h.run_steps(4);
    assert!(h.state().designer.extended);
    let form = shown_form(&h);
    assert!(
        has_flow(&form) && !has_custom(&form),
        "the if is a node now"
    );
    let inside = designer_node(h.state(), "Label \"Inside\"");
    designer_step(&mut h);
    assert!(
        h.state().designer.rects.contains_key(&inside),
        "the preview draws the label inside the flow node"
    );
    assert!(!h.state().designer.is_dirty(), "a fresh read, no edits");

    // With unsaved edits the switch is disabled, and the action is refused anyway.
    h.state_mut()
        .designer
        .add(
            designer::edit::Item::Widget(formust_model::WidgetId::Button),
            None,
        )
        .unwrap();
    h.run_steps(3);
    assert!(h.state().designer.is_dirty());
    assert!(
        h.get_by_label("Extended import")
            .accesskit_node()
            .is_disabled()
    );
    h.state_mut()
        .designer
        .actions
        .push(designer::ui::Action::Extended(false));
    h.state_mut().run_designer_actions();
    assert!(h.state().designer.extended, "refused with unsaved edits");
    h.state_mut().designer.undo();
    h.run_steps(3);
    assert!(!h.state().designer.is_dirty());

    h.get_by_label("Extended import").click();
    h.run_steps(4);
    assert!(!h.state().designer.extended);
    let form = shown_form(&h);
    assert!(
        !has_flow(&form) && has_custom(&form),
        "off again: locked code"
    );
}
