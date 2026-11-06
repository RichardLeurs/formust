//! What the designer's editing needs besides drawing: the undo history, where a drop lands, where a
//! click on the palette adds, and how a new palette item starts. Pure functions over the form and
//! the node rects of the last kept preview pass, unit-tested.
//!
//! Drop rules (docs/specs.md, "Drop and move"): the insertion index comes from the midpoints of the
//! children that have a rect, shown as an indicator line. Within [`EDGE`] points of a container's
//! edge the drop targets that container's parent instead, so a widget can be put beside a container
//! as well as into it. An empty container is a target too: its dashed placeholder is its rect.

use std::collections::HashMap;

use eframe::egui::{Pos2, Rect, pos2};
use formust_model::{
    ComboItem, ContainerId, DefaultValue, Form, Node, NodeId, NodeKind, PropValue, WidgetId,
    container_def, widget_def,
};

/// How many steps undo keeps (Formust's limit).
pub const UNDO_LIMIT: usize = 200;
/// Within this distance of a container's edge, a drop goes to its parent.
pub const EDGE: f32 = 6.0;

/// Undo and redo as whole-file snapshots: every form of the file, so node ids (and with them the
/// selection) survive. Changes with the same `key` in a row (typing in one field) are one step.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Vec<Form>>,
    redo: Vec<Vec<Form>>,
    last_key: Option<String>,
}

impl History {
    /// Records the state before a change. `snapshot` is only called when a step is pushed: a change
    /// with the same `key` as the previous one joins it. Returns whether a step was pushed.
    pub fn checkpoint(&mut self, key: Option<&str>, snapshot: impl FnOnce() -> Vec<Form>) -> bool {
        if key.is_some() && key == self.last_key.as_deref() {
            return false;
        }
        self.undo.push(snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last_key = key.map(str::to_string);
        true
    }

    /// Ends a run of joined changes, so the next change is a step of its own.
    pub fn break_run(&mut self) {
        self.last_key = None;
    }

    /// The state to go back to, given the current one (which redo then restores).
    pub fn undo(&mut self, current: Vec<Form>) -> Option<Vec<Form>> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        self.last_key = None;
        Some(previous)
    }

    pub fn redo(&mut self, current: Vec<Form>) -> Option<Vec<Form>> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        self.last_key = None;
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    #[cfg(test)]
    pub fn steps(&self) -> usize {
        self.undo.len()
    }
}

/// A palette entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Widget(WidgetId),
    Container(ContainerId),
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Widget(w) => widget_def(w).label,
            Item::Container(c) => container_def(c).label,
        }
    }
}

/// The palette: every first-palette widget, then the containers it offers (not the form itself).
pub fn palette() -> Vec<Item> {
    let mut items: Vec<Item> = formust_model::ALL_WIDGETS
        .iter()
        .map(|&w| Item::Widget(w))
        .collect();
    items.extend(
        formust_model::ALL_CONTAINERS
            .iter()
            .filter(|&&c| c != ContainerId::Root && container_def(c).in_palette)
            .map(|&c| Item::Container(c)),
    );
    items
}

/// What is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    New(Item),
    Move(NodeId),
}

/// Where a drop lands: under `parent` at `index` (an index among its current children), and the
/// indicator line to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drop {
    pub parent: NodeId,
    pub index: usize,
    pub line: [Pos2; 2],
}

/// Do a container's children run across (left to right, wrapping into rows) or down?
fn runs_across(def: ContainerId) -> bool {
    match def {
        ContainerId::Horizontal
        | ContainerId::HorizontalWrapped
        | ContainerId::HorizontalCentered
        | ContainerId::HorizontalTop
        | ContainerId::Grid => true,
        ContainerId::Root
        | ContainerId::Vertical
        | ContainerId::VerticalCentered
        | ContainerId::VerticalCenteredJustified
        | ContainerId::CenteredAndJustified
        | ContainerId::ScrollArea
        | ContainerId::CollapsingHeader
        | ContainerId::Group
        | ContainerId::Indent
        | ContainerId::PanelTop
        | ContainerId::PanelBottom
        | ContainerId::PanelLeft
        | ContainerId::PanelRight
        | ContainerId::CentralPanel
        | ContainerId::Scope
        | ContainerId::WithLayout
        | ContainerId::MenuButton
        | ContainerId::AddEnabledUi
        | ContainerId::Frame
        | ContainerId::Window
        | ContainerId::DialogCall
        | ContainerId::Flow
        | ContainerId::FlowGroup => false,
    }
}

