//! The app's own icons: the SVG set in `assets/icons` (the design in `svg/`). They are compiled in and drawn with
//! egui's SVG loader in the colour the caller gives (the files use `currentColor`, so the colour is applied as a
//! tint). `style::paint_icon` uses them for every `Icon` that has a file.

use eframe::egui;

/// `(name, file)` for every icon in `assets/icons`.
const SVG_ICONS: &[(&str, &[u8])] = &[
    (
        "alert-triangle",
        include_bytes!("../assets/icons/alert-triangle.svg"),
    ),
    ("apple", include_bytes!("../assets/icons/apple.svg")),
    (
        "app-window-plus",
        include_bytes!("../assets/icons/app-window-plus.svg"),
    ),
    ("box-app", include_bytes!("../assets/icons/box-app.svg")),
    (
        "chevron-down",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "chevron-left",
        include_bytes!("../assets/icons/chevron-left.svg"),
    ),
    (
        "chevron-right",
        include_bytes!("../assets/icons/chevron-right.svg"),
    ),
    ("circle", include_bytes!("../assets/icons/circle.svg")),
    ("cpu", include_bytes!("../assets/icons/cpu.svg")),
    ("database", include_bytes!("../assets/icons/database.svg")),
    ("file-code", include_bytes!("../assets/icons/file-code.svg")),
    ("file-plus", include_bytes!("../assets/icons/file-plus.svg")),
    ("files", include_bytes!("../assets/icons/files.svg")),
    ("folder", include_bytes!("../assets/icons/folder.svg")),
    (
        "folder-plus",
        include_bytes!("../assets/icons/folder-plus.svg"),
    ),
    ("intel", include_bytes!("../assets/icons/intel.svg")),
    ("list-tree", include_bytes!("../assets/icons/list-tree.svg")),
    (
        "maximize-2",
        include_bytes!("../assets/icons/maximize-2.svg"),
    ),
    (
        "minimize-2",
        include_bytes!("../assets/icons/minimize-2.svg"),
    ),
    (
        "panel-bottom",
        include_bytes!("../assets/icons/panel-bottom.svg"),
    ),
    ("play", include_bytes!("../assets/icons/play.svg")),
    ("plus", include_bytes!("../assets/icons/plus.svg")),
    ("save", include_bytes!("../assets/icons/save.svg")),
    ("search", include_bytes!("../assets/icons/search.svg")),
    ("send", include_bytes!("../assets/icons/send.svg")),
    ("settings", include_bytes!("../assets/icons/settings.svg")),
    ("sidebar", include_bytes!("../assets/icons/sidebar.svg")),
    ("sparkles", include_bytes!("../assets/icons/sparkles.svg")),
    ("square", include_bytes!("../assets/icons/square.svg")),
    ("terminal", include_bytes!("../assets/icons/terminal.svg")),
    ("trash-2", include_bytes!("../assets/icons/trash-2.svg")),
    ("windows", include_bytes!("../assets/icons/windows.svg")),
    ("x", include_bytes!("../assets/icons/x.svg")),
];

/// Adds egui's SVG loader to `ctx`, once per context (the theme sets up every context this way).
pub fn install(ctx: &egui::Context) {
    let done = egui::Id::NULL.with("formust-svg-loader");
    if ctx.data(|d| d.get_temp::<bool>(done).unwrap_or(false)) {
        return;
    }
    egui_extras::install_image_loaders(ctx);
    ctx.data_mut(|d| d.insert_temp(done, true));
}

/// The icon with its strokes drawn white. The files say `currentColor`, which renders black, and a tint
/// multiplies with the texture (black stays black), so the colour is put on with the tint instead: white
/// times the wanted colour is that colour, in light and dark mode alike.
fn white(bytes: &[u8]) -> std::sync::Arc<[u8]> {
    let text = std::str::from_utf8(bytes).unwrap_or_default();
    std::sync::Arc::from(text.replace("currentColor", "#ffffff").into_bytes())
}

/// Paints the icon called `name` (a file name without `.svg`) inside `rect`, tinted with `color`. False when
/// there is no such icon or it cannot be drawn.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, name: &str, color: egui::Color32) -> bool {
    let Some((_, bytes)) = SVG_ICONS.iter().find(|(n, _)| *n == name) else {
        return false;
    };
    let ctx = painter.ctx();
    let uri = format!("bytes://formust/icons/{name}.svg");
    ctx.include_bytes(uri.clone(), white(bytes));
    let scale = ctx.pixels_per_point();
    match ctx.try_load_texture(
        &uri,
        egui::TextureOptions::LINEAR,
        egui::SizeHint::Scale(scale.into()),
    ) {
        Ok(egui::load::TexturePoll::Ready { texture }) => {
            let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id, rect, full, color);
        }
        Ok(egui::load::TexturePoll::Pending { .. }) => ctx.request_repaint(),
        Err(_) => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_app_icon_file_is_a_well_formed_svg() {
        for (name, bytes) in SVG_ICONS {
            let text = std::str::from_utf8(bytes).unwrap_or_else(|_| panic!("{name} is not UTF-8"));
            assert!(
                text.trim_start().starts_with("<svg"),
                "{name} is not an SVG"
            );
            assert!(
                text.contains("viewBox=\"0 0 24 24\""),
                "{name} has no 24-unit viewBox"
            );
        }
        assert_eq!(SVG_ICONS.len(), 33, "assets/icons has 33 icons");
    }
}
