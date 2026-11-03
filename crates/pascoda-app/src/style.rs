//! The look of the IDE shell: colours modelled on Xcode's default light and dark themes, and
//! the toolbar icons, drawn with the painter because egui has no icon font.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, vec2};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Toolbar, tab strip, jump bar, status bar.
    pub chrome: Color32,
    pub sidebar: Color32,
    pub editor: Color32,
    pub separator: Color32,
    pub row_selected: Color32,
    pub row_hover: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub line_number: Color32,
    pub line_number_current: Color32,
    pub current_line: Color32,
    pub exec_line: Color32,
    pub exec_label_bg: Color32,
    pub exec_label_text: Color32,
    pub accent: Color32,
    pub breakpoint: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub ok: Color32,
    // Code
    pub code_text: Color32,
    pub keyword: Color32,
    pub user_type: Color32,
    pub builtin_type: Color32,
    pub call: Color32,
    pub comment: Color32,
    pub string: Color32,
    pub number: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        dark: false,
        chrome: rgb(0xFC, 0xFC, 0xFD),
        sidebar: rgb(0xEC, 0xEA, 0xE9),
        editor: rgb(0xFF, 0xFF, 0xFF),
        separator: rgb(0xDD, 0xDD, 0xDF),
        row_selected: rgb(0xD7, 0xD6, 0xD6),
        row_hover: rgb(0xE2, 0xE1, 0xE0),
        text: rgb(0x26, 0x26, 0x26),
        text_secondary: rgb(0x6E, 0x6E, 0x73),
        text_tertiary: rgb(0xA3, 0xA3, 0xA8),
        line_number: rgb(0xA3, 0xA3, 0xA8),
        line_number_current: rgb(0x5E, 0x5E, 0x63),
        current_line: rgb(0xEF, 0xEF, 0xF1),
        exec_line: rgb(0xDD, 0xF3, 0xD6),
        exec_label_bg: rgb(0xBE, 0xDF, 0xB9),
        exec_label_text: rgb(0x3E, 0x8E, 0x3B),
        accent: rgb(0x0A, 0x7A, 0xFF),
        breakpoint: rgb(0x5B, 0x9B, 0xF8),
        error: rgb(0xD1, 0x2F, 0x1B),
        warning: rgb(0xB8, 0x7B, 0x00),
        ok: rgb(0x3E, 0x8E, 0x3B),
        code_text: rgb(0x26, 0x26, 0x26),
        keyword: rgb(0xB5, 0x37, 0x6E),
        user_type: rgb(0x3B, 0x80, 0x9E),
        builtin_type: rgb(0x9A, 0x49, 0xCC),
        call: rgb(0x3F, 0x78, 0x6E),
        comment: rgb(0x6B, 0x70, 0x83),
        string: rgb(0xC4, 0x38, 0x2A),
        number: rgb(0x27, 0x2A, 0xD8),
    };

    pub const DARK: Palette = Palette {
        dark: true,
        chrome: rgb(0x1E, 0x1E, 0x22),
        sidebar: rgb(0x26, 0x26, 0x2B),
        editor: rgb(0x1F, 0x1F, 0x24),
        separator: rgb(0x38, 0x38, 0x3D),
        row_selected: rgb(0x3A, 0x3A, 0x40),
        row_hover: rgb(0x30, 0x30, 0x35),
        text: rgb(0xE5, 0xE5, 0xE7),
        text_secondary: rgb(0x98, 0x98, 0x9F),
        text_tertiary: rgb(0x6B, 0x6B, 0x72),
        line_number: rgb(0x6B, 0x6B, 0x72),
        line_number_current: rgb(0xB0, 0xB0, 0xB6),
        current_line: rgb(0x2A, 0x2B, 0x30),
        exec_line: rgb(0x23, 0x3B, 0x28),
        exec_label_bg: rgb(0x2F, 0x5A, 0x37),
        exec_label_text: rgb(0xB6, 0xE8, 0xB0),
        accent: rgb(0x0A, 0x84, 0xFF),
        breakpoint: rgb(0x4F, 0x8D, 0xF5),
        error: rgb(0xFF, 0x6B, 0x5E),
        warning: rgb(0xFF, 0xCC, 0x4D),
        ok: rgb(0x6F, 0xD2, 0x6B),
        code_text: rgb(0xE5, 0xE5, 0xE7),
        keyword: rgb(0xFC, 0x5F, 0xA3),
        user_type: rgb(0x5D, 0xD8, 0xFF),
        builtin_type: rgb(0xD0, 0xA8, 0xFF),
        call: rgb(0x67, 0xB7, 0xA4),
        comment: rgb(0x7F, 0x8C, 0x98),
        string: rgb(0xFC, 0x6A, 0x5D),
        number: rgb(0xD0, 0xBF, 0x69),
    };

    pub fn of(ctx: &egui::Context) -> Palette {
        if ctx.global_style().visuals.dark_mode {
            Self::DARK
        } else {
            Self::LIGHT
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Stop,
    Bug,
    Plus,
    Folder,
    Save,
    Sidebar,
    Continue,
    StepOver,
    StepInto,
    StepOut,
    RunToCursor,
    Files,
    Stack,
    Filter,
    Outline,
    Search,
    Breakpoint,
    Close,
    Sparkle,
    Gear,
    Send,
    Trash,
    NewFile,
    NewForm,
    NewFolder,
    PanelBottom,
}

/// Draws `icon` inside `rect` (about 14 points square) with `color`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    if let Some(name) = icon_file(icon)
        && crate::app_icons::paint(painter, rect, name, color)
    {
        return;
    }
    // A filled dot, the breakpoint's mark in the gutter (the outline icon set has no filled dot).
    let r = rect.width().min(rect.height()) / 2.0;
    painter.circle_filled(rect.center(), r * 0.6, color);
}