fn container_id(form: &Form, id: NodeId) -> Option<ContainerId> {
    match form.find(id).map(|n| &n.kind) {
        Some(NodeKind::Container { def, .. }) => Some(*def),
        _ => None,
    }
}

/// Is `id` the moving node or inside it?
fn in_moving(form: &Form, moving: Option<NodeId>, id: NodeId) -> bool {
    moving
        .and_then(|m| form.find(m))
        .is_some_and(|m| m.walk_find(id))
}

trait WalkFind {
    fn walk_find(&self, id: NodeId) -> bool;
}

impl WalkFind for Node {
    fn walk_find(&self, id: NodeId) -> bool {
        self.id == id || self.children().iter().any(|c| c.walk_find(id))
    }
}

/// The drop under `pos` on the canvas, for a new item (`moving` is `None`) or a moving node.
pub fn canvas_drop(
    form: &Form,
    rects: &HashMap<NodeId, Rect>,
    pos: Pos2,
    moving: Option<NodeId>,
) -> Option<Drop> {
    // The deepest container under the pointer that can take children, outside the moving subtree.
    fn walk(
        form: &Form,
        n: &Node,
        depth: u32,
        rects: &HashMap<NodeId, Rect>,
        pos: Pos2,
        moving: Option<NodeId>,
        best: &mut Option<(u32, NodeId)>,
    ) {
        if moving == Some(n.id) {
            return;
        }
        if form.accepts_children(n.id)
            && rects.get(&n.id).is_some_and(|r| r.contains(pos))
            && best.is_none_or(|(d, _)| depth >= d)
        {
            *best = Some((depth, n.id));
        }
        for c in n.children() {
            walk(form, c, depth + 1, rects, pos, moving, best);
        }
    }
    let mut best = None;
    walk(form, &form.root, 0, rects, pos, moving, &mut best);
    let (_, target) = best?;
    let drop = canvas_drop_at(form, rects, pos, moving, target);
    drop.filter(|d| fits(form, d.parent, moving))
}

/// Can the moving node (or a new palette item, never a `Window`) go under `parent`? A dialog's root
/// has a `&Context` and takes only its `Window`s and dialog helper calls.
fn fits(form: &Form, parent: NodeId, moving: Option<NodeId>) -> bool {
    match moving.and_then(|m| form.find(m)) {
        Some(node) => form.fits(parent, node),
        None => form.takes_widgets(parent),
    }
}

fn canvas_drop_at(
    form: &Form,
    rects: &HashMap<NodeId, Rect>,
    pos: Pos2,
    moving: Option<NodeId>,
    target: NodeId,
) -> Option<Drop> {
    let rect = rects[&target];
    if target != form.root.id
        && !rect.shrink(EDGE).contains(pos)
        && let Some((parent, at)) = form.parent_of(target)
        && form.accepts_children(parent)
        && fits(form, parent, moving)
        && !in_moving(form, moving, parent)
    {
        // Near the edge: beside the container, in its parent.
        let across = container_id(form, parent).is_some_and(runs_across);
        let after = if across {
            pos.x > rect.center().x
        } else {
            pos.y > rect.center().y
        };
        let line = if across {
            let x = if after {
                rect.right() + 1.0
            } else {
                rect.left() - 1.0
            };
            [pos2(x, rect.top()), pos2(x, rect.bottom())]
        } else {
            let y = if after {
                rect.bottom() + 1.0
            } else {
                rect.top() - 1.0
            };
            [pos2(rect.left(), y), pos2(rect.right(), y)]
        };
        return Some(Drop {
            parent,
            index: at + usize::from(after),
            line,
        });
    }
    Some(index_in(form, rects, target, pos))
}

