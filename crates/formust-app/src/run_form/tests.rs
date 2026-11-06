//! Run Form: the generated manifest and program, the placement of their diagnostics, the choice of
//! form, and runs through cargo (headless, so no window opens): one shows an edit that is not saved,
//! one runs the copy after its source changed, one shows a free dialog, one starts a dialog open that
//! its flag keeps closed, one runs a free `fn(ui)` under the caret, and Run on a Rust file with a
//! form starts Run Form (in the designer or in code view), Run on a Rust file with no form runs the
//! project's own program. Also the cleaning of old program folders.

use super::*;
use formust_build::Event;
use formust_model::{NodeKind, PropValue, WidgetId};

const LOGIN: &str = "pub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Name\");\n        ui.text_edit_singleline(&mut self.name);\n    }\n}\n";

fn project_info(manifest_text: &str) -> ProjectInfo {
    ProjectInfo {
        manifest: PathBuf::from("/p/Cargo.toml"),
        manifest_text: manifest_text.into(),
        workspace: None,
        lock: Some("version = 3\n".into()),
    }
}

fn egui_project() -> ProjectInfo {
    project_info(
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\negui = \"0.37\"\n",
    )
}

/// The inputs of the form `Login::ui` in `/p/src/login.rs`, as the designer would give them.
fn inputs(owner_text: &str) -> Inputs {
    Inputs {
        owner: PathBuf::from("/p/src/login.rs"),
        owner_text: owner_text.into(),
        form_id: "Login::ui".into(),
        owner_type: "Login".into(),
        method: "ui".into(),
        kind: Kind::Ui,
    }
}

fn parse(text: &str) -> toml::Table {
    text.parse().expect("the manifest is TOML")
}

#[test]
fn the_manifest_makes_path_dependencies_absolute_and_keeps_the_edition() {
    let mut project = project_info(
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\negui = \"0.37\"\neframe = { version = \"0.37\", default-features = false, features = [\"glow\"] }\nmylib = { path = \"../mylib\" }\n\n[features]\nfoo = []\n",
    );
    project.manifest = PathBuf::from("/p/app/Cargo.toml");
    let text = manifest(&project, "formust-run-abc").unwrap();
    let table = parse(&text);
    assert_eq!(table["package"]["name"].as_str(), Some("formust-run-abc"));
    assert_eq!(table["package"]["edition"].as_str(), Some("2021"));
    let deps = table["dependencies"].as_table().unwrap();
    assert_eq!(deps["mylib"]["path"].as_str(), Some("/p/mylib"));
    assert_eq!(deps["eframe"]["features"][0].as_str(), Some("glow"));
    assert_eq!(deps["eframe"]["default-features"].as_bool(), Some(false));
    assert!(
        table.get("features").is_none(),
        "only the dependencies are copied"
    );
    assert!(
        table.contains_key("workspace"),
        "the generated package is its own workspace"
    );
}

#[test]
fn an_egui_only_project_gets_eframe_at_egui_version_and_neither_is_refused() {
    let table = parse(&manifest(&egui_project(), "formust-run-abc").unwrap());
    assert_eq!(
        table["dependencies"]["eframe"]["version"].as_str(),
        Some("0.37")
    );
    let none = project_info("[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2024\"\n");
    let error = manifest(&none, "formust-run-abc").unwrap_err();
    assert!(
        error.contains("needs `eframe` in [dependencies] of Cargo.toml"),
        "{error}"
    );
}