/// The file in `assets/icons` (without `.svg`) that draws `icon`; `None` for the breakpoint's filled dot.
fn icon_file(icon: Icon) -> Option<&'static str> {
    Some(match icon {
        Icon::Play => "play",
        Icon::Stop => "square",
        Icon::Continue => "debug-continue",
        Icon::Bug => "bug",
        Icon::Plus => "plus",
        Icon::Folder => "folder",
        Icon::Save => "save",
        Icon::Sidebar => "sidebar",
        Icon::StepOver => "step-over",
        Icon::StepInto => "step-into",
        Icon::StepOut => "step-out",
        Icon::RunToCursor => "run-to-cursor",
        Icon::Files => "files",
        Icon::Stack => "layers",
        Icon::Outline => "list-tree",
        Icon::Filter | Icon::Search => "search",
        Icon::Close => "x",
        Icon::Sparkle => "sparkles",
        Icon::Gear => "settings",
        Icon::Send => "send",
        Icon::NewFile => "file-plus",
        Icon::NewForm => "app-window-plus",
        Icon::NewFolder => "folder-plus",
        Icon::PanelBottom => "panel-bottom",
        Icon::Trash => "trash-2",
        Icon::Breakpoint => return None,
    })
}

/// A flat icon button (about 26x24 points). Returns the response; `selected` draws the
/// rounded highlight Xcode uses for the active mode.
/// A show/hide button (navigator, bottom pane, assistant): no background unless hovered; the icon is bright
/// while the pane it shows is open.
pub fn toggle_button(ui: &mut egui::Ui, pal: &Palette, icon: Icon, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.hovered() { pal.row_hover } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, 6.0, fill);
        let color = if on { pal.text } else { pal.text_secondary };
        paint_icon(ui.painter(), Rect::from_center_size(rect.center(), vec2(13.0, 13.0)), icon, color);
    }
    resp
}

