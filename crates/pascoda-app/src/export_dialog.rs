//! The "Export Application" dialog: name, bundle identifier, version, icon and the CPUs it runs on,
//! remembered per project in `pascoda.toml`.

use eframe::egui;
use pascoda_fpc::Arch;
use pascoda_fpc::lcl::BundleInfo;
use pascoda_project::ExportSettings;
use std::path::{Path, PathBuf};

pub struct ExportDialog {
    pub settings: ExportSettings,
    /// A GUI program becomes an app bundle; a console program is only a renamed executable.
    pub gui: bool,
    /// Whether the compiler that makes Apple Silicon code is installed (it is not on an Intel Mac).
    pub arm: bool,
    /// Whether the compiler for Intel Macs is installed (needed for an Intel or a universal app).
    pub intel: bool,
    /// Whether the Windows units are installed (needed for a Windows exe).
    pub windows: bool,
    pub error: Option<String>,
}

pub enum Answer {
    Cancel,
    /// Valid settings: ask for a folder and export.
    Export(String, BundleInfo, Arch),
}

/// The icon path as the compiler side wants it: relative paths are from the project folder.
pub fn resolve_icon(root: &Path, icon: &str) -> Option<PathBuf> {
    let icon = icon.trim();
    (!icon.is_empty()).then(|| {
        let p = Path::new(icon);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    })
}

impl ExportDialog {
    pub fn new(settings: ExportSettings, program_name: &str, gui: bool) -> Self {
        let intel = pascoda_fpc::bundled::cross_fpc().is_some();
        let windows = pascoda_fpc::bundled::cross_fpc_for(pascoda_fpc::Target::WinX64).is_some();
        ExportDialog::with_targets(settings, program_name, gui, intel, windows)
    }

    /// `intel` and `windows` say which cross targets are installed; the Apple Silicon compiler is looked for here.
    pub fn with_targets(mut settings: ExportSettings, program_name: &str, gui: bool, intel: bool, windows: bool) -> Self {
        if settings.app_name.trim().is_empty() {
            settings.app_name = program_name.to_string();
        }
        ExportDialog {
            settings,
            gui,
            arm: pascoda_fpc::Target::MacArm64.compiler().is_some(),
            intel,
            windows,
            error: None,
        }
    }

    /// What is chosen. A stored choice whose compiler is not installed falls back to Apple Silicon, so the
    /// export cannot fail for that reason.
    pub fn arch(&self) -> Arch {
        let default = if self.arm { Arch::Arm64 } else { Arch::Intel };
        match Arch::from_name(&self.settings.arch) {
            Arch::Universal if self.arm && self.intel => Arch::Universal,
            Arch::Windows if self.windows => Arch::Windows,
            Arch::Intel if self.intel => Arch::Intel,
            Arch::Arm64 if self.arm => Arch::Arm64,
            _ => default,
        }
    }

    /// What Export will create, with the file names as they will really be (the name as typed,
    /// made safe for a file name).
    pub fn summary(&self) -> String {
        let name = pascoda_fpc::export::clean_name(&self.settings.app_name);
        let runs = format!("Runs on {}.", self.arch().label());
        if self.arch() == Arch::Windows {
            return format!("Creates {name}.exe and {name}.zip in the folder you choose. {runs}");
        }
        if self.gui {
            format!(
                "Creates {name}.app and {name}.dmg in the folder you choose. {runs} The identifier is what macOS knows the app by; use your own reverse-domain name."
            )
        } else {
            format!(
                "Creates the executable {name} and {name}.dmg holding it, in the folder you choose. {runs}"
            )
        }
    }

    /// The checked result of the form, or the sentence to show.
    pub fn validate(&self, root: &Path) -> Result<(String, BundleInfo), String> {
        let name = self.settings.app_name.trim();
        if name.is_empty() {
            return Err("The app needs a name.".into());
        }
        // A Windows program has no bundle identifier; whatever is left in the field is not checked.
        let windows = self.arch() == Arch::Windows;
        let info = BundleInfo {
            bundle_id: if windows { String::new() } else { self.settings.bundle_id.trim().to_string() },
            version: self.settings.version.trim().to_string(),
            icon: resolve_icon(root, &self.settings.icon),
            // Each minimum applies to its own kind of build only; the other field is not checked.
            min_macos: if windows { String::new() } else { self.settings.min_macos.trim().to_string() },
            min_windows: if windows { self.settings.min_windows.trim().to_string() } else { String::new() },
        };
        info.check_minimums()?;
        if self.gui || windows {
            info.check()?;
        }
        Ok((name.to_string(), info))
    }

