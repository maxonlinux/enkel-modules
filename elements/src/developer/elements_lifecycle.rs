//! Initialize inspector state and release DOM/CSS subscriptions at lifecycle boundaries.
use super::*;

impl Default for Elements {
    // Start with empty collections and no active request or selection.
    fn default() -> Self {
        Self {
            active: false,
            picking: false,
            inspect_revision: 0,
            inspect_confirmed_revision: 0,
            inspect_confirmed: false,
            status: "Select an element to inspect".into(),
            notice: None,
            nodes: HashMap::new(),
            root: 0,
            selected: None,
            rules: vec![],
            sheets: HashMap::new(),
            computed: vec![],
            box_model: None,
            html: String::new(),
            structure: true,
            changed: HashSet::new(),
            detail_revision: 0,
            styles_revision: 0,
            search_count: 0,
            search_index: 0,
            section: Section::Styles,
            clipboard: None,
            epoch: protocol_id() as u64,
            selection_generation: 0,
            retained_bytes: 0,
            html_truncated: false,
            html_dirty: false,
            pending: HashMap::new(),
            commands: vec![],
            search_id: None,
            search_query: String::new(),
            search_generation: 0,
            styles_dirty: false,
            computed_dirty: false,
            layout_pending: false,
            styles_pending: false,
            requested_children: HashSet::new(),
            forced: None,
            inspect_point: None,
        }
    }
}

impl Elements {
    // Enable or release protocol capture when this tool's active state changes.
    pub fn set_active(&mut self, active: bool) {
        // An unchanged active state needs no protocol transition.
        if self.active == active {
            return;
        }
        // Deactivation must cancel picking and release selection-specific work.
        if !active {
            self.stop();
            return;
        }
        self.active = true;
        self.status = "Loading DOM…".into();
        self.command("DOM.enable", json!({}), Some(Reply::Enable));
    }

    // Disable page picking and discard selection-specific in-flight work.
    fn stop(&mut self) {
        self.clear_forced();
        self.discard_search();
        self.set_inspect_mode(false);
        for method in [
            "Overlay.hideHighlight",
            "Overlay.disable",
            "CSS.disable",
            "DOM.disable",
        ] {
            self.command(method, json!({}), None);
        }
        let commands = self.drain();
        let pending = self.retired_searches();
        *self = Self {
            pending,
            commands,
            ..Self::default()
        };
    }

    // Reload the DOM snapshot for the active document generation.
    pub fn refresh(&mut self) {
        // Do not start work after teardown or before this owner is ready.
        if !self.active {
            return;
        }
        self.clear_forced();
        self.discard_search();
        self.search_query.clear();
        self.epoch = protocol_id() as u64;
        self.pending = self.retired_searches();
        self.nodes.clear();
        self.retained_bytes = 0;
        self.requested_children.clear();
        self.changed.clear();
        self.root = 0;
        self.selected = None;
        self.clear_details();
        self.structure = true;
        self.command(
            "DOM.getDocument",
            json!({"depth":2,"pierce":true}),
            Some(Reply::Document(self.epoch)),
        );
    }
}
