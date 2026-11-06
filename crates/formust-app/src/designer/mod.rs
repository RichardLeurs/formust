//! The form designer for egui forms in Rust files.
//!
//! The active `.rs` or `.frm.rs` tab is imported with `formust_rust`, every form in it can be
//! picked by `owner::method`, and the chosen one is drawn as a live preview (see [`preview`]): real
//! egui widgets that never react to input. A click on the canvas or a row of the outline selects a
//! node; the palette adds, drag and drop moves, the inspector edits (see [`edit`]), and saving
//! exports the edited forms to the `.frm.rs`, a standalone copy of the `.rs` with the forms in it
//! (see [`export`]). The `.rs` is never written and never refers to the copy.
//!
//! Edits live in the [`Loaded`] of their file until exported, with their undo history. Switching to
//! another tab parks an edited file's state, so its edits survive until it is saved or discarded.
//!
//! Which text is read: a `.rs` tab is merged with its form file (`login.frm.rs`) when that exists,
//! and the form file wins for each form it defines (docs/specs.md, "Source of truth"). The form file
//! is a whole copy of the source, so it usually defines them all; each form is read once. A
//! `.frm.rs` tab shows its own forms, compared with its source for the conflict state. An open
//! tab's text wins over the disk. A form file that doesn't parse blocks the designer.

pub mod edit;
pub mod export;
pub mod preview;
pub mod ui;
pub mod viewport;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{Rect, Vec2};
use formust_model::{
    ContainerId, EditError, Form, Node, NodeId, NodeKind, PropValue, container_def, widget_def,
};
use formust_rust::{Conflict, FileKind, Header, ImportedForm, Origin, Severity, file_kind};

/// One form as the designer shows it.
#[derive(Clone, Debug)]
pub struct DesignForm {
    pub imported: ImportedForm,
    /// Which file of a pair it came from; `None` when only one file was read.
    pub origin: Option<Origin>,
    /// For a form from a form file, compared with its source.
    pub conflict: Option<Conflict>,
    /// The form file's header line for this form; `None` for a form from the source (export makes
    /// one) or from a form file without a header.
    pub header: Option<Header>,
}

impl DesignForm {
    pub fn form(&self) -> &Form {
        &self.imported.form
    }

    /// The form's identity in its source: the header's (a legacy `X::update` keeps it), else its own.
    pub fn source_id(&self) -> String {
        self.header
            .as_ref()
            .map_or_else(|| self.form().id(), |h| h.id.clone())
    }

    /// The token hash of the source body this form is based on: the header's for a form from the
    /// form file, the one at import for a form from the source.
    pub fn base_hash(&self) -> Option<u32> {
        match self.origin {
            Some(Origin::Frm) => self.header.as_ref().map(|h| h.src_hash),
            _ => Some(self.imported.src_hash),
        }
    }

    fn set_base_hash(&mut self, hash: u32) {
        match (self.origin, self.header.as_mut()) {
            (Some(Origin::Frm), Some(h)) => h.src_hash = hash,
            _ => self.imported.src_hash = hash,
        }
    }

    /// Why a custom node is custom, from the importer.
    pub fn custom_reason(&self, id: NodeId) -> Option<&str> {
        self.imported
            .custom
            .iter()
            .find(|c| c.node == id)
            .map(|c| c.reason.as_str())
    }
}

/// The result of reading a file for the designer.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub path: PathBuf,
    /// What was read, so an unchanged file isn't imported again.
    pub(crate) key: u64,
    pub forms: Vec<DesignForm>,
    /// Importer messages worth showing (warnings and errors).
    pub notes: Vec<String>,
    /// Why nothing can be shown: the file (or its form file) doesn't parse.
    pub error: Option<String>,
    /// The other file of the pair that was read with it.
    pub companion: Option<PathBuf>,
    /// The form file's text as read (the tab's, else the disk's); export checks it is still that.
    pub frm_text: Option<String>,
    /// Each form and header as read or last exported: the designer has unsaved edits when the
    /// forms differ from it.
    pub clean: Vec<(Form, Option<Header>)>,
    pub history: edit::History,
    /// The file changed on disk or in its tab while this held unsaved edits, which were kept.
    pub stale: bool,
}

impl Loaded {
    pub fn is_dirty(&self) -> bool {
        self.forms.len() != self.clean.len()
            || self
                .forms
                .iter()
                .zip(&self.clean)
                .any(|(f, (form, header))| f.form() != form || &f.header != header)
    }

    pub fn mark_clean(&mut self) {
        self.clean = self
            .forms
            .iter()
            .map(|f| (f.form().clone(), f.header.clone()))
            .collect();
    }

    fn snapshot(&self) -> Vec<Form> {
        self.forms.iter().map(|f| f.form().clone()).collect()
    }

    fn restore(&mut self, forms: Vec<Form>) {
        for (f, form) in self.forms.iter_mut().zip(forms) {
            f.imported.form = form;
        }
    }
}

