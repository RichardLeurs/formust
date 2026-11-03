# Pascoda

A modern Pascal IDE with coding agents for macOS.

![Pascoda: the code editor, the AI assistant pane with a local Ollama model, and the terminal](pascoda.png)

A modern IDE for Pascal on macOS (Universal), written in Rust. It wraps the Free Pascal compiler and builds app for macOS (ARM & Universal).

Supports coding agents (OpenAI API and compatible, OpenAI Codex CLI, Claude Console API, Claude Code CLI for now). 
Connect local LLM-models using oMLX and Ollama. OpenAI Codex CLI and Claude Code CLI (and other CLI's) run in the Pascoda terminal inside the IDE.

> ** Status: early work in progress. The Pascoda IDE app runs Pascal code and builds .app and .dmg files ** 

Website: https://pascoda.richardleurs.nl/

## Specs

| Area | Status | Notes |
|---|---|---|
| Debugger | working | lldb through `lldb-dap`: breakpoints, step, call stack, variables; Pascal sets are not shown |
| Code completion | working | `pasls`: Ctrl+Space and as-you-type completion, hover, F12 and Cmd-click go-to-definition, parameter hints, outline, find references and rename; the project's required packages are on its search path (a changed project file needs a restart) |
| Build and run | working | console and GUI programs, progress bar, Problems pane, export to `.app` and `.dmg` |
| Project files | working | `.lpi` and `.dproj` give the main program, search paths, defines, target and switches; `.lpk` and `.dpk` packages are read and their folders join the builds; `.dfm` opens in the designer; Project Options edits mode, defines, paths, run arguments, Debug or Release and the Lazarus folder whose LCL builds the project; a unit in a subfolder builds the project's program; the `.lpi` `RequiredPackages` are followed (from imports, the project or the Lazarus in use) and a package not found is a Problems-pane warning |
| Form designer | working | native controls, a palette of 115 components on six pages, menus, action lists, anchors, frames (File > New Frame); the project's own components appear on a "Project" palette page |
| File tree | working | new file, new folder, rename, move by drag and drop, delete to the Trash |
| Terminal | working | a real login shell in the bottom pane |
| AI assistant | working | Claude Console API, Claude Code CLI, OpenAI API, Codex CLI, and OpenAI-compatible servers (oMLX, Ollama, Open WebUI, any other); keys in the Keychain |
| Packaging | partly | `packaging/bundle.sh` builds three apps with a disk image each (Apple Silicon, Intel, universal) that ship the language server, the LCL, the FPC sources and the compilers; none was run on an Intel Mac; the notarization path has not been run |

### Spec progress

`docs/specs.md` is the checklist of what the IDE should do: 65 of its 87 items are done and tested (an item is ticked only when the code does what it says and a test covers it). Most of the open ones are small or later work:

- **Medium (P2):** trying a real Lazarus project (first findings with CudaText: item 71).
- **Later (P3):** bookmarks, snippets, keyboard shortcut customisation, threads, disassembly and memory views, a Tools menu, F1 help lookup, profiling, project groups, designer zoom, localisation of forms.
- **Much later (P4):** code folding, Git in the IDE, a notarized release, iOS and iPadOS, trying a real Delphi project, an IDE plugin system, a Universal (Intel) export.

Done recently: Format Code (Ctrl+Cmd+I, and on save), Find and replace, rename symbol, the Tests pane (FPCUnit), watches, conditional breakpoints, Project Options, Debug and Release builds, inherited forms, data modules, the OpenAI-compatible providers, the missing-tools window that installs the language server, the LCL or the FPC sources from the Terminal, and Free Pascal itself, which now ships inside the app.

## Documentation

The documentation site is in `vitepress/` (`cd vitepress && npm install && npm run docs:dev`). `docs/specs.md` is the full checklist. The `docs-writer` agent (`.claude/agents/docs-writer.md`) updates the README, the specs and the site after a change.

## Requirements

You need an Apple Silicon Mac (the IDE's oldest supported macOS is set by `min_macos` under `[workspace.metadata.pascoda]` in the root `Cargo.toml`, now 11; `packaging/bundle.sh` builds with it). The IDE itself was developed and run on macOS 27 only, so older systems are untested. Generated apps target 14.0 unless a project sets its own `min_macos`.

| Tool | Used for | Notes |
|---|---|---|
| Rust (stable, tested with 1.98) | building the IDE | edition 2024 features are used |
| Free Pascal 3.2.4-rc2 (`fpc`) | compiling Pascal | shipped in the `Pascoda-*.app` bundles (a release candidate, tag `release_3_2_4_rc2`; there is no final 3.2.4 tag); to run from a checkout, `brew install fpc` (only to start the build) then `packaging/fpc-build.sh` (an `fpc` on the `PATH` is used when there is no `vendor/fpc`; `PASCODA_FPC` overrides both) |
| Xcode (Command Line Tools at minimum) | linker, `codesign`, `lldb-dap` | the debugger adapter `lldb-dap` ships only inside Xcode |

### Vendored sources (in `vendor/`, not committed)

| Folder | What | Source | Used for |
|---|---|---|---|
| `vendor/fpc` | the Free Pascal compilers (`bin/fpc` for arm64, `bin/ppcrossx64` for Intel) and their precompiled units for both CPUs, with licences and a source offer | built from `vendor/fpcsrc` by `packaging/fpc-build.sh` | compiling Pascal; wins over an `fpc` on the `PATH` |
| `vendor/pasls` | Pascal language server | github.com/genericptr/pascal-language-server | completion, go-to-definition |
| `vendor/lazarus` | Lazarus source tree (tested at commit `2dbcbed8`) | gitlab.com/freepascal.org/lazarus/lazarus | `pasls` CodeTools backend and the LCL for GUI apps |
| `vendor/fpcsrc` | FPC 3.2.4-rc2 `compiler`, `rtl`, `packages` and `utils` (tag `release_3_2_4_rc2`) | gitlab.com/freepascal.org/fpc/source | builds `vendor/fpc`; lets the language server read standard units, and the database and FPCUnit sources |

Set them up from the project root. Install Free Pascal first (`brew install fpc`): it starts the build of `vendor/fpc`, which `pasls` is then built with:

```sh
mkdir -p vendor && cd vendor
git clone --depth 1 --recurse-submodules https://github.com/genericptr/pascal-language-server.git pasls
git clone --depth 1 --filter=blob:none https://gitlab.com/freepascal.org/lazarus/lazarus.git lazarus
git clone --depth 1 --branch release_3_2_4_rc2 --filter=blob:none --sparse \
  https://gitlab.com/freepascal.org/fpc/source.git fpcsrc
(cd fpcsrc && git sparse-checkout set compiler rtl packages utils)
(cd pasls/src && LAZARUSDIR="$PWD/../../lazarus" bash build_fpc.sh)
cd .. && packaging/fpc-build.sh                 # about 5 minutes: arm64 and Intel compilers into vendor/fpc
```

The language server binary ends up in `vendor/pasls/dist/aarch64-darwin/pasls`. Without it the
IDE still works; completion is simply off. Without `vendor/lazarus`, GUI programs cannot be
built. Without `vendor/fpc` the IDE uses the `fpc` on your `PATH` (or `PASCODA_FPC`), so a checkout also
works with only `brew install fpc`; `vendor/fpc` makes it use the same compiler and units as the shipped app.

The built app does not need this step: `packaging/bundle.sh` ships the needed parts of `vendor/`
inside each app (sizes in the table under Packaging; the Apple Silicon app holds the Free Pascal compilers and units for arm64, Intel and Windows,
copied from `vendor/fpc`, which `packaging/fpc-build.sh` builds). Only the Command Line Tools (linker and macOS SDK) must be
installed on the Mac.

A copy that lacks them (a checkout without `vendor/`, a damaged app) opens a "Some tools are missing"
window when a feature needs one. Its **Install in Terminal** button types a pinned install command at the
Terminal's prompt, and you press Enter to run it. The tools go to
`~/Library/Application Support/Pascoda/vendor`, which the IDE searches too; restart Pascoda afterwards.

## Build and run

```sh
cargo run -p pascoda-app -- tests/fixtures/debug.pas
cargo test
```

The tests compile Pascal programs, launch `lldb-dap` and `pasls`, and open a GUI window for a few
seconds, so they need the tools above. The first GUI build compiles the LCL into
`~/Library/Caches/Pascoda/lcl/` (about 20 seconds); later builds reuse it.

## Packaging

```sh
packaging/bundle.sh [output-dir]     # three apps and three disk images, default dir ~/Downloads/Pascoda
ARCHES="aarch64-darwin universal-darwin" packaging/bundle.sh   # only the listed builds (default: all three)
NO_DMG=1 packaging/bundle.sh         # apps only, no disk images
```

`bundle.sh` makes one app and one disk image (app + Applications shortcut) per build:

| Build | Files | Holds | dmg | app |
|---|---|---|---|---|
| Apple Silicon | `Pascoda-aarch64-darwin.app` / `.dmg` | arm64 executable, `vendor/fpc` (arm64 compiler, Intel cross compiler, Windows units), arm64 `pasls` | about 431 MB | about 970 MB |
| Intel | `Pascoda-x86_64-darwin.app` / `.dmg` | x86_64 executable, `vendor/fpc-intel` (the compiler that runs on Intel), x86_64 `pasls`; no Windows export (the Intel compiler cannot build Windows programs, the app falls back) | about 189 MB | about 343 MB |
| Universal | `Pascoda-universal-darwin.app` / `.dmg` | both executables joined with `lipo`, both compilers, both `pasls` | about 524 MB | about 1.1 GB |

Why the Intel app is smaller: most of the difference is Free Pascal. The Apple Silicon app's `vendor/fpc` is about 770 MB: the arm64 units (about 275 MB), the x86_64 units for the Intel cross compiler (about 304 MB), the Windows 64-bit units (about 164 MB) and the arm64 compiler (about 26 MB). The Intel app carries only `vendor/fpc-intel` (about 144 MB), the compiler and units for Intel, and no Windows units, which is why it cannot export for Windows. The FPC sources, the LCL sources and `pasls` (about 170 MB) are in both apps.

A bundle is replaced only when the same build is made again; other files in the output folder (including an older
`Pascoda.app`) are not deleted. `packaging/make-dmg.sh` still expects the old single `Pascoda.app` and has not been
updated for these names.

Each build (release build, `packaging/Pascoda.icns`, `Info.plist` with the name, bundle identifier, version,
copyright and minimum macOS) and signs it ad hoc, which runs on this Mac only. The name, bundle identifier, copyright and
minimum macOS are set in `[workspace.metadata.pascoda]` in the root `Cargo.toml`; the About window (Pascoda > About Pascoda) shows the name and copyright from the same table, in the app's light or dark appearance.
The bundle carries the toolchain sources it needs (`Contents/Resources/vendor`: the LCL, CodeTools,
FPC sources, the `pasls` binary and the Free Pascal compilers with their units), so it does not
depend on this checkout. `packaging/bundle.sh` copies the compilers from `vendor/fpc` (`packaging/fpc-build.sh`) and, for the Intel and universal builds, `vendor/fpc-intel` (`packaging/fpc-intel-build.sh`). The three apps were built and checked (executable architecture, signature, disk image checksums); none was started on an Intel Mac. The Mac that runs the app needs the Command Line Tools (the linker and the macOS
SDK) and Xcode's `lldb-dap` for debugging.
`packaging/make-icon.sh` regenerates the icons (`make-icon.swift`, no dependencies): `Pascoda.icns` is
the P whose counter is a run triangle (used by the bundle), `Pascoda-classic.icns` is the plain P.

For distribution set `CODESIGN_IDENTITY="Developer ID Application: ..."` (hardened runtime and
timestamp), and `NOTARY_PROFILE=<keychain profile from notarytool store-credentials>` to also notarize
and staple. The bundle identifier `com.richardleurs.pascoda` is a placeholder. The notarization path
has not been run.

The app finds `vendor/` (`pasls`, `lazarus`, `fpcsrc`) in this order: the opened file's folder and its
parents, `$PASCODA_HOME` (a folder that contains `vendor/`), the bundle's `Contents/Resources`, and,
in debug builds only, the checkout the app was built in. It adds `/opt/homebrew/bin` and
`/usr/local/bin` to `PATH` at start, because an app opened from Finder does not get them.

## Rust dependencies

| Crate | Version | Used in | Purpose |
|---|---|---|---|
| `eframe` / `egui` | 0.37.0 | `pascoda-app` | window, immediate-mode UI |
| `egui_extras` (with `svg`, which brings in `resvg`) | 0.37.0 | `pascoda-app` | drawing the SVG icons |
| `muda` | 0.21.2 | `pascoda-app` | native macOS menu bar |
| `rfd` | 0.17.2 | `pascoda-app` | native Open dialog |
| `image` | 0.25.10 | `pascoda-app` | decoding the component icons |
| `portable-pty` | 0.9.0 | `pascoda-app` | pseudo-terminal for the integrated terminal |
| `notify` | 8.2.1 | `pascoda-app` | file-system watcher (FSEvents) for external-change detection |
| `vt100` | 0.18.1 | `pascoda-app` | terminal emulator (escape sequences, colours, scrollback) |
| `libc` | 0.2.191 | `pascoda-app` | hanging up and killing the terminal's shell |
| `objc2`, `objc2-app-kit`, `objc2-foundation` | 0.6.5 / 0.3.2 / 0.3.2 | `pascoda-app` | native controls, window chrome |
| `serde_json` | 1.0.154 | `pascoda-app`, `pascoda-dap`, `pascoda-lsp` | JSON for the DAP and LSP protocols |

`pascoda-fpc` and `pascoda-project` use only the standard library; `pascoda-forms` has no UI dependency. See `Cargo.lock` for the
transitive dependencies.

## Layout

| Crate | Role |
|---|---|
| `pascoda-app` | the IDE: UI, editor, debugger and language-server threads, theme, menu |
| `pascoda-fpc` | runs `fpc` with progress reporting, parses its diagnostics, signs binaries for debugging; `lcl` module builds GUI apps and `.app` bundles |
| `pascoda-dap` | Debug Adapter Protocol client for `lldb-dap` |
| `pascoda-lsp` | Language Server Protocol client for `pasls` |
| `pascoda-ai` | AI assistant backend: Anthropic Messages API client and OpenAI-compatible Chat Completions client (SSE streaming, tool use), the agent loop whose file changes are only proposals, Keychain key storage, and the Claude Code (subscription) driver that runs the user's own `claude` CLI; no UI |
| `pascoda-forms` | Lazarus `.lfm` form files: lossless parser, writer and text-patch editing (`FormDoc`); no UI dependency |
| `pascoda-project` | project model (minimal so far) |

Other folders: `packaging` (app bundle, DMG and icon scripts), `tests/fixtures` (sample Pascal programs), `docs/design` (design notes), `.claude` (project notes and agent definitions for Claude Code).

Demo projects in `examples/` (open the main file and use Build & Run; the Tests pane for the last one):

| Folder | Kind | What it shows |
|---|---|---|
| `gui-demo` | GUI | a three-form Notes app: menus, shortcuts, dialogs, status bar, settings and about forms (`notes.lpr`) |
| `console-demo` | console | `WordCount.lpr`, reads standard input; run `./WordCount < file` in the Terminal |
| `library-demo` | library | a unit (`Maths.pas`) and the program that uses it (`MathsDemo.lpr`) |
| `package-demo` | package | a `.lpk` package (`TextTools`) and a program that finds its unit through the package |
| `db-demo` | GUI | a window on `ideas.sqlite3` (SQLite): a parameterised search in a grid |
| `vlc-demo` | GUI | a video and audio player on VLC (needs VLC.app and the Pascoda VLC package, imported first: File > Import Component Package...) |
| `tests-demo` | unit tests and program | `TestsDemo.lpr` runs `CalcTests.pas` (FPCUnit tests for `Calc.pas`) with the console runner; one test fails on purpose. Cmd+U runs the same tests in the Tests pane |

## What works

**Editor**
- Tabs, file tree, line numbers, Pascal highlighting. Compiler errors and warnings are underlined.
- Hover, completion as you type (Ctrl+Space) and go-to-definition (F12), through the language server `pasls`.
- Format Code (Edit > Format Code, Ctrl+Cmd+I) formats the Pascal file with the language server's JCF; Preferences has **Format on save** (off by default). The server's default settings are used: a space before the `;` of a parameter list and blank lines between sections come from them.
- Preferences (Cmd+,): font (SF Mono, Menlo or Courier) and size, Tab width, line numbers, word wrap, and colours for each code token and the background, set separately for light and dark. Saved in `~/Library/Application Support/Pascoda/preferences`.
- Files changed by other programs (`claude`, git, an editor in the terminal) show up. Clean tabs reload silently. A tab with unsaved edits shows a banner with **Reload** or **Keep mine**.
- Closing a changed tab asks Save, Don't Save or Cancel. The last session's tabs and project come back at launch, and File > Open Recent lists recent projects and files.
- File > Open project... (Cmd+O, also the toolbar folder button) only offers the program files of a project: `.lpr` and `.dpr`, and the Delphi project file `.dproj` (which opens its main program). Units, forms and other files open from the file tree.

**Build, run and test**
- Console and GUI programs (the LCL, Cocoa widgetset). GUI programs are packaged as a `.app`, and Build > Export makes a `.app` or `.dmg`. Demos are in `examples/`.
- Component packages: the Pascoda VLC package is always available. File > Import Component Package... adds any other Lazarus `.lpk`; its components appear on palette pages named by the package. VLC: a video and audio player on the VLC library installed in `/Applications/VLC.app`, with a video panel in the form. The database components (SQLdb and SQLite connections, queries, scripts, data sources) are not a package: they are on the designer's own palette, built from the FPC database sources that ship with the compiler.
- Unit tests (FPCUnit): Cmd+U runs the project's test units. The **Tests** pane lists each result and its failure message. You can run one test, the failed ones or all of them, and clicking a test opens its method in the editor. Runner is generated in `lib/tests/`.
- Bottom pane tabs: Problems, Output, Debug, Tests and Terminal. It switches to Output when a build or run starts, to Problems when a build fails, and to Debug at a breakpoint. It leaves your choice alone for a few seconds after you click a tab.

**Debugger** (through `lldb-dap`)
- Gutter breakpoints, continue, step, call stack, and a variables pane with type badges, expandable records, classes and arrays, and Pascal-formatted values.

**Terminal**
- Your login shell on a real pseudo-terminal, with colours, 5000 lines of scrollback and full-screen programs such as `vim`, `less` and `top`.
- Cmd+C, Cmd+V and Cmd+K work as in Terminal, and Ctrl+C interrupts. Menu shortcuts keep working.

**Form designer** (`File > New GUI Application`, then the `Form` toggle)
- Native macOS controls. The palette has 115 components on six pages (Standard, Additional, Common Controls, Misc, System, Data Controls); every class the LCL registers is there except TFrame, which is handled as project frames (item 45: File > New Frame, a placed frame shows its components), and a tray for non-visual ones (timers, dialogs, menus, action lists, and the database components: SQLdb, TBufDataset, TMemDataset, TDbf).
- Select, move, resize and multi-select, with copy and paste, alignment guides, align and spacing commands, and tab order.
- Database forms: TDBGrid (Data Controls page) and a SQLdb connection editor (Edit Connection... in the inspector for a TSQLConnector: SQLite3 or PostgreSQL, with the driver unit added to uses). The data components come with the app (no import needed).
- Data modules (File > New Data Module): a window-less unit for non-visual components (timers, dialogs, menus, action lists), used by forms through their unit.
- Anchors, `Align`, `AutoSize`, `AnchorSide` and `BorderSpacing`. The canvas scrolls for large forms.
- Inspector editors for colours (named Lazarus colours or any RGB), fonts (Name, Size, Style, Color) and collection lists (Add, Remove, Up, Down).
- Inline editors for menus and action lists. Double-click a control to create its event handler, and the `.pas` unit stays in step.

**Window**
- Xcode-style layout with a toolbar, navigator (file tree, outline, debug), tabs and breadcrumbs, and a status bar. Light and dark (View > Appearance), with native menus.

**AI assistant** (View > Show Assistant, Cmd+Opt+A, or the sparkle button)
- Providers: your Anthropic API key, your OpenAI key or a compatible local server (Ollama, LM Studio), or your installed Claude Code with its subscription. Keys are kept in the macOS Keychain, and the OpenAI base URL and model in plain settings files. The choice is remembered.
- It can see the open file, your selection, the build errors and a project overview. It reads project files and proposes edits and new files as diffs that you Apply or Reject. It builds only when you allow it.
- What it sends goes only to the provider you chose, and each chip can be switched off. It cannot reach files outside the project folder or run commands. Claude Code mode has no Bash, so no builds.
- **Allow edits this session** applies edits without asking. It is off at launch and never saved. Every change stays in the chat with a **Revert** button.

## Not done yet

- A per-project settings screen. The Delphi language mode exists in code, but has no setting.
- Developer ID signing and notarization (the code path exists but has never been run), and a real bundle identifier.
- Debugger: Pascal set values cannot be shown (lldb cannot read set types).
- The language server publishes no diagnostics, so build errors are the only ones shown.
- Designer: zoom, `ChildSizing`, and dragging menu items with the mouse.
- Assistant: the OpenAI provider has only been tested against mock servers, not the real OpenAI or a live local server. Cost figures, a terminal pane like the Claude CLI and chat history across restarts are missing.
- Most mouse and keyboard interaction has been tested only through unit tests and offscreen renders. Please report anything that misbehaves.
- Multilingual forms and interface.
- Design notes are in `docs/design/`.

## Things worth knowing

- `fpc` binaries are only linker-signed, and `lldb` cannot launch them until they are re-signed
  with the `get-task-allow` entitlement. The build does this automatically.
- fpc 3.2's Objective-C output fails with Apple's current linker. GUI builds pass
  `-k-ld_classic` as a workaround; Apple has deprecated that flag, so it may stop working.
- `pasls` crashes during `initialize` if the client sends no `clientInfo`; the client always does.
- Generated app bundles must not set `NSPrincipalClass`, or the LCL app crashes at startup.
- Generated GUI apps target macOS 14.0 or later.
- Apps opened from Finder get a minimal `PATH`; Pascoda extends it so `fpc` is found.
- The component icons are Lazarus's (modified LGPL); see `crates/pascoda-app/assets/`.
- The IDE's toolbar, navigator and debugger icons are SVG files in `crates/pascoda-app/assets/icons/` (compiled in; the design set is in `svg/`, see `svg/README.md`). `style::paint_icon` draws them tinted with the theme colour; the filled breakpoint dot is still drawn by hand.
- A `{$R file.res}` resource needs `fpcres`, which the compiler runs on macOS. It is built from the FPC sources and ships in the compiler's folder (`vendor/fpc/bin`, `vendor/fpc-intel/bin`, and the bundle's copies). The IDE puts that folder first on `PATH` when it runs `fpc`, so a Mac without Homebrew's FPC works; without it the build fails with `resource compiler "fpcres" not found`.

## Licences

GNU GENERAL PUBLIC LICENSE Version 3

