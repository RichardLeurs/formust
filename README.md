# Formust

A form builder for Rust with egui (Form + Rust). Formust is a macOS IDE that imports `.rs` files, lets you edit their egui forms in a visual designer and exports the form code to a `.frm.rs` file next to the source. The `.frm.rs` is a standalone copy of the source with the edited forms' functions replaced; the original is never changed.

Formust is based on the Pascoda Pascal IDE (https://github.com/RichardLeurs/pascoda), from which the IDE shell comes. Copyright © Richard Leurs 2026 | GNU GPL-3.0 License.

## How it works

1. **Import.** Open a `.rs` file (or a Cargo project). The importer (`crates/formust-rust`, built on `syn`) reads every `impl` function with a `&mut egui::Ui` parameter, `ui` or `update` methods taking a `Context`, and dialogs, as a form. A dialog is a function taking `&egui::Context` whose body has a top-level `Window::new(..).show(ctx, |ui| ..)`, or a top-level call of a plain path function with `ctx` first and a one-parameter closure last (for example `crate::style::dialog(ctx, "about", None, 540.0, |ui| ..)` in `crates/formust-app/src/about.rs`). The default import accepts free functions that are dialogs; a plain free `fn(ui)` still needs `free_functions: true`. A dialog keeps its name and signature on export; widgets go into its Window, or into the helper call (shown as a "Dialog" container that keeps the call head as written). Known egui calls become widgets and containers. Everything else (`let`, `if`, `for`, `match`, helper calls, macros, comments inside a statement) becomes a locked custom-code block, kept as the original text.
2. **Designer.** The `Form` toggle on a `.rs` or `.frm.rs` tab opens the designer. Opening a `.rs` file from the tree, File > Open, Open Recent, Open Folder or the command line shows the Form view by itself if the file has a form or dialog; otherwise, and after a jump from Problems, search or bookmarks, the code shows. A file you switched to Code stays in code for the session. It draws a live preview of real egui widgets, in light or dark mode. You add widgets from the palette (click or drag), move, reorder, delete, duplicate, copy and paste them, edit properties in a schema-driven inspector (colours have an optional dark value), add event handlers, and undo and redo (200 steps).
3. **Export.** Cmd+S writes `login.frm.rs` next to `login.rs`. The first save copies the whole original and replaces only the edited forms' functions in the copy (`copy.rs`: `standalone_base`, `update_frm`, `emit_fn`). Later saves patch the copy, so hand edits in it survive. The `.rs` is never written by an export and nothing is removed from it. Unedited nodes are written back verbatim. One header block at the top has one line per form; it stores a hash of the source form so Formust can tell when the source changed (a `| state X.y` suffix is added where the state struct or field has a non-default name). A form added later is appended to the copy as its impl. A source directly in `src/bin`, `examples`, `tests` or `benches` gets its copy in a `forms/` subfolder (`forms2` if `forms/main.rs` exists), because there `login.frm.rs` would break the build or become a second program. For a new widget with state, the copy gets `#[derive(Default)] pub struct <Owner>Form` after the owner struct, a private `form: <Owner>Form` field, and `form: Default::default()` in each struct literal of the owner that lists its fields (not `..rest` literals, not literals inside macros or other files or in the edited form's own function; not for unit or tuple owners). An `include!`-style `.frm.rs` from an earlier Formust is migrated to a copy on the next save.
4. **Use in code.** The "Use in code" tab says the copy stands alone: it compiles and runs without an `include!`, and the original needs no edit. It lists the handler stubs (`fn on_x_clicked(&mut self) {}`) you must write yourself, and gives an optional `#[path = "login.frm.rs"] mod login;` line if you want the copy in your crate. If the original changed since the export, a conflict prompt offers Keep .frm.rs, Re-import or Show diff; the copy still runs either way.

