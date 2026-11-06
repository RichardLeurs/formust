// formust 1 | form1.rs | Form1::ui | src-hash 9a10d06f

use eframe::egui;

#[derive(Default)]
pub struct Form1 {
    name: String,
    agree: bool,
    form: Form1Form,
}

impl Form1 {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("panel_3").resizable(false).show_separator_line(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_width(173.0);
                ui.set_min_height(27.0);
                ui.horizontal(|ui| {
                    ui.horizontal(|ui| {
                        ui.horizontal(|ui| {
                            ui.horizontal(|ui| {
                                ui.horizontal(|ui| {
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    ui.add_sized([124.0, 20.0], egui::Label::new("Test"));
                });
                                });
                            });
                        });
                    });
                });
                ui.add_sized([280.0, 22.0], egui::TextEdit::singleline(&mut self.form.text).desired_rows(1));
            });
        });
        egui::Panel::left("panel_8").resizable(false).show(ui, |ui| {
            egui::Panel::bottom("panel_7").show(ui, |ui| {});
        });
    }
}

#[derive(Default)] pub struct Form1Form { pub text: String }