/// Where among `parent`'s children `pos` falls: before the first child whose midpoint it is before.
/// Children with no rect (a space, an end of row, something scrolled away) are passed over.
pub fn index_in(form: &Form, rects: &HashMap<NodeId, Rect>, parent: NodeId, pos: Pos2) -> Drop {
    let node = form.find(parent).expect("a container of the form");
    let area = rects
        .get(&parent)
        .copied()
        .unwrap_or(Rect::from_center_size(pos, eframe::egui::vec2(40.0, 20.0)));
    let across = container_id(form, parent).is_some_and(runs_across);
    let children = node.children();
    let visible: Vec<(usize, Rect)> = children
        .iter()
        .enumerate()
        .filter_map(|(i, c)| rects.get(&c.id).map(|r| (i, *r)))
        .collect();
    let before = |r: &Rect| {
        if across {
            pos.y < r.top() || (pos.y <= r.bottom() && pos.x < r.center().x)
        } else {
            pos.y < r.center().y
        }
    };
    let found = visible.iter().position(|(_, r)| before(r));
    let index = found.map_or(children.len(), |k| visible[k].0);
    let line = match (found, visible.last()) {
        (_, None) => {
            // Empty (or nothing visible): a line through the middle of the container.
            let inner = area.shrink(2.0);
            if across {
                [
                    pos2(inner.left() + 2.0, inner.top()),
                    pos2(inner.left() + 2.0, inner.bottom()),
                ]
            } else {
                [
                    pos2(inner.left(), inner.center().y),
                    pos2(inner.right(), inner.center().y),
                ]
            }
        }
        (Some(k), _) => {
            let r = visible[k].1;
            if across {
                [
                    pos2(r.left() - 2.0, r.top()),
                    pos2(r.left() - 2.0, r.bottom()),
                ]
            } else {
                let y = match k.checked_sub(1).map(|p| visible[p].1) {
                    Some(prev) => (prev.bottom() + r.top()) / 2.0,
                    None => r.top() - 2.0,
                };
                [pos2(area.left(), y), pos2(area.right(), y)]
            }
        }
        (None, Some((_, last))) => {
            if across {
                [
                    pos2(last.right() + 2.0, last.top()),
                    pos2(last.right() + 2.0, last.bottom()),
                ]
            } else {
                [
                    pos2(area.left(), last.bottom() + 2.0),
                    pos2(area.right(), last.bottom() + 2.0),
                ]
            }
        }
    };
    Drop {
        parent,
        index,
        line,
    }
}

/// One row of the outline as drawn: its node, its rect and its depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub id: NodeId,
    pub rect: Rect,
    pub depth: usize,
}

/// Indent of a row's text in the outline, per level.
pub const ROW_INDENT: f32 = 14.0;

/// The drop under `pos` in the outline: on the upper or lower part of a row, before or after its
/// node; on the middle of a container's row, into it (at the end).
pub fn outline_drop(form: &Form, rows: &[Row], pos: Pos2, moving: Option<NodeId>) -> Option<Drop> {
    let row = rows
        .iter()
        .find(|r| r.rect.y_range().contains(pos.y))
        .or_else(|| {
            rows.last().filter(|last| {
                pos.y > last.rect.bottom()
                    && pos.x >= last.rect.left()
                    && pos.x <= last.rect.right()
            })
        })?;
    let line_at = |y: f32, depth: usize| {
        [
            pos2(row.rect.left() + 4.0 + depth as f32 * ROW_INDENT, y),
            pos2(row.rect.right(), y),
        ]
    };
    let into = |id: NodeId, depth: usize| {
        let n = form.find(id)?.children().len();
        Some(Drop {
            parent: id,
            index: n,
            line: line_at(row.rect.bottom(), depth + 1),
        })
    };
    let result = if row.id == form.root.id || pos.y > row.rect.bottom() {
        into(form.root.id, 0)
    } else {
        let frac = (pos.y - row.rect.top()) / row.rect.height().max(1.0);
        let container = form.accepts_children(row.id);
        let parent = form
            .parent_of(row.id)
            .filter(|_| !form.is_locked_branch(row.id));
        match (container, parent) {
            (true, _) if (0.25..=0.75).contains(&frac) => into(row.id, row.depth),
            (true, None) => into(row.id, row.depth),
            (_, Some((parent, at))) if frac < 0.5 => Some(Drop {
                parent,
                index: at,
                line: line_at(row.rect.top(), row.depth),
            }),
            (_, Some((parent, at))) => Some(Drop {
                parent,
                index: at + 1,
                line: line_at(row.rect.bottom(), row.depth),
            }),
            (false, None) => None,
        }
    }?;
    (!in_moving(form, moving, result.parent) && fits(form, result.parent, moving)).then_some(result)
}

