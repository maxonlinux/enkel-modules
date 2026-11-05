//! Own the bounded DOM mirror and route on-demand inspector commands and replies.
use super::protocol_id;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[path = "elements_dom.rs"]
mod dom;
#[path = "elements_edit.rs"]
mod edit;
#[path = "elements_lifecycle.rs"]
mod lifecycle;
#[path = "elements_styles.rs"]
mod styles;
#[path = "elements_tag.rs"]
mod tag;
pub use edit::NodeEdit;

pub use edit::NodeTarget;
pub use styles::{BoxModel, StyleRule};
const MAX_NODES: usize = 5000;
const MAX_TEXT: usize = 4096;
const MAX_DOM_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct Node {
    pub id: i64,
    pub parent: i64,
    pub kind: u64,
    pub name: String,
    pub value: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<i64>,
    pub count: usize,
    pub loaded: bool,
    pub expanded: bool,
    pub truncated: bool,
}
impl Node {
    // Account for retained text and metadata before accepting more DOM data.
    fn retained_bytes(&self) -> usize {
        256 + self.name.capacity()
            + self.value.capacity()
            + self.children.capacity() * 8
            + self
                .attributes
                .iter()
                .map(|(k, v)| 48 + k.capacity() + v.capacity())
                .sum::<usize>()
    }

    // Build the short presentation label independently of native action behavior.
    fn caption(&self) -> String {
        let mut title = self.name.clone();
        for (key, value) in &self.attributes {
            // Resolve the stable identity before using a row index or applying a retained reply.
            if key == "id" {
                title.push_str(&format!("#{}", clipped(value, 40)));
                continue;
            }
            // Resolve the stable identity before using a row index or applying a retained reply.
            if key != "class" {
                continue;
            }
            for name in value.split_whitespace().take(2) {
                title.push('.');
                title.push_str(&clipped(name, 30));
            }
        }
        title
    }

    // Create the presentation label used by this owner.
    pub fn label(&self) -> String {
        match self.kind {
            9 => return "#document".into(),
            10 => return format!("<!DOCTYPE {}>", self.name),
            11 => return "#shadow-root".into(),
            _ => {}
        }
        // Text and comment nodes use a bounded content preview instead of an opening tag.
        if self.kind == 3 || self.kind == 8 {
            return clipped(&format!("\"{}\"", self.value.trim()), 512);
        }
        tag::source(&self.name, &self.attributes, true)
    }
}
#[derive(Clone)]
pub struct DomRow {
    pub id: i64,
    pub depth: usize,
    pub expandable: bool,
    pub expanded: bool,
    pub label: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Styles,
    Computed,
    Attributes,
    Html,
}
pub enum Action {
    Select(i64),
    Toggle(i64),
    Pick,
    Refresh,
    Search(String),
    Next(bool),
    Section(Section),
    Markup(String),
    Rule(usize, u64, String),
    Pseudo(String),
    Delete,
    Duplicate,
    Undo,
    Redo,
    Scroll,
    Focus,
    Hide,
    CopyHtml,
    EditNode(NodeEdit, String),
}
#[derive(Clone, Copy)]
enum Reply {
    Enable,
    Inspect(u64, bool),
    DiscardSearch,
    Location(u64),
    Document(u64),
    Reveal(u64),
    Search(u64),
    SearchResult(u64),
    Styles(u64, i64),
    Computed(u64, i64),
    Box(u64, i64),
    Html(u64, i64),
    CopyHtml(u64, i64),
    Edit(u64),
    Rename(u64, i64),
}
pub struct Elements {
    pub active: bool,
    pub picking: bool,
    inspect_revision: u64,
    inspect_confirmed_revision: u64,
    inspect_confirmed: bool,
    pub status: String,
    pub notice: Option<String>,
    pub nodes: HashMap<i64, Node>,
    pub root: i64,
    pub selected: Option<i64>,
    pub rules: Vec<Rc<StyleRule>>,
    sheets: HashMap<String, String>,
    pub computed: Vec<(String, String)>,
    pub box_model: Option<BoxModel>,
    pub html: String,
    pub structure: bool,
    pub changed: HashSet<i64>,
    pub detail_revision: u64,
    pub styles_revision: u64,
    pub search_count: usize,
    pub search_index: usize,
    pub section: Section,
    pub clipboard: Option<String>,
    epoch: u64,
    selection_generation: u64,
    retained_bytes: usize,
    html_truncated: bool,
    html_dirty: bool,
    pending: HashMap<i32, Reply>,
    commands: Vec<Value>,
    search_id: Option<String>,
    search_query: String,
    search_generation: u64,
    styles_dirty: bool,
    computed_dirty: bool,
    layout_pending: bool,
    styles_pending: bool,
    requested_children: HashSet<i64>,
    forced: Option<i64>,
    inspect_point: Option<(i32, i32)>,
}
impl Elements {
    // Queue a protocol command with the reply metadata needed to validate it.
    fn command(&mut self, method: &str, params: Value, reply: Option<Reply>) {
        let id = protocol_id();
        // Apply the optional result only while its owning state is available.
        if let Some(reply) = reply {
            self.pending.insert(id, reply);
        }
        self.commands
            .push(json!({"id":id,"method":method,"params":params}));
    }