/// Reads `text` (the file at `path`) for the designer. `companion` is the other file of the pair,
/// with its text: the form file of a source, or the source of a form file.
pub fn load(path: &Path, text: &str, companion: Option<(&Path, &str)>) -> Loaded {
    let mut out = Loaded {
        path: path.to_path_buf(),
        companion: companion.map(|(p, _)| p.to_path_buf()),
        ..Loaded::default()
    };
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    let note = |out: &mut Loaded, d: &formust_rust::Diagnostic| {
        if d.severity != Severity::Info {
            out.notes.push(d.message.clone());
        }
    };
    match file_kind(path) {
        FileKind::Form => {
            let frm = match formust_rust::parse_frm(text) {
                Ok(frm) => frm,
                Err(e) => {
                    out.error = Some(format!(
                        "{} has a syntax error at line {}, column {}: {}. The designer stays closed until it is fixed in the code editor; Formust never overwrites a form file that doesn't parse.",
                        name(path),
                        e.line,
                        e.column,
                        e.message
                    ));
                    return out;
                }
            };
            for d in &frm.diagnostics {
                note(&mut out, d);
            }
            let source = companion.and_then(|(p, t)| Some((p, formust_rust::import(t).ok()?)));
            match source {
                Some((src_path, source)) => {
                    out.forms = formust_rust::merge(&name(src_path), &source, &frm)
                        .into_iter()
                        .filter(|m| m.origin == Origin::Frm)
                        .map(design_form)
                        .collect();
                }
                None => {
                    out.forms = frm
                        .forms
                        .into_iter()
                        .map(|f| DesignForm {
                            imported: f.imported,
                            origin: Some(Origin::Frm),
                            conflict: None,
                            header: f.header,
                        })
                        .collect();
                }
            }
            out.frm_text = Some(text.to_string());
        }
        FileKind::Source | FileKind::Other => {
            let source = match formust_rust::import(text) {
                Ok(s) => s,
                Err(e) => {
                    out.error = Some(format!(
                        "{} doesn't parse at line {}, column {}: {}",
                        name(path),
                        e.line,
                        e.column,
                        e.message
                    ));
                    return out;
                }
            };
            for d in &source.diagnostics {
                note(&mut out, d);
            }
            match companion {
                Some((frm_path, frm_text)) => match formust_rust::parse_frm(frm_text) {
                    Ok(frm) => {
                        for d in &frm.diagnostics {
                            note(&mut out, d);
                        }
                        out.forms = formust_rust::merge(&name(path), &source, &frm)
                            .into_iter()
                            .map(design_form)
                            .collect();
                        out.frm_text = Some(frm_text.to_string());
                    }
                    Err(e) => {
                        out.error = Some(format!(
                            "{} has a syntax error at line {}, column {}: {}. The designer stays closed until it is fixed in the code editor; Formust never overwrites a form file that doesn't parse.",
                            name(frm_path),
                            e.line,
                            e.column,
                            e.message
                        ));
                    }
                },
                None => {
                    out.forms = source
                        .forms
                        .into_iter()
                        .map(|f| DesignForm {
                            imported: f,
                            origin: Some(Origin::Rs),
                            conflict: None,
                            header: None,
                        })
                        .collect();
                }
            }
        }
    }
    out.mark_clean();
    out
}

/// A merged form as the designer keeps it: a form file's form with its header (none when the file
/// had none), a source's form without one.
fn design_form(m: formust_rust::MergedForm) -> DesignForm {
    let header =
        (m.origin == Origin::Frm && m.conflict != Some(Conflict::NoHeader)).then_some(m.header);
    DesignForm {
        imported: m.imported,
        origin: Some(m.origin),
        conflict: m.conflict,
        header,
    }
}

/// A cheap fingerprint of what a load reads.
pub fn key_of(path: &Path, text: &str, companion: Option<(&Path, &str)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    text.hash(&mut h);
    companion.hash(&mut h);
    h.finish()
}

/// The designer's state: what was read, the form shown and the selection.
#[derive(Default)]
pub struct Designer {
    pub loaded: Option<Loaded>,
    /// The paths that have had forms this session, so a syntax error while typing doesn't hide the
    /// Form button.
    pub had_forms: std::collections::HashSet<PathBuf>,
    /// The form shown, by `owner::method`, so it survives a re-import.
    pub form_id: Option<String>,
    pub selected: Option<NodeId>,
    pub hovered: Option<NodeId>,
    /// The preview's mode; `None` follows the IDE.
    pub dark: Option<bool>,
    /// Canvas scroll offset.
    pub scroll: Vec2,
    /// Node rects of the last kept preview pass, for hit-testing.
    pub rects: HashMap<NodeId, Rect>,
    /// How big the form laid itself out in the last pass.
    pub content: Vec2,
    /// The outline should scroll the selected row into view (set by a canvas click).
    pub reveal_row: bool,
    /// The canvas should scroll the selected node into view (set by an outline click).
    pub reveal_node: bool,
    /// Preview widgets that reacted to input in the last pass; always empty unless the shield
    /// broke. Kept for the tests.
    pub reacted: Vec<NodeId>,
    /// When the active file was last compared with the disk.
    pub(crate) checked: Option<std::time::Instant>,
    /// The active tab's text as last read, to notice typing at once.
    pub(crate) text_key: u64,
    /// Where the form's client area was drawn, and the preview's layer, in the last pass.
    pub client: Option<Rect>,
    pub preview_layer: Option<eframe::egui::LayerId>,
    /// A test's control run: let AccessKit actions through to the preview widgets.
    pub open_accesskit: bool,
    /// Edited files that aren't the active one, by path, with the form shown and the selection.
    pub parked: HashMap<PathBuf, (Loaded, Option<String>, Option<NodeId>)>,
    /// What is being dragged (from the palette, the canvas or the outline), and where it would land.
    pub drag: Option<edit::Payload>,
    pub drop: Option<edit::Drop>,
    /// The outline's rows as last drawn, for drops there.
    pub rows: Vec<edit::Row>,
    /// A question that stops the export until answered, and a difference being shown.
    pub prompt: Option<export::Prompt>,
    pub diff: Option<Diff>,
    /// The last message about an edit or export (a refused move, an export error, "Saved").
    pub status: Option<String>,
    /// The text the last Copy or Cut put on the clipboard.
    pub clipboard: Option<String>,
    /// The preview's width, set with the canvas corner grip; `None` is the default width.
    pub preview_width: Option<f32>,
    /// The right panel shows "Use in code" instead of the inspector.
    pub show_usage: bool,
    /// What the designer asks the app to do (save, the prompt's answers), run after drawing.
    pub actions: Vec<ui::Action>,
}

/// Two texts side by side, for the conflict prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    pub title: String,
    pub old_label: String,
    pub old: String,
    pub new_label: String,
    pub new: String,
}

