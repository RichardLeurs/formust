//! Tabs built from buttons: a bar of `ui.selectable_value(&mut field, value, label)` (or buttons)
//! and a `match field { value => { page } .. }` with one [`ContainerId::TabPage`] per tab.
//!
//! The tabs themselves (each a value and a label) are the Tabs' `tabs` property, a
//! [`PropValue::Items`] list in page order; the pages are its children, in the same order. The
//! operations here change both together, as one edit each, so they never get out of step.

use crate::form::{ComboItem, EditError, Form, Node, NodeId, NodeKind, PropValue};
use crate::registry::ContainerId;

/// Does a `match` on these tab values need a final `_ => {}` arm? Yes when one of them is a number
/// (a `usize` field, as the designer makes); an enum's variants can cover every case.
pub fn tabs_need_wildcard(tabs: &[ComboItem]) -> bool {
    tabs.iter().any(|t| {
        let v = t
            .value
            .trim()
            .trim_end_matches("usize")
            .trim_end_matches('_');
        !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '_')
    })
}

/// The integer value of a tab, if it is a plain number.
fn int_value(text: &str) -> Option<u64> {
    let t = text.trim().trim_end_matches("usize").replace('_', "");
    t.parse().ok()
}

impl Form {
    /// A new, empty page of a Tabs or TabsVertical (`holder`).
    pub(crate) fn new_page(&mut self, holder: ContainerId) -> Node {
        let _ = holder;
        let mut page = self.bare_container(ContainerId::TabPage);
        if let NodeKind::Container {
            ident,
            recv,
            closure,
            ..
        } = &mut page.kind
        {
            *ident = "ui".to_string();
            *recv = "ui".to_string();
            closure.clear();
        }
        page
    }

    /// A new Tabs (`def` is [`ContainerId::Tabs`] or [`ContainerId::TabsVertical`]) with two tabs,
    /// bound to a new `usize` field of the form's state struct.
    pub fn new_tabs(&mut self, def: ContainerId) -> Node {
        let mut node = self.new_container(def);
        let name = self.unique_field("tab");
        self.add_state_field(&name, "usize");
        if let NodeKind::Container { props, .. } = &mut node.kind {
            props.insert(
                "field",
                PropValue::Expr(format!("self.{}.{name}", self.state_field)),
            );
        }
        node
    }

    /// The tabs of a Tabs node, in page order.
    pub fn tabs(&self, id: NodeId) -> Option<Vec<ComboItem>> {
        match &self.find(id)?.kind {
            NodeKind::Container { def, props, .. } if def.is_tabs() => match props.get("tabs") {
                Some(PropValue::Items(items)) => Some(items.clone()),
                _ => Some(Vec::new()),
            },
            _ => None,
        }
    }

    /// Can a tab be added? Only when every value is a number: the new one is the next number. An
    /// enum's tabs need a new variant, which is written by hand.
    pub fn can_add_tab(&self, id: NodeId) -> bool {
        self.tabs(id)
            .is_some_and(|t| t.iter().all(|i| int_value(&i.value).is_some()))
    }

    /// Adds a tab at the end with an empty page; its value is one more than the largest. Returns
    /// the new page.
    pub fn add_tab(&mut self, id: NodeId, label: &str) -> Result<NodeId, EditError> {
        let tabs = self.tabs(id).ok_or(EditError::NoSuchProp("tabs".into()))?;
        if !self.can_add_tab(id) {
            return Err(EditError::Locked(id));
        }
        let next = tabs
            .iter()
            .filter_map(|t| int_value(&t.value))
            .max()
            .map_or(0, |m| m + 1);
        let def = match &self.find(id).map(|n| &n.kind) {
            Some(NodeKind::Container { def, .. }) => *def,
            _ => return Err(EditError::NotAContainer(id)),
        };
        let mut page = self.new_page(def);
        let page_id = page.id;
        self.edit(id, |node| {
            if let NodeKind::Container {
                props, children, ..
            } = &mut node.kind
            {
                // The same layout as the pages before it.
                page.leading = children
                    .last()
                    .map(|c| c.leading.clone())
                    .unwrap_or_default();
                children.push(page);
                let mut tabs = tabs;
                tabs.push(ComboItem {
                    value: next.to_string(),
                    label: PropValue::Text(label.to_string()),
                });
                props.insert("tabs", PropValue::Items(tabs));
            }
        })?;
        Ok(page_id)
    }