**Run Form.** Cmd+R (menu and toolbar "Run Form"), in the designer or in Code view, runs the form of any Rust file that contains one: the form under the caret in Code view, else the form selected in the designer, else the first. A file with no form runs the project (Output says "no form in this file, running the project"). It builds a generated package under `<target>/formust-run/<hash>/` from the project's `[dependencies]` and `[patch]`, using the unsaved designer edits and unsaved owner text, and runs it with eframe. Nothing is written to the project. While the form is unedited and has no `.frm.rs`, it runs the source; otherwise it runs the copy as Save would write it, including unsaved edits. The owner type needs `impl Default`. A form that is the `ui(ui, frame)` method of an `eframe::App` impl is called through the trait with the frame (`Kind::App`). A dialog is called every frame and shows only if the owner's own `open` flag allows it. A free dialog (`pub fn show(ctx)`) is runnable; a free function taking `ui` is refused. Only the generated package is built. If the code uses `crate::`, Output says that Run Form copies only this file. Tests: `crates/formust-app/src/run_form/tests.rs` and `crates/formust-build/tests/run_form.rs`.

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
| Cmd+J, Cmd+Shift+J | Wrap the selection (a node, or a shift-click range of siblings) in a new Horizontal row; unwrap the selected container (designer) |
| Cmd+E | Turn Fill (full width) on or off for the selected widget (designer) |
| Cmd-click, Shift-click | Add or remove a node from the selection; select a range of siblings (canvas and outline) (designer) |
| Delete, arrow keys | Delete or reorder the selected node or set (arrow keys: siblings only) (designer) |
| Escape | Clear the selection; during a resize drag, cancel it |

Layout in the designer (batch A, 2026-11-06; details in `docs/specs.md`):

- **Fill** is a width value, not a property. The inspector LAYOUT section, the context menu and Cmd+E set it. Written per widget: TextEdit `.desired_width(f32::INFINITY)`, ProgressBar `.desired_width(ui.available_width())`, ComboBox `.width(ui.available_width())`, Button and DragValue `ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], ..)`. A Slider has no Fill.
- **Wrap in row / Unwrap** (context menu, inspector buttons, Cmd+J / Cmd+Shift+J), one undo step each.
- **Anchor** (Left, Right, Top, Bottom, Center, Fill) shows an exact single-node `ui.with_layout(..)` wrapper as a value of the wrapped node. A Window has an `anchor` property (`egui::Align2::X, [dx, dy]`) with a 9-position picker.
- **Mouse resize:** handles on the right edge, bottom edge and corner of widgets that have a size property, with a live W x H tooltip. Esc cancels, Shift keeps the aspect, Alt snaps to 4 pt. The inspector shows "On canvas W x H pt" and Reset size.

Multi-select, columns, overlays and tab order (batch C, 2026-11-06; details in `docs/specs.md`):

- **Palette:** a filter box and six closable groups (Text, Buttons and choices, Input, Display, Layout, Panels); the list scrolls at 240 pt.
- **Multi-select:** Cmd-click, Shift-click, rubber band on the empty canvas. Copy, cut, paste, delete, duplicate, arrow keys, drag and wrap in row act on the set, one undo step each. The inspector shows "N selected", shared properties and "Mixed values".
- **Align and distribute** (context menu Align submenu, multi-selection inspector): same width or height, align left/right/centre/top/bottom (anchor wrapper), distribute evenly (`ui.add_space(n)`, 3 or more neighbours in one container, not in Grid, Columns or Sides), parent item spacing, make a column or row.
- **Columns** (`ui.columns(n, |cols| ..)`, imported only in the exact shape, count 1..=12) and **Sides** (`egui::Sides::new().show(ui, left, right)`). A form with bound widgets or handlers in both Sides closures does not compile, because both closures borrow `self`.
- **Overlays** (rulers, 8/16 pt grid, distance guides) are preview only and never exported. **Width presets** (Default, 320, 480, 640, 800, 1024 pt) and the Light/Dark switch are in the canvas toolbar; they are not saved across restarts.
- **Tab order mode** (canvas toolbar, third tab of the right panel): numbers, drag to reorder, star = Initial focus (`if ui.memory(|m| m.focused().is_none()) { response.request_focus(); }`). Not checked by hand in a real window.

