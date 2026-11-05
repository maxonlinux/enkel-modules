//! Apply DOM events to the loaded mirror and implement tree, search and editing actions.
use super::*;

impl Elements {
    // Report whether the page overlay currently owns pointer inspection.
    pub fn inspecting(&self) -> bool {
        self.inspect_confirmed
    }

    // Enable or disable the page picker through the Overlay domain.
    pub fn inspect(&mut self) {
        // A second picker activation cancels the existing inspection gesture.
        if self.cancel_inspect() {
            return;
        }
        self.set_inspect_mode(true);
    }

    // Set inspect mode.
    pub(super) fn set_inspect_mode(&mut self, picking: bool) {
        self.picking = picking;
        self.inspect_revision += 1;
        // Chromium validates highlightConfig even for mode "none".
        self.command(
            "Overlay.setInspectMode",
            json!({"mode":if picking {"searchForNode"} else {"none"},"highlightConfig":highlight_config()}),
            Some(Reply::Inspect(self.inspect_revision, picking)),
        );
    }

    // Cancel inspect.
    pub fn cancel_inspect(&mut self) -> bool {
        // Do not queue another Overlay cancellation when picking is already inactive.
        if !self.picking {
            return false;
        }
        self.set_inspect_mode(false);
        self.highlight(None);
        true
    }

    // Resolve an overlay pick into the retained DOM selection.
    pub(super) fn inspected(&mut self, revision: u64, picking: bool, message: &Value) {
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if revision < self.inspect_confirmed_revision {
            return;
        }
        // Keep picker failures visible while retaining the last confirmed overlay state.
        if let Some(error) = message.get("error") {
            // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
            if revision != self.inspect_revision {
                return;
            }
            self.picking = self.inspect_confirmed;
            self.notice = Some(format!(
                "Element picker: {}",
                clipped(error["message"].as_str().unwrap_or("command failed"), 256)
            ));
            self.detail_revision += 1;
            return;
        }
        self.inspect_confirmed_revision = revision;
        self.inspect_confirmed = picking;
    }

    // Highlight a live selected node through the page overlay.
    pub fn highlight(&mut self, node: Option<i64>) {
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(id) = node else {
            self.command("Overlay.hideHighlight", json!({}), None);
            return;
        };
        self.command(
            "Overlay.highlightNode",
            json!({"nodeId":id,"highlightConfig":highlight_config()}),
            None,
        );
    }

    // Install a new DOM snapshot while invalidating stale node identities.
    pub(super) fn document(&mut self, value: &Value) {
        self.root = value["nodeId"].as_i64().unwrap_or_default();
        self.insert_node(value, 0);
        // Resolve the live tree identity before reading or mutating its retained state.
        if let Some(root) = self.nodes.get_mut(&self.root) {
            root.expanded = true;
        }
        self.structure = true;
        self.status = "Pick an element or expand the DOM tree".into();
    }