/// Where a click on the palette adds: into the selected container (at its end), after the selected
/// widget, or at the end of the form.
pub fn click_target(form: &Form, selected: Option<NodeId>) -> (NodeId, usize) {
    // A dialog's root has a `&Context`: new widgets go into its Window.
    let root = form.widget_root();
    let end_of = |id: NodeId| (id, form.find(id).map_or(0, |n| n.children().len()));
    match selected.filter(|s| form.find(*s).is_some()) {
        Some(id) if form.accepts_children(id) && form.takes_widgets(id) => end_of(id),
        Some(id) => match form.parent_of(id) {
            Some((parent, at)) if form.accepts_children(parent) && form.takes_widgets(parent) => {
                (parent, at + 1)
            }
            _ => end_of(root),
        },
        None => end_of(root),
    }
}

/// The index `Form::move_node` wants (a position after the move) for a drop at `index` among the
/// current children; `None` when the drop would leave the node where it is.
pub fn move_index(form: &Form, id: NodeId, parent: NodeId, index: usize) -> Option<usize> {
    let (old_parent, old) = form.parent_of(id)?;
    if old_parent != parent {
        return Some(index);
    }
    if index == old || index == old + 1 {
        return None;
    }
    Some(if index > old { index - 1 } else { index })
}

/// The field a new stateful widget binds to, from the registry's default binding
/// (`&mut self.form.checked` gives `checked`).
fn field_hint(def: WidgetId) -> String {
    let wd = widget_def(def);
    wd.state
        .and_then(|s| wd.prop(s.prop))
        .and_then(|p| match p.default {
            DefaultValue::Expr(e) => e.rsplit('.').next().map(str::to_string),
            _ => None,
        })
        .unwrap_or_else(|| wd.name.to_string())
}