pub fn icon_button(
    ui: &mut egui::Ui,
    pal: &Palette,
    icon: Icon,
    enabled: bool,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), egui::Sense::click());
    let resp = if enabled {
        resp
    } else {
        resp.on_hover_cursor(egui::CursorIcon::NotAllowed)
    };
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            pal.row_selected
        } else if resp.hovered() && enabled {
            pal.row_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6.0, fill);
        let color = if !enabled {
            pal.text_tertiary
        } else if selected && icon != Icon::Play {
            pal.text
        } else {
            pal.text_secondary
        };
        paint_icon(
            ui.painter(),
            Rect::from_center_size(rect.center(), vec2(13.0, 13.0)),
            icon,
            color,
        );
    }
    resp
}

/// The rounded group Xcode puts around related toolbar buttons.
/// A modal-looking dialog in the app's own colours (no system-grey title bar): rounded, with a
/// soft shadow. `title`, when given, is the bold first line. Centred over the window.
// ---- Dialog colours: edit these to restyle the app's dialog windows (Preferences, Project Options, Export...). ----


/// The band behind the dialog's title (dark mode; light mode has no band).
pub const DIALOG_TITLE_DARK: Color32 = Color32::from_rgb(0x19,0x1A,0x22);//(0x1A, 0x1B, 0x21);
/// The body of a dialog (the window behind its fields).
pub const DIALOG_BG_DARK: Color32 = Color32::from_rgb(0x13, 0x14, 0x1B);
pub const DIALOG_BG_LIGHT: Color32 = Color32::WHITE;
/// The outline of the dialog and the rule under its title (dark mode).
pub const DIALOG_BORDER_DARK: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x30);
/// The background of the Project Options tab panel (the left side).
pub const DIALOG_SIDE_BG: Color32 = Color32::from_rgb(0x13,0x14,0x1A);//from_rgb(0x1A, 0x1B, 0x21);
/// Unselected option cards (Export's "Runs on") in dark mode.
pub const DIALOG_PANE_DARK: Color32 = Color32::from_rgb(0x16,0x17,0x1D);//rgb(0x20, 0x21, 0x29);
/// Text fields and the boxes that sit on the dialog body (the textfield look).
pub const DIALOG_FIELD_DARK: Color32 = Color32::from_rgb(0x22,0x23,0x29);//(0x20, 0x21, 0x29);
pub const DIALOG_FIELD_LIGHT: Color32 = Color32::from_rgb(0xEE, 0xEE, 0xF0);

/// The footer row with the buttons (Save, Cancel), and the rule along its top.
pub const DIALOG_FOOTER_DARK: Color32 = Color32::from_rgb(0x19,0x1B,0x22);//(0x1A, 0x1B, 0x21);
pub const DIALOG_FOOTER_LIGHT: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF6);
pub const DIALOG_FOOTER_BORDER_DARK: Color32 = Color32::from_rgb(0x4A, 0x4A, 0x50);
pub const DIALOG_FOOTER_BORDER_LIGHT: Color32 = Color32::from_rgb(0xD8, 0xD8, 0xDC);

/// The background of the app's dialogs.
pub fn dialog_fill(pal: &Palette) -> Color32 {
    if pal.dark { DIALOG_BG_DARK } else { DIALOG_BG_LIGHT }
}

pub fn dialog<R>(
    ctx: &egui::Context,
    id: &str,
    title: Option<&str>,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) {
    dialog_flush(ctx, id, title, width, 30, add);
}

/// A dialog whose title has the app's box tile in front of it (the Export Application dialog).
pub fn dialog_with_tile<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) {
    dialog_body(ctx, id, Some(title), width, 30, 18, 18, 12.0, Some("box-glyph"), add);
}

/// The app's box tile: an accent square with the white box drawn on it (the same blue as the selected choice's
/// border), `size` points across.
pub fn app_tile(ui: &mut egui::Ui, pal: &Palette, size: f32) {
    tile(ui, pal, size, "box-glyph");
}