    /// Removes tab `index` and its page, with what is on it. The last tab stays.
    pub fn remove_tab(&mut self, id: NodeId, index: usize) -> Result<(), EditError> {
        let tabs = self.tabs(id).ok_or(EditError::NoSuchProp("tabs".into()))?;
        let pages = self.find(id).map_or(0, |n| n.children().len());
        if tabs.len() <= 1 || index >= tabs.len() || index >= pages {
            return Err(EditError::Locked(id));
        }
        self.edit(id, |node| {
            if let NodeKind::Container {
                props, children, ..
            } = &mut node.kind
            {
                let removed = children.remove(index);
                // The first page keeps the layout text before it.
                if index == 0
                    && let Some(first) = children.first_mut()
                {
                    first.leading = removed.leading;
                }
                let mut tabs = tabs;
                tabs.remove(index);
                props.insert("tabs", PropValue::Items(tabs));
            }
        })
    }

    /// Sets the label of tab `index` (a text or an expression).
    pub fn rename_tab(
        &mut self,
        id: NodeId,
        index: usize,
        label: PropValue,
    ) -> Result<(), EditError> {
        let mut tabs = self.tabs(id).ok_or(EditError::NoSuchProp("tabs".into()))?;
        let tab = tabs.get_mut(index).ok_or(EditError::Locked(id))?;
        tab.label = label;
        self.edit(id, |node| {
            if let NodeKind::Container { props, .. } = &mut node.kind {
                props.insert("tabs", PropValue::Items(tabs));
            }
        })
    }

    /// Moves tab `from` (and its page) to place `to`.
    pub fn move_tab(&mut self, id: NodeId, from: usize, to: usize) -> Result<(), EditError> {
        let mut tabs = self.tabs(id).ok_or(EditError::NoSuchProp("tabs".into()))?;
        let pages = self.find(id).map_or(0, |n| n.children().len());
        if from >= tabs.len() || to >= tabs.len() || from >= pages || to >= pages {
            return Err(EditError::Locked(id));
        }
        if from == to {
            return Ok(());
        }
        let tab = tabs.remove(from);
        tabs.insert(to, tab);
        self.edit(id, |node| {
            if let NodeKind::Container {
                props, children, ..
            } = &mut node.kind
            {
                // The layout text before the first arm stays first.
                let first = children[0].leading.clone();
                let page = children.remove(from);
                children.insert(to, page);
                if let Some(p) = children.first_mut() {
                    p.leading = first;
                }
                props.insert("tabs", PropValue::Items(tabs));
            }
        })
    }

    /// Replaces the tabs' values and labels, keeping the pages (same count only). A page whose
    /// value changed gets its arm head regenerated.
    pub(crate) fn set_tabs(&mut self, id: NodeId, new: Vec<ComboItem>) -> Result<(), EditError> {
        let old = self.tabs(id).ok_or(EditError::NoSuchProp("tabs".into()))?;
        let pages = self.find(id).map_or(0, |n| n.children().len());
        if new.len() != pages || new.is_empty() {
            return Err(EditError::NoSuchProp("tabs".into()));
        }
        self.edit(id, |node| {
            if let NodeKind::Container {
                props, children, ..
            } = &mut node.kind
            {
                for (k, page) in children.iter_mut().enumerate() {
                    if old.get(k).map(|t| t.value.trim()) != Some(new[k].value.trim()) {
                        page.src = None;
                        if let NodeKind::Container {
                            shell: Some(shell), ..
                        } = &mut page.kind
                        {
                            shell.head = None;
                        }
                    }
                }
                props.insert("tabs", PropValue::Items(new));
            }
        })
    }