    // Retain a bounded DOM node and its known children in the tree cache.
    fn insert_node(&mut self, value: &Value, parent: i64) -> Option<i64> {
        let mut queue = vec![(value, parent, 0)];
        let mut first = None;
        while let Some((value, parent, depth)) = queue.pop() {
            // Resolve the live tree identity before reading or mutating its retained state.
            let Some(id) = value["nodeId"].as_i64().filter(|id| *id > 0) else {
                continue;
            };
            // Bound retained data and per-event work before accepting more content.
            if depth > 128 || (self.nodes.len() >= MAX_NODES && !self.nodes.contains_key(&id)) {
                self.status =
                    "DOM cache limit reached · reload Elements to release loaded branches".into();
                continue;
            }
            let name = value["localName"]
                .as_str()
                .filter(|name| !name.is_empty())
                .or(value["nodeName"].as_str())
                .unwrap_or("node");
            let mut node = Node {
                id,
                parent,
                kind: value["nodeType"].as_u64().unwrap_or_default(),
                name: clipped(name, 128),
                value: clipped(value["nodeValue"].as_str().unwrap_or_default(), MAX_TEXT),
                count: value["childNodeCount"].as_u64().unwrap_or_default() as usize,
                attributes: attributes(&value["attributes"]),
                truncated: name.chars().nth(128).is_some()
                    || text_truncated(&value["nodeValue"], MAX_TEXT)
                    || attributes_truncated(&value["attributes"]),
                ..Node::default()
            };
            let children = value["children"].as_array();
            node.loaded = children.is_some() || node.count == 0;
            // Resolve the live tree identity before reading or mutating its retained state.
            if let Some(old) = self.nodes.get(&id) {
                node.expanded = old.expanded;
            }
            let extras = value["shadowRoots"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(value.get("contentDocument"));
            let descendants = children
                .into_iter()
                .flatten()
                .chain(extras)
                .take(MAX_NODES - self.nodes.len())
                .collect::<Vec<_>>();
            node.children = descendants
                .iter()
                .filter_map(|n| n["nodeId"].as_i64())
                .collect();
            node.count = node.count.max(node.children.len());
            let previous = self.nodes.get(&id).map_or(0, Node::retained_bytes);
            let next = self.retained_bytes.saturating_sub(previous) + node.retained_bytes();
            // Bound retained data and per-event work before accepting more content.
            if next > MAX_DOM_BYTES {
                self.status = "DOM cache budget reached · reload Elements to reset".into();
                continue;
            }
            self.retained_bytes = next;
            first.get_or_insert(id);
            self.nodes.insert(id, node);
            queue.extend(
                descendants
                    .into_iter()
                    .rev()
                    .map(|child| (child, id, depth + 1)),
            );
        }
        first
    }

    // Merge DOM mutation events into the retained tree snapshot.
    pub(super) fn event(&mut self, method: &str, params: &Value) -> bool {
        let id = params["nodeId"].as_i64().unwrap_or_default();
        self.html_dirty |= self.section == Section::Html && method.starts_with("DOM.");
        match method {
            "DOM.documentUpdated" => self.refresh(),
            "CSS.styleSheetAdded" => self.sheet_added(&params["header"]),
            "CSS.styleSheetRemoved" => {
                self.sheets
                    .remove(params["styleSheetId"].as_str().unwrap_or(""));
            }
            "DOM.setChildNodes" => self.set_children(params),
            "DOM.childNodeInserted" => self.inserted(params),
            "DOM.childNodeRemoved" => self.removed(params),
            "DOM.attributeModified" | "DOM.attributeRemoved" => {
                self.attribute_event(method, id, params)
            }
            "DOM.characterDataModified" => {
                // Resolve the live tree identity before reading or mutating its retained state.
                if let Some(node) = self.nodes.get_mut(&id) {
                    let old_bytes = node.retained_bytes();
                    node.truncated = params["characterData"]
                        .as_str()
                        .is_some_and(|text| text.chars().nth(MAX_TEXT).is_some());
                    node.value = clipped(
                        params["characterData"].as_str().unwrap_or_default(),
                        MAX_TEXT,
                    );
                    self.retained_bytes =
                        self.retained_bytes.saturating_sub(old_bytes) + node.retained_bytes();
                    self.changed.insert(id);
                }
                // Bound retained data and per-event work before accepting more content.
                if self.retained_bytes > MAX_DOM_BYTES {
                    self.refresh();
                }
            }
            "DOM.childNodeCountUpdated" => {
                // Resolve the live tree identity before reading or mutating its retained state.
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.count = params["childNodeCount"].as_u64().unwrap_or_default() as usize;
                    self.changed.insert(id);
                }
            }
            "DOM.shadowRootPushed" => {
                let parent = params["hostId"].as_i64().unwrap_or_default();
                // Resolve the live tree identity before reading or mutating its retained state.
                if let Some(id) = self.insert_node(&params["root"], parent)
                    && let Some(node) = self.nodes.get_mut(&parent)
                {
                    node.children.push(id);
                }
                self.structure = true;
            }
            "DOM.shadowRootPopped" => {
                self.remove_subtree(params["rootId"].as_i64().unwrap_or_default())
            }
            "DOM.inlineStyleInvalidated"
            | "CSS.styleSheetChanged"
            | "CSS.mediaQueryResultChanged" => self.styles_dirty = true,
            "CSS.computedStyleUpdated" => self.computed_dirty = true,
            "Overlay.inspectModeCanceled" => {
                self.inspect_confirmed = false;
                self.cancel_inspect();
            }
            "Overlay.inspectNodeRequested" => {
                self.cancel_inspect();
                self.command(
                    "DOM.pushNodesByBackendIdsToFrontend",
                    json!({"backendNodeIds":[params["backendNodeId"]]}),
                    Some(Reply::Reveal(self.epoch)),
                );
            }
            _ => return false,
        }
        true
    }