Styling and behaviour in the designer (batch B, 2026-11-06; details in `docs/specs.md`):

- **Frame** is in the palette (`.fill`, `.stroke`, `.corner_radius`, `.inner_margin`, `.outer_margin`); a new one gets inner margin 8, radius 4 and a 1 pt stroke with light and dark colours. The preview draws it in light and dark mode.
- **Spacing and padding** (item spacing, button padding, indent) are statements at the start of a container's body, for example `ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);`. **Min/max size** on containers is `ui.set_min_width/height`, `ui.set_max_width/height`; on Button, TextEdit and DragValue it is `min_size`. Resize handles now also work on plain layout containers, not on Frame, Group or Indent.
- **Tooltip** (`.on_hover_text`), **Enabled** and **Visible** (`ui.add_enabled`, `ui.add_visible`, or `add_enabled_ui` / `scope` with `set_invisible` for widgets without a builder). A literal `false` previews as disabled or at 0.3 opacity with a dashed outline.
- **Text styling** (size, light/dark colour, bold, italic, underline, strikethrough, monospace) on Label, headings, Button and Link, written as an `egui::RichText::new(..)` chain.
- **Events:** `double_clicked`, `secondary_clicked`, `gained_focus`, `lost_focus` added.
- Inspector groups: PROPERTIES, TEXT STYLE, BEHAVIOUR, SPACING AND PADDING, SIZE.

Media, navigation and tables in the designer (batch D, 2026-11-06; details in `docs/specs.md`):

- **Image** (`ui.image`, `egui::Image::new/from_uri` with size, tint and rounding). The canvas draws a local PNG; anything else is a placeholder. The program must install image loaders.
- **Colour pickers** (`color_edit_button_srgba`, `color_edit_button_rgb`) and **RangeSlider** (two bound values).
- **MenuBar** (`egui::MenuBar::new().ui(..)`); a menu shows open on the canvas while it is selected.
- **Table** (`egui_extras::TableBuilder`): header, rows, cells and columns edited together. The program must depend on `egui_extras`; Formust does not add it. "Use in code" lists the requirement.
- **Tabs** built from `selectable_value` buttons (horizontal or vertical) with a `match` of pages; a click on a tab shows its page. Add tab works with integer values only.
- **Modal** (`egui::Modal::new(id).show(ctx, ..)`, root of a dialog) and more **Window** options (pivot, order, constrain, interactable, fade in/out).

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

The keychain test `the_provider_is_remembered_in_a_plain_text_file` opens a macOS dialog; skip it with `-- --skip the_provider_is_remembered_in_a_plain_text_file`. Last full run of the whole workspace (2026-11-06, keychain test skipped): 892 passed, 0 failed, 5 ignored. Coverage includes golden `.rs`/`.frm.rs` round trips, a test that compiles every exported golden as a standalone copy, `egui_kittest` tests for the designer (headless), a test that edits, exports and runs a form with `cargo run`, and 20 Run Form tests (3 with real cargo, including a headless run). Tests use `FormustTest-*` keychain services and temporary folders. `docs/` and `tests/` are git-ignored, so test folders under `crates/*/tests/` do not reach git.

## Upgrading from Pascoda

At start, `crates/formust-app/src/migrate.rs` copies `~/Library/Application Support/Pascoda` to `.../Formust` if `Formust` does not exist yet. It copies to a temporary name and renames it; it skips symlinks, never changes or removes the old folder, and prints failures to stderr. The API key is read from the Keychain service `Formust`, then the older `Pascoda`, then `Pascora`. A key found in an older service is copied into `Formust` and the old item stays. Deleting the key in Formust removes it from all three. Keys of OpenAI-compatible servers are also looked up under the old lowercase service `pascoda`. Expect one macOS keychain prompt on the first real launch; choose Always Allow.