/// An accent tile with a white glyph (an icon name from `app_icons`) on it, `size` points across.
pub fn tile(ui: &mut egui::Ui, pal: &Palette, size: f32, glyph: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    // The corner radius matches the icon's own: 6 of 24.
    ui.painter().rect_filled(rect, size * 0.25, pal.accent);
    crate::app_icons::paint(ui.painter(), rect, glyph, Color32::WHITE);
}

/// A dialog that is flush with its top and bottom rules: `top` and `bottom` are the frame's vertical insets and
/// `title_gap` the space under the title rule. Dialogs with a tab column and a footer use 0 for these.
#[allow(clippy::too_many_arguments)]
pub fn dialog_edges<R>(
    ctx: &egui::Context,
    id: &str,
    title: Option<&str>,
    width: f32,
    pad: i8,
    top: i8,
    bottom: i8,
    title_gap: f32,
    tile: Option<&'static str>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) {
    dialog_body(ctx, id, title, width, pad, top, bottom, title_gap, tile, add);
}

/// A dialog whose body keeps `pad` points clear of its left and right edges (30 for the usual dialog; 0 lets
/// the body, such as a sidebar, run flush to the edges). The title band and its rule follow the same padding.
pub fn dialog_flush<R>(
    ctx: &egui::Context,
    id: &str,
    title: Option<&str>,
    width: f32,
    pad: i8,
    add: impl FnOnce(&mut egui::Ui) -> R,
) {
    dialog_body(ctx, id, title, width, pad, 18, 18, 12.0, None, add);
}

#[allow(clippy::too_many_arguments)]
fn dialog_body<R>(
    ctx: &egui::Context,
    id: &str,
    title: Option<&str>,
    width: f32,
    pad: i8,
    top: i8,
    bottom: i8,
    title_gap: f32,
    tile: Option<&'static str>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) {
    let pad_f = pad as f32;
    let pal = Palette::of(ctx);
    // A little lighter than the window behind it, so the dialog lifts off the page.
    let fill = dialog_fill(&pal);
    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, if pal.dark { DIALOG_BORDER_DARK } else { pal.separator }))
        .corner_radius(10)
        .inner_margin(egui::Margin { left: pad, right: pad, top, bottom })
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(if pal.dark { 140 } else { 55 }),
        });
    // Modal: a backdrop covers the app and the widgets behind it neither click nor take keys.
    egui::Modal::new(egui::Id::NULL.with(("dialog", id)))
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(if pal.dark { 110 } else { 50 }))
        .show(ctx, |ui| {
            // The dialog is exactly `width` wide: its contents wrap or scroll inside it, never widen it.
            ui.set_width(width);
            ui.set_max_width(width);
            // Input fields sit darker than the dialog around them.
            ui.visuals_mut().extreme_bg_color = if pal.dark { DIALOG_FIELD_DARK } else { DIALOG_FIELD_LIGHT };
            // The title band (dark mode) and the rule under the title span the whole dialog. Their shapes are reserved
            // now, so they are drawn behind the title, and filled in once the body has been laid out (so they follow the
            // dialog's real width, not the one asked for).
            let mut title_band = None;
            if let Some(t) = title {
                let left = ui.cursor().left();
                let band_top = ui.cursor().top() - top as f32;
                let band = ui.painter().add(egui::Shape::Noop);
                // The title keeps the usual 30 pt indent, whatever the body's padding is.
                // Dialogs with a tile put the tile and title 16 pt from the edge; the others keep the usual 30 pt.
                let indent = if tile.is_some() { 16.0 } else { 30.0 };
                ui.horizontal(|ui| {
                    ui.add_space(indent - pad_f);
                    if let Some(glyph) = tile {
                        self::tile(ui, &pal, 20.0, glyph);
                        ui.add_space(8.0);
                    }
                    ui.label(egui::RichText::new(t).size(15.0).strong().color(pal.text));
                });
                ui.add_space(10.0);
                let y = ui.cursor().top();
                ui.add_space(title_gap);
                title_band = Some((left, band_top, y, band));
            }
            add(ui);
            if let Some((left, band_top, y, band)) = title_band {
                let right = (left + width).max(ui.min_rect().right());
                let line = if pal.dark { DIALOG_BORDER_DARK } else { pal.separator };
                // Painted after the body, so fills in the body (the Project Options panes) do not cover the rule.
                ui.painter().hline(left - pad_f..=right + pad_f, y, Stroke::new(1.0, line));
                if pal.dark {
                    let rect = egui::Rect::from_min_max(
                        egui::pos2(left - pad_f, band_top),
                        egui::pos2(right + pad_f, y),
                    );
                    let corners = egui::CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 };
                    ui.painter().set(band, egui::Shape::rect_filled(rect, corners, DIALOG_TITLE_DARK));
                }
            }
        });
}