impl Designer {
    /// Takes a new load, keeping the chosen form and the selection when they still exist.
    pub fn set_loaded(&mut self, loaded: Loaded) {
        let same_file = self.loaded.as_ref().is_some_and(|l| l.path == loaded.path);
        if !loaded.forms.is_empty() {
            self.had_forms.insert(loaded.path.clone());
        }
        self.loaded = Some(loaded);
        if !same_file {
            self.form_id = None;
            self.selected = None;
            self.hovered = None;
            self.scroll = Vec2::ZERO;
            self.rects.clear();
        }
        let forms = self.forms();
        if self
            .form_id
            .as_ref()
            .is_none_or(|id| !forms.iter().any(|f| &f.form().id() == id))
        {
            self.form_id = forms.first().map(|f| f.form().id());
            self.selected = None;
            self.rects.clear();
        }
        if let Some(s) = self.selected
            && self.current().is_none_or(|f| f.form().find(s).is_none())
        {
            self.selected = None;
        }
    }

    pub fn is_loaded(&self, path: &Path, key: u64) -> bool {
        self.loaded
            .as_ref()
            .is_some_and(|l| l.path == path && l.key == key)
    }

    /// Takes what was read for a file. Unsaved edits are never replaced: the same file keeps its
    /// edits and is marked stale; another file parks them until it is shown again.
    pub fn store(&mut self, mut loaded: Loaded, key: u64) {
        loaded.key = key;
        if let Some(current) = self.loaded.as_mut()
            && current.path == loaded.path
            && current.is_dirty()
        {
            current.key = key;
            current.stale = true;
            return;
        }
        if let Some(current) = self.loaded.take() {
            if current.path != loaded.path && current.is_dirty() {
                let path = current.path.clone();
                self.parked
                    .insert(path, (current, self.form_id.take(), self.selected.take()));
            } else {
                self.loaded = Some(current);
            }
        }
        if let Some((mut parked, form_id, selected)) = self.parked.remove(&loaded.path) {
            if parked.key != key {
                parked.key = key;
                parked.stale = true;
            }
            if !parked.forms.is_empty() {
                self.had_forms.insert(parked.path.clone());
            }
            self.loaded = Some(parked);
            self.form_id = form_id;
            self.selected = selected;
            self.hovered = None;
            self.rects.clear();
            return;
        }
        self.set_loaded(loaded);
    }

    /// Unsaved designer edits for `path`, shown or parked.
    pub fn dirty_for(&self, path: &Path) -> bool {
        self.loaded
            .as_ref()
            .is_some_and(|l| l.path == path && l.is_dirty())
            || self.parked.contains_key(path)
    }

    pub fn is_dirty(&self) -> bool {
        self.loaded.as_ref().is_some_and(Loaded::is_dirty)
    }

    /// Drops the unsaved edits for `path` (its tab was closed without saving).
    pub fn discard(&mut self, path: &Path) {
        self.parked.remove(path);
        if self.loaded.as_ref().is_some_and(|l| l.path == path) {
            self.loaded = None;
            self.prompt = None;
        }
    }

    /// Shows a parked file's state for `f`, then puts things back. For saving a file whose tab
    /// isn't the active one.
    pub fn with_parked<R>(&mut self, path: &Path, f: impl FnOnce(&mut Loaded) -> R) -> Option<R> {
        if self.loaded.as_ref().is_some_and(|l| l.path == path) {
            return self.loaded.as_mut().map(f);
        }
        let (mut loaded, form_id, selected) = self.parked.remove(path)?;
        let out = f(&mut loaded);
        if loaded.is_dirty() {
            self.parked
                .insert(path.to_path_buf(), (loaded, form_id, selected));
        }
        Some(out)
    }

    pub fn forms(&self) -> &[DesignForm] {
        self.loaded.as_ref().map_or(&[], |l| &l.forms)
    }

    /// The form shown.
    pub fn current(&self) -> Option<&DesignForm> {
        let id = self.form_id.as_ref()?;
        self.forms().iter().find(|f| &f.form().id() == id)
    }

    fn current_index(&self) -> Option<usize> {
        let id = self.form_id.as_ref()?;
        self.forms().iter().position(|f| &f.form().id() == id)
    }

    /// Changes the form shown as one undo step. Changes with the same `key` in a row join one step
    /// (typing in one field). A refused change leaves the form as it was.
    pub fn edit<R>(
        &mut self,
        key: Option<&str>,
        f: impl FnOnce(&mut Form) -> Result<R, EditError>,
    ) -> Result<R, EditError> {
        let Some(i) = self.current_index() else {
            return Err(EditError::NoSuchNode(NodeId(0)));
        };
        let loaded = self.loaded.as_mut().expect("a form is shown");
        let before = loaded.snapshot();
        let result = f(&mut loaded.forms[i].imported.form);
        match &result {
            Ok(_) if loaded.forms[i].form() != &before[i] => {
                loaded.history.checkpoint(key, || before);
            }
            Ok(_) => {}
            Err(_) => loaded.forms[i].imported.form = before[i].clone(),
        }
        if result.is_ok() {
            self.status = None;
        }
        result
    }

    /// Runs an edit and says why it was refused, if it was.
    pub fn try_edit(
        &mut self,
        key: Option<&str>,
        f: impl FnOnce(&mut Form) -> Result<(), EditError>,
    ) -> bool {
        match self.edit(key, f) {
            Ok(()) => true,
            Err(e) => {
                self.status = Some(edit_error_text(&e));
                false
            }
        }
    }

    pub fn undo(&mut self) -> bool {
        self.step_history(true)
    }

    pub fn redo(&mut self) -> bool {
        self.step_history(false)
    }

    fn step_history(&mut self, back: bool) -> bool {
        let Some(loaded) = self.loaded.as_mut() else {
            return false;
        };
        let current = loaded.snapshot();
        let state = if back {
            loaded.history.undo(current)
        } else {
            loaded.history.redo(current)
        };
        let Some(state) = state else { return false };
        loaded.restore(state);
        if let Some(s) = self.selected
            && self.current().is_none_or(|f| f.form().find(s).is_none())
        {
            self.selected = None;
        }
        true
    }

