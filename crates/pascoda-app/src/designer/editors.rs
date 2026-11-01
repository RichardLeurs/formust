//! Property editors for colours, fonts and collections (spec 47). Each value is edited with a widget built for
//! it instead of as text. The pure parts (what a value means, what an edit writes) are here and tested; the
//! widgets draw them in the inspector and return the value to write.

use eframe::egui::{self, RichText};
use objc2_app_kit::NSFontManager;
use objc2_foundation::MainThreadMarker;
use pascoda_forms::PropValue;
use std::sync::OnceLock;

/// Lazarus's named colours with a fixed value, as `TColor` is stored (`$00BBGGRR`).
pub const NAMED: &[(&str, u32)] = &[
    ("clBlack", 0x000000),
    ("clMaroon", 0x000080),
    ("clGreen", 0x008000),
    ("clOlive", 0x008080),
    ("clNavy", 0x800000),
    ("clPurple", 0x800080),
    ("clTeal", 0x808000),
    ("clGray", 0x808080),
    ("clSilver", 0xC0C0C0),
    ("clRed", 0x0000FF),
    ("clLime", 0x00FF00),
    ("clYellow", 0x00FFFF),
    ("clBlue", 0xFF0000),
    ("clFuchsia", 0xFF00FF),
    ("clAqua", 0xFFFF00),
    ("clWhite", 0xFFFFFF),
    ("clMoneyGreen", 0xC0DCC0),
    ("clSkyBlue", 0xF0CAA6),
    ("clCream", 0xF0FBFF),
    ("clMedGray", 0xA4A0A0),
];

/// System colours: their value comes from the theme, so there is no fixed RGB for them.
pub const SYSTEM: &[&str] = &[
    "clWindow",
    "clWindowText",
    "clBtnFace",
    "clBtnText",
    "clHighlight",
    "clHighlightText",
    "clInfoBk",
    "clInfoText",
    "clNone",
    "clDefault",
];

/// The font styles of a `Font.Style` set, in the order Lazarus writes them.
pub const STYLES: &[(&str, &str)] = &[
    ("fsBold", "Bold"),
    ("fsItalic", "Italic"),
    ("fsUnderline", "Underline"),
    ("fsStrikeOut", "Strikeout"),
];

/// The font families installed on this Mac, for the Font.Name list. AppKit's font manager is main-thread only, so
/// this is empty when asked from another thread (nothing is cached then).
pub fn system_font_families() -> Vec<String> {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    if let Some(families) = FAMILIES.get() {
        return families.clone();
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let raw: Vec<String> = NSFontManager::sharedFontManager(mtm)
        .availableFontFamilies()
        .iter()
        .map(|name| name.to_string())
        .collect();
    FAMILIES.get_or_init(|| font_menu(raw)).clone()
}

/// Family names for a menu: sorted without case, duplicates and hidden families (a leading dot) dropped.
pub fn font_menu(mut names: Vec<String>) -> Vec<String> {
    names.retain(|n| !n.is_empty() && !n.starts_with('.'));
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

/// One item of a collection: its property list.
pub type Item = Vec<(String, PropValue)>;

fn named_colour(name: &str) -> Option<(&'static str, u32)> {
    NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).copied()
}

/// The colour a value stands for, as (red, green, blue), when it has a fixed one.
pub fn rgb(value: &PropValue) -> Option<[u8; 3]> {
    let bgr = match value {
        PropValue::Int(v) => u32::try_from(*v).ok()?,
        PropValue::Ident(name) => named_colour(name)?.1,
        _ => return None,
    };
    (bgr <= 0xFF_FFFF).then(|| [(bgr & 0xFF) as u8, ((bgr >> 8) & 0xFF) as u8, ((bgr >> 16) & 0xFF) as u8])
}

/// The Lazarus name of a colour value (`clRed`, `clWindow`), in its canonical spelling.
pub fn colour_name(value: &PropValue) -> Option<&'static str> {
    let PropValue::Ident(name) = value else {
        return None;
    };
    named_colour(name)
        .map(|(n, _)| n)
        .or_else(|| SYSTEM.iter().copied().find(|n| n.eq_ignore_ascii_case(name)))
}

