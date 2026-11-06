//! The recognised-node measurement of `docs/specs.md`, Risk 2: how much of real egui code becomes
//! Widget and Container nodes rather than locked Custom blocks.

use std::collections::BTreeMap;

use formust_model::{ALL_CONTAINERS, ALL_WIDGETS, Node, NodeKind};

use crate::forms;
use crate::import::{ImportedForm, SourceFile};

/// Counts of statements. Byte counts cover a node's own text: all of a widget or custom block, and
/// for a container only its head and tail, because its children are counted themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub forms: usize,
    pub widgets: usize,
    pub containers: usize,
    pub customs: usize,
    /// Control-flow containers (`ImportOptions::extended`). Their heads are locked code, so they
    /// count against the ratio, like custom blocks.
    pub flows: usize,
    pub flow_bytes: usize,
    pub widget_bytes: usize,
    pub container_bytes: usize,
    pub custom_bytes: usize,
    /// Palette calls inside custom text: widgets the designer can't reach.
    pub hidden_calls: usize,
    pub reasons: BTreeMap<String, ReasonTally>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReasonTally {
    pub count: usize,
    pub bytes: usize,
    pub hidden_calls: usize,
}

impl Tally {
    pub fn add_file(&mut self, file: &SourceFile) {
        for form in &file.forms {
            self.add_form(form);
        }
    }

    pub fn add_form(&mut self, imported: &ImportedForm) {
        self.forms += 1;
        for child in imported.form.root.children() {
            self.add_node(child, imported);
        }
    }

    fn add_node(&mut self, node: &Node, imported: &ImportedForm) {
        match &node.kind {
            NodeKind::Widget { .. } => {
                self.widgets += 1;
                self.widget_bytes += node.src.as_deref().map_or(0, str::len);
            }
            NodeKind::Container {
                def,
                children,
                shell,
                ..
            } => {
                let own = shell
                    .as_ref()
                    .map_or(0, |s| s.head.as_deref().map_or(0, str::len) + s.tail.len());
                if def.is_flow() {
                    self.flows += 1;
                    self.flow_bytes += own;
                } else {
                    self.containers += 1;
                    self.container_bytes += own;
                }
                for child in children {
                    self.add_node(child, imported);
                }
            }
            NodeKind::Custom { text } => {
                self.customs += 1;
                self.custom_bytes += text.len();
                let hidden = hidden_calls(text);
                self.hidden_calls += hidden;
                let reason = imported
                    .custom
                    .iter()
                    .find(|c| c.node == node.id)
                    .map_or("unknown", |c| c.reason.as_str());
                let entry = self.reasons.entry(reason.to_string()).or_default();
                entry.count += 1;
                entry.bytes += text.len();
                entry.hidden_calls += hidden;
            }
        }
    }

    pub fn add(&mut self, other: &Tally) {
        self.forms += other.forms;
        self.widgets += other.widgets;
        self.containers += other.containers;
        self.customs += other.customs;
        self.flows += other.flows;
        self.flow_bytes += other.flow_bytes;
        self.widget_bytes += other.widget_bytes;
        self.container_bytes += other.container_bytes;
        self.custom_bytes += other.custom_bytes;
        self.hidden_calls += other.hidden_calls;
        for (reason, tally) in &other.reasons {
            let entry = self.reasons.entry(reason.clone()).or_default();
            entry.count += tally.count;
            entry.bytes += tally.bytes;
            entry.hidden_calls += tally.hidden_calls;
        }
    }

    pub fn nodes(&self) -> usize {
        self.widgets + self.containers + self.customs + self.flows
    }

    /// Recognised statements as a share of all statements, 0 to 1.
    pub fn count_ratio(&self) -> f64 {
        ratio(self.widgets + self.containers, self.nodes())
    }

    /// Recognised bytes as a share of all statement bytes, 0 to 1. Custom blocks are large, so this
    /// is the less flattering of the two and the fairer one.
    pub fn byte_ratio(&self) -> f64 {
        let known = self.widget_bytes + self.container_bytes;
        ratio(known, known + self.custom_bytes + self.flow_bytes)
    }
}

impl Tally {
    /// Palette calls that are nodes, as a share of all palette calls including those hidden in
    /// custom blocks. This leaves logic out, so it says how much of the *UI* the designer can reach.
    pub fn ui_call_ratio(&self) -> f64 {
        let known = self.widgets + self.containers;
        ratio(known, known + self.hidden_calls)
    }
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// How many `.method(` calls in this text are palette methods, a rough count of the widgets and
/// containers inside a custom block. It may count a word in a string or comment.
pub fn hidden_calls(text: &str) -> usize {
    let mut names: Vec<&str> = Vec::new();
    for &w in ALL_WIDGETS {
        names.extend(forms::widget_forms(w).shorthand.iter().map(|(m, _)| *m));
    }
    for &c in ALL_CONTAINERS {
        names.extend(forms::container_forms(c).shorthand.map(|(m, _)| m));
    }
    names.extend(["add", "add_sized", "add_enabled", "add_visible"]);
    names
        .iter()
        .map(|name| text.matches(&format!(".{name}(")).count())
        .sum()
}