    /// Adds a palette item under `parent` at `index` (a click: into the selected container or
    /// after the selected widget) and selects it.
    pub fn add(&mut self, item: edit::Item, at: Option<(NodeId, usize)>) -> Option<NodeId> {
        let selected = self.selected;
        let id = self
            .edit(None, |f| {
                let (parent, index) = at.unwrap_or_else(|| edit::click_target(f, selected));
                let node = edit::new_node(f, item);
                let id = node.id;
                f.insert(parent, index, node).map(|()| id)
            })
            .map_err(|e| self.status = Some(edit_error_text(&e)))
            .ok()?;
        self.select(Some(id), true);
        Some(id)
    }

    /// Moves `id` to the drop place `index` among `parent`'s current children.
    pub fn move_to(&mut self, id: NodeId, parent: NodeId, index: usize) -> bool {
        let Some(form) = self.current().map(|f| f.form().clone()) else {
            return false;
        };
        let Some(index) = edit::move_index(&form, id, parent, index) else {
            return false;
        };
        let moved = self.try_edit(None, |f| f.move_node(id, parent, index));
        if moved {
            self.select(Some(id), true);
        }
        moved
    }

    /// One place earlier (`-1`) or later (`1`) among its siblings.
    pub fn nudge(&mut self, delta: i32) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let Some((parent, at)) = self.current().and_then(|f| f.form().parent_of(id)) else {
            return false;
        };
        let len = self
            .current()
            .and_then(|f| f.form().find(parent))
            .map_or(0, |p| p.children().len());
        let to = at as i64 + i64::from(delta);
        if to < 0 || to >= len as i64 {
            return false;
        }
        self.try_edit(None, |f| f.move_node(id, parent, to as usize))
    }

    /// Deletes the selected node and selects its neighbour (or its parent).
    pub fn delete_selected(&mut self) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let Some((parent, at)) = self.current().and_then(|f| f.form().parent_of(id)) else {
            return false;
        };
        if !self.try_edit(None, |f| f.remove(id).map(|_| ())) {
            return false;
        }
        let next = self
            .current()
            .and_then(|f| f.form().find(parent))
            .and_then(|p| {
                let kids = p.children();
                kids.get(at)
                    .or_else(|| at.checked_sub(1).and_then(|i| kids.get(i)))
                    .map(|n| n.id)
            });
        self.select(next.or(Some(parent)), true);
        true
    }

    /// Duplicates the selected node after itself and selects the copy.
    pub fn duplicate_selected(&mut self) -> Option<NodeId> {
        let id = self.selected?;
        let new = self
            .edit(None, |f| {
                let (parent, at) = f.parent_of(id).ok_or(EditError::IsRoot)?;
                let copy = f.clone_subtree(id)?;
                let new = copy.id;
                f.insert(parent, at + 1, copy).map(|()| new)
            })
            .map_err(|e| self.status = Some(edit_error_text(&e)))
            .ok()?;
        self.select(Some(new), true);
        Some(new)
    }

    /// The selected node as Rust text (custom code too: it is copied, never opened).
    pub fn copy_selected(&mut self) -> Option<String> {
        let id = self.selected?;
        let f = self.current()?.form();
        if id == f.root.id {
            return None;
        }
        let text = formust_rust::clip::copy_text(f, &[f.find(id)?]);
        self.clipboard = Some(text.clone());
        Some(text)
    }

    pub fn cut_selected(&mut self) -> Option<String> {
        let text = self.copy_selected()?;
        self.delete_selected().then_some(text)
    }

    /// Pastes Rust statements: read by the importer, inserted where a palette click would add.
    pub fn paste(&mut self, text: &str) -> bool {
        let nodes = match formust_rust::clip::parse_statements(text) {
            Ok(n) => n,
            Err(e) => {
                self.status = Some(format!("Can't paste: {}", e.message));
                return false;
            }
        };
        let selected = self.selected;
        let last = self.edit(None, |f| {
            let (parent, mut index) = edit::click_target(f, selected);
            let mut last = None;
            for node in nodes {
                let node = f.adopt(node);
                last = Some(node.id);
                f.insert(parent, index, node)?;
                index += 1;
            }
            Ok(last)
        });
        match last {
            Ok(last) => {
                self.select(last, true);
                true
            }
            Err(e) => {
                self.status = Some(edit_error_text(&e));
                false
            }
        }
    }

    /// The conflict prompt's "keep": the form stays as it is and is now based on the source as it
    /// is, so the question doesn't come back. Exported with the next save.
    pub fn keep_form(&mut self, form: &str, read: &dyn Fn(&Path) -> Option<String>) {
        let Some(loaded) = self.loaded.as_mut() else {
            return;
        };
        let Ok(t) = export::target(loaded, &|p| p.exists()) else {
            return;
        };
        let Some(source) = t
            .source
            .as_deref()
            .and_then(read)
            .and_then(|text| formust_rust::import(&text).ok())
        else {
            return;
        };
        if let Some(now) = source.form(form)
            && let Some(f) = loaded.forms.iter_mut().find(|f| f.source_id() == form)
        {
            f.set_base_hash(now.src_hash);
            if f.conflict
                .is_some_and(|c| matches!(c, Conflict::Changed { .. }))
            {
                f.conflict = Some(Conflict::Unchanged);
            }
        }
        self.prompt = None;
    }

    /// The conflict prompt's "re-import": the form as the source has it now, replacing the
    /// designer's (and, after saving, the form file's). Starts a new history.
    pub fn reimport_form(&mut self, form: &str, read: &dyn Fn(&Path) -> Option<String>) -> bool {
        let Some(loaded) = self.loaded.as_mut() else {
            return false;
        };
        let Ok(t) = export::target(loaded, &|p| p.exists()) else {
            return false;
        };
        let Some(source) = t
            .source
            .as_deref()
            .and_then(read)
            .and_then(|text| formust_rust::import(&text).ok())
        else {
            self.status = Some("The source can't be read or doesn't parse".to_string());
            return false;
        };
        let Some(now) = source.form(form).cloned() else {
            self.status = Some(format!("The source has no {form} any more"));
            return false;
        };
        let Some(f) = loaded.forms.iter_mut().find(|f| f.source_id() == form) else {
            return false;
        };
        let header = f.header.clone().map(|h| Header {
            src_hash: now.src_hash,
            ..h
        });
        f.imported = now;
        f.header = header;
        f.conflict = Some(Conflict::Unchanged);
        loaded.history = edit::History::default();
        self.selected = None;
        self.prompt = None;
        self.rects.clear();
        true
    }

    /// The form file's version of the forms back: drops the designer's edits (the "changed form
    /// file" prompt's reload). The next sync reads the file again.
    pub fn reload(&mut self) {
        if let Some(l) = self.loaded.as_mut() {
            l.forms = l
                .clean
                .iter()
                .zip(&l.forms)
                .map(|((form, header), f)| DesignForm {
                    imported: ImportedForm {
                        form: form.clone(),
                        ..f.imported.clone()
                    },
                    header: header.clone(),
                    ..f.clone()
                })
                .collect();
            l.key = 0;
            l.stale = false;
            l.history = edit::History::default();
        }
        self.prompt = None;
        self.selected = None;
    }

    pub fn choose_form(&mut self, id: String) {
        if self.form_id.as_ref() != Some(&id) {
            self.form_id = Some(id);
            self.selected = None;
            self.hovered = None;
            self.scroll = Vec2::ZERO;
            self.rects.clear();
        }
    }

    /// Selects a node (or nothing). `from_canvas` decides which side follows: the outline scrolls
    /// to a node picked on the canvas, the canvas to one picked in the outline.
    pub fn select(&mut self, id: Option<NodeId>, from_canvas: bool) {
        if self.selected != id
            && let Some(l) = self.loaded.as_mut()
        {
            // Typing in another node's field is a step of its own.
            l.history.break_run();
        }
        self.selected = id;
        if from_canvas {
            self.reveal_row = id.is_some();
        } else {
            self.reveal_node = id.is_some();
        }
    }
}