    // Set children.
    fn set_children(&mut self, params: &Value) {
        let parent = params["parentId"].as_i64().unwrap_or_default();
        self.requested_children.remove(&parent);
        // Resolve the live tree identity before reading or mutating its retained state.
        if !self.nodes.contains_key(&parent) {
            return;
        }
        let mut children = vec![];
        for value in params["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX_NODES)
        {
            // Resolve the live tree identity before reading or mutating its retained state.
            if let Some(id) = self.insert_node(value, parent) {
                children.push(id);
            }
        }
        // Shadow/frame roots are delivered separately from regular children.
        if let Some(node) = self.nodes.get(&parent) {
            children.extend(node.children.iter().copied().filter(|id| {
                self.nodes
                    .get(id)
                    .is_some_and(|n| n.kind == 9 || n.kind == 11)
            }));
        }
        // Resolve the live tree identity before reading or mutating its retained state.
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children = children;
            node.loaded = true;
        }
        self.changed.insert(parent);
        self.structure |= self.nodes.get(&parent).is_some_and(|node| node.expanded);
    }

    // Merge a DOM insertion event into the retained tree and dirty-row set.
    fn inserted(&mut self, params: &Value) {
        let parent = params["parentNodeId"].as_i64().unwrap_or_default();
        // Resolve the live tree identity before reading or mutating its retained state.
        if !self.nodes.get(&parent).is_some_and(|node| node.loaded) {
            return;
        }
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(id) = self.insert_node(&params["node"], parent) else {
            return;
        };
        // Resolve the live tree identity before reading or mutating its retained state.
        if let Some(node) = self.nodes.get_mut(&parent) {
            let previous = params["previousNodeId"].as_i64().unwrap_or_default();
            let index = node
                .children
                .iter()
                .position(|id| *id == previous)
                .map_or(0, |index| index + 1);
            node.children.insert(index, id);
            node.count = node.children.len();
        }
        self.changed.insert(id);
        self.changed.insert(parent);
        self.structure |= self.nodes.get(&parent).is_some_and(|node| node.expanded);
    }

