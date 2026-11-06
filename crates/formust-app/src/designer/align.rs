//! Align and distribute for a selection. egui places widgets by layout, not by coordinates, so each
//! action writes something egui has: a size property, an anchor wrapper, `ui.add_space(..)`, the
//! parent's item spacing, or a new row or column. [`AlignAction::writes`] says exactly what, and the
//! UI shows it as the action's hover text. Each action is one undo step.

use eframe::egui::Rect;
use formust_model::{
    Anchor, ContainerId, Form, NodeId, NodeKind, PropValue, SizeRequest, container_def,
    resize_axes, vec2_text,
};

use super::Designer;

/// An align or distribute action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignAction {
    SameWidth,
    SameHeight,
    Left,
    Right,
    Center,
    Top,
    Bottom,
    Distribute,
    ParentSpacing,
    MakeColumn,
    MakeRow,
}

pub const ALL_ALIGN: &[AlignAction] = &[
    AlignAction::SameWidth,
    AlignAction::SameHeight,
    AlignAction::Left,
    AlignAction::Right,
    AlignAction::Center,
    AlignAction::Top,
    AlignAction::Bottom,
    AlignAction::Distribute,
    AlignAction::ParentSpacing,
    AlignAction::MakeColumn,
    AlignAction::MakeRow,
];

impl AlignAction {
    pub fn label(self) -> &'static str {
        match self {
            AlignAction::SameWidth => "Same width",
            AlignAction::SameHeight => "Same height",
            AlignAction::Left => "Align left",
            AlignAction::Right => "Align right",
            AlignAction::Center => "Align centre",
            AlignAction::Top => "Align top",
            AlignAction::Bottom => "Align bottom",
            AlignAction::Distribute => "Distribute evenly",
            AlignAction::ParentSpacing => "Spacing of the parent",
            AlignAction::MakeColumn => "Make a column",
            AlignAction::MakeRow => "Make a row",
        }
    }

    /// The anchor an align action sets.
    pub fn anchor(self) -> Option<Anchor> {
        match self {
            AlignAction::Left => Some(Anchor::Left),
            AlignAction::Right => Some(Anchor::Right),
            AlignAction::Center => Some(Anchor::Center),
            AlignAction::Top => Some(Anchor::Top),
            AlignAction::Bottom => Some(Anchor::Bottom),
            AlignAction::SameWidth
            | AlignAction::SameHeight
            | AlignAction::Distribute
            | AlignAction::ParentSpacing
            | AlignAction::MakeColumn
            | AlignAction::MakeRow => None,
        }
    }

    /// Exactly what the action writes into the code, with the form's egui path.
    pub fn writes(self, egui_path: &str) -> String {
        if let Some(a) = self.anchor() {
            return format!(
                "Wraps each selected node in ui.with_layout({}, |ui| {{ .. }}) (its anchor {}); a node that already has an anchor gets its layout rewritten. Nodes that can't be wrapped are skipped.",
                a.layout_text(egui_path),
                a.label().to_lowercase()
            );
        }
        match self {
            AlignAction::SameWidth => "Sets each node's own width property to the widest selected node (or the value typed): .desired_width(w) on a text edit or progress bar, .width(w) on a combo box, .min_size(egui::vec2(w, h)) on a button or drag value, ui.add_sized([w, h], ..) on a label, checkbox, slider and the like, .max_width(w) on a scroll area, ui.set_min_width(w) in a layout container. Nodes without one (headings, toggles, ..) are skipped.".to_string(),
            AlignAction::SameHeight => "Sets each node's own height property to the tallest selected node (or the value typed): .desired_height(h) on a progress bar, .desired_rows(n) on a multiline text edit, .min_size(egui::vec2(w, h)) on a button or drag value, ui.add_sized([w, h], ..) on a label, checkbox, slider and the like, .max_height(h) on a scroll area, ui.set_min_height(h) in a layout container. Nodes without one are skipped.".to_string(),
            AlignAction::Distribute => "Puts ui.add_space(n) after each selected node but the last, so every gap between them equals the largest gap now. An ui.add_space already there is updated, and one that comes to 0 is removed. The nodes must be neighbours in one container (spaces between them are fine), three or more.".to_string(),
            AlignAction::ParentSpacing => format!(
                "Writes ui.spacing_mut().item_spacing = {}; at the start of the selection's container: every gap in that container changes. Not for the form itself (it has no body statement of its own).",
                vec2_text(egui_path, 8.0, 8.0).replace("8.0", "n")
            ),
            AlignAction::MakeColumn => "Wraps the selection in ui.vertical(|ui| { .. }) at the first node's place; nodes elsewhere are moved there first, in tree order.".to_string(),
            AlignAction::MakeRow => "Wraps the selection in ui.horizontal(|ui| { .. }) at the first node's place; nodes elsewhere are moved there first, in tree order.".to_string(),
            AlignAction::Left
            | AlignAction::Right
            | AlignAction::Center
            | AlignAction::Top
            | AlignAction::Bottom => unreachable!("anchors are handled above"),
        }
    }
}