/// The text of a new Rust form `name`: an egui struct with a `ui` method, ready for the designer.
/// `eframe`: the project uses eframe, so egui comes through it.
pub fn new_form_text(name: &str, eframe: bool) -> String {
    let import = if eframe { "use eframe::egui;\n\n" } else { "" };
    format!(
        "//! {name}: a Formust form. Edit it in the designer (Form), or here as code.\n\n{import}#[derive(Default)]\npub struct {name} {{\n    name: String,\n    agree: bool,\n}}\n\nimpl {name} {{\n    pub fn ui(&mut self, ui: &mut egui::Ui) {{\n        ui.heading(\"{name}\");\n        ui.horizontal(|ui| {{\n            ui.label(\"Name\");\n            ui.text_edit_singleline(&mut self.name);\n        }});\n        ui.checkbox(&mut self.agree, \"I agree\");\n        if ui.button(\"OK\").clicked() {{\n            self.on_ok_clicked();\n        }}\n    }}\n\n    fn on_ok_clicked(&mut self) {{}}\n}}\n"
    )
}

/// Creates `form1.rs` (or the first free `formN.rs` whose form file is free too) in `dir` with a
/// new form `FormN`. Never overwrites a file.
pub fn new_form_file(dir: &Path, eframe: bool) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    for n in 1..10_000 {
        let path = dir.join(format!("form{n}.rs"));
        if path.exists()
            || dir
                .join(format!("form{n}{}", formust_rust::FORM_SUFFIX))
                .exists()
        {
            continue;
        }
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        file.write_all(new_form_text(&format!("Form{n}"), eframe).as_bytes())?;
        return Ok(path);
    }
    Err(std::io::Error::other("no free formN.rs name"))
}

/// A refused edit in words.
pub fn edit_error_text(e: &EditError) -> String {
    match e {
        EditError::NoSuchNode(_) => "That node is gone".to_string(),
        EditError::IsRoot => "The form itself can't be moved, deleted or copied".to_string(),
        EditError::NotAContainer(_) => "Only a container can hold other widgets".to_string(),
        EditError::IntoOwnSubtree => "A container can't go inside itself".to_string(),
        EditError::NoSuchProp(p) => format!("No property {p} here"),
        EditError::IsCustom(_) => "Custom code is locked: edit it in the code editor".to_string(),
        EditError::WrapperTaken(w) => {
            format!("This widget already uses {w}; clear it first (one add_* wrapper per widget)")
        }
        EditError::Locked(_) => {
            "A branch of an if or match can't be moved or removed on its own".to_string()
        }
        EditError::NeedsUi(_) => {
            "Here is the dialog's context, not a Ui: put widgets inside its Window or dialog"
                .to_string()
        }
    }
}

/// Is `path` a file the designer reads?
pub fn is_rust(path: &Path) -> bool {
    file_kind(path) != FileKind::Other
}

/// How the outline and the property list name a node: its kind, then its text when it has one.
pub fn node_title(n: &Node) -> String {
    match &n.kind {
        NodeKind::Widget {
            def, props, events, ..
        } => {
            let d = widget_def(*def);
            let text = ["text", "url", "label", "value"]
                .iter()
                .find_map(|p| props.get(p))
                .and_then(short_value);
            let mut s = match text {
                Some(t) => format!("{} {t}", d.label),
                None => d.label.to_string(),
            };
            if let Some((e, _)) = events.first() {
                s.push_str(&format!(" · {}", e.method()));
            }
            s
        }
        NodeKind::Container {
            def, props, shell, ..
        } => {
            let d = container_def(*def);
            if let (ContainerId::DialogCall, Some(PropValue::Expr(call))) = (def, props.get("call"))
            {
                // `Dialog crate::style::dialog(ctx, "about", None, 540.0`.
                let call = call.trim().trim_end_matches(',').trim();
                return format!("{} {}", d.label, one_line(call, 40));
            }
            if matches!(def, ContainerId::Flow) {
                let head = shell
                    .as_ref()
                    .and_then(|s| s.head.as_deref())
                    .unwrap_or_default();
                return format!(
                    "{} {}",
                    d.label,
                    one_line(head.trim().trim_end_matches('{').trim(), 40)
                );
            }
            let text = ["text", "title"]
                .iter()
                .find_map(|p| props.get(p))
                .and_then(short_value);
            match text {
                Some(t) => format!("{} {t}", d.label),
                None => d.label.to_string(),
            }
        }
        NodeKind::Custom { text } => format!("Custom code: {}", one_line(text.trim(), 40)),
    }
}

