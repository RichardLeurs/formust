//! Copy and paste of nodes as Rust text. Copying writes each node with [`crate::emit::emit_node`];
//! pasting puts the text in a throwaway form and reads it with the one importer, so a paste from a
//! code editor works too, and anything the importer doesn't know arrives as locked custom code.

use formust_model::{Form, Node, NodeKind};

use crate::emit::emit_node;
use crate::import::{ImportError, import};

const OWNER: &str = "FormustClipboard";

/// The text of these nodes for the clipboard: one statement per line, written for a `Ui` named `ui`
/// so it pastes anywhere.
pub fn copy_text(form: &Form, nodes: &[&Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        if !out.is_empty() {
            out.push('\n');
        }
        // Comments above a statement belong to it; the indentation does not.
        for line in node
            .leading
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(emit_node(form, node, "ui").trim_end());
    }
    out
}

/// Statements from the clipboard as nodes (ids from 1, not yet in any form; use
/// [`Form::adopt`]). An error when the text isn't a list of statements.
pub fn parse_statements(text: &str) -> Result<Vec<Node>, ImportError> {
    let head = format!("impl {OWNER} {{\n    fn ui(&mut self, ui: &mut egui::Ui) {{\n");
    let wrapped = format!("{head}{text}\n    }}\n}}\n");
    let file = import(&wrapped)?;
    let not_statements = || ImportError {
        message: "the clipboard holds no Rust statements for a form".to_string(),
        line: 0,
        column: 0,
    };
    // The body must be the whole text: a `}` in it could otherwise close the function early.
    let [form] = file.forms.as_slice() else {
        return Err(not_statements());
    };
    let body_start = head.len() - "{\n".len();
    if form.form.owner != OWNER
        || form.body.start != body_start
        || form.body.end != wrapped.len() - "\n}\n".len()
    {
        return Err(not_statements());
    }
    let NodeKind::Container { children, .. } = &form.form.root.kind else {
        return Err(not_statements());
    };
    if children.is_empty() {
        return Err(not_statements());
    }
    Ok(children.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use formust_model::{ContainerId, WidgetId};

    fn sample() -> Form {
        let src = "impl X {\n    fn ui(&mut self, ui: &mut egui::Ui) {\n        // the name\n        ui.label(\"Name\"); // trailing\n        ui.horizontal(|inner| {\n            inner.button(\"Go\");\n        });\n        if self.busy { ui.spinner(); }\n    }\n}\n";
        import(src).unwrap().forms.remove(0).form
    }

    #[test]
    fn copied_nodes_paste_back_as_the_same_nodes() {
        let form = sample();
        let nodes: Vec<&Node> = form.root.children().iter().collect();
        let text = copy_text(&form, &nodes);
        assert_eq!(
            text,
            "// the name\nui.label(\"Name\"); // trailing\nui.horizontal(|inner| {\n            inner.button(\"Go\");\n        });\nif self.busy { ui.spinner(); }"
        );
        let back = parse_statements(&text).unwrap();
        assert_eq!(back.len(), 3);
        assert!(matches!(
            back[0].kind,
            NodeKind::Widget {
                def: WidgetId::Label,
                ..
            }
        ));
        assert!(back[0].leading.contains("// the name"));
        assert!(matches!(
            back[1].kind,
            NodeKind::Container {
                def: ContainerId::Horizontal,
                ..
            }
        ));
        assert!(matches!(back[2].kind, NodeKind::Custom { .. }));
    }

    /// A node inside a closure with another `Ui` name is written for `ui`, so it pastes anywhere.
    #[test]
    fn a_nested_node_is_copied_for_a_ui_named_ui() {
        let form = sample();
        let button = &form.root.children()[1].children()[0];
        let text = copy_text(&form, &[button]);
        assert_eq!(text, "ui.button(\"Go\");");
        assert!(matches!(
            parse_statements(&text).unwrap()[0].kind,
            NodeKind::Widget {
                def: WidgetId::Button,
                ..
            }
        ));
    }

    #[test]
    fn text_that_is_no_statement_list_is_refused() {
        for bad in [
            "",
            "   ",
            "ui.label(",
            "} fn evil() {",
            "fn a() {}\n}\nfn b() {",
        ] {
            assert!(parse_statements(bad).is_err(), "{bad:?}");
        }
        assert!(
            parse_statements("let x = 1;").is_ok(),
            "custom code is fine"
        );
    }
}