impl Designer {
    /// Runs an align action on the selection. Says in the status what it did and what it skipped.
    pub fn align(&mut self, a: AlignAction) -> bool {
        let hosts = self.selection_hosts();
        let Some(form) = self.current().map(|f| f.form().clone()) else {
            return false;
        };
        if hosts.is_empty() || hosts.contains(&form.root.id) {
            self.status = Some("Select the nodes to align first".to_string());
            return false;
        }
        let nodes: Vec<NodeId> = hosts.iter().map(|h| form.anchor_child(*h)).collect();
        let (keep, multi, range) = (self.selected, self.multi.clone(), self.range_to);
        let done = match a {
            AlignAction::SameWidth | AlignAction::SameHeight => {
                self.same_size(&form, &nodes, a == AlignAction::SameWidth)
            }
            AlignAction::Distribute => self.distribute(&form, &hosts),
            AlignAction::ParentSpacing => self.parent_spacing(&form, &hosts),
            AlignAction::MakeColumn => return self.wrap_selection(ContainerId::Vertical),
            AlignAction::MakeRow => return self.wrap_selection(ContainerId::Horizontal),
            AlignAction::Left
            | AlignAction::Right
            | AlignAction::Center
            | AlignAction::Top
            | AlignAction::Bottom => {
                let anchor = a.anchor();
                let mut skipped = 0;
                let done = self.try_edit(None, |f| {
                    for id in &nodes {
                        if f.anchor_of(*id).is_some() || f.can_wrap(*id) {
                            f.set_anchor(*id, anchor)?;
                        } else {
                            skipped += 1;
                        }
                    }
                    Ok(())
                });
                if done {
                    self.status = Some(skip_note(a, nodes.len() - skipped, skipped));
                }
                done
            }
        };
        // The nodes keep their ids; anchor wrappers are shown as their node.
        self.selected = keep;
        self.multi = multi;
        self.range_to = range;
        done
    }

    fn same_size(&mut self, form: &Form, nodes: &[NodeId], width: bool) -> bool {
        let measured: Vec<(NodeId, Rect)> = nodes
            .iter()
            .filter_map(|n| self.rects.get(n).map(|r| (*n, *r)))
            .collect();
        if measured.is_empty() {
            self.status = Some("The selection isn't on the canvas".to_string());
            return false;
        }
        let along = |r: &Rect| if width { r.width() } else { r.height() };
        let target = self
            .size_value
            .unwrap_or_else(|| measured.iter().map(|(_, r)| along(r)).fold(0.0, f32::max))
            .round();
        let mut skipped = 0;
        let action = if width {
            AlignAction::SameWidth
        } else {
            AlignAction::SameHeight
        };
        let done = self.try_edit(None, |f| {
            for (id, r) in &measured {
                let Some(node) = f.find(*id) else { continue };
                let (w, h) = resize_axes(node);
                if !(if width { w } else { h }) {
                    skipped += 1;
                    continue;
                }
                // A multiline text edit's height is a row count.
                let rows = match &node.kind {
                    NodeKind::Widget {
                        def: formust_model::WidgetId::TextEditMultiline,
                        props,
                        ..
                    } => {
                        let now = match props.get("desired_rows") {
                            Some(PropValue::Int(n)) => *n,
                            _ => 4,
                        };
                        let row = (r.height() / now.max(1) as f32).max(1.0);
                        Some(((target / row).round() as i64).max(1))
                    }
                    _ => None,
                };
                let (rw, rh) = (r.width().round(), r.height().round());
                f.resize(
                    *id,
                    SizeRequest {
                        width: f64::from(if width { target } else { rw }),
                        height: f64::from(if width { rh } else { target }),
                        rows,
                        change_width: width,
                        change_height: !width,
                    },
                )?;
            }
            Ok(())
        });
        let _ = form;
        if done {
            let set = measured.len() - skipped;
            self.status = Some(format!(
                "{}: {set} set to {target:.0} pt{}",
                action.label(),
                if skipped > 0 {
                    format!(", {skipped} skipped (no size property)")
                } else {
                    String::new()
                }
            ));
        }
        done
    }

