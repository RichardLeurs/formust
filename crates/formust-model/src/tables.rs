//! Tables (`egui_extras::TableBuilder`): a [`ContainerId::Table`] holds its rows, the header row
//! first when the `header` property (its height) is set; each [`ContainerId::TableRow`] holds one
//! [`ContainerId::TableCell`] per column; the columns' kinds are the `columns` property, an
//! [`PropValue::Items`] list of `egui_extras::Column` expressions. The operations here keep rows,
//! cells and columns in step, as one edit each, and never drop what is in a cell.

use crate::form::{ComboItem, EditError, Form, Node, NodeId, NodeKind, PropValue, Props};
use crate::registry::ContainerId;

/// The kind of a new column.
pub const TABLE_COLUMN: &str = "egui_extras::Column::auto()";

/// The most columns and body rows the designer makes.
pub const MAX_TABLE_COLUMNS: usize = 16;
pub const MAX_TABLE_ROWS: usize = 100;

pub(crate) fn column_item() -> ComboItem {
    ComboItem {
        value: TABLE_COLUMN.to_string(),
        label: PropValue::Text(String::new()),
    }
}

/// Does a table with these properties have a header row (its first child)?
pub fn table_has_header(props: &Props) -> bool {
    props.contains_key("header")
}

impl Form {
    /// A new table row with `cells` empty cells.
    pub(crate) fn new_table_row(&mut self, cells: usize) -> Node {
        let mut row = self.bare_container(ContainerId::TableRow);
        let kids: Vec<Node> = (0..cells).map(|_| self.new_table_cell()).collect();
        if let NodeKind::Container {
            children, closure, ..
        } = &mut row.kind
        {
            *children = kids;
            closure.clear();
        }
        row
    }

    fn new_table_cell(&mut self) -> Node {
        self.bare_container(ContainerId::TableCell)
    }

    /// The indent of a cell's closing line: a header cell's `header.col(|ui| {` is one step in
    /// from the table, a body cell's `row.col(|ui| {` two.
    pub(crate) fn table_cell_indent(&self, cell: NodeId) -> Option<String> {
        let (row, at) = self.parent_of(cell).map(|(r, _)| r).and_then(|r| {
            let (_, at) = self.parent_of(r)?;
            Some((r, at))
        })?;
        let (table, _) = self.parent_of(row)?;
        let node = self.find(table)?;
        let header = match &node.kind {
            NodeKind::Container { props, .. } => table_has_header(props) && at == 0,
            _ => false,
        };
        let step = if header { "    " } else { "        " };
        Some(format!("{}{step}", node.indent().unwrap_or("")))
    }

    /// (columns, body rows, has a header) of a Table.
    pub fn table_size(&self, id: NodeId) -> Option<(usize, usize, bool)> {
        match &self.find(id)?.kind {
            NodeKind::Container {
                def: ContainerId::Table,
                props,
                children,
                ..
            } => {
                let header = table_has_header(props);
                let columns = match props.get("columns") {
                    Some(PropValue::Items(items)) => items.len(),
                    _ => children.first().map_or(0, |r| r.children().len()),
                };
                Some((columns, children.len() - usize::from(header), header))
            }
            _ => None,
        }
    }

