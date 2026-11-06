# Formust

A form builder for Rust with egui (Form + Rust). Formust is a macOS IDE that imports `.rs` files, lets you edit their egui forms in a visual designer and exports the form code to a `.frm.rs` file next to the source. A program pulls that file in with `include!("login.frm.rs");`.

Formust is based on the Pascoda Pascal IDE (https://github.com/RichardLeurs/pascoda), from which the IDE shell comes. Copyright © Richard Leurs 2026 | GNU GPL-3.0 License.

## How it works

1. **Import.** Open a `.rs` file (or a Cargo project). The importer (`crates/formust-rust`, built on `syn`) reads every `impl` function with a `&mut egui::Ui` parameter, `ui` or `update` methods taking a `Context`, and dialogs, as a form. A dialog is a function taking `&egui::Context` whose body has a top-level `Window::new(..).show(ctx, |ui| ..)`, or a top-level call of a plain path function with `ctx` first and a one-parameter closure last (for example `crate::style::dialog(ctx, "about", None, 540.0, |ui| ..)` in `crates/formust-app/src/about.rs`). The default import accepts free functions that are dialogs; a plain free `fn(ui)` still needs `free_functions: true`. A dialog keeps its name and signature on export; widgets go into its Window, or into the helper call (shown as a "Dialog" container that keeps the call head as written). Known egui calls become widgets and containers. Everything else (`let`, `if`, `for`, `match`, helper calls, macros, comments inside a statement) becomes a locked custom-code block, kept as the original text.
2. **Designer.** The `Form` toggle on a `.rs` or `.frm.rs` tab opens the designer. Opening a `.rs` file from the tree, File > Open, Open Recent, Open Folder or the command line shows the Form view by itself if the file has a form or dialog; otherwise, and after a jump from Problems, search or bookmarks, the code shows. A file you switched to Code stays in code for the session. It draws a live preview of real egui widgets, in light or dark mode. You add widgets from the palette (click or drag), move, reorder, delete, duplicate, copy and paste them, edit properties in a schema-driven inspector (colours have an optional dark value), add event handlers, and undo and redo (200 steps).
3. **Export.** Cmd+S writes `login.frm.rs` next to `login.rs`. The `.rs` is not written by an export. Unedited nodes are written back verbatim. The header line stores a hash of the source form so Formust can tell when the source changed. A source directly in `src/bin`, `examples`, `tests` or `benches` gets its form in a `forms/` subfolder, because Cargo rejects a dot in those crate names.
4. **Use in code.** The "Use in code" tab lists the three edits to make in `login.rs`: delete the original `fn ui`, add a `form: LoginForm` field if the `.frm.rs` generates that struct, and add `include!("login.frm.rs");`. The form then compiles into the program like any other source.

**Run Form.** Cmd+R (menu and toolbar "Run Form"), in the designer or in Code view, runs the form of any Rust file that contains one: the form under the caret in Code view, else the form selected in the designer, else the first. A file with no form runs the project (Output says "no form in this file, running the project"). It builds a generated package under `<target>/formust-run/<hash>/` from the project's `[dependencies]` and `[patch]`, using the unsaved designer edits and unsaved owner text, and runs it with eframe. Nothing is written to the project. The owner type needs `impl Default`, and an edited form needs the source to `include!` the form file. A dialog is called every frame and shows only if the owner's own `open` flag allows it. A free dialog (`pub fn show(ctx)`) is runnable; a free function taking `ui` is refused. Only the generated package is built. If the code uses `crate::`, Output says that Run Form copies only this file. Tests: `crates/formust-app/src/run_form/tests.rs` and `crates/formust-build/tests/run_form.rs`.

## Build and run

Requires macOS on Apple silicon (ARM64) and a Rust toolchain. rust-analyzer comes from rustup: `rustup component add rust-analyzer`.

```
cargo run -p formust-app          # debug build of the IDE (binary: formust)
cargo build --release -p formust-app
packaging/bundle.sh               # Formust.app and .dmg per CPU, written to ~/Downloads/Formust
```

`packaging/bundle.sh` reads name, bundle id, copyright and minimum macOS from `[workspace.metadata.formust]` in the root `Cargo.toml`. By default it makes `Formust-aarch64-darwin`, `Formust-x86_64-darwin` and `Formust-universal-darwin`; `ARCHES="aarch64-darwin"` limits it and `NO_DMG=1` skips the disk images. `packaging/make-dmg.sh` and `packaging/make-icon.sh` are helpers; the icon `packaging/Formust.icns` is built from `packaging/formust-icon-1024.png`. The default signature is ad hoc (runs on the Mac that built it). `CODESIGN_IDENTITY` and `NOTARY_PROFILE` enable signing and notarization. The bundle id `com.richardleurs.formust` is a placeholder; change it in `[workspace.metadata.formust]`.

Settings: the environment variables `FORMUST_CLAUDE_BIN` and `FORMUST_CODEX_BIN` name the Claude Code and Codex binaries for the assistant. `FORMUST_NAME` and `FORMUST_COPYRIGHT` are set by `crates/formust-app/build.rs` from the workspace metadata. The old `PASCODA_*` names are not read.

## The IDE

A code editor with Rust highlighting, a file tree (`.rs`, `.frm.rs`, `toml`, `md`), tabs, find, bookmarks, a terminal, Problems and Output tabs, an AI assistant (with tools such as `read_rust_form`), a Tools menu (Line Count, Git Status, Open Project Folder; macros `$(File)`, `$(Dir)`, `$(Name)`, `$(ProjectDir)`, `$(Selection)`, `$(Line)`), and rust-analyzer for diagnostics, hover, go-to-definition and formatting (rustfmt). Renaming, moving or deleting a `.rs` file does the same to its `.frm.rs`.

Shortcuts:

| Keys | Action |
|---|---|
| Cmd+S | Save; in the designer, export the `.frm.rs` |
| Cmd+Shift+B | Cargo check |
| Cmd+B | Cargo build |
| Cmd+R | Run Form (any Rust file with a form, also in code view; a file without a form runs the project) |
| Cmd+. | Stop |
| Cmd+D | Duplicate the selected node (designer) |
| Cmd+C, Cmd+X, Cmd+V | Copy, cut, paste nodes (designer) |
| Cmd+Z, Cmd+Shift+Z | Undo, redo |
| Delete, arrow keys | Delete or reorder the selected node (designer) |
| Escape | Clear the selection |

The Cargo Options window (package, binary, release, arguments) keeps its settings for the session only.

## Workspace layout

| Crate | Purpose |
|---|---|
| `crates/formust-model` | Form model and widget registry; no UI dependency |
| `crates/formust-rust` | Importer, `.frm.rs` exporter, file pairing rules |
| `crates/formust-build` | Cargo driver, build types, tools; maps errors in `.rs` and `.frm.rs` to file and line |
| `crates/formust-ai` | Assistant: providers, keychain, prompt, tools |
| `crates/formust-lsp` | rust-analyzer client |
| `crates/formust-app` | The IDE (binary `formust`), including `designer/` |
| `packaging/` | `bundle.sh`, `make-dmg.sh`, `make-icon.sh`, icon files |
| `docs/specs.md` | Decisions, conversion plan and phase results |

## Tests

```
cargo check --workspace
cargo test --offline --workspace --no-fail-fast
cargo test --offline -p formust-model -p formust-rust
```

The keychain test `the_provider_is_remembered_in_a_plain_text_file` opens a macOS dialog; skip it with `-- --skip the_provider_is_remembered_in_a_plain_text_file`. Last full run (2026-11-05): 809 passed, 0 failed, 5 ignored. Coverage includes golden `.rs`/`.frm.rs` round trips, a test that compiles every exported golden, `egui_kittest` tests for the designer (headless), a test that edits, exports and runs a form with `cargo run`, and 20 Run Form tests (3 with real cargo, including a headless run). Tests use `FormustTest-*` keychain services and temporary folders. `docs/` and `tests/` are git-ignored, so test folders under `crates/*/tests/` do not reach git.

## Upgrading from Pascoda

At start, `crates/formust-app/src/migrate.rs` copies `~/Library/Application Support/Pascoda` to `.../Formust` if `Formust` does not exist yet. It copies to a temporary name and renames it; it skips symlinks, never changes or removes the old folder, and prints failures to stderr. The API key is read from the Keychain service `Formust`, then the older `Pascoda`, then `Pascora`. A key found in an older service is copied into `Formust` and the old item stays. Deleting the key in Formust removes it from all three. Keys of OpenAI-compatible servers are also looked up under the old lowercase service `pascoda`. Expect one macOS keychain prompt on the first real launch; choose Always Allow.

## Limits and what is not verified

- Not checked by hand: a real eframe window, the About window and icon in Finder and Dock, native menus (New Rust Form, Copy/Paste), mouse drags, trackpad input, colour picker popups, the corner grip, the toolbar and Cargo Options window, rust-analyzer in the running app, format on save, the Tools menu and Configure Tools, the rename box, migration of real user data, and the first-run keychain prompt.
- `packaging/bundle.sh` passed `bash -n` only. It has not been run end to end, and no codesign or notarization run was made.
- The preview differs from the running program in fonts (the IDE's fonts are used), a frozen Spinner, a ProgressBar that does not animate, and bindings that show empty values.
- rust-analyzer gives no useful completion inside an included `.frm.rs`, does not analyse a `.frm.rs` that no crate `include!`s, and its own diagnostics are not in the Problems list (cargo diagnostics are).
- By default the importer recognises about half of the nodes in real egui code (50.2% by count, 30.6% by bytes on `egui_demo_lib` 0.37, measured before dialogs were added; they moved the ratios only slightly); the rest stays as locked custom blocks. Moving a widget past a `let` can break scope; `cargo check` reports it.
- Run Form: Cargo Options are ignored; only `[dependencies]` and `[patch]` are copied (no features, `target.*`, dev or build dependencies, lints); only the owner copy is built, so items from the rest of the crate (`mod`, `crate::`) fail with a note; free functions that take `ui` are refused (free dialogs run); old `formust-run` folders are not cleaned; the build is not offline. Not checked: a real Run Form window, the toolbar "Running..." state in a real window, Cmd+R via the native menu, cold eframe build time, dialog and legacy forms through cargo, the auto-open on a real window, the native Open dialog and the command-line path.
- Dialogs: the FORM panel has no "dialog" row. A `Window` inside `if`/`match`/`let`, `Area`/`Modal`/panels in non-`update` methods, `show` on another value and `&mut Context` are not read as forms.
- Formust writes only `.frm.rs` files, not the `.rs` source. Only egui is supported.
- Clippy reports 24 style warnings that were there before.

## Origin and attribution

Formust started as a conversion of the Pascoda Pascal IDE. The Pascal, Lazarus and debugger parts are deleted; the editor, file tree, terminal, assistant, language server client and egui shell remain. The About window carries the product name, the copyright and the line "Based on the Pascoda Pascal IDE, GNU GPL-3.0 licence" (`about::ORIGIN`, shown under the copyright, outside the credits list, which a test keeps free of the word Pascal).

## License

GNU GPL-3.0, Copyright © Richard Leurs 2026.

Details and phase results: [docs/specs.md](docs/specs.md).