    // Remove a DOM subtree and its stale selection-dependent details.
    fn removed(&mut self, params: &Value) {
        let parent = params["parentNodeId"].as_i64().unwrap_or_default();
        let id = params["nodeId"].as_i64().unwrap_or_default();
        self.remove_subtree(id);
        // Resolve the live tree identity before reading or mutating its retained state.
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.retain(|child| *child != id);
            node.count = node.children.len();
        }
        self.changed.insert(parent);
        self.structure |= self.nodes.get(&parent).is_some_and(|node| node.expanded);
    }

    // Remove subtree.
    fn remove_subtree(&mut self, id: i64) {
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            // Resolve the live tree identity before reading or mutating its retained state.
            if let Some(node) = self.nodes.remove(&id) {
                self.retained_bytes = self.retained_bytes.saturating_sub(node.retained_bytes());
                stack.extend(node.children);
            }
            self.changed.remove(&id);
            self.requested_children.remove(&id);
            // Resolve the stable identity before using a row index or applying a retained reply.
            if self.selected == Some(id) {
                self.selected = None;
                self.clear_details();
                self.command("Overlay.hideHighlight", json!({}), None);
                self.command("CSS.trackComputedStyleUpdatesForNode", json!({}), None);
            }
        }
    }

    // Merge an attribute mutation into the retained node caption.
    fn attribute_event(&mut self, method: &str, id: i64, params: &Value) {
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        let old_bytes = node.retained_bytes();
        node.truncated |=
            text_truncated(&params["value"], MAX_TEXT) || text_truncated(&params["name"], 128);
        let name = clipped(params["name"].as_str().unwrap_or_default(), 128);
        node.attributes.retain(|(key, _)| *key != name);
        // A new attribute beyond the cache limit makes this mirror incomplete and unsafe as an edit source.
        node.truncated |= method == "DOM.attributeModified" && node.attributes.len() >= 64;
        // Keep at most the same 64 attributes admitted by the initial DOM snapshot.
        if method == "DOM.attributeModified" && node.attributes.len() < 64 {
            node.attributes.push((
                name,
                clipped(params["value"].as_str().unwrap_or_default(), MAX_TEXT),
            ));
        }
        self.retained_bytes = self.retained_bytes.saturating_sub(old_bytes) + node.retained_bytes();
        // Bound retained data and per-event work before accepting more content.
        if self.retained_bytes > MAX_DOM_BYTES {
            self.refresh();
            return;
        }
        self.changed.insert(id);
        self.styles_dirty |= self.selected == Some(id);
        // Resolve the stable identity before using a row index or applying a retained reply.
        if self.selected == Some(id) {
            self.detail_revision += 1;
        }
    }

    // Expand a node, requesting unknown children only when needed.
    pub fn toggle(&mut self, id: i64) {
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        node.expanded = !node.expanded;
        let request = node.expanded && !node.loaded && self.requested_children.insert(id);
        self.structure = true;
        // Request children only when expansion reveals a node whose children are not loaded.
        if request {
            self.command(
                "DOM.requestChildNodes",
                json!({"nodeId":id,"depth":1,"pierce":true}),
                None,
            );
        }
    }

    // Flatten expanded DOM nodes into stable presentation rows.
    pub fn rows(&self) -> Vec<DomRow> {
        let mut rows = Vec::with_capacity(self.nodes.len().min(256));
        let mut stack = vec![(self.root, 0)];
        let mut seen = HashSet::new();
        while let Some((id, depth)) = stack.pop() {
            // Resolve the stable identity before using a row index or applying a retained reply.
            if !seen.insert(id) || depth > 128 {
                continue;
            }
            // Resolve the live tree identity before reading or mutating its retained state.
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            rows.push(DomRow {
                id,
                depth,
                expandable: node.count > 0,
                expanded: node.expanded,
                label: node.label(),
            });
            // Resolve the live tree identity before reading or mutating its retained state.
            if node.expanded {
                stack.extend(node.children.iter().rev().map(|id| (*id, depth + 1)));
            }
        }
        rows
    }

    // Describe the selected node's retained ancestry for navigation.
    pub fn breadcrumbs(&self) -> Vec<(i64, String)> {
        let mut result = vec![];
        let mut id = self.selected.unwrap_or_default();
        for _ in 0..32 {
            // Resolve the live tree identity before reading or mutating its retained state.
            let Some(node) = self.nodes.get(&id) else {
                break;
            };
            result.push((id, node.caption()));
            id = node.parent;
        }
        result.reverse();
        result
    }

    // Release protocol search handles before replacing their result set.
    pub(super) fn discard_search(&mut self) {
        // Use only the current search state before selecting or requesting another result.
        if let Some(id) = self.search_id.take() {
            self.command("DOM.discardSearchResults", json!({"searchId":id}), None);
        }
        self.search_count = 0;
        self.search_index = 0;
        self.search_generation += 1;
    }

    // Start a new bounded DOM search for the current query.
    pub fn search(&mut self, query: &str) {
        // Do not start work after teardown or before this owner is ready.
        if !self.active || query == self.search_query {
            return;
        }
        // Avoid duplicate in-flight work and wait for the reply that already owns this state.
        if self.pending.len() > 32 {
            self.status = "Waiting for pending inspector replies".into();
            return;
        }
        self.discard_search();
        self.search_query = clipped(query.trim(), 512);
        // Use only the current search state before selecting or requesting another result.
        if self.search_query.is_empty() {
            return;
        }
        self.command(
            "DOM.performSearch",
            json!({"query":self.search_query,"includeUserAgentShadowDOM":false}),
            Some(Reply::Search(self.search_generation)),
        );
    }

    // Retain the current query's result handle only if its generation still matches.
    pub(super) fn searched(&mut self, result: &Value) {
        self.search_id = result["searchId"].as_str().map(str::to_owned);
        self.search_count = result["resultCount"].as_u64().unwrap_or_default() as usize;
        self.search_index = 0;
        self.search_result();
    }

    // Move within the current search results and request the matching node.
    pub fn next_match(&mut self, previous: bool) {
        // Use only the current search state before selecting or requesting another result.
        if self.search_count == 0 {
            return;
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        self.search_index = if previous {
            (self.search_index + self.search_count - 1) % self.search_count
        } else {
            (self.search_index + 1) % self.search_count
        };
        self.search_result();
    }

    // Reveal a search result only while its query and document remain current.
    fn search_result(&mut self) {
        // Use only the current search state before selecting or requesting another result.
        let Some(id) = self.search_id.as_ref().filter(|_| self.search_count > 0) else {
            return;
        };
        self.command(
            "DOM.getSearchResults",
            json!({"searchId":id,"fromIndex":self.search_index,"toIndex":self.search_index+1}),
            Some(Reply::SearchResult(self.search_generation)),
        );
    }

    // Dispatch a declared DOM mutation using the current node identity.
    pub fn action(&mut self, action: &str) {
        // Undo and redo belong to the document and do not require a selected node.
        if matches!(action, "DOM.undo" | "DOM.redo") {
            self.command(action, json!({}), None);
            return;
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        let Some(id) = self.selected else {
            return;
        };
        let reply = (action == "DOM.removeNode").then_some(Reply::Edit(self.epoch));
        self.command(action, json!({"nodeId":id}), reply);
    }

    // Request a sibling clone while preserving the original node identity.
    pub fn duplicate(&mut self) {
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(node) = self.selected.and_then(|id| self.nodes.get(&id)) else {
            return;
        };
        let params = json!({"nodeId":node.id,"targetNodeId":node.parent});
        self.command("DOM.copyTo", params, Some(Reply::Edit(self.epoch)));
    }
}

// Apply explicit attribute source through Chromium's DOM parser.
fn attributes(value: &Value) -> Vec<(String, String)> {
    // Reject a missing or mismatched value before the following operation relies on it.
    let Some(values) = value.as_array() else {
        return vec![];
    };
    values
        .as_chunks::<2>()
        .0
        .iter()
        .take(64)
        .map(|pair| {
            (
                clipped(pair[0].as_str().unwrap_or_default(), 128),
                clipped(pair[1].as_str().unwrap_or_default(), MAX_TEXT),
            )
        })
        .collect()
}

// Detect oversized protocol strings after at most the retained character limit plus one.
fn text_truncated(value: &Value, limit: usize) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.chars().nth(limit).is_some())
}

// Names and values have different retention limits; either kind of clipping makes the tag read-only.
fn attributes_truncated(value: &Value) -> bool {
    value.as_array().is_some_and(|values| {
        values.len() > 128
            || values
                .as_chunks::<2>()
                .0
                .iter()
                .any(|[name, value]| text_truncated(name, 128) || text_truncated(value, MAX_TEXT))
    })
}