/// A custom colour, written the way Lazarus writes a `TColor` literal.
pub fn custom(rgb: [u8; 3]) -> PropValue {
    PropValue::Raw(format!("$00{:02X}{:02X}{:02X}", rgb[2], rgb[1], rgb[0]))
}

/// Whether a property is a colour: a `...Color` property (not `ParentColor`, a flag) with a colour value.
pub fn is_colour_prop(name: &str, value: &PropValue) -> bool {
    if !name.ends_with("Color") || name.starts_with("Parent") {
        return false;
    }
    match value {
        PropValue::Int(_) | PropValue::Raw(_) => true,
        PropValue::Ident(s) => !matches!(s.to_ascii_lowercase().as_str(), "true" | "false"),
        _ => false,
    }
}

/// The styles of a set after `style` is switched on or off, in Lazarus's order.
pub fn toggle_style(current: &[String], style: &str, on: bool) -> Vec<String> {
    STYLES
        .iter()
        .filter(|(s, _)| {
            if s.eq_ignore_ascii_case(style) {
                on
            } else {
                current.iter().any(|c| c.eq_ignore_ascii_case(s))
            }
        })
        .map(|(s, _)| s.to_string())
        .collect()
}

/// Whether a property is one of a font's (`Font.Name`, `Font.Style`, ...).
pub fn is_font_prop(name: &str) -> bool {
    name.starts_with("Font.")
}

/// The heading of an item in a list: its text, caption, name or title, else its number.
pub fn title(index: usize, item: &Item) -> String {
    for key in ["Text", "Caption", "Name", "Title"] {
        if let Some((_, PropValue::Str(s) | PropValue::Ident(s))) =
            item.iter().find(|(n, _)| n.eq_ignore_ascii_case(key))
            && !s.is_empty()
        {
            return s.clone();
        }
    }
    format!("Item {}", index + 1)
}

/// The items with an empty one appended.
pub fn add_item(items: &[Item]) -> Vec<Item> {
    let mut out = items.to_vec();
    out.push(Vec::new());
    out
}

/// The items without item `index` (unchanged when there is none).
pub fn remove_item(items: &[Item], index: usize) -> Vec<Item> {
    let mut out = items.to_vec();
    if index < out.len() {
        out.remove(index);
    }
    out
}

/// The items with item `from` moved to position `to` (unchanged when either is out of range).
pub fn move_item(items: &[Item], from: usize, to: usize) -> Vec<Item> {
    let mut out = items.to_vec();
    if from < out.len() && to < out.len() {
        let item = out.remove(from);
        out.insert(to, item);
    }
    out
}

fn property<'a>(props: &'a [(String, PropValue)], name: &str) -> Option<&'a PropValue> {
    props.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v)
}

/// A colour editor: a list of the named colours and a picker for any other. Returns the new value when the
/// user changes it.
pub fn colour_editor(ui: &mut egui::Ui, id: egui::Id, label: &str, value: &PropValue) -> Option<PropValue> {
    let mut changed = None;
    let mut rgb_value = rgb(value).unwrap_or([0x80, 0x80, 0x80]);
    let shown = match (colour_name(value), rgb(value)) {
        (Some(name), _) => name.to_string(),
        (None, Some(_)) => format!("#{:02X}{:02X}{:02X}", rgb_value[0], rgb_value[1], rgb_value[2]),
        (None, None) => "system".to_string(),
    };
    ui.horizontal(|ui| {
        let combo = egui::ComboBox::from_id_salt(id.with("name"))
            .selected_text(shown)
            .width(130.0)
            .show_ui(ui, |ui| {
                for name in NAMED.iter().map(|(n, _)| *n).chain(SYSTEM.iter().copied()) {
                    if ui.selectable_label(colour_name(value) == Some(name), name).clicked() {
                        changed = Some(PropValue::Ident(name.to_string()));
                    }
                }
            });
        let name = label.to_string();
        combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name.clone()));
        if ui.color_edit_button_srgb(&mut rgb_value).changed() {
            changed = Some(custom(rgb_value));
        }
    });
    changed
}

