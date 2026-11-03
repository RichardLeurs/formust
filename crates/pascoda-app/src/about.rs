//! The About window: what Pascoda is, who holds the copyright, and what it is built with (the
//! credits, in a scrolling box under the version).

use eframe::egui;
use std::process::Command;

/// The app name and copyright from `[workspace.metadata.pascoda]` in the root Cargo.toml (see build.rs).
pub const NAME: &str = env!("PASCODA_NAME");
pub const COPYRIGHT: &str = env!("PASCODA_COPYRIGHT");
const INFO: &str = "A modern IDE for Pascal on macOS.";

/// Entries of `CRATES` that are not Cargo crates (the test checks the others against Cargo.lock).
#[cfg(test)]
const NOT_CRATES: &[&str] = &["Rust & rustc", "Lucide"];

/// Rust crates the IDE is built on: `(crate, version, what it is used for, licence)`.
/// `tests::versions_match_cargo_lock` keeps the versions honest.
pub const CRATES: &[(&str, &str, &str, &str)] = &[
 (
        "Rust & rustc",
        "1.98.1",
        "programming language & compiler",
        "MIT or Apache-2.0",
    ),
    (
        "Lucide",
        "icons",
        "the style of the toolbar, navigator and debugger icons (see svg/README.md)",
        "ISC",
    ),    
(
        "eframe / egui",
        "0.37.0",
        "user interface",
        "MIT or Apache-2.0",
    ),
    (
        "egui_extras",
        "0.37.0",
        "drawing the SVG icons",
        "MIT or Apache-2.0",
    ),
    ("muda", "0.21.2", "native menu bar", "Apache-2.0 or MIT"),
    ("rfd", "0.17.2", "file dialogs", "MIT"),
    (
        "serde_json",
        "1.0.154",
        "debugger and language-server messages",
        "MIT or Apache-2.0",
    ),
    (
        "image",
        "0.25.10",
        "decoding the component icons",
        "MIT or Apache-2.0",
    ),
    (
        "portable-pty",
        "0.9.0",
        "pseudo-terminal for the integrated terminal",
        "MIT",
    ),
    (
        "notify",
        "8.2.1",
        "noticing files changed by other programs (FSEvents)",
        "CC0-1.0",
    ),
    (
        "vt100",
        "0.18.1",
        "terminal emulator state for the integrated terminal",
        "MIT",
    ),
    (
        "libc",
        "0.2.191",
        "signals for stopping shells",
        "MIT or Apache-2.0",
    ),
    ("objc2", "0.6.5", "calling AppKit from Rust", "MIT"),
    (
        "objc2-app-kit",
        "0.3.2",
        "AppKit controls",
        "Zlib or Apache-2.0 or MIT",
    ),
    ("objc2-foundation", "0.3.2", "Foundation types", "MIT"),
    (
        "raw-window-handle",
        "0.6.2",
        "access to the native window",
        "MIT or Apache-2.0 or Zlib",
    ),
];

/// The Free Pascal version on this machine, if the compiler is installed: the full one (`-iW`, so a release
/// candidate reads `3.2.4-rc2`, not `3.2.4`).
fn fpc_version() -> Option<String> {
    let out = Command::new(pascoda_fpc::locate_fpc()?)
        .arg("-iW")
        .output()
        .ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// The credits text. `fpc` is the installed compiler version, when known.
pub fn credits(fpc: Option<&str>) -> String {
    let mut s = format!("{INFO}\n\nBuilt with:\n");
    for (name, version, purpose, licence) in CRATES {
        s += &format!("\u{2022} {name} {version}: {purpose} ({licence})\n");
    }
    s += "\nUsed by Pascoda and the programs it builds:\n";
    s += &format!(
        "\u{2022} Free Pascal {}: the compiler, included in the app with its source offer (GPL; runtime library LGPL with linking exception)\n",
        fpc.unwrap_or("(not found)")
    );
    s += "\u{2022} Lazarus Component Library, Cocoa widgetset: runs the forms you design, and Lazarus's component icons (modified LGPL, closed-source programs allowed)\n";
    s += "\u{2022} lldb-dap, from Xcode: the debugger\n";
    s += "\u{2022} pascal-language-server (pasls): code completion (GPL-3.0, run as a separate program)\n";
    s
}

static CREDITS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The app icon (256 px, `assets/pascoda-icon.png`) as an egui texture.
fn icon(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::NULL.with("about-icon");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let img = image::load_from_memory(include_bytes!("../assets/pascoda-icon.png")).expect("the embedded icon is a PNG");
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    let t = ctx.load_texture("about-icon", color, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// The About window, in the app's own dialog so it follows the light or dark appearance. Returns true when
/// it should close (Done).
pub fn show(ctx: &egui::Context) -> bool {
    let icon = icon(ctx);
    let credits = CREDITS.get_or_init(|| credits(fpc_version().as_deref()));
    let muted = crate::style::Palette::of(ctx).text_tertiary;
    let mut done = false;
    crate::style::dialog(ctx, "about", None, 540.0, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal_top(|ui| {
            ui.add(egui::Image::new(&icon).fit_to_exact_size(egui::vec2(96.0, 96.0)));
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(NAME).size(24.0).strong());
                ui.label(egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(muted));
                ui.label(COPYRIGHT);
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.label(egui::RichText::new(credits.as_str()).size(14.0));
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::style::primary_button(ui, "Close").clicked() {
                done = true;
            }
        });
    });
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn credits_cover_info_dependencies_and_tools() {
        let text = credits(Some("3.2.4-rc2"));
        assert!(text.contains("A modern IDE for Pascal"));
        for (name, version, _, licence) in CRATES {
            assert!(
                text.contains(&format!("{name} {version}")) && text.contains(licence),
                "{name}"
            );
        }
        assert!(
            text.contains("Free Pascal 3.2.4-rc2")
                && text.contains("lldb-dap")
                && text.contains("pasls")
        );
        assert!(credits(None).contains("Free Pascal (not found)"));
        assert!(COPYRIGHT.starts_with("Copyright") && COPYRIGHT.contains("Richard Leurs 2026"));
    }

    /// The list above must name the versions the program is actually built with.
    #[test]
    fn versions_match_cargo_lock() {
        let lock =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
                .unwrap();
        for (name, version, _, _) in CRATES {
            // The Rust toolchain and the icon set are not crates, so they have no Cargo.lock entry.
            if NOT_CRATES.contains(name) {
                continue;
            }
            for krate in name.split(" / ") {
                let krate = if krate == "eframe" || krate == "egui" {
                    krate
                } else {
                    name
                };
                let entry = format!("name = \"{krate}\"\nversion = \"{version}\"");
                assert!(
                    lock.contains(&entry),
                    "{krate} {version} is not in Cargo.lock"
                );
            }
        }
    }
}