    fn row_ids(&self, id: NodeId) -> Vec<(NodeId, Vec<NodeId>)> {
        self.find(id)
            .map(|t| {
                t.children()
                    .iter()
                    .map(|r| (r.id, r.children().iter().map(|c| c.id).collect()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn atomic(
        &mut self,
        f: impl FnOnce(&mut Form) -> Result<(), EditError>,
    ) -> Result<(), EditError> {
        let backup = self.clone();
        let result = f(self);
        if result.is_err() {
            *self = backup;
        }
        result
    }

    /// Sets the number of columns (new ones `egui_extras::Column::auto()`). The contents of removed
    /// cells go to the end of the last cell kept in their row.
    pub fn set_table_columns(&mut self, id: NodeId, count: usize) -> Result<(), EditError> {
        let (now, _, _) = self.table_size(id).ok_or(EditError::NotAContainer(id))?;
        let mut items = match self.find(id).map(|n| &n.kind) {
            Some(NodeKind::Container { props, .. }) => match props.get("columns") {
                Some(PropValue::Items(items)) => items.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        items.resize_with(now, column_item);
        items.truncate(count);
        items.resize_with(count, column_item);
        self.set_table_columns_list(id, items)
    }

    /// Sets the columns' kinds; their number is the number of cells in every row.
    pub(crate) fn set_table_columns_list(
        &mut self,
        id: NodeId,
        items: Vec<ComboItem>,
    ) -> Result<(), EditError> {
        let count = items.len();
        if !(1..=MAX_TABLE_COLUMNS).contains(&count) {
            return Err(EditError::NoSuchProp("columns".into()));
        }
        self.atomic(|f| {
            let rows = f.row_ids(id);
            for (_, cells) in &rows {
                if cells.len() > count {
                    let kept = cells[count - 1];
                    for &gone in &cells[count..] {
                        let moving: Vec<NodeId> = f
                            .find(gone)
                            .map_or(Vec::new(), |c| c.children().iter().map(|k| k.id).collect());
                        for k in moving {
                            f.move_node(k, kept, usize::MAX)?;
                        }
                    }
                }
            }
            let extra: Vec<Vec<Node>> = rows
                .iter()
                .map(|(_, cells)| (cells.len()..count).map(|_| f.new_table_cell()).collect())
                .collect();
            f.edit(id, |node| {
                if let NodeKind::Container {
                    props, children, ..
                } = &mut node.kind
                {
                    for (row, more) in children.iter_mut().zip(extra) {
                        if let NodeKind::Container {
                            children: cells, ..
                        } = &mut row.kind
                        {
                            cells.truncate(count);
                            cells.extend(more);
                        }
                        row.src = None;
                    }
                    props.insert("columns", PropValue::Items(items));
                }
            })
        })
    }

    /// Sets the number of body rows. The contents of removed rows go to the end of the matching
    /// cells of the last row kept.
    pub fn set_table_rows(&mut self, id: NodeId, count: usize) -> Result<(), EditError> {
        let (columns, _, header) = self.table_size(id).ok_or(EditError::NotAContainer(id))?;
        if !(1..=MAX_TABLE_ROWS).contains(&count) {
            return Err(EditError::NoSuchProp("rows".into()));
        }
        let first = usize::from(header);
        self.atomic(|f| {
            let rows = f.row_ids(id);
            let body = &rows[first..];
            if body.len() > count {
                let kept = body[count - 1].1.clone();
                for (_, cells) in &body[count..] {
                    for (k, &cell) in cells.iter().enumerate() {
                        let Some(&target) = kept.get(k).or(kept.last()) else {
                            continue;
                        };
                        let moving: Vec<NodeId> = f
                            .find(cell)
                            .map_or(Vec::new(), |c| c.children().iter().map(|n| n.id).collect());
                        for n in moving {
                            f.move_node(n, target, usize::MAX)?;
                        }
                    }
                }
            }
            let more: Vec<Node> = (body.len()..count)
                .map(|_| f.new_table_row(columns))
                .collect();
            f.edit(id, |node| {
                if let NodeKind::Container { children, .. } = &mut node.kind {
                    let leading = children
                        .last()
                        .map(|r| r.leading.clone())
                        .unwrap_or_default();
                    children.truncate(first + count);
                    for mut row in more {
                        row.leading = leading.clone();
                        children.push(row);
                    }
                }
            })
        })
    }

    /// Gives the table a header row of this height, or takes it away (`None`): the header cells'
    /// contents then go to the start of the first body row's cells.
    pub fn set_table_header(&mut self, id: NodeId, height: Option<f64>) -> Result<(), EditError> {
        let (columns, _, header) = self.table_size(id).ok_or(EditError::NotAContainer(id))?;
        self.atomic(|f| match (header, height) {
            (true, Some(h)) | (false, Some(h)) => {
                let row = (!header).then(|| f.new_table_row(columns));
                f.edit(id, |node| {
                    if let NodeKind::Container {
                        props, children, ..
                    } = &mut node.kind
                    {
                        if let Some(row) = row {
                            children.insert(0, row);
                        }
                        props.insert("header", PropValue::Float(h));
                    }
                })
            }
            (true, None) => {
                let rows = f.row_ids(id);
                if let (Some((_, head)), Some((_, first))) = (rows.first(), rows.get(1)) {
                    for (k, &cell) in head.iter().enumerate() {
                        let Some(&target) = first.get(k).or(first.last()) else {
                            continue;
                        };
                        let moving: Vec<NodeId> = f
                            .find(cell)
                            .map_or(Vec::new(), |c| c.children().iter().map(|n| n.id).collect());
                        for (at, n) in moving.into_iter().enumerate() {
                            f.move_node(n, target, at)?;
                        }
                    }
                }
                f.edit(id, |node| {
                    if let NodeKind::Container {
                        props, children, ..
                    } = &mut node.kind
                    {
                        if children.len() > 1 {
                            children.remove(0);
                        }
                        props.remove("header");
                    }
                })
            }
            (false, None) => Ok(()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::WidgetId;

    fn sizes(form: &Form, id: NodeId) -> Vec<usize> {
        form.find(id)
            .unwrap()
            .children()
            .iter()
            .map(|r| r.children().len())
            .collect()
    }

    #[test]
    fn rows_columns_and_header_change_together_and_keep_what_cells_hold() {
        let mut form = Form::new("F", "ui");
        let table = form.new_container(ContainerId::Table);
        let id = table.id;
        form.insert(form.root.id, 0, table).unwrap();
        assert_eq!(form.table_size(id), Some((2, 2, true)));
        assert_eq!(sizes(&form, id), [2, 2, 2]);
        // Widgets go into a cell only.
        let label = form.new_widget(WidgetId::Label);
        assert!(form.insert(id, 0, label).is_err());
        let row = form.find(id).unwrap().children()[1].id;
        let label = form.new_widget(WidgetId::Label);
        assert!(form.insert(row, 0, label).is_err());
        let last_body = form.find(id).unwrap().children()[2].clone();
        let cell = last_body.children()[1].id;
        let label = form.new_widget(WidgetId::Label);
        let label_id = label.id;
        form.insert(cell, 0, label).unwrap();
        assert!(form.remove(cell).is_err() && form.remove(row).is_err());
        // Fewer columns: the label moves to the last cell kept in its row.
        form.set_table_columns(id, 1).unwrap();
        assert_eq!(sizes(&form, id), [1, 1, 1]);
        assert_eq!(
            form.parent_of(label_id).unwrap().0,
            last_body.children()[0].id
        );
        form.set_table_columns(id, 3).unwrap();
        assert_eq!(sizes(&form, id), [3, 3, 3]);
        // Fewer rows: the label moves up to the last row kept.
        form.set_table_rows(id, 1).unwrap();
        assert_eq!(form.table_size(id), Some((3, 1, true)));
        let kept = form.find(id).unwrap().children()[1].children()[0].id;
        assert_eq!(form.parent_of(label_id).unwrap().0, kept);
        form.set_table_rows(id, 4).unwrap();
        assert_eq!(sizes(&form, id), [3, 3, 3, 3, 3]);
        // No header: its row goes, and what it held moves into the first body row.
        let head_cell = form.find(id).unwrap().children()[0].children()[0].id;
        let title = form.new_widget(WidgetId::Heading);
        let title_id = title.id;
        form.insert(head_cell, 0, title).unwrap();
        form.clear_prop(id, "header").unwrap();
        assert_eq!(form.table_size(id), Some((3, 4, false)));
        assert_eq!(form.parent_of(title_id), Some((kept, 0)));
        form.set_prop(id, "header", PropValue::Float(24.0)).unwrap();
        assert_eq!(form.table_size(id), Some((3, 4, true)));
        assert!(form.set_table_columns(id, 0).is_err());
        assert!(form.set_table_rows(id, 0).is_err());
    }
}