    fn distribute(&mut self, form: &Form, hosts: &[NodeId]) -> bool {
        let refuse = |d: &mut Designer, why: &str| {
            d.status = Some(format!("Distribute evenly: {why}"));
            false
        };
        if hosts.len() < 3 {
            return refuse(self, "select three or more nodes");
        }
        let Some((parent, _)) = form.parent_of(hosts[0]) else {
            return refuse(self, "the form itself can't move");
        };
        let mut placed: Vec<(usize, NodeId)> = Vec::new();
        for h in hosts {
            match form.parent_of(*h) {
                Some((p, i)) if p == parent => placed.push((i, *h)),
                _ => return refuse(self, "the nodes must be in one container"),
            }
        }
        placed.sort();
        let def = match form.find(parent).map(|n| &n.kind) {
            Some(NodeKind::Container { def, .. }) => *def,
            _ => return refuse(self, "no container"),
        };
        if matches!(
            def,
            ContainerId::Grid | ContainerId::Columns | ContainerId::Sides
        ) {
            return refuse(self, "a grid places its cells itself");
        }
        let across = super::edit::runs_across(def);
        let kids = form
            .find(parent)
            .map(|n| n.children().to_vec())
            .unwrap_or_default();
        let mut pairs: Vec<(NodeId, f32)> = Vec::new();
        for w in placed.windows(2) {
            let ((i, a), (j, b)) = (w[0], w[1]);
            let between = &kids[i + 1..j];
            let only_space = between.len() <= 1
                && between.iter().all(|n| {
                    matches!(
                        n.kind,
                        NodeKind::Widget {
                            def: formust_model::WidgetId::AddSpace,
                            ..
                        }
                    )
                });
            if !only_space {
                return refuse(
                    self,
                    "the nodes must be neighbours (spaces between are fine)",
                );
            }
            let (Some(ra), Some(rb)) = (self.host_rect(form, a), self.host_rect(form, b)) else {
                return refuse(self, "a node isn't on the canvas");
            };
            let gap = if across {
                rb.left() - ra.right()
            } else {
                rb.top() - ra.bottom()
            };
            pairs.push((a, gap));
        }
        let target = pairs.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        let done = self.try_edit(None, |f| {
            for (a, gap) in &pairs {
                let now = f.space_after(*a).unwrap_or(0.0);
                f.set_space_after(*a, now + f64::from(target - gap))?;
            }
            Ok(())
        });
        if done {
            self.status = Some(format!(
                "Distribute evenly: every gap is {target:.0} pt (ui.add_space)"
            ));
        }
        done
    }

    fn parent_spacing(&mut self, form: &Form, hosts: &[NodeId]) -> bool {
        let parents: Vec<NodeId> = hosts
            .iter()
            .filter_map(|h| form.parent_of(*h).map(|p| p.0))
            .collect();
        let Some(&parent) = parents.first() else {
            return false;
        };
        let has = match form.find(parent).map(|n| &n.kind) {
            Some(NodeKind::Container { def, .. }) => {
                container_def(*def).prop("item_spacing").is_some()
            }
            _ => false,
        };
        if parents.iter().any(|p| *p != parent) || !has {
            self.status = Some(
                "Spacing of the parent: the nodes must share a container that has item spacing (not the form itself)".to_string(),
            );
            return false;
        }
        let v = f64::from(self.spacing());
        let text = vec2_text(&form.egui_path, v, v);
        let done = self.try_edit(None, |f| {
            f.set_prop(parent, "item_spacing", PropValue::Expr(text))
        });
        if done {
            self.status = Some(format!(
                "Spacing of the parent: {v:.0} pt between all its items"
            ));
        }
        done
    }

    /// The rect of a host as drawn: its own, or its anchored node's.
    fn host_rect(&self, form: &Form, host: NodeId) -> Option<Rect> {
        self.rects
            .get(&host)
            .or_else(|| self.rects.get(&form.anchor_child(host)))
            .copied()
    }

    /// Can the action run on the selection at all (for greying it out)?
    pub fn can_align(&self, a: AlignAction) -> bool {
        let hosts = self.selection_hosts();
        let Some(form) = self.current().map(|f| f.form()) else {
            return false;
        };
        if hosts.is_empty() || hosts.contains(&form.root.id) {
            return false;
        }
        match a {
            AlignAction::Distribute => hosts.len() >= 3,
            AlignAction::MakeColumn | AlignAction::MakeRow => self.can_wrap_selection(),
            AlignAction::ParentSpacing => {
                let p = form.parent_of(hosts[0]).map(|p| p.0);
                p.is_some_and(|p| {
                    hosts.iter().all(|h| form.parent_of(*h).map(|x| x.0) == Some(p))
                        && matches!(form.find(p).map(|n| &n.kind), Some(NodeKind::Container { def, .. }) if container_def(*def).prop("item_spacing").is_some())
                })
            }
            _ => true,
        }
    }
}

fn skip_note(a: AlignAction, set: usize, skipped: usize) -> String {
    if skipped == 0 {
        format!("{}: {set} anchored", a.label())
    } else {
        format!(
            "{}: {set} anchored, {skipped} skipped (can't be wrapped here)",
            a.label()
        )
    }
}
