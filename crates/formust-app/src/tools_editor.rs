//! The Configure Tools window (spec 39): the user's external tools, with Add, Remove, Up and Down, and the
//! fields of the selected tool. The editing rules are pure and tested here; the window only draws them.

use eframe::egui::{self, RichText};
use formust_build::tools::{self, Tool};

/// The tools being edited and which one is selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editor {
    pub tools: Vec<Tool>,
    pub selected: usize,
    /// Why the last Save was refused.
    pub error: Option<String>,
}

impl Editor {
    pub fn new(tools: Vec<Tool>) -> Self {
        Self {
            tools,
            selected: 0,
            error: None,
        }
    }

    /// Adds a tool after the selected one and selects it.
    pub fn add(&mut self) {
        let at = if self.tools.is_empty() {
            0
        } else {
            self.selected + 1
        };
        self.tools.insert(
            at,
            Tool {
                name: "New Tool".into(),
                ..Tool::default()
            },
        );
        self.selected = at;
    }

    /// Removes the selected tool; the selection stays on a neighbour.
    pub fn remove(&mut self) {
        if self.selected < self.tools.len() {
            self.tools.remove(self.selected);
            self.selected = self.selected.min(self.tools.len().saturating_sub(1));
        }
    }

    /// Moves the selected tool up (`-1`) or down (`1`) and keeps it selected; nothing happens at the ends.
    pub fn move_by(&mut self, step: isize) {
        let Some(to) = self
            .selected
            .checked_add_signed(step)
            .filter(|&t| t < self.tools.len())
        else {
            return;
        };
        if self.selected < self.tools.len() {
            self.tools.swap(self.selected, to);
            self.selected = to;
        }
    }

    /// Why the tools cannot be saved, if they cannot: every tool needs a name and a command.
    pub fn problem(&self) -> Option<String> {
        self.tools.iter().enumerate().find_map(|(i, t)| {
            if t.name.trim().is_empty() {
                Some(format!("Tool {} has no name.", i + 1))
            } else if t.command.trim().is_empty() {
                Some(format!("{} has no command.", t.name.trim()))
            } else {
                None
            }
        })
    }

    /// Saves the tools when they are valid, else keeps the problem for the window to show.
    pub fn save(&mut self) -> bool {
        if let Some(problem) = self.problem() {
            self.error = Some(problem);
            return false;
        }
        match tools::save(&self.tools) {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(format!("The tools could not be saved: {e}"));
                false
            }
        }
    }
}

/// What the window did this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Saved,
    Cancelled,
}

/// Draws the window. Save writes the tools (see [`Editor::save`]); Cancel and Esc leave them unsaved.
pub fn dialog(ctx: &egui::Context, editor: &mut Editor) -> Outcome {
    let mut outcome = Outcome::Open;
    crate::style::dialog(
        ctx,
        "configure-tools",
        Some("Configure Tools"),
        620.0,
        |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new("Tools run from the Tools menu. Macros: ")
                    .size(12.0)
                    .color(egui::Color32::GRAY),
            );
            ui.label(RichText::new(tools::MACROS).size(11.0).monospace());
            ui.horizontal(|ui| {
                if ui.button("Add").clicked() {
                    editor.add();
                }
                if ui
                    .add_enabled(!editor.tools.is_empty(), egui::Button::new("Remove"))
                    .clicked()
                {
                    editor.remove();
                }
                if ui
                    .add_enabled(editor.selected > 0, egui::Button::new("Up"))
                    .clicked()
                {
                    editor.move_by(-1);
                }
                if ui
                    .add_enabled(
                        editor.selected + 1 < editor.tools.len(),
                        egui::Button::new("Down"),
                    )
                    .clicked()
                {
                    editor.move_by(1);
                }
            });
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(170.0);
                    for (i, tool) in editor.tools.iter().enumerate() {
                        let name = if tool.name.trim().is_empty() {
                            "(no name)"
                        } else {
                            tool.name.as_str()
                        };
                        if ui.selectable_label(editor.selected == i, name).clicked() {
                            editor.selected = i;
                        }
                    }
                });
                ui.vertical(|ui| {
                    if let Some(tool) = editor.tools.get_mut(editor.selected) {
                        egui::Grid::new("tool-fields")
                            .num_columns(2)
                            .spacing([8.0, 6.0])
                            .show(ui, |ui| {
                                field(ui, "Name", &mut tool.name, 330.0);
                                field(ui, "Command", &mut tool.command, 330.0);
                                field(ui, "Arguments", &mut tool.args, 330.0);
                                field(ui, "Working folder", &mut tool.workdir, 330.0);
                            });
                        ui.label(
                            RichText::new("Empty working folder: the project folder.")
                                .size(11.0)
                                .color(egui::Color32::GRAY),
                        );
                    } else {
                        ui.label("Add a tool to start.");
                    }
                });
            });
            if let Some(error) = &editor.error {
                ui.label(RichText::new(error).color(egui::Color32::RED));
            }
            ui.horizontal(|ui| {
                if crate::style::secondary_button(ui, "Cancel").clicked() {
                    outcome = Outcome::Cancelled;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::style::primary_button(ui, "Save").clicked() && editor.save() {
                        outcome = Outcome::Saved;
                    }
                });
            });
        },
    );
    outcome
}