## Limits and what is not verified

- Not checked by hand: a real eframe window, the About window and icon in Finder and Dock, native menus (New Rust Form, Copy/Paste), mouse drags (including the resize handles and their cursors), trackpad input, the designer context menus, colour picker popups, the corner grip, the toolbar and Cargo Options window, rust-analyzer in the running app, format on save, the Tools menu and Configure Tools, the rename box, migration of real user data, and the first-run keychain prompt.
- `packaging/bundle.sh` passed `bash -n` only. It has not been run end to end, and no codesign or notarization run was made.
- The preview differs from the running program in fonts (the IDE's fonts are used), a frozen Spinner, a ProgressBar that does not animate, and bindings that show empty values.
- rust-analyzer analyses a `.frm.rs` only when a crate includes it (phase 5 tested `include!`; the `mod` route for the new standalone copy was not tested), gave no useful completion there, and rust-analyzer's own diagnostics for it were not tested (they reach the Problems list only for files synced to the server).
- By default the importer recognises about half of the nodes in real egui code (50.2% by count, 30.6% by bytes on `egui_demo_lib` 0.37, measured before dialogs were added; they moved the ratios only slightly); the rest stays as locked custom blocks. Moving a widget past a `let` can break scope; `cargo check` reports it.
- Run Form: Cargo Options are ignored; only `[dependencies]` and `[patch]` are copied (no features, `target.*`, dev or build dependencies, lints); only the owner copy is built, so items from the rest of the crate (`mod`, `crate::`) fail with a note; free functions that take `ui` are refused (free dialogs run); old `formust-run` folders are not cleaned; the build is not offline. Not checked: a real Run Form window, the toolbar "Running..." state in a real window, Cmd+R via the native menu, cold eframe build time, dialog and legacy forms through cargo, the auto-open on a real window, the native Open dialog and the command-line path.
- Designer batch D (image, colour pickers, range slider, menu bar, table, tabs, modal, window options): tested headlessly only, not in a real window. Only a local PNG is drawn on the canvas. Tables and images need `egui_extras` in the program; not run through Run Form.
- Designer batch B (Frame, spacing and padding, min/max size, tooltip, enabled, visible, text styling, more events): tested headlessly only, not in a real window. No form-level spacing; spacing and size are on containers only; no visible-by-`if` form; the palette list is taller than its panel at the test window size.
- Designer batch A: a wrapped node keeps its original indentation (wrapped nodes are not re-indented). Fill and resize were tested headlessly with `egui_kittest`, not in a real window.
- Dialogs: the FORM panel has a "Dialog" row (headless-tested only). A `Window` inside `if`/`match`/`let`, `Area`/`Modal`/panels in non-`update` methods, `show` on another value and `&mut Context` are not read as forms.
- "Extended import" (designer top bar, off by default, session only, disabled while there are unsaved edits) reads control flow holding widgets and `let x = <widget>;` as nodes. Export still imports with default options. Tested headlessly only.
- Formust writes only `.frm.rs` files, not the `.rs` source. Only egui is supported. Handler stubs are not added to the copy; "Use in code" lists them. If the original changes after the export, the copy does not follow until you re-import.
- Clippy reports 24 style warnings that were there before.

## Origin and attribution

Formust started as a conversion of the Pascoda Pascal IDE. The Pascal, Lazarus and debugger parts are deleted; the editor, file tree, terminal, assistant, language server client and egui shell remain. The About window carries the product name, the copyright and the line "Based on the Pascoda Pascal IDE, GNU GPL-3.0 licence" (`about::ORIGIN`, shown under the copyright, outside the credits list, which a test keeps free of the word Pascal).

## License

GNU GPL-3.0, Copyright © Richard Leurs 2026.

Details and phase results: [docs/specs.md](docs/specs.md).