#[test]
fn inherited_dependencies_and_edition_come_from_the_workspace() {
    let project = ProjectInfo {
        manifest: PathBuf::from("/ws/app/Cargo.toml"),
        manifest_text: "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition.workspace = true\n\n[dependencies]\neframe = { workspace = true }\nserde = { workspace = true, features = [\"rc\"] }\nmylib = { workspace = true }\n".into(),
        workspace: Some((
            PathBuf::from("/ws"),
            "[workspace]\nmembers = [\"app\"]\n\n[workspace.package]\nedition = \"2024\"\n\n[workspace.dependencies]\neframe = \"0.37\"\nserde = { version = \"1\", features = [\"derive\"] }\nmylib = { path = \"crates/mylib\" }\n".into(),
        )),
        lock: None,
    };
    let table = parse(&manifest(&project, "formust-run-abc").unwrap());
    assert_eq!(table["package"]["edition"].as_str(), Some("2024"));
    assert_eq!(
        table["dependencies"]["eframe"]["version"].as_str(),
        Some("0.37")
    );
    assert_eq!(
        table["dependencies"]["serde"]["version"].as_str(),
        Some("1")
    );
    let features: Vec<&str> = table["dependencies"]["serde"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(features, ["derive", "rc"], "the local features are added");
    assert_eq!(
        table["dependencies"]["mylib"]["path"].as_str(),
        Some("/ws/crates/mylib"),
        "an inherited path is relative to the workspace"
    );
}

#[test]
fn a_ui_form_is_drawn_inside_a_central_panel() {
    let h = harness("Login", "ui", Kind::Ui, &[]);
    assert!(h.contains("egui::CentralPanel::default().show(ui"), "{h}");
    assert!(h.contains("let _ = app.owner.ui(ui);"), "{h}");
    assert!(h.contains("owner: super::Login,"), "{h}");
    assert!(
        h.contains("Runner { owner: super::Login::default() }"),
        "{h}"
    );
    assert!(h.contains("pub fn headless()"), "{h}");
    assert!(h.contains("fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame)"));
}

#[test]
fn a_dialog_is_called_with_the_context_every_frame() {
    let h = harness("Login", "show", Kind::Context, &[]);
    assert!(!h.contains("CentralPanel"), "{h}");
    assert!(h.contains("let _ = app.owner.show(ui.ctx());"), "{h}");
    assert!(h.contains("draw(self, ui);"), "{h}");
}

#[test]
fn a_legacy_update_is_called_with_its_frame_and_has_no_headless_drawing() {
    let h = harness("Login", "update", Kind::Legacy, &[]);
    assert!(
        h.contains("fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame)"),
        "{h}"
    );
    assert!(
        h.contains("let _ = self.owner.update(ui.ctx(), frame);"),
        "{h}"
    );
    assert!(!h.contains("fn draw("), "{h}");
    assert!(h.contains("        Vec::new()"), "{h}");
}

/// In the copy a legacy `update` is the `eframe::App` method `ui(ui, frame)`: called through the trait
/// with the frame. A dialog keeps its name.
#[test]
fn in_the_copy_a_legacy_update_is_the_apps_ui_and_a_dialog_stays() {
    assert_eq!(in_copy(Kind::Legacy, "update"), (Kind::App, "ui".into()));
    assert_eq!(
        in_copy(Kind::Context, "show"),
        (Kind::Context, "show".into())
    );
    assert_eq!(in_copy(Kind::Ui, "ui"), (Kind::Ui, "ui".into()));
    let h = harness("Login", "ui", Kind::App, &[]);
    assert!(
        h.contains("eframe::App::ui(&mut self.owner, ui, frame);"),
        "{h}"
    );
    assert!(!h.contains("fn draw("), "{h}");
    assert!(h.contains("        Vec::new()"), "{h}");
}

/// A form without an owner type that is neither a free dialog nor a free `fn(ui)` is refused rather
/// than given a program that doesn't compile.
#[test]
fn a_free_function_is_refused() {
    let mut free = inputs(LOGIN);
    free.owner_type = String::new();
    free.form_id = "::ui".into();
    let problems = package(&free, &egui_project()).unwrap_err();
    assert!(
        problems[0].message.contains("`ui` is a free function"),
        "{}",
        problems[0].message
    );
}

/// A free dialog (the app's own About window) runs as `owner::show(ctx)`. Its copy is the source
/// unchanged plus the harness; the source uses `crate::` paths, so the copy is noted for Output.
#[test]
fn a_free_dialog_runs_as_a_call_from_the_owner_module() {
    let mut free = inputs(include_str!("../about.rs"));
    free.owner_type = String::new();
    free.form_id = "::show".into();
    free.method = "show".into();
    free.kind = Kind::FreeDialog;
    let pkg = package(&free, &egui_project()).unwrap();
    assert!(pkg.owner.starts_with(include_str!("../about.rs")));
    assert!(pkg.owner.contains("let _ = super::show(ui.ctx());"));
    assert!(!pkg.owner.contains("owner:"), "a free dialog has no owner");
    assert!(
        crate_note(&free.owner_text).is_some(),
        "the About source uses crate:: paths"
    );
}

/// The harness of a free dialog: no owner, the function called with the `ctx` in the draw.
#[test]
fn a_free_dialog_harness_calls_the_function_with_the_context() {
    let h = harness("", "show", Kind::FreeDialog, &[]);
    assert!(h.contains("pub struct Runner;"), "{h}");
    assert!(h.contains("let _ = super::show(ui.ctx());"), "{h}");
    assert!(!h.contains("CentralPanel"), "{h}");
    assert!(h.contains("draw(self, ui);"), "{h}");
    assert!(h.contains("pub fn headless()"), "{h}");
}

/// A free function with no owner that is not called as a free dialog or a free `fn(ui)` is refused;
/// so is a free `ui` function that takes more than the `ui`, since the run has nothing to pass.
#[test]
fn a_free_function_that_takes_more_than_the_ui_is_refused() {
    let mut free = inputs(LOGIN);
    free.owner_type = String::new();
    free.form_id = "::ui".into();
    free.kind = Kind::Ui;
    let problems = package(&free, &egui_project()).unwrap_err();
    assert!(
        problems[0]
            .message
            .contains("`ui` is a free function that is neither"),
        "{}",
        problems[0].message
    );
    let mut two = inputs("pub fn row(ui: &mut egui::Ui, name: &str) {\n    ui.label(name);\n}\n");
    two.owner_type = String::new();
    two.form_id = "::row".into();
    two.method = "row".into();
    two.kind = Kind::FreeUi;
    let problems = package(&two, &egui_project()).unwrap_err();
    assert!(
        problems[0].message.contains(
            "a free function whose only parameter is the ui (`fn row(ui: &mut egui::Ui)`)"
        ),
        "{}",
        problems[0].message
    );
}

/// A free `fn form(ui: &mut egui::Ui)` runs as `owner::form(ui)` inside a `CentralPanel`, with no
/// owner value and nothing opened.
#[test]
fn a_free_ui_function_runs_as_a_call_inside_a_central_panel() {
    let h = harness("", "form", Kind::FreeUi, &[]);
    assert!(h.contains("pub struct Runner;"), "{h}");
    assert!(h.contains("egui::CentralPanel::default().show(ui"), "{h}");
    assert!(h.contains("let _ = super::form(ui);"), "{h}");
    assert!(h.contains("pub fn headless()"), "{h}");
    let text = "pub fn form(ui: &mut egui::Ui) {\n    ui.label(\"Free form\");\n}\n";
    let mut free = inputs(text);
    free.owner_type = String::new();
    free.form_id = "::form".into();
    free.method = "form".into();
    free.kind = Kind::FreeUi;
    let pkg = package(&free, &egui_project()).unwrap();
    assert!(pkg.owner.starts_with(text));
    assert!(pkg.owner.contains("let _ = super::form(ui);"));
    assert!(pkg.opened.is_empty());
    // As a form (with `free_functions`), a free `ui` function is this kind.
    let source = formust_rust::import_with(
        text,
        formust_rust::ImportOptions {
            free_functions: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(kind_of(&source.forms[0].form), Kind::FreeUi);
    assert_eq!(in_copy(Kind::FreeUi, "form"), (Kind::FreeUi, "form".into()));
}

/// A dialog that its owner's `open` flag keeps closed starts open: the run sets the flag before the
/// first frame, and Output says so.
#[test]
fn a_dialog_kept_closed_by_its_flag_starts_open() {
    let h = harness(
        "Login",
        "show",
        Kind::Context,
        &["open".into(), "shown".into()],
    );
    assert!(
        h.contains("let mut owner = super::Login::default();\n            owner.open = true;\n            owner.shown = true;\n            Runner { owner }"),
        "{h}"
    );
    let mut dialog = inputs(DIALOG_SOURCE);
    dialog.owner_type = "Dialog".into();
    dialog.method = "show".into();
    dialog.form_id = "Dialog::show".into();
    dialog.kind = Kind::Context;
    let pkg = package(&dialog, &egui_project()).unwrap();
    assert_eq!(pkg.opened, ["open"]);
    assert!(pkg.owner.contains("owner.open = true;"), "{}", pkg.owner);
    assert_eq!(
        open_note(&pkg.opened).as_deref(),
        Some(
            "the dialog starts open: Run Form sets `self.open` to true in the run (the project's code is not changed)."
        )
    );
    assert!(open_note(&[]).is_none());
    // A `ui` form opens nothing, even with the same fields.
    let mut ui = dialog.clone();
    ui.method = "ui".into();
    ui.kind = Kind::Ui;
    assert!(package(&ui, &egui_project()).unwrap().opened.is_empty());
}

/// A scratch `formust-run` folder with program folders `names`, each last run `days` ago.
fn run_root(name: &str, folders: &[(&str, u64)]) -> PathBuf {
    let root = crate::target_tmp()
        .join(format!("formust-run-clean-{name}"))
        .join(FOLDER);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let now = std::time::SystemTime::now();
    for (folder, days) in folders {
        let dir = root.join(folder);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/owner.rs"), "").unwrap();
        let file = std::fs::File::create(dir.join(STAMP)).unwrap();
        file.set_modified(now - std::time::Duration::from_secs(days * 24 * 60 * 60))
            .unwrap();
    }
    root
}

const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Old program folders go: those not run for longer than the age, and those beyond the count, the
/// oldest first. The folder of the run starting now always stays.
#[test]
fn old_program_folders_are_removed_by_age_and_count() {
    let keys = [
        "000000000000000a",
        "000000000000000b",
        "000000000000000c",
        "000000000000000d",
        "000000000000000e",
    ];
    let root = run_root(
        "age",
        &[
            (keys[0], 0),
            (keys[1], 1),
            (keys[2], 30),
            (keys[3], 2),
            (keys[4], 40),
        ],
    );
    let now = std::time::SystemTime::now();
    // By age: the 30- and 40-day-old ones; the 40-day-old one is the run starting now and stays.
    let removed = clean_old(&root, &root.join(keys[4]), now, 14 * DAY, 100);
    assert_eq!(removed, [root.join(keys[2])]);
    assert!(root.join(keys[4]).exists(), "the current run stays");
    // By count: keep 2 (the current one and the most recent other), so `d` (2 days) and `b` go.
    let removed = clean_old(&root, &root.join(keys[4]), now, 14 * DAY, 2);
    let mut removed: Vec<_> = removed.into_iter().collect();
    removed.sort();
    assert_eq!(removed, [root.join(keys[1]), root.join(keys[3])]);
    assert!(root.join(keys[0]).exists() && root.join(keys[4]).exists());
}

/// Cleaning touches nothing but key-named folders directly in a real `formust-run` folder: not a
/// file, not a folder with another name, not a link (nor what it points to), and nothing at all when
/// the folder given is not a `formust-run` folder.
#[test]
fn cleaning_never_removes_anything_but_old_program_folders() {
    let root = run_root("safe", &[("00000000000000aa", 60)]);
    let other = root.join("keep-me");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(root.join("0000000000000bbb"), "a file, not a folder").unwrap();
    std::fs::create_dir_all(root.join("00000000000000FF")).unwrap();
    // A link with a key's name to a folder outside: neither is removed.
    let outside = root.parent().unwrap().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious"), "x").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("00000000000000cc")).unwrap();
    let now = std::time::SystemTime::now() + 365 * DAY;
    // Not a `formust-run` folder: nothing happens.
    assert!(clean_old(root.parent().unwrap(), Path::new("/none"), now, DAY, 0).is_empty());
    assert!(root.join("00000000000000aa").exists());
    let removed = clean_old(&root, Path::new("/none"), now, DAY, 0);
    assert_eq!(removed, [root.join("00000000000000aa")]);
    assert!(other.exists());
    assert!(root.join("0000000000000bbb").exists());
    assert!(
        root.join("00000000000000FF").exists(),
        "upper case is no key (a different name: macOS folders ignore case)"
    );
    assert!(root.join("00000000000000cc").symlink_metadata().is_ok());
    assert!(outside.join("precious").exists());
    // A `formust-run` that is itself a link is left alone.
    let linked = root.parent().unwrap().join("linked");
    std::fs::create_dir_all(&linked).unwrap();
    let real = linked.join("real");
    std::fs::create_dir_all(real.join("00000000000000dd")).unwrap();
    let link = linked.join(FOLDER);
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(clean_old(&link, Path::new("/none"), now, DAY, 0).is_empty());
    assert!(real.join("00000000000000dd").exists());
}

/// Output says when the copy uses `crate::` paths, and nothing when it doesn't.
#[test]
fn a_copy_that_uses_the_crate_gets_a_line_in_output() {
    assert_eq!(
        crate_note("use crate::style::Palette;\n").as_deref(),
        Some(
            "this form uses code from elsewhere in the project (crate::…); Run Form copies only this file."
        )
    );
    assert!(crate_note("fn main() {}\nimpl A {}\n").is_none());
}

/// The form Run Form runs: in code view the one under the caret's line, else the designer's, else the
/// first. A span of `None` is a form read from the other file, which can't be under the caret.
#[test]
fn the_form_run_is_the_one_under_the_caret_else_the_designers_else_the_first() {
    let text = "fn a() {\n}\nfn b() {\n}\nfn c() {\n}\n";
    let ids: Vec<String> = ["X::a", "X::b", "X::c"].map(String::from).into();
    let spans = vec![Some(0..10), Some(11..22), None];
    // The caret on line 4 is inside `b`.
    assert_eq!(
        pick_form(&ids, &spans, text, Some("X::a"), Some(4)),
        Some(1),
        "the caret wins in code view"
    );
    // Off every function: the designer's form.
    assert_eq!(
        pick_form(&ids, &spans, text, Some("X::a"), Some(5)),
        Some(0)
    );
    assert_eq!(pick_form(&ids, &spans, text, Some("X::c"), None), Some(2));
    // The designer's form is not in this file: the first.
    assert_eq!(
        pick_form(&ids, &spans, text, Some("Other::z"), None),
        Some(0)
    );
    assert_eq!(pick_form(&ids, &spans, text, None, None), Some(0));
    assert_eq!(pick_form(&[], &[], text, None, None), None);
    // A form read from the other file is never under the caret.
    assert_eq!(pick_form(&ids, &spans, text, None, Some(6)), Some(0));
}

#[test]
fn the_main_program_runs_the_headless_frame_or_the_window() {
    let main = main_rs("Login::ui");
    assert!(main.contains("mod owner;"), "{main}");
    assert!(main.contains("FORMUST_RUN_FORM_TEST"), "{main}");
    assert!(main.contains("eframe::run_native("), "{main}");
    assert!(main.contains("\"Login::ui\""), "{main}");
    assert!(main.contains("owner::formust_run::app()"), "{main}");
}

#[test]
fn a_source_whose_type_has_no_default_is_refused_at_its_impl_line() {
    let owner = format!("// the login form\n{LOGIN}");
    let problems = package(&inputs(&owner), &egui_project()).unwrap_err();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].file, PathBuf::from("/p/src/login.rs"));
    assert_eq!(problems[0].line, 3, "the line of `impl Login`");
    assert_eq!(
        problems[0].message,
        "Run Form needs `impl Default for Login`: the window starts from `Login::default()`. Add `#[derive(Default)]` above `struct Login`, or write `impl Default for Login` by hand."
    );
}

#[test]
fn a_derive_or_a_written_default_satisfies_the_check() {
    let derived = LOGIN.replace("pub struct Login", "#[derive(Default)]\npub struct Login");
    assert!(package(&inputs(&derived), &egui_project()).is_ok());
    let written = format!(
        "{LOGIN}impl Default for Login {{\n    fn default() -> Self {{\n        Login {{ name: String::new() }}\n    }}\n}}\n"
    );
    assert!(package(&inputs(&written), &egui_project()).is_ok());
}

#[test]
fn an_unedited_form_of_the_source_runs_without_a_form_file() {
    let derived = LOGIN.replace("pub struct Login", "#[derive(Default)]\npub struct Login");
    let pkg = package(&inputs(&derived), &egui_project()).unwrap();
    assert!(
        pkg.owner.starts_with(&derived),
        "the source comes first, unchanged"
    );
    assert!(pkg.owner.contains("pub mod formust_run"));
}

/// A copy runs as it is: no `include!` and no other file, its own lines first, then the harness.
#[test]
fn a_copy_runs_on_its_own_and_keeps_its_lines() {
    let copy = "// formust 1 | login.rs | Login::ui | src-hash 00000000\n#[derive(Default)]\npub struct Login { name: String }\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Saved later\");\n    }\n}\n";
    let mut input = inputs(copy);
    input.owner = PathBuf::from("/p/src/login.frm.rs");
    let pkg = package(&input, &egui_project()).unwrap();
    assert!(
        pkg.owner.starts_with(copy),
        "the copy's own lines keep their numbers, so a diagnostic maps to the project"
    );
    assert!(!pkg.owner.contains("include!"), "{}", pkg.owner);
    assert!(pkg.owner.contains("app.owner.ui(ui)"));
}

#[test]
fn a_problem_in_a_copy_is_placed_in_the_project_file() {
    let dir = PathBuf::from("/t/formust-run/abc");
    let places = Places {
        dir: dir.clone(),
        canonical: dir.clone(),
        owner_real: PathBuf::from("/p/src/login.frm.rs"),
    };
    let at = |file: &str, line: u32| Diagnostic {
        file: PathBuf::from(file),
        line,
        col: 4,
        severity: Severity::Error,
        message: "x".into(),
    };
    let owner = places.real(at("/t/formust-run/abc/src/owner.rs", 5));
    assert_eq!(
        (owner.file.as_path(), owner.line),
        (Path::new("/p/src/login.frm.rs"), 5)
    );
    let manifest = places.real(at("/t/formust-run/abc/Cargo.toml", 9));
    assert_eq!(
        (manifest.file.as_path(), manifest.line),
        (Path::new("/p/src/login.frm.rs"), 0),
        "a message about the generated files has no line in the project"
    );
    let elsewhere = places.real(at("/other/lib.rs", 3));
    assert_eq!(elsewhere.file, PathBuf::from("/other/lib.rs"));
}

#[test]
fn the_program_folder_is_under_the_target_folder_and_stays_the_same_for_a_source() {
    let dir = dir_for(Path::new("/t"), Path::new("/p/src/login.rs"));
    assert!(dir.starts_with("/t/formust-run"), "{}", dir.display());
    assert_eq!(dir, dir_for(Path::new("/t"), Path::new("/p/src/login.rs")));
    assert_ne!(dir, dir_for(Path::new("/t"), Path::new("/p/src/other.rs")));
}

/// Each source's copy has its own package name, so cargo builds and runs each one's own binary; the
/// name is the folder's key, so the two stay together.
#[test]
fn each_source_has_its_own_package_name_as_its_folder() {
    let login = Path::new("/p/src/login.rs");
    let other = Path::new("/p/src/other.rs");
    let name = package_name(login);
    assert!(name.starts_with("formust-run-"), "{name}");
    assert_eq!(name, package_name(login), "the same source keeps its name");
    assert_ne!(name, package_name(other));
    let dir = dir_for(Path::new("/t"), login);
    assert_eq!(
        dir.file_name().unwrap().to_string_lossy(),
        &name["formust-run-".len()..]
    );
}

#[test]
fn the_form_kind_follows_the_form() {
    let source = formust_rust::import(LOGIN).unwrap();
    assert_eq!(kind_of(&source.forms[0].form), Kind::Ui);
    let dialog = formust_rust::import(
        "pub struct A;\nimpl A {\n    pub fn show(&mut self, ctx: &egui::Context) {\n        egui::Window::new(\"w\").show(ctx, |ui| { ui.label(\"x\"); });\n    }\n}\n",
    )
    .unwrap();
    assert_eq!(kind_of(&dialog.forms[0].form), Kind::Context);
    // A free dialog: no owner type, a `ctx` function.
    let free = formust_rust::import(
        "pub fn show(ctx: &egui::Context) {\n    egui::Window::new(\"w\").show(ctx, |ui| { ui.label(\"x\"); });\n}\n",
    )
    .unwrap();
    assert_eq!(free.forms[0].form.owner, "");
    assert_eq!(kind_of(&free.forms[0].form), Kind::FreeDialog);
    // An `eframe::App`'s `ui` takes the frame too.
    let app = formust_rust::import(
        "#[derive(Default)]\npub struct A;\nimpl eframe::App for A {\n    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {\n        ui.label(\"x\");\n    }\n}\n",
    )
    .unwrap();
    assert_eq!(kind_of(&app.forms[0].form), Kind::App);
}

// ---- through cargo -------------------------------------------------------------------------------

/// The version of `name` in the workspace's `Cargo.lock`, so the test projects build offline.
fn locked(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the workspace root");
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let needle = format!("name = \"{name}\"\nversion = \"");
    let start = lock.find(&needle).expect("in the lock") + needle.len();
    lock[start..].split('"').next().unwrap().to_string()
}

/// A fresh project folder under the test scratch folder, with the workspace's lock.
fn project_dir(name: &str) -> PathBuf {
    let dir = crate::target_tmp().join(format!("formust-run-form-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    std::fs::copy(root.join("Cargo.lock"), dir.join("Cargo.lock")).unwrap();
    dir
}

/// The project's `Cargo.toml`, with egui and eframe pinned to the workspace's versions.
fn egui_manifest(dir: &Path, name: &str) {
    let (egui, eframe) = (locked("egui"), locked("eframe"));
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\negui = \"={egui}\"\neframe = \"={eframe}\"\n\n[workspace]\n"
        ),
    )
    .unwrap();
}

/// The source of the form project: a whole form, which needs no other file.
const LOGIN_SOURCE: &str = "#[derive(Default)]\npub struct Login {\n    name: String,\n}\n\nimpl Login {\n    pub fn ui(&mut self, ui: &mut egui::Ui) {\n        ui.label(\"Name\");\n        ui.text_edit_singleline(&mut self.name);\n        if ui.button(\"Go\").clicked() {}\n    }\n}\n";

/// A project with a form in `login.rs` and its saved copy `login.frm.rs` (as Save writes it: the
/// whole source under its header), egui and eframe pinned.
fn form_project(name: &str) -> PathBuf {
    let dir = project_dir(name);
    egui_manifest(&dir, "login-app");
    std::fs::write(dir.join("src/main.rs"), "mod login;\n\nfn main() {}\n").unwrap();
    std::fs::write(dir.join("src/login.rs"), LOGIN_SOURCE).unwrap();
    let source = formust_rust::import(LOGIN_SOURCE).unwrap();
    let copy = formust_rust::update_frm(
        &formust_rust::parse_frm(LOGIN_SOURCE).unwrap(),
        &[(
            source.forms[0].form.clone(),
            Some(formust_rust::Header::for_source(
                "login.rs",
                &source.forms[0],
            )),
        )],
    );
    std::fs::write(dir.join("src/login.frm.rs"), copy).unwrap();
    dir
}

/// A dialog whose `open` flag is `false` by default: before the run set it, nothing showed.
const DIALOG_SOURCE: &str = "#[derive(Default)]\npub struct Dialog {\n    open: bool,\n    name: String,\n}\n\nimpl Dialog {\n    pub fn show(&mut self, ctx: &egui::Context) {\n        let mut open = self.open;\n        egui::Window::new(\"About\").open(&mut open).show(ctx, |ui| {\n            ui.label(\"Hello dialog\");\n            ui.text_edit_singleline(&mut self.name);\n        });\n        self.open = open;\n    }\n}\n";

/// A project with a free dialog in `dialog.rs`: `show(ctx)` opens a window with one label.
fn dialog_project(name: &str) -> PathBuf {
    let dir = project_dir(name);
    egui_manifest(&dir, "dialog-app");
    std::fs::write(dir.join("src/main.rs"), "mod dialog;\n\nfn main() {}\n").unwrap();
    std::fs::write(
        dir.join("src/dialog.rs"),
        "pub fn show(ctx: &egui::Context) {\n    egui::Window::new(\"Free\").show(ctx, |ui| {\n        ui.label(\"Free hello\");\n    });\n}\n",
    )
    .unwrap();
    dir
}

/// The shared target folder of the test builds, as the formust-build tests use it.
fn shared_target() -> PathBuf {
    crate::target_tmp().join("formust-targets/target")
}

/// Runs `request` to its end: the stdout lines and the outcome.
fn run_to_end(request: Request) -> (Vec<String>, formust_build::Outcome) {
    let build = formust_build::start(request, || {});
    let mut stdout = Vec::new();
    loop {
        match build
            .events()
            .recv_timeout(std::time::Duration::from_secs(900))
            .expect("cargo ends")
        {
            Event::Output {
                stream: formust_build::Stream::Stdout,
                line,
            } => stdout.push(line),
            Event::Finished(outcome) => return (stdout, outcome),
            _ => {}
        }
    }
}

/// The app with `file` open and its form shown in the designer.
fn designer_app(file: &Path) -> crate::App {
    let mut app = crate::App::bare(Some(file.to_path_buf()));
    app.sync_designer(true);
    app.show_form();
    assert!(app.form_showing(), "the form shows");
    app
}

/// The headless run of the form with an edit that is not saved: the edited label is what the frame
/// draws. The edit goes through the designer and is not exported, so the form file on disk keeps "Name".
#[test]
fn the_headless_run_draws_an_unsaved_designer_edit() {
    let dir = form_project("headless");
    let file = dir.join("src/login.rs");
    let mut app = designer_app(&file);
    let form = app.designer.current().unwrap().form().clone();
    let mut label = None;
    form.root.walk(&mut |n| {
        if let NodeKind::Widget {
            def: WidgetId::Label,
            ..
        } = &n.kind
        {
            label.get_or_insert(n.id);
        }
    });
    let label = label.expect("the form has a label");
    app.designer
        .edit(None, |f| {
            f.set_prop(label, "text", PropValue::Text("Saved later".into()))
        })
        .unwrap();
    assert!(app.designer.dirty_for(&file), "the edit is unsaved");

    let inputs = app.run_form_inputs().unwrap();
    let mut plan = prepare(inputs, Some(shared_target())).unwrap();
    plan.request.cargo_args.push("--offline".into());
    plan.request
        .env
        .push(("FORMUST_RUN_FORM_TEST".into(), "1".into()));
    let (stdout, outcome) = run_to_end(plan.request);
    assert!(
        outcome.result.success,
        "the program didn't build:\n{}",
        outcome.result.raw_output
    );
    assert_eq!(outcome.code, Some(0), "{stdout:?}");
    let drawn = stdout
        .iter()
        .find(|l| l.starts_with("drawn: "))
        .unwrap_or_else(|| panic!("no drawn line: {stdout:?}"));
    assert!(drawn.contains("Saved later"), "{drawn}");
    assert!(
        !drawn.contains("Name"),
        "the saved text is replaced: {drawn}"
    );
    assert!(drawn.contains("Go"), "{drawn}");
    let on_disk = std::fs::read_to_string(dir.join("src/login.frm.rs")).unwrap();
    assert!(on_disk.contains("\"Name\""), "the form file is not written");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        LOGIN_SOURCE,
        "the source is not written"
    );
    assert!(
        !dir.join("src/formust-run").exists(),
        "nothing is written in the project"
    );
}

