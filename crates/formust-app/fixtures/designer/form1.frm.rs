// formust 1 | form1.rs | Form1::ui | src-hash 9a10d06f
impl Form1 {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("panel_8").show(ui, |ui| {
            egui::Panel::bottom("panel_7").show(ui, |ui| {});
        });
    }
}