/// One plain property as a label and an editor on the current grid row. Returns the new value when changed.
pub fn field(ui: &mut egui::Ui, label: &str, value: &PropValue) -> Option<PropValue> {
    let lbl = ui.label(label);
    let changed = match value {
        PropValue::Int(i) => {
            let mut v = *i;
            ui.add(egui::DragValue::new(&mut v))
                .labelled_by(lbl.id)
                .changed()
                .then_some(PropValue::Int(v))
        }
        PropValue::Str(s) => {
            let mut v = s.clone();
            ui.add(egui::TextEdit::singleline(&mut v).desired_width(140.0))
                .labelled_by(lbl.id)
                .changed()
                .then_some(PropValue::Str(v))
        }
        PropValue::Ident(s) | PropValue::Raw(s) => {
            let mut v = s.clone();
            ui.add(egui::TextEdit::singleline(&mut v).desired_width(140.0))
                .labelled_by(lbl.id)
                .changed()
                .then_some(PropValue::Ident(v))
        }
        other => {
            ui.weak(format!("{other:?}").chars().take(24).collect::<String>());
            None
        }
    };
    ui.end_row();
    changed
}

/// A font name: free text, with a list of the installed families beside it. Returns the new name when changed.
pub fn family_field(ui: &mut egui::Ui, label: &str, value: &PropValue) -> Option<String> {
    let lbl = ui.label(label);
    let mut text = match value {
        PropValue::Str(s) | PropValue::Ident(s) | PropValue::Raw(s) => s.clone(),
        _ => String::new(),
    };
    let mut changed = None;
    ui.horizontal(|ui| {
        if ui
            .add(egui::TextEdit::singleline(&mut text).desired_width(120.0))
            .labelled_by(lbl.id)
            .changed()
        {
            changed = Some(text.clone());
        }
        let families = system_font_families();
        let combo = egui::ComboBox::from_id_salt("font-families")
            .selected_text("")
            .width(24.0)
            .show_ui(ui, |ui| {
                for family in &families {
                    if ui.selectable_label(*family == text, family).clicked() {
                        changed = Some(family.clone());
                    }
                }
            });
        combo
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Font family"));
    });
    ui.end_row();
    changed
}

/// The `Font.*` properties of a component as fields: Name, Size or Height, the styles, and Color. A font
/// property the component does not have yet is created when it is first changed. Returns the changes;
/// `None` removes a property.
pub fn font_editor(ui: &mut egui::Ui, props: &[(String, PropValue)]) -> Vec<(String, Option<PropValue>)> {
    let mut out = Vec::new();
    egui::Grid::new(("font-editor", props.len()))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            let name = property(props, "Font.Name").cloned().unwrap_or(PropValue::Str(String::new()));
            if let Some(family) = family_field(ui, "Name", &name) {
                out.push(("Font.Name".to_string(), Some(PropValue::Str(family))));
            }
            // A component carries Size or Height, whichever Lazarus wrote (Height is negative pixels).
            for key in ["Font.Size", "Font.Height"] {
                if let Some(v) = property(props, key) {
                    if let Some(v) = field(ui, &key["Font.".len()..], v) {
                        out.push((key.to_string(), Some(v)));
                    }
                    break;
                }
            }
            let current: Vec<String> = match property(props, "Font.Style") {
                Some(PropValue::Set(s)) => s.clone(),
                _ => Vec::new(),
            };
            ui.label("Style");
            ui.horizontal(|ui| {
                for (style, label) in STYLES {
                    let mut on = current.iter().any(|c| c.eq_ignore_ascii_case(style));
                    if ui.checkbox(&mut on, *label).changed() {
                        let set = toggle_style(&current, style, on);
                        let value = (!set.is_empty()).then(|| PropValue::Set(set));
                        out.push(("Font.Style".to_string(), value));
                    }
                }
            });
            ui.end_row();
            let colour = property(props, "Font.Color").cloned().unwrap_or(PropValue::Ident("clWindowText".into()));
            ui.label("Color");
            if let Some(v) = colour_editor(ui, ui.make_persistent_id(("font-colour", props.len())), "Color", &colour) {
                out.push(("Font.Color".to_string(), Some(v)));
            }
            ui.end_row();
        });
    out
}