/// A dialog button drawn like a macOS push button: `fill`/`hover` behind `fg` text.
fn push_button(ui: &mut egui::Ui, text: &str, enabled: bool, fill: [Color32; 3], stroke: Color32, fg: Color32, bold: bool) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for (state, f) in [(&mut w.inactive, fill[0]), (&mut w.hovered, fill[1]), (&mut w.active, fill[2])] {
            state.weak_bg_fill = f;
            state.bg_fill = f;
            state.bg_stroke = Stroke::new(1.0, stroke);
            state.fg_stroke = Stroke::new(1.0, fg);
            state.corner_radius = egui::CornerRadius::same(6);
        }
        let mut label = egui::RichText::new(text).color(fg);
        if bold {
            label = label.strong();
        }
        ui.add_enabled(enabled, egui::Button::new(label).min_size(vec2(72.0, 24.0)))
    })
    .inner
}

/// A row of dialog buttons against the right edge. Add the main button first: it ends up rightmost,
/// the others to its left, as in a macOS dialog.
pub fn buttons<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add).inner).inner
}

/// The main button of a dialog (Save, Export, Done): macOS blue with white text.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    primary_button_enabled(ui, true, text)
}

pub fn primary_button_enabled(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let pal = Palette::of(ui.ctx());
    let a = pal.accent;
    let (hover, active) = if pal.dark {
        (rgb(0x3D, 0x95, 0xFF), rgb(0x00, 0x6F, 0xE0))
    } else {
        (rgb(0x2F, 0x8C, 0xFF), rgb(0x00, 0x62, 0xD6))
    };
    let fg = if enabled { Color32::WHITE } else { Color32::from_white_alpha(170) };
    push_button(ui, text, enabled, [a, hover, active], a, fg, true)
}

/// A button that destroys something (Move to Trash): macOS red with white text, the same size as the others.
pub fn destructive_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let red = rgb(0xFF, 0x3B, 0x30);
    let (hover, active) = (rgb(0xFF, 0x5E, 0x55), rgb(0xD7, 0x00, 0x15));
    push_button(ui, text, true, [red, hover, active], red, Color32::WHITE, true)
}

/// The other buttons of a dialog (Cancel, Close): dark grey in dark mode, white with a hairline in light mode.
pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let pal = Palette::of(ui.ctx());
    let (fill, stroke) = if pal.dark {
        ([rgb(0x5A, 0x5A, 0x5E), rgb(0x68, 0x68, 0x6C), rgb(0x4A, 0x4A, 0x4E)], rgb(0x6E, 0x6E, 0x72))
    } else {
        ([rgb(0xFF, 0xFF, 0xFF), rgb(0xF2, 0xF2, 0xF4), rgb(0xE4, 0xE4, 0xE7)], rgb(0xC9, 0xC9, 0xCE))
    };
    push_button(ui, text, true, fill, stroke, pal.text, false)
}

