# Formust

**Form + Rust.** A macOS IDE for designing egui forms and dialogs for Rust programs.

Open a `.rs` file, edit its forms in a visual designer, and get plain egui code back. Your own code stays byte for byte as you wrote it.

## Highlights

- **Import real code.** Reads `.rs` files with `syn`: every function taking `&mut egui::Ui`, `update` methods and dialogs (`Window`, `Modal`) become forms. Code it does not recognise is kept as locked custom blocks.
- **Visual designer.** A live canvas of real egui widgets, a palette with filter and groups, an outline, and an inspector driven by the widget's properties. Drag and drop, multi-select, copy and paste, undo and redo, mouse resize, align and distribute, rulers and grid, tab order, light and dark preview.
- **Rich widget set.** Labels, buttons, inputs, sliders, combo boxes, images, colour pickers, tables, tabs, menu bars, columns, frames, panels, windows and modals, with spacing, sizes, tooltips, text styling and events.
- **Egui code out.** Save writes a standalone copy of your source with only the edited forms replaced. A separate, confirmed "Save into .rs" action writes straight into the source, with conflict checks and a diff.
- **Run Form.** Cmd+R builds and runs the real form from a generated temporary package; nothing is written into your project. Dialogs start open.
- **Live Run.** A toggle that rebuilds and relaunches the form after each edit and keeps the last working window if a build fails. The window has a Light/Dark selector in its title bar.
- **A real Rust IDE.** Code editor with highlighting, file tree, terminal, rust-analyzer (diagnostics, hover, go to definition, format), cargo check/build/run, Problems and Output, and an AI assistant with form-aware tools.

## Build and run

Requires macOS on Apple silicon and a Rust toolchain. rust-analyzer comes from rustup: `rustup component add rust-analyzer`.

```
cargo run -p formust-app            # the IDE (binary: formust)
cargo build --release -p formust-app
packaging/bundle.sh                 # Formust.app and .dmg, written to ~/Downloads/Formust
```

## Key shortcuts

| Keys | Action |
|---|---|
| Cmd+S | Save |
| Cmd+R | Run Form (a file without a form runs the project) |
| Cmd+. | Stop (also ends Live Run) |
| Cmd+B, Cmd+Shift+B | Cargo build, cargo check |
| Cmd+Z, Cmd+Shift+Z | Undo, redo |
| Cmd+C, Cmd+X, Cmd+V, Cmd+D | Copy, cut, paste, duplicate nodes |
| Cmd+J, Cmd+Shift+J | Wrap in a row, unwrap |
| Cmd+E | Fill width |

## Status

- Everything is tested headlessly (`cargo test --workspace`; the keychain test opens a macOS dialog, skip it with `-- --skip the_provider_is_remembered_in_a_plain_text_file`). Mouse drags, menus and the real windows have not all been checked by hand.
- egui only for now; Iced and Slint are open ideas.
- Packaging is not signed or notarized yet.

More detail: [`docs/specs.md`](docs/specs.md) holds the design, decisions and per-phase results; the user guide is in `vitepress/`.


Copyright © Richard Leurs 2026 | GNU GPL-3.0 License.

Formust is based on the [Pascoda](https://github.com/RichardLeurs/pascoda) Pascal IDE, from which the IDE shell comes. Copyright © Richard Leurs 2026 | GNU GPL-3.0 License.