    pub fn show(&mut self, ctx: &egui::Context, root: &Path) -> Option<Answer> {
        let mut answer = None;
        let pal = crate::style::Palette::of(ctx);
        crate::style::dialog_with_tile(ctx, "export-dialog", "Export Application", DIALOG_WIDTH, |ui| {
            let gui = self.gui;
            ui.spacing_mut().item_spacing.y = 10.0;
            let current = self.arch();
            let windows = current == Arch::Windows;
            egui::Grid::new("export-fields")
                .num_columns(2)
                .spacing([16.0, 18.0])
                .min_col_width(LABEL_WIDTH)
                .show(ui, |ui| {
                    let l = ui.label(egui::RichText::new("App name").color(pal.text_secondary));
                    ui.add(egui::TextEdit::singleline(&mut self.settings.app_name).desired_width(FIELD_WIDTH))
                        .labelled_by(l.id);
                    ui.end_row();

                    // Runs on: four cards in a grid. A choice whose compiler is not installed is disabled, and the
                    // reason shows on hover.
                    let l = ui.label(egui::RichText::new("Runs on").color(pal.text_secondary));
                    let mut choice = current;
                    egui::Grid::new("export-runs-on").num_columns(2).spacing([10.0, 10.0]).show(ui, |ui| {
                        for (i, (arch, title, subtitle, ok, missing)) in [
                            (
                                Arch::Universal,
                                "Apple Silicon + Intel",
                                "Universal 2 Binary",
                                self.arm && self.intel,
                                "A universal app needs both compilers, and this installation does not have them.",
                            ),
                            (
                                Arch::Arm64,
                                "Apple Silicon",
                                "M1 / M2 / M3 / M4 (ARM64)",
                                self.arm,
                                "The Apple Silicon compiler is not in this installation (an Intel Mac has only the Intel one).",
                            ),
                            (
                                Arch::Intel,
                                "Apple Intel",
                                "Legacy x86_64 Architecture",
                                self.intel,
                                "The Intel compiler is missing from this installation (a checkout builds it with packaging/fpc-build.sh and packaging/fpc-intel-build.sh).",
                            ),
                            (
                                Arch::Windows,
                                "Windows 64-bit",
                                "Executable (.exe)",
                                self.windows,
                                "The Windows units are missing from this installation (a checkout builds them with packaging/fpc-build.sh).",
                            ),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if run_card(ui, &pal, title, subtitle, choice == arch, ok, missing) && ok {
                                choice = arch;
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                    self.settings.arch = choice.name().into();
                    ui.end_row();
                    let _ = l;

                    if !windows {
                        let l = ui.label(egui::RichText::new("Minimum macOS").color(pal.text_secondary));
                        let value = {
                            let v = self.settings.min_macos.trim();
                            if v.is_empty() { "14.0".to_string() } else { v.to_string() }
                        };
                        let shown = MACOS_VERSIONS
                            .iter()
                            .find(|(v, _)| *v == value)
                            .map_or(value.clone(), |(_, name)| name.to_string());
                        egui::ComboBox::from_id_salt("export-min-macos")
                            .width(FIELD_WIDTH)
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                for (v, name) in MACOS_VERSIONS {
                                    if ui.selectable_label(value == *v, (*name).to_string()).clicked() {
                                        self.settings.min_macos = (*v).to_string();
                                    }
                                }
                            })
                            .response
                            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Minimum macOS"));
                        let _ = l;
                        ui.end_row();
                    } else {
                        let l = ui.label(egui::RichText::new("Minimum Windows").color(pal.text_secondary));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.min_windows)
                                .hint_text("6.1 (Windows 7), 10.0 (Windows 10)")
                                .desired_width(FIELD_WIDTH),
                        )
                        .labelled_by(l.id);
                        ui.end_row();
                    }

                    if gui && !windows {
                        let l = ui.label(egui::RichText::new("Bundle identifier").color(pal.text_secondary));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.bundle_id)
                                .hint_text("com.example.myapp")
                                .desired_width(FIELD_WIDTH),
                        )
                        .labelled_by(l.id);
                        ui.end_row();
                    }

                    // Version and icon: an app bundle's, or a Windows program's version information and icon.
                    if gui || windows {
                        let l = ui.label(egui::RichText::new("Version").color(pal.text_secondary));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.version)
                                .hint_text("1.0")
                                .desired_width(FIELD_WIDTH),
                        )
                        .labelled_by(l.id);
                        ui.end_row();

                        let _ = ui.label(egui::RichText::new("Icon").color(pal.text_secondary));
                        ui.horizontal(|ui| {
                            icon_tile(ui, &pal);
                            let file = icon_name(&self.settings.icon);
                            egui::Frame::new()
                                .stroke(egui::Stroke::new(1.0, pal.separator))
                                .corner_radius(8)
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.set_width(FIELD_WIDTH - 46.0);
                                    ui.horizontal(|ui| {
                                        let text = if file.is_empty() { "No icon chosen" } else { file.as_str() };
                                        ui.label(egui::RichText::new(text).monospace().size(12.5).color(
                                            if file.is_empty() { pal.text_tertiary } else { pal.text },
                                        ));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.button("Choose...").clicked()
                                                && let Some(f) = rfd::FileDialog::new()
                                                    .add_filter("Icon", &["png", "icns"])
                                                    .set_directory(root)
                                                    .pick_file()
                                            {
                                                self.settings.icon = f.to_string_lossy().into_owned();
                                            }
                                        });
                                    });
                                });
                        });
                        ui.end_row();
                    }
                });

            // What Export creates, the architecture and the identifier.
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(pal.sidebar)
                .stroke(egui::Stroke::new(1.0, pal.separator))
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(egui::RichText::new(self.summary()).color(pal.text_secondary).size(12.5)).wrap());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Architecture:").color(pal.text_tertiary).size(12.0));
                        ui.label(egui::RichText::new(current.label()).size(12.0));
                        if !windows && gui {
                            ui.add_space(14.0);
                            ui.label(egui::RichText::new("Identifier:").color(pal.text_tertiary).size(12.0));
                            let id = self.settings.bundle_id.trim();
                            let id = if id.is_empty() { "(none)" } else { id };
                            ui.label(egui::RichText::new(id).monospace().size(12.0));
                        }
                    });
                });
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(6.0);
            crate::style::buttons(ui, |ui| {
                if crate::style::primary_button(ui, "Export...").clicked() {
                    match self.validate(root) {
                        Ok((name, info)) => {
                            self.error = None;
                            answer = Some(Answer::Export(name, info, self.arch()));
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                if crate::style::secondary_button(ui, "Cancel").clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(Answer::Cancel);
        }
        answer
    }
}

/// Every field of the form is this wide: a 120 pt label column, a 16 pt gap and this column. The dialog is as wide as
/// the label, the gap and the column together, so the title rule and the fields line up.
const FIELD_WIDTH: f32 = 470.0;
/// The label column.
const LABEL_WIDTH: f32 = 120.0;
const DIALOG_WIDTH: f32 = LABEL_WIDTH + 16.0 + FIELD_WIDTH;

/// The macOS releases the Minimum macOS list offers: the stored value, and the name shown for it.
const MACOS_VERSIONS: [(&str, &str); 7] = [
    ("11.0", "11.0 Big Sur"),
    ("12.0", "12.0 Monterey"),
    ("13.0", "13.0 Ventura"),
    ("14.0", "14.0 Sonoma (Standard)"),
    ("15.0", "15.0 Sequoia"),
    ("26.0", "26.0 Tahoe"),
    ("27.0", "27.0"),
];

/// One option card of "Runs on": a title, a caption and a radio dot. Returns true when clicked.
fn run_card(
    ui: &mut egui::Ui,
    pal: &crate::style::Palette,
    title: &str,
    subtitle: &str,
    selected: bool,
    enabled: bool,
    reason: &str,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(230.0, 52.0), egui::Sense::click());
    let resp = if enabled { resp } else { resp.on_hover_text(reason) };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, title));
    let painter = ui.painter();
    let fill = if selected {
        pal.accent.gamma_multiply(0.12)
    } else if pal.dark {
        crate::style::DIALOG_PANE_DARK
    } else {
        egui::Color32::TRANSPARENT
    };
    let stroke = if selected { pal.accent } else { pal.separator };
    painter.rect(rect, 8.0, fill, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    let text_color = if !enabled {
        pal.text_tertiary
    } else if selected {
        pal.accent
    } else {
        pal.text
    };
    painter.text(
        rect.left_top() + egui::vec2(12.0, 14.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(13.0),
        text_color,
    );
    painter.text(
        rect.left_top() + egui::vec2(12.0, 36.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        egui::FontId::proportional(11.0),
        pal.text_tertiary,
    );
    // The radio dot.
    let dot = egui::pos2(rect.right() - 16.0, rect.center().y);
    painter.circle_stroke(dot, 7.0, egui::Stroke::new(1.5, stroke));
    if selected {
        painter.circle_filled(dot, 3.5, pal.accent);
    }
    resp.clicked() && enabled
}

/// The preview tile of the icon row: the blue box.
fn icon_tile(ui: &mut egui::Ui, pal: &crate::style::Palette) {
    crate::style::app_tile(ui, pal, 38.0);
}

/// The file name of an icon path, for the icon row ("" when none).
fn icon_name(icon: &str) -> String {
    Path::new(icon.trim())
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(gui: bool) -> ExportDialog {
        ExportDialog::with_targets(ExportSettings::default(), "notes", gui, true, true)
    }

    #[test]
    fn universal_needs_the_intel_compiler() {
        let mut d = dialog(true);
        assert_eq!(d.arch(), Arch::Arm64, "the default");
        d.settings.arch = "universal".into();
        assert_eq!(d.arch(), Arch::Universal);
        assert!(d.summary().contains("Apple Silicon + Intel"), "{}", d.summary());
        d.intel = false;
        assert_eq!(d.arch(), Arch::Arm64, "no Intel compiler: back to Apple Silicon");
        assert!(d.summary().contains("Runs on Apple Silicon."), "{}", d.summary());
    }

    #[test]
    fn windows_needs_its_units_and_names_the_exe_and_zip() {
        let mut d = dialog(true);
        d.settings.arch = "windows".into();
        assert_eq!(d.arch(), Arch::Windows);
        assert!(d.summary().contains("notes.exe and notes.zip"), "{}", d.summary());
        assert!(!d.summary().contains(".dmg"), "{}", d.summary());
        d.windows = false;
        assert_eq!(d.arch(), Arch::Arm64, "no Windows units: back to Apple Silicon");
    }

    #[test]
    fn the_name_defaults_to_the_program_and_empty_fields_are_fine() {
        let d = dialog(true);
        assert_eq!(d.settings.app_name, "notes");
        let (name, info) = d.validate(Path::new("/p")).unwrap();
        assert_eq!(name, "notes");
        assert_eq!(info, BundleInfo::default());
    }

    #[test]
    fn the_summary_names_the_real_files() {
        let mut d = dialog(true);
        assert!(
            d.summary().contains("notes.app and notes.dmg"),
            "{}",
            d.summary()
        );
        d.settings.app_name = "My App/1".into();
        assert!(
            d.summary().contains("My App_1.app and My App_1.dmg"),
            "{}",
            d.summary()
        );
        let c = dialog(false);
        assert!(
            c.summary().contains("executable notes and notes.dmg"),
            "{}",
            c.summary()
        );
        assert!(!d.summary().contains('<'));
    }

    #[test]
    fn bad_fields_are_explained_and_relative_icons_follow_the_project() {
        let mut d = dialog(true);
        d.settings.bundle_id = "nodots".into();
        assert!(
            d.validate(Path::new("/p"))
                .unwrap_err()
                .contains("bundle identifier")
        );
        d.settings.bundle_id = "com.example.n".into();
        d.settings.icon = "art/i.png".into();
        assert!(
            d.validate(Path::new("/p")).unwrap_err().contains("icon"),
            "missing file"
        );
        assert_eq!(
            resolve_icon(Path::new("/p"), "art/i.png"),
            Some("/p/art/i.png".into())
        );
        assert_eq!(
            resolve_icon(Path::new("/p"), "/x/i.icns"),
            Some("/x/i.icns".into())
        );
        assert_eq!(resolve_icon(Path::new("/p"), "  "), None);
        d.settings.app_name = "  ".into();
        assert!(d.validate(Path::new("/p")).unwrap_err().contains("name"));
        // a console program ignores the bundle fields
        let mut c = dialog(false);
        c.settings.bundle_id = "nodots".into();
        assert!(c.validate(Path::new("/p")).is_ok());
    }
}