/// The Run button: a rounded button with a green play triangle and the word "Run". Disabled, it is dimmed and
/// does not react to clicks (check `enabled` before acting on the response).
pub fn run_button(ui: &mut egui::Ui, pal: &Palette, enabled: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.5);
    let text_color = if enabled { pal.text } else { pal.text_tertiary };
    let label_width = ui
        .painter()
        .layout_no_wrap("Run".into(), font.clone(), text_color)
        .size()
        .x;
    let (rect, resp) = ui.allocate_exact_size(vec2(label_width + 34.0, 24.0), egui::Sense::click());
    let painter = ui.painter();
    let fill = if pal.dark {
        rgb(0x2C, 0x2C, 0x30)
    } else {
        rgb(0xF1, 0xF1, 0xF3)
    };
    let fill = if enabled && resp.hovered() { pal.row_selected } else { fill };
    painter.rect(
        rect,
        6.0,
        fill,
        Stroke::new(1.0, pal.separator),
        egui::StrokeKind::Inside,
    );
    // A filled play triangle, green when it can run.
    let play = if enabled { rgb(0x34, 0xC7, 0x59) } else { pal.text_tertiary };
    let c = egui::pos2(rect.left() + 14.0, rect.center().y);
    painter.add(Shape::convex_polygon(
        vec![c + vec2(-3.5, -4.5), c + vec2(-3.5, 4.5), c + vec2(4.5, 0.0)],
        play,
        Stroke::NONE,
    ));
    painter.text(
        egui::pos2(rect.left() + 26.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Run",
        font,
        text_color,
    );
    let state = if enabled { "Run" } else { "Run (unavailable)" };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, state));
    resp
}

pub fn pill<R>(ui: &mut egui::Ui, pal: &Palette, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(pal.chrome)
        .stroke(Stroke::new(1.0, pal.separator))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(5, 3))
        .shadow(egui::epaint::Shadow {
            offset: [0, 1],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(if pal.dark { 90 } else { 22 }),
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(add).inner
        })
        .inner
}

/// A thin horizontal rule across the full width of `ui`.
pub fn hline(ui: &mut egui::Ui, pal: &Palette) {
    let r = ui.available_rect_before_wrap();
    let y = ui.cursor().min.y;
    ui.painter()
        .hline(r.x_range(), y, Stroke::new(1.0, pal.separator));
    ui.add_space(1.0);
}

/// The blue-grey folder Xcode shows in the navigator.
pub fn paint_folder(painter: &egui::Painter, rect: Rect, pal: &Palette) {
    let col = if pal.dark {
        Color32::from_rgb(0x6C, 0x9B, 0xD6)
    } else {
        Color32::from_rgb(0x5E, 0x93, 0xD6)
    };
    crate::app_icons::paint(painter, rect, "folder", col);
}

/// A small page with a colour that depends on the file type.
pub fn paint_file(painter: &egui::Painter, rect: Rect, ext: &str, pal: &Palette, active: bool) {
    // Grey like the other rows; the database is amber; the open file takes the accent colour.
    let ext = ext.to_ascii_lowercase();
    let (name, col) = match ext.as_str() {
        "sqlite" | "sqlite3" | "db" => ("database", Color32::from_rgb(0xF5, 0xA5, 0x24)),
        _ => ("file-code", pal.text_secondary),
    };
    let col = if active { pal.accent } else { col };
    crate::app_icons::paint(painter, rect, name, col);
}