/// A collection (`Panels`, `Columns`, ...): its items in a list with Add, Remove, Up and Down, and the
/// selected item's properties as fields. Returns the collection to write when the user changes it.
pub fn collection_editor(ui: &mut egui::Ui, id: egui::Id, name: &str, items: &[Item]) -> Option<PropValue> {
    let mut sel = ui
        .data(|d| d.get_temp::<usize>(id))
        .unwrap_or(0)
        .min(items.len().saturating_sub(1));
    let mut out: Option<Vec<Item>> = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{name} ({})", items.len())).strong());
        if ui.small_button("Add").clicked() {
            out = Some(add_item(items));
            sel = items.len();
        }
        if ui.add_enabled(!items.is_empty(), egui::Button::new("Remove").small()).clicked() {
            out = Some(remove_item(items, sel));
            sel = sel.saturating_sub(1);
        }
        if ui.add_enabled(sel > 0 && sel < items.len(), egui::Button::new("Up").small()).clicked() {
            out = Some(move_item(items, sel, sel - 1));
            sel -= 1;
        }
        if ui
            .add_enabled(sel + 1 < items.len(), egui::Button::new("Down").small())
            .clicked()
        {
            out = Some(move_item(items, sel, sel + 1));
            sel += 1;
        }
    });
    for (i, item) in items.iter().enumerate() {
        if ui.selectable_label(sel == i, title(i, item)).clicked() {
            sel = i;
        }
    }
    if let Some(item) = items.get(sel) {
        egui::Grid::new(id.with("fields"))
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (j, (key, value)) in item.iter().enumerate() {
                    let changed = if is_colour_prop(key, value) {
                        ui.label(key);
                        let v = colour_editor(ui, id.with(("colour", j)), key, value);
                        ui.end_row();
                        v
                    } else {
                        field(ui, key, value)
                    };
                    if let Some(v) = changed {
                        let mut new = items.to_vec();
                        new[sel][j].1 = v;
                        out = Some(new);
                    }
                }
            });
    }
    ui.data_mut(|d| d.insert_temp(id, sel));
    out.map(PropValue::Collection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> PropValue {
        PropValue::Str(text.into())
    }

    #[test]
    fn a_named_colour_has_its_rgb_and_a_hex_literal_is_read_as_bgr() {
        assert_eq!(rgb(&PropValue::Ident("clRed".into())), Some([255, 0, 0]));
        assert_eq!(rgb(&PropValue::Ident("CLNAVY".into())), Some([0, 0, 128]));
        // $00BBGGRR: blue 0x20, green 0x10, red 0x08.
        assert_eq!(rgb(&PropValue::Int(0x0020_1008)), Some([8, 16, 32]));
        assert_eq!(rgb(&PropValue::Ident("clWindow".into())), None, "a system colour has no fixed value");
        assert_eq!(rgb(&PropValue::Int(0x1FFF_FFFF)), None, "clNone is not a colour");
    }

    #[test]
    fn a_custom_colour_is_written_as_a_lazarus_literal_and_read_back() {
        let v = custom([8, 16, 32]);
        assert_eq!(v, PropValue::Raw("$00201008".into()));
        // The file reads the literal back as an integer, which is the same colour.
        assert_eq!(rgb(&PropValue::Int(0x0020_1008)), Some([8, 16, 32]));
        assert_eq!(colour_name(&PropValue::Ident("clTeal".into())), Some("clTeal"));
        assert_eq!(colour_name(&PropValue::Int(0)), None);
    }

    #[test]
    fn only_real_colour_properties_get_the_colour_editor() {
        assert!(is_colour_prop("Color", &PropValue::Ident("clBtnFace".into())));
        assert!(is_colour_prop("Font.Color", &PropValue::Ident("clWindowText".into())));
        assert!(is_colour_prop("Color", &PropValue::Int(0)));
        assert!(!is_colour_prop("ParentColor", &PropValue::Ident("True".into())), "a flag");
        assert!(!is_colour_prop("Color", &PropValue::Ident("False".into())), "a flag");
        assert!(!is_colour_prop("Caption", &PropValue::Str("Color".into())));
    }

    #[test]
    fn styles_are_kept_in_lazarus_order_and_removed_when_off() {
        let now = vec!["fsItalic".to_string()];
        assert_eq!(toggle_style(&now, "fsBold", true), ["fsBold", "fsItalic"]);
        assert_eq!(toggle_style(&now, "fsItalic", false), Vec::<String>::new());
        assert_eq!(toggle_style(&[], "fsUnderline", true), ["fsUnderline"]);
    }

    #[test]
    fn an_item_is_titled_by_its_text_or_its_number() {
        let item: Item = vec![("Width".into(), PropValue::Int(50)), ("Text".into(), s("Ready"))];
        assert_eq!(title(0, &item), "Ready");
        assert_eq!(title(2, &vec![("Width".into(), PropValue::Int(50))]), "Item 3");
        assert_eq!(title(0, &vec![("Text".into(), s(""))]), "Item 1", "an empty text does not name it");
    }

    #[test]
    fn the_font_menu_is_sorted_without_hidden_or_repeated_families() {
        let names = vec!["Times", ".SF NS", "helvetica", "", "Menlo", "Helvetica", "Arial"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(font_menu(names), ["Arial", "helvetica", "Menlo", "Times"]);
        assert!(font_menu(Vec::new()).is_empty());
    }

    #[test]
    fn items_are_added_removed_and_moved_without_losing_any() {
        let a: Item = vec![("Text".into(), s("a"))];
        let b: Item = vec![("Text".into(), s("b"))];
        let items = vec![a.clone(), b.clone()];
        assert_eq!(add_item(&items).len(), 3);
        assert!(add_item(&items)[2].is_empty());
        assert_eq!(remove_item(&items, 0), vec![b.clone()]);
        assert_eq!(remove_item(&items, 9), items, "out of range: unchanged");
        assert_eq!(move_item(&items, 0, 1), vec![b.clone(), a.clone()]);
        assert_eq!(move_item(&items, 1, 0), vec![b, a.clone()]);
        assert_eq!(move_item(&items, 0, 5), items, "out of range: unchanged");
    }

    /// The edits the inspector makes reach the form text: a font, a colour and a collection are written, a
    /// removed style is taken out, and the form still reads back.
    #[test]
    fn a_font_a_colour_and_a_collection_are_written_to_the_form() {
        let mut d = crate::designer::tests::designer();
        let p = d.add_component("TButton", &[], 21, 30).unwrap();
        d.set_prop(&p, "Font.Name", PropValue::Str("Helvetica".into())).unwrap();
        d.set_prop(&p, "Font.Style", PropValue::Set(vec!["fsBold".into()])).unwrap();
        d.set_prop(&p, "Color", PropValue::Ident("clRed".into())).unwrap();
        let items = vec![vec![("Text".to_string(), PropValue::Str("Ready".into()))]];
        d.set_prop(&p, "Items", PropValue::Collection(items.clone())).unwrap();
        let text = d.doc.source().to_string();
        assert!(text.contains("Font.Name = 'Helvetica'"), "{text}");
        assert!(text.contains("Font.Style = [fsBold]"), "{text}");
        assert!(text.contains("Color = clRed"), "{text}");
        assert!(text.contains("Items = <"), "{text}");

        // An empty style is taken out, as Lazarus omits it.
        d.remove_prop(&p, "Font.Style").unwrap();
        assert!(!d.doc.source().contains("Font.Style"));
        let back = pascoda_forms::FormDoc::parse(d.doc.source()).unwrap();
        let button = back.component(&p).unwrap();
        assert_eq!(button.prop("Items"), Some(&PropValue::Collection(items)));
        assert_eq!(button.prop("Font.Name"), Some(&PropValue::Str("Helvetica".into())));
    }

    /// Colours read from the file come back as the same colour, whatever literal they were written with.
    #[test]
    fn a_custom_colour_survives_a_write_and_a_read() {
        let mut d = crate::designer::tests::designer();
        let p = d.add_component("TButton", &[], 21, 30).unwrap();
        d.set_prop(&p, "Color", custom([8, 16, 32])).unwrap();
        let back = pascoda_forms::FormDoc::parse(d.doc.source()).unwrap();
        let value = back.component(&p).unwrap().prop("Color").cloned().unwrap();
        assert_eq!(rgb(&value), Some([8, 16, 32]));
    }
}