/// The first line of `s`, cut to `max` characters.
pub fn one_line(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or_default();
    if first.chars().count() > max || s.lines().nth(1).is_some() {
        format!("{}…", first.chars().take(max).collect::<String>())
    } else {
        first.to_string()
    }
}

fn short_value(v: &PropValue) -> Option<String> {
    match v {
        PropValue::Text(t) => Some(format!("\"{}\"", one_line(t, 30))),
        PropValue::Expr(e) => Some(one_line(e, 30)),
        _ => None,
    }
}

/// A property value as the property list shows it.
pub fn show_value(v: &PropValue) -> String {
    let hex = |c: &[u8; 4]| {
        if c[3] == 255 {
            format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3])
        }
    };
    match v {
        PropValue::Bool(b) => b.to_string(),
        PropValue::Int(i) => i.to_string(),
        PropValue::Float(f) => format!("{f:?}"),
        PropValue::Text(t) => format!("{t:?}"),
        PropValue::Enum(e) => (*e).to_string(),
        PropValue::Color { light, dark: None } => hex(light),
        PropValue::Color {
            light,
            dark: Some(d),
        } => format!("{} light, {} dark", hex(light), hex(d)),
        PropValue::Expr(e) => e.clone(),
        PropValue::Items(items) => items
            .iter()
            .map(|i| match &i.label {
                PropValue::Text(t) => format!("{t:?}"),
                other => show_value(other),
            })
            .collect::<Vec<_>>()
            .join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN: &str = r#"pub struct Login { name: String, remember: bool }
impl Login {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.name);
        });
        ui.checkbox(&mut self.remember, "Remember");
        if ui.button("Go").clicked() { self.go(); }
        for e in 0..3 { ui.label(format!("{e}")); }
    }
    fn go(&mut self) {}
}
"#;

    #[test]
    fn a_source_shows_its_forms_with_custom_blocks_locked() {
        let l = load(Path::new("/p/login.rs"), LOGIN, None);
        assert!(l.error.is_none());
        assert_eq!(l.forms.len(), 1);
        let f = &l.forms[0];
        assert_eq!(f.form().id(), "Login::ui");
        let mut titles = vec![];
        f.form().root.walk(&mut |n| titles.push(node_title(n)));
        assert_eq!(
            titles,
            [
                "Form",
                "Horizontal",
                "Label \"Name\"",
                "Text edit &mut self.name",
                "Checkbox \"Remember\"",
                "Button \"Go\" · clicked",
                "Custom code: for e in 0..3 { ui.label(format!(\"{e}\"))…",
            ]
        );
        let custom = f.form().root.children().last().unwrap();
        assert_eq!(f.custom_reason(custom.id), Some("for loop"));
    }

    #[test]
    fn a_syntax_error_is_reported_not_shown() {
        let l = load(
            Path::new("/p/login.rs"),
            "impl X { fn ui(&mut self, ui: &mut egui::Ui) { ui.label( } }",
            None,
        );
        assert!(
            l.error
                .as_deref()
                .is_some_and(|e| e.starts_with("login.rs doesn't parse at line 1")),
            "{:?}",
            l.error
        );
        assert!(l.forms.is_empty());
    }

    /// The form file wins for the forms it defines, and a broken form file blocks the designer.
    #[test]
    fn a_source_is_read_with_its_form_file() {
        // The copy: the whole source under its header, with an edit.
        let frm = format!(
            "// formust 1 | login.rs | Login::ui | src-hash 00000000\n{}",
            LOGIN.replace("\"Remember\"", "\"Remember me\"")
        );
        let l = load(
            Path::new("/p/login.rs"),
            LOGIN,
            Some((Path::new("/p/login.frm.rs"), &frm)),
        );
        assert!(l.error.is_none(), "{:?}", l.error);
        assert_eq!(l.forms.len(), 1);
        assert_eq!(l.forms[0].origin, Some(Origin::Frm));
        assert!(matches!(
            l.forms[0].conflict,
            Some(Conflict::Changed { .. })
        ));
        let mut titles = vec![];
        l.forms[0]
            .form()
            .root
            .walk(&mut |n| titles.push(node_title(n)));
        assert!(
            titles.contains(&"Checkbox \"Remember me\"".to_string()),
            "{titles:?}"
        );

        let broken = load(
            Path::new("/p/login.rs"),
            LOGIN,
            Some((Path::new("/p/login.frm.rs"), "impl Login { fn ui(")),
        );
        assert!(broken.forms.is_empty());
        assert!(
            broken
                .error
                .as_deref()
                .is_some_and(|e| e.contains("login.frm.rs has a syntax error")),
            "{:?}",
            broken.error
        );

        let own = load(
            Path::new("/p/login.frm.rs"),
            &frm,
            Some((Path::new("/p/login.rs"), LOGIN)),
        );
        assert_eq!(own.forms.len(), 1);
        assert_eq!(own.forms[0].origin, Some(Origin::Frm));
    }

    #[test]
    fn a_reload_keeps_the_form_and_a_selection_that_still_exists() {
        let mut d = Designer::default();
        let two = format!(
            "{LOGIN}\nstruct B;\nimpl B {{ fn ui(&mut self, ui: &mut egui::Ui) {{ ui.label(\"b\"); }} }}\n"
        );
        d.store(load(Path::new("/p/login.rs"), &two, None), 1);
        assert_eq!(d.form_id.as_deref(), Some("Login::ui"));
        d.choose_form("B::ui".into());
        d.select(Some(NodeId(1)), true);
        assert!(d.reveal_row);
        d.store(
            load(
                Path::new("/p/login.rs"),
                &two.replace("\"b\"", "\"c\""),
                None,
            ),
            2,
        );
        assert_eq!(d.form_id.as_deref(), Some("B::ui"));
        assert_eq!(d.selected, Some(NodeId(1)));
        // Another file starts afresh.
        d.store(load(Path::new("/p/other.rs"), LOGIN, None), 3);
        assert_eq!(d.form_id.as_deref(), Some("Login::ui"));
        assert_eq!(d.selected, None);
        assert!(d.had_forms.contains(Path::new("/p/login.rs")));
    }

    /// A new Rust form is a form the designer reads, with nothing locked, and it never overwrites.
    #[test]
    fn a_new_rust_form_is_a_designer_form() {
        let dir = crate::target_tmp().join("designer-new-form");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("form1.rs"), "keep me").unwrap();
        let path = new_form_file(&dir, true).unwrap();
        assert_eq!(path, dir.join("form2.rs"));
        assert_eq!(
            std::fs::read_to_string(dir.join("form1.rs")).unwrap(),
            "keep me"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("use eframe::egui;"));
        let l = load(&path, &text, None);
        assert_eq!(l.forms.len(), 1);
        assert_eq!(l.forms[0].form().id(), "Form2::ui");
        assert!(
            l.forms[0].imported.custom.is_empty(),
            "{:?}",
            l.forms[0].imported.custom
        );
        assert!(!new_form_text("Form1", false).contains("eframe"));
    }

    /// Edits survive undo with their ids, so the selection does; switching files parks the edits.
    #[test]
    fn undo_keeps_ids_and_switching_files_parks_edits() {
        let mut d = Designer::default();
        d.store(load(Path::new("/p/login.rs"), LOGIN, None), 1);
        let label = d.current().unwrap().form().root.children()[0].children()[0].id;
        d.select(Some(label), true);
        d.edit(Some("t"), |f| {
            f.set_prop(label, "text", PropValue::Text("N".into()))
        })
        .unwrap();
        d.edit(Some("t"), |f| {
            f.set_prop(label, "text", PropValue::Text("Na".into()))
        })
        .unwrap();
        assert!(d.is_dirty());
        assert_eq!(
            d.loaded.as_ref().unwrap().history.steps(),
            1,
            "typing in one field is one step"
        );
        assert!(d.delete_selected());
        assert!(d.current().unwrap().form().find(label).is_none());
        assert!(d.undo());
        assert!(
            d.current().unwrap().form().find(label).is_some(),
            "the same id is back"
        );
        d.select(Some(label), true);
        assert!(d.undo());
        assert_eq!(d.selected, Some(label), "the selection survives undo");
        assert!(!d.is_dirty());
        assert!(d.redo());
        assert!(d.is_dirty());
        // Another file: the edits are parked, then come back with the selection.
        d.store(load(Path::new("/p/other.rs"), LOGIN, None), 2);
        assert!(d.dirty_for(Path::new("/p/login.rs")) && !d.is_dirty());
        d.store(load(Path::new("/p/login.rs"), LOGIN, None), 1);
        assert!(d.is_dirty() && !d.loaded.as_ref().unwrap().stale);
        assert_eq!(d.selected, Some(label));
        // The file changes under the edits: they are kept, and it is marked stale.
        d.store(
            load(
                Path::new("/p/login.rs"),
                &LOGIN.replace("Name", "Nom"),
                None,
            ),
            7,
        );
        assert!(d.is_dirty() && d.loaded.as_ref().unwrap().stale);
        d.discard(Path::new("/p/login.rs"));
        assert!(!d.dirty_for(Path::new("/p/login.rs")));
    }

    #[test]
    fn copy_paste_duplicate_and_nudge() {
        let mut d = Designer::default();
        d.store(load(Path::new("/p/login.rs"), LOGIN, None), 1);
        let checkbox = d.current().unwrap().form().root.children()[1].id;
        d.select(Some(checkbox), true);
        let text = d.copy_selected().unwrap();
        assert_eq!(text, "ui.checkbox(&mut self.remember, \"Remember\");");
        assert!(d.paste(&text));
        let kids: Vec<String> = d
            .current()
            .unwrap()
            .form()
            .root
            .children()
            .iter()
            .map(node_title)
            .collect();
        assert_eq!(kids[1], kids[2], "pasted after the selection: {kids:?}");
        let copy = d.duplicate_selected().unwrap();
        assert_eq!(d.selected, Some(copy));
        assert!(d.nudge(-1));
        assert_eq!(d.current().unwrap().form().parent_of(copy).unwrap().1, 2);
        assert!(!d.paste("this is not rust ("));
        assert!(
            d.status
                .as_deref()
                .is_some_and(|s| s.starts_with("Can't paste"))
        );
        // Locked custom code can be copied, cut and moved, never opened.
        let custom = d
            .current()
            .unwrap()
            .form()
            .root
            .children()
            .last()
            .unwrap()
            .id;
        d.select(Some(custom), true);
        assert!(d.copy_selected().unwrap().starts_with("for e in 0..3"));
        assert!(d.cut_selected().is_some());
        assert!(d.current().unwrap().form().find(custom).is_none());
    }

    const ABOUT: &str = r#"pub struct About { name: String, open: bool }