/// The ▸ / ▾ in front of a folder row.
pub fn paint_disclosure(painter: &egui::Painter, center: Pos2, open: bool, color: Color32) {
    let pts = if open {
        vec![
            center + vec2(-3.5, -2.0),
            center + vec2(3.5, -2.0),
            center + vec2(0.0, 2.5),
        ]
    } else {
        vec![
            center + vec2(-2.0, -3.5),
            center + vec2(-2.0, 3.5),
            center + vec2(2.5, 0.0),
        ]
    };
    painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// A small rounded tag with one or two letters, tinted with `color` (the kind of a variable).
pub fn paint_badge(painter: &egui::Painter, rect: Rect, text: &str, color: Color32) {
    painter.rect_filled(rect, 4.0, color.gamma_multiply(0.2));
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(0.75, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(9.5),
        color,
    );
}

/// A flat text segment, like the Code / Form switch in Xcode's toolbar.
pub fn segment(ui: &mut egui::Ui, pal: &Palette, label: &str, selected: bool) -> egui::Response {
    let color = if selected {
        pal.text
    } else {
        pal.text_secondary
    };
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), color);
    let (rect, resp) =
        ui.allocate_exact_size(vec2(galley.size().x + 18.0, 22.0), egui::Sense::click());
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let fill = if selected {
        pal.row_selected
    } else if resp.hovered() {
        pal.row_hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// Runs `add` with thin, dark scroll bars: a narrow handle in grey with a track that blends into the strip.
/// For the horizontal scroll rows (the editor's tabs, the designer's component tabs).
pub fn thin_scroll<R>(ui: &mut egui::Ui, pal: &Palette, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().scroll = egui::style::ScrollStyle::thin();
        let visuals = ui.visuals_mut();
        let handle = if pal.dark { rgb(0x4A, 0x4A, 0x50) } else { rgb(0xC4, 0xC4, 0xC8) };
        visuals.widgets.inactive.bg_fill = handle;
        visuals.widgets.hovered.bg_fill = handle.gamma_multiply(1.3);
        visuals.widgets.active.bg_fill = handle.gamma_multiply(1.5);
        visuals.extreme_bg_color = pal.sidebar;
        add(ui)
    })
    .inner
}

/// A tab button for a side panel: full width, blue when it is the chosen tab. True when clicked.
pub fn side_tab(ui: &mut egui::Ui, pal: &Palette, icon: &str, label: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::click());
    let fill = if selected {
        pal.accent
    } else if resp.hovered() {
        pal.row_hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let color = if selected { Color32::WHITE } else { pal.text_secondary };
    // The icon, then the label after it.
    let icon_rect = Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(15.0, 15.0));
    crate::app_icons::paint(ui.painter(), icon_rect, icon, color);
    ui.painter().text(
        rect.left_center() + vec2(34.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        color,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A segment of a segmented control in the accent colour when selected (blue, white text); a flat label
/// otherwise. Used by the Preferences window.
pub fn accent_segment(ui: &mut egui::Ui, pal: &Palette, label: &str, selected: bool) -> egui::Response {
    let color = if selected {
        egui::Color32::WHITE
    } else {
        pal.text_secondary
    };
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), color);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 22.0), egui::Sense::click());
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let fill = if selected {
        pal.accent
    } else if resp.hovered() {
        pal.row_hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// A full-width segmented control (a pill with one equal segment per label). The chosen segment is blue. Returns the
/// index of the segment clicked this frame.
pub fn segmented<const N: usize>(ui: &mut egui::Ui, pal: &Palette, labels: [&str; N], selected: [bool; N]) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(pal.chrome)
        .stroke(Stroke::new(1.0, pal.separator))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(3, 3))
        .shadow(egui::epaint::Shadow {
            offset: [0, 1],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(if pal.dark { 90 } else { 22 }),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let gap = 2.0;
            let width = (ui.available_width() - gap * (N as f32 - 1.0)) / N as f32;
            ui.spacing_mut().item_spacing.x = gap;
            ui.horizontal(|ui| {
                for (i, (label, on)) in labels.iter().zip(selected).enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), egui::Sense::click());
                    let enabled = ui.is_enabled();
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
                    let color = if on { Color32::WHITE } else { pal.text_secondary };
                    let fill = if on {
                        pal.accent
                    } else if resp.hovered() {
                        pal.row_hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, 6.0, fill);
                    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), color);
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}