/// The source changes after the copy was saved: the designer marks the conflict (Save would ask),
/// but the copy still runs on its own, and what runs is the copy, not the changed source.
#[test]
fn a_changed_source_is_a_conflict_but_the_copy_still_runs() {
    let dir = form_project("changed");
    let file = dir.join("src/login.rs");
    std::fs::write(
        &file,
        LOGIN_SOURCE.replace("ui.label(\"Name\");", "ui.label(\"Only in the source\");"),
    )
    .unwrap();
    let app = designer_app(&file);
    let shown = app.designer.current().unwrap();
    assert_eq!(shown.origin, Some(formust_rust::Origin::Frm));
    assert!(
        matches!(shown.conflict, Some(formust_rust::Conflict::Changed { .. })),
        "{:?}",
        shown.conflict
    );
    let inputs = app.run_form_inputs().unwrap();
    assert_eq!(inputs.owner, dir.join("src/login.frm.rs"), "the copy runs");
    assert!(!inputs.owner_text.contains("include!"));
    let mut plan = prepare(inputs, Some(shared_target())).unwrap();
    plan.request.cargo_args.push("--offline".into());
    plan.request
        .env
        .push(("FORMUST_RUN_FORM_TEST".into(), "1".into()));
    let (stdout, outcome) = run_to_end(plan.request);
    assert!(
        outcome.result.success,
        "the copy didn't build on its own:\n{}",
        outcome.result.raw_output
    );
    let drawn = stdout
        .iter()
        .find(|l| l.starts_with("drawn: "))
        .unwrap_or_else(|| panic!("no drawn line: {stdout:?}"));
    assert!(
        drawn.contains("Name") && !drawn.contains("Only in the source"),
        "{drawn}"
    );
    // Saving an edit now stops at the conflict prompt; nothing is written.
    let mut app = app;
    let copy_before = std::fs::read_to_string(dir.join("src/login.frm.rs")).unwrap();
    app.designer
        .edit(None, |f| {
            let n = f.new_widget(WidgetId::Separator);
            f.insert(f.root.id, 0, n)
        })
        .unwrap();
    let disk = |p: &Path| std::fs::read_to_string(p).ok();
    let out = crate::designer::export::export(
        app.designer.loaded.as_mut().unwrap(),
        &disk,
        &|p| p.exists(),
        crate::designer::export::Decisions::default(),
    );
    assert_eq!(
        out,
        crate::designer::export::Outcome::Ask(crate::designer::export::Prompt::SourceChanged {
            form: "Login::ui".into()
        })
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("src/login.frm.rs")).unwrap(),
        copy_before
    );
}