/// A new node for a palette item, with fresh ids: a stateful widget bound to a new field of the
/// form's state struct, a combo box with two items, and the containers whose id must be unique at
/// runtime (grid, indent, panels) with one made from their node id.
pub fn new_node(form: &mut Form, item: Item) -> Node {
    match item {
        Item::Widget(def) => {
            let mut node = if widget_def(def).state.is_some() {
                form.new_bound_widget(def, &field_hint(def))
            } else {
                form.new_widget(def)
            };
            if def == WidgetId::ComboBox
                && let NodeKind::Widget { props, .. } = &mut node.kind
            {
                let field = match props.get("value") {
                    Some(PropValue::Expr(e)) => {
                        e.rsplit('.').next().unwrap_or("choice").to_string()
                    }
                    _ => "choice".to_string(),
                };
                props.insert("id_salt", PropValue::Text(field));
                props.insert(
                    "items",
                    PropValue::Items(vec![
                        ComboItem {
                            value: "0".to_string(),
                            label: PropValue::Text("First".to_string()),
                        },
                        ComboItem {
                            value: "1".to_string(),
                            label: PropValue::Text("Second".to_string()),
                        },
                    ]),
                );
            }
            node
        }
        Item::Container(def) => {
            let mut node = form.new_container(def);
            let id = node.id.0;
            if let NodeKind::Container { props, .. } = &mut node.kind
                && let Some(PropValue::Text(name)) = props.get("id").cloned()
            {
                props.insert("id", PropValue::Text(format!("{name}_{id}")));
            }
            node
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    /// root(0) > [label a, vertical v > [b, c], horizontal h > [d, e], group g (empty)]
    fn sample() -> (Form, HashMap<NodeId, Rect>, [NodeId; 8]) {
        let mut f = Form::new("T", "ui");
        let root = f.root.id;
        let add = |f: &mut Form, parent: NodeId, item: Item| {
            let n = new_node(f, item);
            let id = n.id;
            f.insert(parent, usize::MAX, n).unwrap();
            id
        };
        let a = add(&mut f, root, Item::Widget(WidgetId::Label));
        let v = add(&mut f, root, Item::Container(ContainerId::Vertical));
        let b = add(&mut f, v, Item::Widget(WidgetId::Button));
        let c = add(&mut f, v, Item::Widget(WidgetId::Button));
        let h = add(&mut f, root, Item::Container(ContainerId::Horizontal));
        let d = add(&mut f, h, Item::Widget(WidgetId::Button));
        let e = add(&mut f, h, Item::Widget(WidgetId::Button));
        let g = add(&mut f, root, Item::Container(ContainerId::Group));
        let rects = HashMap::from([
            (root, r(0.0, 0.0, 400.0, 400.0)),
            (a, r(10.0, 10.0, 100.0, 30.0)),
            (v, r(10.0, 40.0, 200.0, 140.0)),
            (b, r(12.0, 42.0, 100.0, 80.0)),
            (c, r(12.0, 90.0, 100.0, 130.0)),
            (h, r(10.0, 150.0, 300.0, 200.0)),
            (d, r(20.0, 155.0, 100.0, 195.0)),
            (e, r(120.0, 155.0, 200.0, 195.0)),
            (g, r(10.0, 210.0, 100.0, 250.0)),
        ]);
        (f, rects, [root, a, v, b, c, h, d, e])
    }

    #[test]
    fn a_vertical_container_takes_the_index_from_child_midpoints() {
        let (f, rects, [_, _, v, ..]) = sample();
        let at = |y: f32| canvas_drop(&f, &rects, pos2(50.0, y), None).unwrap();
        assert_eq!(
            (at(50.0).parent, at(50.0).index),
            (v, 0),
            "above b's middle"
        );
        assert_eq!(at(70.0).index, 1, "below b's middle");
        assert_eq!(at(100.0).index, 1, "above c's middle");
        assert_eq!(at(120.0).index, 2, "below c: the end");
        // The line sits between b and c.
        let line = at(70.0).line;
        assert_eq!(line[0].y, 85.0);
        assert_eq!((line[0].x, line[1].x), (10.0, 200.0));
    }

    #[test]
    fn a_horizontal_container_compares_across() {
        let (f, rects, [.., h, _, _]) = sample();
        let at = |x: f32| canvas_drop(&f, &rects, pos2(x, 175.0), None).unwrap();
        assert_eq!((at(30.0).parent, at(30.0).index), (h, 0));
        assert_eq!(at(80.0).index, 1);
        assert_eq!(at(170.0).index, 2);
        assert_eq!(at(250.0).index, 2);
        let line = at(80.0).line;
        assert_eq!(
            (line[0].x, line[1].x),
            (118.0, 118.0),
            "a vertical line before e"
        );
    }

    #[test]
    fn near_a_containers_edge_the_drop_goes_to_its_parent() {
        let (f, rects, [root, _, v, ..]) = sample();
        // 3pt inside the top edge of `v`: before it, in the root.
        let d = canvas_drop(&f, &rects, pos2(100.0, 43.0), None).unwrap();
        assert_eq!((d.parent, d.index), (root, 1));
        // 3pt inside its bottom edge: after it.
        let d = canvas_drop(&f, &rects, pos2(100.0, 137.0), None).unwrap();
        assert_eq!((d.parent, d.index), (root, 2));
        // 7pt inside: into it.
        let d = canvas_drop(&f, &rects, pos2(150.0, 47.0), None).unwrap();
        assert_eq!(d.parent, v);
        // The root itself has no parent to go to.
        let d = canvas_drop(&f, &rects, pos2(2.0, 2.0), None).unwrap();
        assert_eq!(d.parent, root);
    }

    #[test]
    fn an_empty_container_is_a_target() {
        let (f, rects, [root, ..]) = sample();
        let g = f.root.children()[3].id;
        let d = canvas_drop(&f, &rects, pos2(50.0, 230.0), None).unwrap();
        assert_eq!((d.parent, d.index), (g, 0));
        assert!(rects[&g].contains(d.line[0]) && rects[&g].contains(d.line[1]));
        assert_ne!(d.parent, root);
    }

    #[test]
    fn a_node_never_drops_into_itself() {
        let (f, rects, [root, _, v, ..]) = sample();
        let d = canvas_drop(&f, &rects, pos2(150.0, 60.0), Some(v)).unwrap();
        assert_eq!(d.parent, root, "the moving container is no target");
        assert_eq!(d.index, 1, "it lands where it was");
        assert_eq!(move_index(&f, v, d.parent, d.index), None, "a no-op");
    }

    #[test]
    fn move_index_accounts_for_the_node_leaving() {
        let (f, _, [root, a, v, ..]) = sample();
        assert_eq!(move_index(&f, a, root, 0), None);
        assert_eq!(move_index(&f, a, root, 1), None);
        assert_eq!(move_index(&f, a, root, 3), Some(2));
        assert_eq!(move_index(&f, a, v, 1), Some(1));
    }

    #[test]
    fn the_outline_drops_before_after_and_into() {
        let (f, _, [root, a, v, b, ..]) = sample();
        let rows: Vec<Row> = [(root, 0), (a, 1), (v, 1), (b, 2)]
            .iter()
            .enumerate()
            .map(|(i, &(id, depth))| Row {
                id,
                rect: Rect::from_min_size(pos2(0.0, i as f32 * 20.0), vec2(200.0, 20.0)),
                depth,
            })
            .collect();
        let at = |y: f32| outline_drop(&f, &rows, pos2(50.0, y), None).unwrap();
        assert_eq!(
            (at(22.0).parent, at(22.0).index),
            (root, 0),
            "top of a: before it"
        );
        assert_eq!(
            (at(38.0).parent, at(38.0).index),
            (root, 1),
            "bottom of a: after it"
        );
        assert_eq!(
            (at(42.0).parent, at(42.0).index),
            (root, 1),
            "top quarter of v: before it"
        );
        assert_eq!(
            (at(50.0).parent, at(50.0).index),
            (v, 2),
            "middle of v: into it, at the end"
        );
        assert_eq!(
            (at(5.0).parent, at(5.0).index),
            (root, 4),
            "the form row: into the form"
        );
        assert_eq!(
            (at(95.0).parent, at(95.0).index),
            (root, 4),
            "below the rows: the end"
        );
        assert_eq!(
            outline_drop(&f, &rows, pos2(50.0, 50.0), Some(v)),
            None,
            "not into itself"
        );
    }

    #[test]
    fn a_palette_click_adds_into_a_container_or_after_a_widget() {
        let (f, _, [root, a, v, b, ..]) = sample();
        assert_eq!(click_target(&f, None), (root, 4));
        assert_eq!(click_target(&f, Some(root)), (root, 4));
        assert_eq!(click_target(&f, Some(v)), (v, 2));
        assert_eq!(click_target(&f, Some(b)), (v, 1));
        assert_eq!(click_target(&f, Some(a)), (root, 1));
        assert_eq!(click_target(&f, Some(NodeId(999))), (root, 4));
    }

    #[test]
    fn new_items_get_state_fields_and_unique_ids() {
        let mut f = Form::new("T", "ui");
        let a = new_node(&mut f, Item::Widget(WidgetId::Checkbox));
        let b = new_node(&mut f, Item::Widget(WidgetId::Checkbox));
        let combo = new_node(&mut f, Item::Widget(WidgetId::ComboBox));
        let grid = new_node(&mut f, Item::Container(ContainerId::Grid));
        let value = |n: &Node, p: &str| match &n.kind {
            NodeKind::Widget { props, .. } | NodeKind::Container { props, .. } => {
                props.get(p).cloned()
            }
            NodeKind::Custom { .. } => None,
        };
        assert_eq!(
            value(&a, "value"),
            Some(PropValue::Expr("&mut self.form.checked".into()))
        );
        assert_eq!(
            value(&b, "value"),
            Some(PropValue::Expr("&mut self.form.checked_2".into()))
        );
        assert_eq!(
            f.state
                .iter()
                .map(|s| (s.name.as_str(), s.ty.as_str()))
                .collect::<Vec<_>>(),
            [
                ("checked", "bool"),
                ("checked_2", "bool"),
                ("choice", "usize")
            ]
        );
        assert_eq!(
            value(&combo, "id_salt"),
            Some(PropValue::Text("choice".into()))
        );
        assert!(matches!(value(&combo, "items"), Some(PropValue::Items(i)) if i.len() == 2));
        assert_eq!(
            value(&grid, "id"),
            Some(PropValue::Text(format!("grid_{}", grid.id.0)))
        );
        assert_eq!(palette().len(), formust_model::ALL_WIDGETS.len() + 14);
    }

    #[test]
    fn the_history_keeps_200_steps_and_joins_runs_of_one_key() {
        let mut h = History::default();
        let form = |n: u32| {
            vec![Form {
                next_id: n,
                ..Form::new("T", "ui")
            }]
        };
        for i in 0..250 {
            assert!(h.checkpoint(None, || form(i)));
        }
        assert_eq!(h.steps(), UNDO_LIMIT);
        let mut h = History::default();
        assert!(h.checkpoint(Some("text"), || form(1)));
        assert!(
            !h.checkpoint(Some("text"), || form(2)),
            "typing joins the step"
        );
        assert!(h.checkpoint(Some("other"), || form(3)));
        assert_eq!(h.steps(), 2);
        let back = h.undo(form(9)).unwrap();
        assert_eq!(back[0].next_id, 3);
        assert_eq!(h.redo(back).unwrap()[0].next_id, 9);
        assert!(
            h.checkpoint(Some("other"), || form(4)),
            "an undo ends the run"
        );
        assert!(!h.can_redo(), "a new change drops the redo steps");
    }
}
