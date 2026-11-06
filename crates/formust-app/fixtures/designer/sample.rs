//! The designer's test form: every widget and container of the first palette, a custom block and
//! a second form made of panels. Read by the designer tests in `src/render_tests.rs`; never built.

use eframe::egui;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Size {
    #[default]
    Small,
    Large,
}

#[derive(Default)]
pub struct Sample {
    name: String,
    notes: String,
    remember: bool,
    dark: bool,
    size: Size,
    volume: f32,
    count: f32,
    busy: bool,
}

impl Sample {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sample heading");
        ui.horizontal(|ui| {
            ui.label("Name label");
            ui.text_edit_singleline(&mut self.name);
        });
        ui.horizontal_wrapped(|ui| {
            ui.small("Small text");
            ui.strong("Strong text");
            ui.weak("Weak text");
            ui.monospace("Mono text");
        });
        ui.checkbox(&mut self.remember, "Remember me");
        ui.toggle_value(&mut self.dark, "Dark toggle");
        egui::Grid::new("grid")
            .num_columns(3)
            .show(ui, |ui| {
                ui.radio_value(&mut self.size, Size::Small, "Small radio");
                ui.selectable_value(&mut self.size, Size::Large, "Large choice");
                if ui.button("Sign in").clicked() {
               //     self.sign_in();
                }
                ui.end_row();
            });
        ui.link("A link");
        ui.hyperlink_to("Egui site", "https://www.egui.rs/");
        ui.add(egui::Slider::new(&mut self.volume, 0.0..=100.0).text("Volume"));
        ui.add(egui::DragValue::new(&mut self.count).prefix("n "));
        ui.add(egui::ProgressBar::new(0.4).show_percentage());
        ui.spinner();
        ui.separator();
        ui.add_space(6.0);
        egui::ComboBox::from_label("Size combo")
            .selected_text("Pick a size")
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.size, Size::Small, "Small item");
                ui.selectable_value(&mut self.size, Size::Large, "Large item");
            });
        egui::Grid::new("fields").num_columns(2).show(ui, |ui| {
            ui.label("Grid cell");
            ui.text_edit_multiline(&mut self.notes);
            ui.end_row();
        });
        egui::CollapsingHeader::new("Advanced section").show(ui, |ui| {
            ui.label("Hidden inside");
        });
        egui::ScrollArea::vertical()
            .max_height(40.0)
            .show(ui, |ui| {
                ui.label("Scrolled label");
            });
        ui.group(|ui| {
            ui.vertical_centered(|ui| {
                ui.label("Centred label");
            });
        });
        ui.indent("indent", |ui| {
            ui.vertical(|ui| {
                ui.label("Indented label");
            });
        });
        if self.busy {
            ui.label("Working...");
        }
    }
}

pub struct Panels;

impl eframe::App for Panels {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.heading("Top panel heading");
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Central label");
        });
    }
}