/// Run while the designer shows a form starts Run Form: a copy is prepared in the target folder, and
/// the build of that copy is the run in flight. Stop ends it at once.
#[test]
fn run_in_the_designer_starts_run_form() {
    let dir = form_project("start");
    let file = dir.join("src/login.rs");
    let mut app = designer_app(&file);
    let ctx = egui::Context::default();
    app.run_active(&ctx);
    assert!(app.form_prep.is_some(), "the run is being prepared");
    let started = std::time::Instant::now();
    while app.building.is_none() && started.elapsed() < std::time::Duration::from_secs(120) {
        app.poll_form_prep(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let session = app.building.as_ref().expect("the run started");
    assert_eq!(session.action, Action::Run);
    assert!(
        session.manifest.to_string_lossy().contains("/formust-run/"),
        "{}",
        session.manifest.display()
    );
    assert!(
        app.console.starts_with("Run Form: Login::ui in a window"),
        "{}",
        app.console
    );
    assert!(
        app.console
            .contains("Only its generated package is built, in ")
            && app.console.contains("not the project's binary"),
        "{}",
        app.console
    );
    app.stop_cargo();
    let started = std::time::Instant::now();
    while app.building.is_some() && started.elapsed() < std::time::Duration::from_secs(120) {
        app.poll_build(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.building.is_none(), "Stop ends the run");
    assert!(app.console.contains("Run stopped."), "{}", app.console);
}

/// Run in code view is the project's own run: `cargo run` of its binary, with its output in Output.
#[test]
fn run_in_code_view_runs_the_project() {
    let dir = project_dir("code");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"plain-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    println!(\"plain code run\");\n}\n").unwrap();
    let mut app = crate::App::bare(Some(file.clone()));
    app.show_code();
    let ctx = egui::Context::default();
    app.run_active(&ctx);
    assert!(
        app.form_prep.is_none(),
        "no form is prepared for a file without one"
    );
    assert!(
        app.console.starts_with(&format!("{NO_FORM}\n")),
        "Output says why: {}",
        app.console
    );
    let session = app.building.as_ref().expect("the project's run started");
    assert_eq!(session.manifest.file_name().unwrap(), "Cargo.toml");
    assert!(!session.manifest.to_string_lossy().contains("formust-run/"));
    let started = std::time::Instant::now();
    while app.building.is_some() && started.elapsed() < std::time::Duration::from_secs(300) {
        app.poll_build(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.building.is_none(), "the run ends");
    assert!(app.console.contains("plain code run"), "{}", app.console);
    assert!(app.console.contains("exit code 0"), "{}", app.console);
    assert!(!app.console.contains("Run Form"), "{}", app.console);
}

/// Run in code view on a Rust file with a form starts Run Form, not the project's program: the
/// designer reads the file for it, and the copy is what cargo builds.
#[test]
fn run_in_code_view_with_a_form_starts_run_form() {
    let dir = form_project("code-form");
    let file = dir.join("src/login.rs");
    let mut app = crate::App::bare(Some(file));
    app.show_code();
    assert!(!app.form_showing(), "the code shows");
    let ctx = egui::Context::default();
    app.run_active(&ctx);
    assert!(app.form_prep.is_some(), "the run is being prepared");
    assert!(
        app.console.starts_with("Preparing Run Form for Login::ui"),
        "{}",
        app.console
    );
    let started = std::time::Instant::now();
    while app.building.is_none() && started.elapsed() < std::time::Duration::from_secs(120) {
        app.poll_form_prep(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let session = app.building.as_ref().expect("the run started");
    assert_eq!(session.action, Action::Run);
    assert!(
        session.manifest.to_string_lossy().contains("/formust-run/"),
        "{}",
        session.manifest.display()
    );
    app.stop_cargo();
    let started = std::time::Instant::now();
    while app.building.is_some() && started.elapsed() < std::time::Duration::from_secs(120) {
        app.poll_build(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.building.is_none(), "Stop ends the run");
}

/// A free dialog runs as `show(ctx)` from its copy: the designer reads it as a form with no owner,
/// and one headless frame draws its window's text.
#[test]
fn a_free_dialog_runs_through_cargo_headless() {
    let dir = dialog_project("free");
    let file = dir.join("src/dialog.rs");
    let app = designer_app(&file);
    let inputs = app.run_form_inputs().unwrap();
    assert_eq!(inputs.kind, Kind::FreeDialog);
    assert_eq!(inputs.form_id, "::show");
    let mut plan = prepare(inputs, Some(shared_target())).unwrap();
    plan.request.cargo_args.push("--offline".into());
    plan.request
        .env
        .push(("FORMUST_RUN_FORM_TEST".into(), "1".into()));
    let (stdout, outcome) = run_to_end(plan.request);
    assert!(
        outcome.result.success,
        "the program didn't build:\n{}",
        outcome.result.raw_output
    );
    let drawn = stdout
        .iter()
        .find(|l| l.starts_with("drawn: "))
        .unwrap_or_else(|| panic!("no drawn line: {stdout:?}"));
    assert!(drawn.contains("Free hello"), "{drawn}");
    assert!(
        !dir.join("src/formust-run").exists(),
        "nothing is written in the project"
    );
}

/// A dialog that its `open` flag keeps closed by default starts open in the run: one headless frame
/// draws its window's text.
#[test]
fn a_closed_dialog_starts_open_through_cargo_headless() {
    let dir = project_dir("closed-dialog");
    egui_manifest(&dir, "closed-dialog-app");
    std::fs::write(dir.join("src/main.rs"), "mod dialog;\n\nfn main() {}\n").unwrap();
    let file = dir.join("src/dialog.rs");
    std::fs::write(&file, DIALOG_SOURCE).unwrap();
    let app = designer_app(&file);
    let inputs = app.run_form_inputs().unwrap();
    assert_eq!(inputs.kind, Kind::Context);
    assert_eq!(inputs.form_id, "Dialog::show");
    let mut plan = prepare(inputs, Some(shared_target())).unwrap();
    assert!(
        plan.note
            .as_deref()
            .is_some_and(|n| n.contains("the dialog starts open")),
        "{:?}",
        plan.note
    );
    plan.request.cargo_args.push("--offline".into());
    plan.request
        .env
        .push(("FORMUST_RUN_FORM_TEST".into(), "1".into()));
    let (stdout, outcome) = run_to_end(plan.request);
    assert!(
        outcome.result.success,
        "the program didn't build:\n{}",
        outcome.result.raw_output
    );
    let drawn = stdout
        .iter()
        .find(|l| l.starts_with("drawn: "))
        .unwrap_or_else(|| panic!("no drawn line: {stdout:?}"));
    assert!(drawn.contains("Hello dialog"), "{drawn}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        DIALOG_SOURCE,
        "the source is not written"
    );
}

/// A file with a free `fn form(ui: &mut egui::Ui)` and no form: in code view with the caret in that
/// function, Run runs it (from the open, unsaved text), and one headless frame draws it.
#[test]
fn a_free_ui_function_under_the_caret_runs_through_cargo_headless() {
    let dir = project_dir("free-ui");
    egui_manifest(&dir, "free-ui-app");
    std::fs::write(dir.join("src/main.rs"), "mod panel;\n\nfn main() {}\n").unwrap();
    let file = dir.join("src/panel.rs");
    let text = "pub fn helper(ui: &mut egui::Ui, n: u32) {\n    ui.label(n.to_string());\n}\n\npub fn form(ui: &mut egui::Ui) {\n    ui.label(\"Free ui saved\");\n    helper(ui, 7);\n}\n";
    std::fs::write(&file, text).unwrap();
    let mut app = crate::App::bare(Some(file.clone()));
    app.show_code();
    app.sync_designer(true);
    // Off the function: no free `ui` function to run.
    app.cursor_pos = (1, 1);
    assert!(
        app.free_ui_at_caret().is_none(),
        "`helper` takes more than the ui"
    );
    app.cursor_pos = (6, 5);
    assert_eq!(
        app.free_ui_at_caret().map(|f| f.name).as_deref(),
        Some("form")
    );
    // An unsaved edit of the open text is what runs.
    let doc = app.docs.get_mut(app.active).unwrap();
    doc.text = doc.text.replace("Free ui saved", "Free ui unsaved");
    let inputs = app.run_form_inputs().unwrap();
    assert_eq!(inputs.kind, Kind::FreeUi);
    assert_eq!(inputs.form_id, "::form");
    assert_eq!(inputs.owner, file);
    let mut plan = prepare(inputs, Some(shared_target())).unwrap();
    plan.request.cargo_args.push("--offline".into());
    plan.request
        .env
        .push(("FORMUST_RUN_FORM_TEST".into(), "1".into()));
    let (stdout, outcome) = run_to_end(plan.request);
    assert!(
        outcome.result.success,
        "the program didn't build:\n{}",
        outcome.result.raw_output
    );
    let drawn = stdout
        .iter()
        .find(|l| l.starts_with("drawn: "))
        .unwrap_or_else(|| panic!("no drawn line: {stdout:?}"));
    assert!(
        drawn.contains("Free ui unsaved") && drawn.contains('7'),
        "{drawn}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        text,
        "the source is not written"
    );

    // Run itself starts Run Form for it, not the project's program.
    let ctx = egui::Context::default();
    app.run_active(&ctx);
    assert!(app.form_prep.is_some(), "the run is being prepared");
    assert!(
        app.console.starts_with("Preparing Run Form for ::form"),
        "{}",
        app.console
    );
    let started = std::time::Instant::now();
    while app.building.is_none() && started.elapsed() < std::time::Duration::from_secs(120) {
        app.poll_form_prep(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.building.is_some(), "the run started: {}", app.console);
    app.stop_cargo();
    let started = std::time::Instant::now();
    while app.building.is_some() && started.elapsed() < std::time::Duration::from_secs(120) {
        app.poll_build(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.building.is_none(), "Stop ends the run");
}
