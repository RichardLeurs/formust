//! Form2: a Formust form. Edit it in the designer (Form), or here as code.

use eframe::egui;

#[derive(Default)]
pub struct Form2 {
    name: String,
    agree: bool,
}

impl Form2 {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Form2");
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.name);
        });
        ui.checkbox(&mut self.agree, "I agree");
        if ui.button("OK").clicked() {
            self.on_ok_clicked();
        }
    }

    fn on_ok_clicked(&mut self) {}
}