    /// The Tabs and the index of the page `id` is on (or is), if any.
    pub fn page_of(&self, id: NodeId) -> Option<(NodeId, usize)> {
        let mut at = id;
        loop {
            let (parent, index) = self.parent_of(at)?;
            if let Some(NodeKind::Container { def, .. }) = self.find(parent).map(|n| &n.kind)
                && def.is_tabs()
            {
                return Some((parent, index));
            }
            at = parent;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::WidgetId;

    fn labels(form: &Form, id: NodeId) -> Vec<String> {
        form.tabs(id)
            .unwrap()
            .iter()
            .map(|t| match &t.label {
                PropValue::Text(s) | PropValue::Expr(s) => s.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn tabs_and_pages_change_together() {
        let mut form = Form::new("F", "ui");
        let tabs = form.new_tabs(ContainerId::Tabs);
        let id = tabs.id;
        form.insert(form.root.id, 0, tabs).unwrap();
        assert_eq!(form.state.len(), 1);
        assert_eq!(
            (form.state[0].name.as_str(), form.state[0].ty.as_str()),
            ("tab", "usize")
        );
        assert_eq!(form.find(id).unwrap().children().len(), 2);
        // Widgets go into a page, not into the Tabs.
        let label = form.new_widget(WidgetId::Label);
        assert!(form.insert(id, 0, label).is_err());
        let first = form.find(id).unwrap().children()[0].id;
        let label = form.new_widget(WidgetId::Label);
        let label_id = label.id;
        form.insert(first, 0, label).unwrap();
        assert!(form.remove(first).is_err(), "a page goes with its tab only");
        let page = form.add_tab(id, "Third").unwrap();
        assert_eq!(labels(&form, id), ["Tab 1", "Tab 2", "Third"]);
        assert_eq!(form.tabs(id).unwrap()[2].value, "2");
        form.move_tab(id, 0, 2).unwrap();
        assert_eq!(labels(&form, id), ["Tab 2", "Third", "Tab 1"]);
        assert_eq!(form.page_of(label_id), Some((id, 2)));
        assert_eq!(form.page_of(page), Some((id, 1)));
        form.rename_tab(id, 2, PropValue::Text("One".into()))
            .unwrap();
        form.remove_tab(id, 0).unwrap();
        assert_eq!(labels(&form, id), ["Third", "One"]);
        assert_eq!(form.find(id).unwrap().children().len(), 2);
        form.remove_tab(id, 0).unwrap();
        assert!(form.remove_tab(id, 0).is_err(), "the last tab stays");
        assert!(tabs_need_wildcard(&form.tabs(id).unwrap()));
    }

    #[test]
    fn enum_tabs_take_no_new_tab_and_need_no_wildcard() {
        let items = vec![
            ComboItem {
                value: "Tab::Main".into(),
                label: PropValue::Text("Main".into()),
            },
            ComboItem {
                value: "Tab::Other".into(),
                label: PropValue::Text("Other".into()),
            },
        ];
        assert!(!tabs_need_wildcard(&items));
        let mut form = Form::new("F", "ui");
        let tabs = form.new_tabs(ContainerId::TabsVertical);
        let id = tabs.id;
        form.insert(form.root.id, 0, tabs).unwrap();
        form.set_prop(id, "tabs", PropValue::Items(items)).unwrap();
        assert!(!form.can_add_tab(id));
        assert!(form.add_tab(id, "x").is_err());
        assert!(
            form.set_prop(id, "tabs", PropValue::Items(Vec::new()))
                .is_err(),
            "the count goes through add and remove"
        );
    }
}