/// A row of real tabs: the chosen one has rounded top corners and the colour `fill` of the panel
/// under it, so the two read as one piece; the others lie flat on the (darker) strip behind them.
/// Draw it on a strip filled with `pal.sidebar`. Returns the tab that was clicked and the chosen
/// tab's rectangle.
pub fn page_tabs(
    ui: &mut egui::Ui,
    pal: &Palette,
    names: &[&str],
    icons: &[&str],
    selected: usize,
    fill: Color32,
) -> (Option<usize>, Option<egui::Rect>) {
    let mut clicked = None;
    let mut active_rect = None;
    for (i, name) in names.iter().enumerate() {
        let active = i == selected;
        let color = if active { pal.text } else { pal.text_secondary };
        let galley =
            ui.painter()
                .layout_no_wrap(name.to_string(), egui::FontId::proportional(12.5), color);
        // An optional icon (a file in assets/icons) before the name.
        let icon = icons.get(i).copied().filter(|s| !s.is_empty());
        let icon_space = if icon.is_some() { 16.0 } else { 0.0 };
        let (rect, resp) = ui.allocate_exact_size(
            vec2(galley.size().x + 28.0 + icon_space, 28.0),
            egui::Sense::click(),
        );
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, *name));
        let top = egui::CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        if active {
            active_rect = Some(rect);
            // One pixel taller, so no seam shows between the tab and the panel below.
            ui.painter()
                .rect_filled(rect.expand2(vec2(0.0, 0.5)), top, fill);
            ui.painter().rect_stroke(
                rect,
                top,
                Stroke::new(1.0, pal.separator),
                egui::StrokeKind::Inside,
            );
            let seam = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 1.0, rect.bottom() - 1.0),
                egui::pos2(rect.right() - 1.0, rect.bottom() + 1.0),
            );
            ui.painter().rect_filled(seam, 0.0, fill);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect.shrink2(vec2(0.0, 3.0)), 6.0, pal.row_hover);
        }
        let text_pos = rect.center() - galley.size() / 2.0 + vec2(icon_space / 2.0, 0.5);
        if let Some(icon) = icon {
            let icon_rect = Rect::from_center_size(
                egui::pos2(text_pos.x - 9.0, rect.center().y),
                vec2(12.0, 12.0),
            );
            crate::app_icons::paint(ui.painter(), icon_rect, icon, color);
        }
        ui.painter().galley(
            text_pos,
            galley,
            color,
        );
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    (clicked, active_rect)
}

/// A strip of real tabs on the `pal.sidebar` colour, with the separator line under it broken
/// where the chosen tab joins the panel below (filled `fill`). `extra` draws controls at the
/// strip's right end. Returns the clicked tab.
pub fn tab_bar(
    ui: &mut egui::Ui,
    pal: &Palette,
    names: &[&str],
    icons: &[&str],
    selected: usize,
    fill: Color32,
    extra: impl FnOnce(&mut egui::Ui),
) -> Option<usize> {
    let mut clicked = None;
    let mut active = None;
    let strip = egui::Frame::new()
        .fill(pal.sidebar)
        .inner_margin(egui::Margin {
            left: 8,
            // The right-hand actions (Clear, Restart...) get the same inset as the first tab.
            right: 16,
            top: 6,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                (clicked, active) = page_tabs(ui, pal, names, icons, selected, fill);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    extra(ui);
                });
            });
        });
    let y = strip.response.rect.bottom() - 0.5;
    let (l, r) = (strip.response.rect.left(), strip.response.rect.right());
    let line = Stroke::new(1.0, pal.separator);
    match active {
        Some(a) => {
            ui.painter()
                .line_segment([egui::pos2(l, y), egui::pos2(a.left(), y)], line);
            ui.painter()
                .line_segment([egui::pos2(a.right(), y), egui::pos2(r, y)], line);
        }
        None => {
            ui.painter()
                .line_segment([egui::pos2(l, y), egui::pos2(r, y)], line);
        }
    }
    clicked
}