    // Transfer queued commands to the caller without keeping a second copy.
    pub fn drain(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.commands)
    }

    // Route incoming protocol data to its owner and ignore replies from stale work.
    pub fn receive(&mut self, message: &Value) -> bool {
        // Route only the protocol event or reply owned by this handler.
        if let Some(id) = message["id"].as_i64().and_then(|id| i32::try_from(id).ok()) {
            // Avoid duplicate in-flight work and wait for the reply that already owns this state.
            let Some(reply) = self.pending.remove(&id) else {
                return false;
            };
            // Use only the current search state before selecting or requesting another result.
            if matches!(reply, Reply::DiscardSearch) {
                self.discard_result(message);
                return true;
            }
            // Do not start work after teardown or before this owner is ready.
            if !self.active {
                return false;
            }
            // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
            if let Reply::Inspect(revision, picking) = reply {
                self.inspected(revision, picking, message);
                return true;
            }
            self.replied(reply, message);
            return true;
        }
        // Do not start work after teardown or before this owner is ready.
        if !self.active {
            return false;
        }
        self.event(
            message["method"].as_str().unwrap_or_default(),
            &message["params"],
        )
    }

    // Release search handles belonging to work that is no longer current.
    fn discard_result(&mut self, message: &Value) {
        // Route only the protocol event or reply owned by this handler.
        if let Some(id) = message["result"]["searchId"].as_str() {
            self.command("DOM.discardSearchResults", json!({"searchId":id}), None);
        }
    }

    // Apply the protocol result only to the state that originally requested it.
    fn replied(&mut self, reply: Reply, message: &Value) {
        // Reject work from an earlier document or selection; protocol IDs may be reused.
        if let Reply::Search(generation) = reply
            && (generation != self.search_generation || message.get("error").is_some())
        {
            // Route only the protocol event or reply owned by this handler.
            if let Some(id) = message["result"]["searchId"].as_str() {
                self.command("DOM.discardSearchResults", json!({"searchId":id}), None);
            }
            return;
        }
        // Resolve the current target before applying work that could otherwise affect a stale owner.
        if !self.current(reply) {
            return;
        }
        // Handle failure before exposing a partial result as usable state.
        if let Some(error) = message.get("error") {
            // Elements without a layout box (head, display:none) have no diagram.
            if matches!(reply, Reply::Box(_, _)) {
                self.layout_pending = false;
                self.box_model = None;
                self.detail_revision += 1;
                return;
            }
            self.status = clipped(
                error["message"]
                    .as_str()
                    .unwrap_or("Inspector command failed"),
                256,
            );
            self.notice = Some(self.status.clone());
            self.styles_pending = false;
            self.layout_pending = false;
            self.detail_revision += 1;
            return;
        }
        let result = &message["result"];
        match reply {
            Reply::DiscardSearch | Reply::Inspect(..) => {}
            Reply::Enable => {
                self.command("CSS.enable", json!({}), None);
                self.command("Overlay.enable", json!({}), None);
                self.refresh();
            }
            Reply::Document(_) => {
                self.document(&result["root"]);
                // Apply the optional result only while its owning state is available.
                if let Some((x, y)) = self.inspect_point.take() {
                    self.inspect_at(x, y);
                }
            }
            Reply::Location(_) => {
                // Resolve the live tree identity before reading or mutating its retained state.
                if let Some(id) = result["nodeId"].as_i64().filter(|id| *id > 0) {
                    self.reveal(id);
                } else {
                    self.command(
                        "DOM.pushNodesByBackendIdsToFrontend",
                        json!({"backendNodeIds":[result["backendNodeId"]]}),
                        Some(Reply::Reveal(self.epoch)),
                    );
                }
            }
            Reply::Reveal(_) | Reply::SearchResult(_) => {
                // Resolve the live tree identity before reading or mutating its retained state.
                if let Some(id) = result["nodeIds"]
                    .as_array()
                    .and_then(|ids| ids.first())
                    .and_then(Value::as_i64)
                {
                    self.reveal(id);
                }
            }
            Reply::Search(_) => self.searched(result),
            Reply::Styles(_, _) => self.set_styles(result),
            Reply::Computed(_, _) => self.set_computed(result),
            Reply::Box(_, _) => self.set_box(&result["model"]),
            Reply::Html(_, _) => {
                self.html_truncated = result["outerHTML"]
                    .as_str()
                    .is_some_and(|s| s.len() > 65536);
                self.html = clipped(result["outerHTML"].as_str().unwrap_or_default(), 65536);
                // A shortened preview is not a complete edit source and must not overwrite the page.
                if self.html_truncated {
                    self.status =
                        "Markup preview exceeds 64 KiB · choose a smaller element to edit".into();
                }
                self.detail_revision += 1;
            }
            Reply::CopyHtml(_, _) => {
                self.clipboard = Some(clipped(
                    result["outerHTML"].as_str().unwrap_or_default(),
                    65536,
                ))
            }
            Reply::Edit(_) => {
                self.styles_dirty = true;
                self.command("DOM.markUndoableState", json!({}), None);
            }
            Reply::Rename(_, previous) => {
                self.command("DOM.markUndoableState", json!({}), None);
                // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
                if (self.selected == Some(previous) || self.selected.is_none())
                    && let Some(id) = result["nodeId"].as_i64()
                {
                    self.reveal(id);
                }
            }
        }
    }

    // Check the generation and selection before using an asynchronous reply.
    fn current(&self, reply: Reply) -> bool {
        match reply {
            Reply::Enable => true,
            Reply::DiscardSearch | Reply::Inspect(..) => false,
            Reply::Search(g) | Reply::SearchResult(g) => g == self.search_generation,
            Reply::Styles(e, id)
            | Reply::Computed(e, id)
            | Reply::Box(e, id)
            | Reply::Html(e, id)
            | Reply::CopyHtml(e, id) => e == self.selection_generation && self.selected == Some(id),
            Reply::Document(e)
            | Reply::Reveal(e)
            | Reply::Location(e)
            | Reply::Edit(e)
            | Reply::Rename(e, _) => e == self.epoch,
        }
    }

    // Change the selected item and invalidate only its dependent detail state.
    pub fn select(&mut self, id: i64) {
        // Resolve the live tree identity before reading or mutating its retained state.
        if self.selected == Some(id) || !self.nodes.contains_key(&id) {
            return;
        }
        self.clear_forced();
        self.selected = Some(id);
        self.clear_details();
        self.styles_dirty = true;
        self.status = "Selected · edits affect this page only".into();
        self.command("DOM.setInspectedNode", json!({"nodeId":id}), None);
        self.highlight(Some(id));
        self.command(
            "CSS.trackComputedStyleUpdatesForNode",
            json!({"nodeId":id}),
            None,
        );
    }

    // Select the inspected node after resolving its ancestors into the retained tree.
    fn reveal(&mut self, id: i64) {
        let mut current = self.nodes.get(&id).map_or(0, |node| node.parent);
        for _ in 0..128 {
            // Resolve the current target before applying work that could otherwise affect a stale owner.
            let Some(node) = self.nodes.get_mut(&current) else {
                break;
            };
            node.expanded = true;
            current = node.parent;
            // Resolve the current target before applying work that could otherwise affect a stale owner.
            if current == 0 {
                break;
            }
        }
        self.structure = true;
        self.select(id);
    }

    // Inspect at.
    pub fn inspect_at(&mut self, x: i32, y: i32) {
        // Do not start work after teardown or before this owner is ready.
        if !self.active || self.root == 0 {
            self.inspect_point = Some((x, y));
            return;
        }
        self.command(
            "DOM.getNodeForLocation",
            json!({"x":x,"y":y,"includeUserAgentShadowDOM":false}),
            Some(Reply::Location(self.epoch)),
        );
    }

    // Invalidate selection-specific data without retaining stale editor content.
    fn clear_details(&mut self) {
        self.selection_generation += 1;
        self.html_truncated = false;
        self.html_dirty = false;
        self.rules.clear();
        self.computed.clear();
        self.box_model = None;
        self.html.clear();
        self.styles_pending = false;
        self.layout_pending = false;
        self.styles_revision += 1;
        self.detail_revision += 1;
    }

    // Switch detail source and request only data needed by the new section.
    pub fn section(&mut self, section: Section) {
        // An unchanged detail section needs no new source request.
        if self.section == section {
            return;
        }
        self.section = section;
        self.computed_dirty |= section == Section::Computed;
        // Release the large full-computed snapshot when leaving the Computed section.
        if section != Section::Computed {
            self.computed = vec![];
        }
        self.html_dirty |= section == Section::Html;
        self.detail_revision += 1;
    }

    // Fetch markup only when an explicit HTML view or copy action needs it.
    fn load_html(&mut self, id: i64) {
        self.command(
            "DOM.getOuterHTML",
            json!({"nodeId":id}),
            Some(Reply::Html(self.selection_generation, id)),
        );
    }

    // Keep only handles that still need protocol cleanup after a document change.
    fn retired_searches(&self) -> HashMap<i32, Reply> {
        self.pending
            .iter()
            .filter(|(_, r)| matches!(r, Reply::Search(_) | Reply::DiscardSearch))
            .map(|(id, _)| (*id, Reply::DiscardSearch))
            .collect()
    }

    // Allow edits only while the complete selected markup is available.
    pub fn markup_editable(&self) -> bool {
        !self.html_truncated
            && self
                .selected
                .and_then(|id| self.nodes.get(&id))
                .is_some_and(|node| !node.truncated)
    }

    // Dispatch a declared inspector action to the state that owns it.
    pub fn perform(&mut self, action: Action) {
        self.notice = None;
        // Do not start work after teardown or before this owner is ready.
        if !self.active {
            return;
        }
        match action {
            Action::Select(id) => self.select(id),
            Action::Toggle(id) => self.toggle(id),
            Action::Pick => self.inspect(),
            Action::Refresh => self.refresh(),
            Action::Search(query) => self.search(&query),
            Action::Next(previous) => self.next_match(previous),
            Action::Section(section) => self.section(section),
            Action::Markup(source) => self.edit_markup(&source),
            Action::EditNode(edit, source) => self.apply_node_edit(edit, &source),
            Action::Rule(index, revision, code) => self.edit_rule(index, revision, &code),
            Action::Pseudo(state) => self.force_state(&state),
            Action::Delete => self.action("DOM.removeNode"),
            Action::Duplicate => self.duplicate(),
            Action::Undo => self.action("DOM.undo"),
            Action::Redo => self.action("DOM.redo"),
            Action::Scroll => self.action("DOM.scrollIntoViewIfNeeded"),
            Action::Focus => self.action("DOM.focus"),
            Action::Hide => self.add_property("visibility", "hidden"),
            Action::CopyHtml => {
                // Resolve the stable identity before using a row index or applying a retained reply.
                if let Some(id) = self.selected {
                    self.command(
                        "DOM.getOuterHTML",
                        json!({"nodeId":id}),
                        Some(Reply::CopyHtml(self.selection_generation, id)),
                    );
                }
            }
        }
    }
}

// Bound retained text without splitting a Unicode code point.
fn clipped(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

// Declare overlay appearance once for all inspector highlight commands.
fn highlight_config() -> Value {
    json!({"showInfo":false,"showStyles":false,"contentColor":{"r":80,"g":150,"b":240,"a":0.18},"paddingColor":{"r":110,"g":190,"b":100,"a":0.2},"borderColor":{"r":240,"g":200,"b":90,"a":0.3},"marginColor":{"r":240,"g":150,"b":90,"a":0.25}})
}

#[cfg(test)]
#[path = "../../tests/elements.rs"]
mod tests;
