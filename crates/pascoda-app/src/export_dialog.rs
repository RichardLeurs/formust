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
        crate::style::dialog(
            ctx,
            "export-dialog",
            Some("Export Application"),
            380.0,
            |ui| {
                let gui = self.gui;
                egui::Grid::new("export-fields")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        let l = ui.label("App name");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.app_name)
                                .desired_width(240.0),
                        )
                        .labelled_by(l.id);
                        ui.end_row();
                        // One row per choice, so the label lines up with the first option. A choice whose compiler
                        // is not installed is greyed out with the reason.
                        let mut choice = self.arch();
                        for (i, (arch, text, ok, missing)) in [
                            (
                                Arch::Universal,
                                "Apple Silicon + Intel (universal)",
                                self.arm && self.intel,
                                "A universal app needs both compilers, and this installation does not have them.",
                            ),
                            (
                                Arch::Arm64,
                                "Apple Silicon",
                                self.arm,
                                "The Apple Silicon compiler is not in this installation (an Intel Mac has only the Intel one).",
                            ),
                            (
                                Arch::Intel,
                                "Apple Intel",
                                self.intel,
                                "The Intel compiler is missing from this installation (a checkout builds it with packaging/fpc-build.sh and packaging/fpc-intel-build.sh).",
                            ),
                            (
                                Arch::Windows,
                                "Windows 64-bit (.exe)",
                                self.windows,
                                "The Windows units are missing from this installation (a checkout builds them with packaging/fpc-build.sh).",
                            ),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            ui.label(if i == 0 { "Runs on" } else { "" });
                            ui.vertical(|ui| {
                                if ui.add_enabled(ok, egui::RadioButton::new(choice == arch, text)).clicked() {
                                    choice = arch;
                                }
                                if !ok {
                                    ui.label(egui::RichText::new(missing).weak().small());
                                }
                            });
                            ui.end_row();
                        }
                        self.settings.arch = choice.name().into();
                        let windows = self.arch() == Arch::Windows;
                        // The oldest system the program runs on. A blank field is the default (macOS 14.0; for
                        // Windows the compiler's own).
                        if !windows {
                            let l = ui.label("Minimum macOS");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.min_macos)
                                    .hint_text("14.0")
                                    .desired_width(240.0),
                            )
                            .labelled_by(l.id);
                            ui.end_row();
                        } else {
                            let l = ui.label("Minimum Windows");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.min_windows)
                                    .hint_text("6.1 (Windows 7), 10.0 (Windows 10)")
                                    .desired_width(240.0),
                            )
                            .labelled_by(l.id);
                            ui.end_row();
                        }
                        if gui && !windows {
                            let l = ui.label("Bundle identifier");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.bundle_id)
                                    .hint_text("com.example.myapp")
                                    .desired_width(240.0),
                            )
                            .labelled_by(l.id);
                            ui.end_row();
                        }
                        // Version and icon: an app bundle's, or a Windows program's version information and icon.
                        if gui || windows {
                            let l = ui.label("Version");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.version)
                                    .hint_text("1.0")
                                    .desired_width(240.0),
                            )
                            .labelled_by(l.id);
                            ui.end_row();
                            let l = ui.label("Icon");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings.icon)
                                        .hint_text(".png or .icns")
                                        .desired_width(160.0),
                                )
                                .labelled_by(l.id);
                                if ui.button("Choose...").clicked()
                                    && let Some(f) = rfd::FileDialog::new()
                                        .add_filter("Icon", &["png", "icns"])
                                        .set_directory(root)
                                        .pick_file()
                                {
                                    self.settings.icon = f.to_string_lossy().into_owned();
                                }
                            });
                            ui.end_row();
                        }
                    });
                ui.add_space(6.0);
                ui.label(egui::RichText::new(self.summary()).weak());
                if let Some(e) = &self.error {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(8.0);
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
            },
        );
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(Answer::Cancel);
        }
        answer
    }
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