impl About {
    pub fn show(&mut self, ctx: &egui::Context) {
        let mut open = self.open;
        egui::Window::new("About").open(&mut open).show(ctx, |ui| {
            ui.label("Hello");
            ui.text_edit_singleline(&mut self.name);
        });
        self.open = open;
    }
    pub fn tick(&self, ctx: &egui::Context) { ctx.request_repaint(); }
}
"#;

    /// A dialog (a method of any name taking `&Context` that shows a Window) is one form, and new
    /// widgets go into its Window: its root has the context, not a `Ui`.
    #[test]
    fn a_dialog_loads_as_one_form_and_takes_widgets_in_its_window() {
        let l = load(Path::new("/p/about.rs"), ABOUT, None);
        assert!(l.error.is_none(), "{:?}", l.error);
        assert_eq!(l.forms.len(), 1);
        assert_eq!(l.forms[0].form().id(), "About::show");
        let mut titles = vec![];
        l.forms[0]
            .form()
            .root
            .walk(&mut |n| titles.push(node_title(n)));
        assert_eq!(
            titles,
            [
                "Form",
                "Custom code: let mut open = self.open;",
                "Window \"About\"",
                "Label \"Hello\"",
                "Text edit &mut self.name",
                "Custom code: self.open = open;",
            ]
        );

        let mut d = Designer::default();
        d.store(l, 1);
        let form = d.current().unwrap().form().clone();
        let window = form.root.children()[1].id;
        let label = form.find(window).unwrap().children()[0].id;
        // Nothing selected, or the root: the palette adds into the Window.
        let added = d
            .add(edit::Item::Widget(formust_model::WidgetId::Button), None)
            .unwrap();
        assert_eq!(
            d.current().unwrap().form().parent_of(added),
            Some((window, 2))
        );
        d.select(Some(form.root.id), true);
        let added = d
            .add(edit::Item::Widget(formust_model::WidgetId::Separator), None)
            .unwrap();
        assert_eq!(
            d.current().unwrap().form().parent_of(added).unwrap().0,
            window
        );
        // A widget can't leave the Window for the root.
        assert!(!d.move_to(label, form.root.id, 0));
        assert_eq!(
            d.current().unwrap().form().parent_of(label),
            Some((window, 0))
        );
        // The exported form keeps the dialog's signature and the Window's chain.
        let text = formust_rust::emit_form_item(d.current().unwrap().form());
        assert!(
            text.contains("pub fn show(&mut self, ctx: &egui::Context)"),
            "{text}"
        );
        assert!(
            text.contains("egui::Window::new(\"About\").open(&mut open).show(ctx, |ui| {"),
            "{text}"
        );
        let again = formust_rust::import(&text).unwrap();
        assert_eq!(again.forms.len(), 1, "{text}");
        assert_eq!(again.forms[0].form.id(), "About::show");
    }

    /// A dialog shown through a helper call (`crate::style::dialog(ctx, .., |ui| ..)`) loads like a
    /// Window dialog: new widgets go into the call's closure, and the call is written verbatim.
    #[test]
    fn a_dialog_helper_call_takes_widgets_and_keeps_its_call() {
        const HELPER: &str = r#"pub struct About { name: String }
impl About {
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut done = false;
        crate::style::dialog(ctx, "about", None, 540.0, |ui| {
            ui.label("Hello");
        });
        done
    }
}
"#;
        let l = load(Path::new("/p/about.rs"), HELPER, None);
        assert!(l.error.is_none(), "{:?}", l.error);
        assert_eq!(l.forms.len(), 1);
        let mut titles = vec![];
        l.forms[0]
            .form()
            .root
            .walk(&mut |n| titles.push(node_title(n)));
        assert_eq!(
            titles,
            [
                "Form",
                "Custom code: let mut done = false;",
                "Dialog crate::style::dialog(ctx, \"about\", None,…",
                "Label \"Hello\"",
                "Custom code: done",
            ]
        );
        let mut d = Designer::default();
        d.store(l, 1);
        let form = d.current().unwrap().form().clone();
        let dialog = form.root.children()[1].id;
        let added = d
            .add(edit::Item::Widget(formust_model::WidgetId::Label), None)
            .unwrap();
        assert_eq!(
            d.current().unwrap().form().parent_of(added),
            Some((dialog, 1))
        );
        let label = form.find(dialog).unwrap().children()[0].id;
        assert!(!d.move_to(label, form.root.id, 0), "not onto the context");
        let text = formust_rust::emit_form_item(d.current().unwrap().form());
        assert!(
            text.contains("pub fn show(&mut self, ctx: &egui::Context) -> bool"),
            "{text}"
        );
        assert!(
            text.contains("crate::style::dialog(ctx, \"about\", None, 540.0, |ui| {"),
            "{text}"
        );
        assert!(text.contains("let mut done = false;") && text.contains("        done\n"));
        let again = formust_rust::import(&text).unwrap();
        assert_eq!(again.forms.len(), 1, "{text}");
        assert!(again.forms[0].form.context_root);
    }

    /// The app's own About window, a free function, loads with the designer's default options.
    #[test]
    fn the_about_window_of_the_app_loads_as_a_dialog() {
        let l = load(Path::new("/p/about.rs"), include_str!("../about.rs"), None);
        assert!(l.error.is_none(), "{:?}", l.error);
        assert_eq!(l.forms.len(), 1);
        let form = l.forms[0].form();
        assert_eq!(form.id(), "::show");
        assert!(form.context_root);
        let dialog = form.widget_root();
        assert_ne!(dialog, form.root.id, "widgets go into the dialog");
        assert!(node_title(form.find(dialog).unwrap()).starts_with("Dialog crate::style::dialog"));
        let mut d = Designer::default();
        d.store(l, 1);
        let added = d
            .add(edit::Item::Widget(formust_model::WidgetId::Label), None)
            .unwrap();
        assert_eq!(
            d.current().unwrap().form().parent_of(added).unwrap().0,
            dialog
        );
        let text = formust_rust::emit_form_item(d.current().unwrap().form());
        assert!(
            text.contains("crate::style::dialog(ctx, \"about\", None, 540.0, |ui| {"),
            "{text}"
        );
        assert!(
            text.contains("pub fn show(ctx: &egui::Context) -> bool {"),
            "{text}"
        );
    }

    #[test]
    fn values_are_shown_as_written() {
        assert_eq!(show_value(&PropValue::Text("Go".into())), "\"Go\"");
        assert_eq!(
            show_value(&PropValue::Expr("&mut self.name".into())),
            "&mut self.name"
        );
        assert_eq!(
            show_value(&PropValue::color_pair([255, 0, 0, 255], [0, 0, 0, 128])),
            "#FF0000 light, #00000080 dark"
        );
        assert_eq!(show_value(&PropValue::Float(2.0)), "2.0");
        assert_eq!(one_line("a\nb", 10), "a…");
    }
}