/// A labelled text field on the current grid row.
fn field(ui: &mut egui::Ui, label: &str, text: &mut String, width: f32) {
    let lbl = ui.label(label);
    ui.add(egui::TextEdit::singleline(text).desired_width(width))
        .labelled_by(lbl.id);
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, command: &str) -> Tool {
        Tool {
            name: name.into(),
            command: command.into(),
            ..Tool::default()
        }
    }

    #[test]
    fn add_inserts_after_the_selection_and_selects_the_new_tool() {
        let mut e = Editor::new(vec![tool("a", "x"), tool("b", "y")]);
        e.add();
        assert_eq!(e.tools.len(), 3);
        assert_eq!(e.selected, 1);
        assert_eq!(e.tools[1].name, "New Tool");
        let mut empty = Editor::new(Vec::new());
        empty.add();
        assert_eq!((empty.tools.len(), empty.selected), (1, 0));
    }

    #[test]
    fn remove_keeps_a_selection_on_a_neighbour() {
        let mut e = Editor::new(vec![tool("a", "x"), tool("b", "y")]);
        e.selected = 1;
        e.remove();
        assert_eq!(e.tools, [tool("a", "x")]);
        assert_eq!(e.selected, 0);
        e.remove();
        assert!(e.tools.is_empty());
        e.remove();
        assert!(e.tools.is_empty(), "nothing selected: nothing to remove");
    }

    #[test]
    fn move_swaps_and_follows_the_selection_and_stops_at_the_ends() {
        let mut e = Editor::new(vec![tool("a", "x"), tool("b", "y"), tool("c", "z")]);
        e.move_by(1);
        assert_eq!(e.tools[1].name, "a");
        assert_eq!(e.selected, 1);
        e.move_by(-1);
        assert_eq!(e.tools[0].name, "a");
        assert_eq!(e.selected, 0);
        e.move_by(-1);
        assert_eq!(e.selected, 0, "already first");
        e.selected = 2;
        e.move_by(1);
        assert_eq!(e.selected, 2, "already last");
    }

    #[test]
    fn a_tool_needs_a_name_and_a_command_before_it_is_saved() {
        let mut e = Editor::new(vec![tool("", "")]);
        assert_eq!(e.problem().as_deref(), Some("Tool 1 has no name."));
        e.tools[0].name = "Ls".into();
        assert_eq!(e.problem().as_deref(), Some("Ls has no command."));
        e.tools[0].command = "  ".into();
        assert!(e.problem().is_some(), "a blank command counts as none");
        assert!(!e.save(), "an invalid list is not written");
        assert!(e.error.is_some());
    }
}
