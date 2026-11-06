//! The About window: what Formust is, who holds the copyright, and what it is built with (the
//! credits, in a scrolling box under the version).

use eframe::egui;
use std::process::Command;

/// The app name and copyright from `[workspace.metadata.formust]` in the root Cargo.toml (see build.rs).
pub const NAME: &str = env!("FORMUST_NAME");
pub const COPYRIGHT: &str = env!("FORMUST_COPYRIGHT");
/// Where this program comes from. Shown under the copyright, not in the credits list.
pub const ORIGIN: &str = "Based on the Pascoda Pascal IDE, GNU GPL-3.0 licence";
const INFO: &str = "An IDE for building egui forms for Rust programs on macOS.";

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
        "the style of the toolbar and navigator icons (see svg/README.md)",
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
        "language-server and assistant messages",
        "MIT or Apache-2.0",
    ),
    (
        "image",
        "0.25.10",
        "decoding the app icon",
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

/// The Rust toolchain on this machine, if cargo is installed: `cargo -V`, such as `cargo 1.90.0 (...)`.
fn cargo_version() -> Option<String> {
    let out = Command::new(formust_build::locate_cargo()?)
        .arg("-V")
        .output()
        .ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// The credits text. `cargo` is the installed toolchain's version, when known.
pub fn credits(cargo: Option<&str>) -> String {
    let mut s = format!("{INFO}\n\nBuilt with:\n");
    for (name, version, purpose, licence) in CRATES {
        s += &format!("\u{2022} {name} {version}: {purpose} ({licence})\n");
    }
    s += "\nUsed by Formust and the programs it builds:\n";
    s += &format!(
        "\u{2022} The Rust toolchain, {}: cargo checks, builds and runs your programs (MIT or Apache-2.0)\n",
        cargo.unwrap_or("(not found)")
    );
    s += "\u{2022} egui and eframe 0.37: the GUI library that the forms are built with (MIT or Apache-2.0)\n";
    s += "\u{2022} rust-analyzer: code completion, navigation and diagnostics, run as a separate program (MIT or Apache-2.0)\n";
    s
}

static CREDITS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The app icon (256 px, `assets/formust-icon.png`) as an egui texture.
fn icon(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::NULL.with("about-icon");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let img = image::load_from_memory(include_bytes!("../assets/formust-icon.png"))
        .expect("the embedded icon is a PNG");
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
    let credits = CREDITS.get_or_init(|| credits(cargo_version().as_deref()));
    let muted = crate::style::Palette::of(ctx).text_tertiary;
    let mut done = false;
    crate::style::dialog(ctx, "about", None, 540.0, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal_top(|ui| {
            ui.add(egui::Image::new(&icon).fit_to_exact_size(egui::vec2(96.0, 96.0)));
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(NAME).size(24.0).strong());
                ui.label(
                    egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .color(muted),
                );
                ui.label(COPYRIGHT);
                ui.label(egui::RichText::new(ORIGIN).color(muted));
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
        let text = credits(Some("cargo 1.90.0 (0a1b2c3d 2026-01-01)"));
        assert!(text.contains("An IDE for building egui forms for Rust programs"));
        for (name, version, _, licence) in CRATES {
            assert!(
                text.contains(&format!("{name} {version}")) && text.contains(licence),
                "{name}"
            );
        }
        assert!(
            text.contains("The Rust toolchain, cargo 1.90.0") && text.contains("rust-analyzer")
        );
        assert!(
            !text.contains("Pascal") && !text.contains("Lazarus"),
            "no Pascal credits"
        );
        assert!(
            !credits(None).contains(ORIGIN),
            "the origin is not in the credits"
        );
        assert!(ORIGIN.contains("Pascoda") && ORIGIN.contains("GPL-3.0"));
        assert!(credits(None).contains("The Rust toolchain, (not found)"));
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
