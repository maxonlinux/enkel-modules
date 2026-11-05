//! Lazy remote-object trees. Property getters are displayed, never invoked.
use super::{MAX_ROW_CHARS, append_text, protocol_id};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write};

const MAX_NODES: usize = 3000;
const MAX_PROPERTIES: usize = 200;
const MAX_DEPTH: usize = 8;
pub struct ObjectView {
    pub text: String,
    pub links: Vec<ObjectLink>,
    utf16: usize,
    remaining: usize,
    truncated: bool,
}
impl Default for ObjectView {
    // Start a Console snapshot with a fresh budget for its first message.
    fn default() -> Self {
        Self::with_text(String::new())
    }
}
impl ObjectView {
    // Append only the part that fits, counting native link offsets in the same Unicode pass.
    pub fn push(&mut self, value: &str) {
        let mut end = 0;
        for character in value.chars().take(self.remaining) {
            end += character.len_utf8();
            self.utf16 += character.len_utf16();
            self.remaining -= 1;
        }
        self.text.push_str(&value[..end]);
        self.truncated |= end < value.len();
    }

    // Reuse an already bounded row allocation when freezing remote objects after a context change.
    pub fn with_text(text: String) -> Self {
        let (characters, utf16) = text.chars().fold((0, 0), |(characters, utf16), character| {
            (characters + 1, utf16 + character.len_utf16())
        });
        Self {
            text,
            links: vec![],
            utf16,
            remaining: MAX_ROW_CHARS.saturating_sub(characters),
            truncated: false,
        }
    }

    // Finish a message with one fixed-size marker, then restore the budget so newer messages remain visible.
    pub(super) fn finish_row(&mut self) {
        // The marker is outside the payload budget and must never acquire an object link.
        if self.truncated {
            let marker = "\n… truncated";
            self.text.push_str(marker);
            self.utf16 += marker.encode_utf16().count();
        }
        self.remaining = MAX_ROW_CHARS;
        self.truncated = false;
    }
}
pub struct ObjectLink {
    pub start: usize,
    pub length: usize,
    pub node: u64,
}
struct Node {
    name: String,
    text: String,
    remote: Option<String>,
    depth: usize,
    expanded: bool,
    loading: bool,
    children: Option<Vec<u64>>,
    error: Option<String>,
}
#[derive(Default)]
pub struct Objects {
    nodes: HashMap<u64, Node>,
    next: u64,
    pending: HashMap<i32, u64>,
    commands: Vec<Value>,
}
impl Objects {
    // Retain an expandable remote value within the object-cache budget.
    pub fn capture(&mut self, name: &str, value: &Value, depth: usize) -> Option<u64> {
        // Bound retained data and per-event work before accepting more content.
        if self.nodes.len() >= MAX_NODES {
            self.release_value(value);
            return None;
        }
        self.next += 1;
        let id = self.next;
        self.nodes.insert(
            id,
            Node {
                name: name.chars().take(256).collect(),
                text: preview(value),
                remote: value["objectId"].as_str().map(str::to_owned),
                depth,
                expanded: false,
                loading: false,
                children: None,
                error: None,
            },
        );
        Some(id)
    }

    // Render the stable text shown for an expandable remote object.
    pub fn description(&self, id: u64) -> &str {
        self.nodes.get(&id).map_or("", |node| node.text.as_str())
    }

    // Expand a cached object or collapse it without repeating protocol requests.
    pub fn toggle(&mut self, id: u64) {
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        // Resolve the live tree identity before reading or mutating its retained state.
        if node.remote.is_none() || node.depth >= MAX_DEPTH {
            return;
        }
        node.expanded = !node.expanded;
        // Resolve the live tree identity before reading or mutating its retained state.
        if !node.expanded || node.loading || node.children.is_some() {
            return;
        }
        // Bound retained data and per-event work before accepting more content.
        if self.pending.len() >= 64 {
            node.error = Some("Too many pending objects — collapse and retry".into());
            return;
        }
        node.loading = true;
        node.error = None;
        let request = protocol_id();
        self.pending.insert(request, id);
        self.commands.push(
            json!({"id":request,"method":"Runtime.getProperties", "params":{
            "objectId":node.remote,"ownProperties":true,"generatePreview":true}}),
        );
    }

    // Route incoming protocol data to its owner and ignore replies from stale work.
    pub fn receive(&mut self, message: &Value) -> bool {
        // Route only the protocol event or reply owned by this handler.
        let Some(request) = message["id"].as_i64().and_then(|id| i32::try_from(id).ok()) else {
            return false;
        };
        // Avoid duplicate in-flight work and wait for the reply that already owns this state.
        let Some(id) = self.pending.remove(&request) else {
            return false;
        };
        let properties = message["result"]["result"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let internal = message["result"]["internalProperties"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(depth) = self.nodes.get(&id).map(|node| node.depth + 1) else {
            for property in properties.iter().chain(internal) {
                self.release_descriptor(property);
            }
            return true;
        };
        let error = message
            .get("error")
            .map(|error| {
                error["message"]
                    .as_str()
                    .unwrap_or("Object is no longer available")
                    .chars()
                    .take(MAX_ROW_CHARS)
                    .collect()
            })
            .or_else(|| {
                message["result"]
                    .get("exceptionDetails")
                    .map(|_| "Properties unavailable".into())
            });
        let mut children = vec![];
        let mut truncated = false;
        for property in properties.iter().chain(internal) {
            // Bound retained data and per-event work before accepting more content.
            if children.len() >= MAX_PROPERTIES || self.nodes.len() >= MAX_NODES {
                truncated = true;
                self.release_descriptor(property);
                continue;
            }
            let name = property["name"].as_str().unwrap_or("?");
            let accessor;
            // Normal properties borrow their protocol value; only accessors need synthetic display text.
            let value = if let Some(value) = property.get("value") {
                value
            } else {
                accessor = json!({"type":"string","value": if property.get("get").is_some_and(|v| v["type"] == "function") {
                    "[Getter — not evaluated]" } else { "[Setter — not evaluated]" }});
                &accessor
            };
            // Resolve the live tree identity before reading or mutating its retained state.
            if let Some(child) = self.capture(name, value, depth) {
                children.push(child);
            }
            // Accessor function handles are not part of the displayed value tree.
            if let Some(get) = property.get("get") {
                self.release_value(get);
            }
            // Apply the optional result only while its owning state is available.
            if let Some(set) = property.get("set") {
                self.release_value(set);
            }
            // Apply the optional result only while its owning state is available.
            if let Some(symbol) = property.get("symbol") {
                self.release_value(symbol);
            }
        }
        // Mark omitted properties without retaining or evaluating their remote values.
        if truncated
            && let Some(child) = self.capture(
                "…",
                &json!({"value":"First 200 properties shown; object limit may also apply"}),
                depth,
            )
        {
            children.push(child);
        }
        // Resolve the live tree identity before reading or mutating its retained state.
        if let Some(node) = self.nodes.get_mut(&id) {
            node.loading = false;
            node.children = Some(children);
            node.error = error;
        }
        true
    }

    // Render the expanded value graph within the retained row budget.
    pub fn render(&self, roots: &[u64], view: &mut ObjectView) {
        for id in roots {
            // A full message needs no more tree traversal or intermediate formatting.
            if view.remaining == 0 {
                view.truncated = true;
                return;
            }
            self.render_node(*id, view);
        }
    }

    // Render node.
    fn render_node(&self, id: u64, view: &mut ObjectView) {
        // Resolve the live tree identity before reading or mutating its retained state.
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        // Separate rendered value rows without adding an initial blank line.
        if !view.text.is_empty() {
            view.push("\n");
        }
        view.push(&"  ".repeat(node.depth));
        let expandable = node.remote.is_some() && node.depth < MAX_DEPTH;
        let start = view.utf16;
        // Render disclosure and child rows only for values that can expand.
        if expandable {
            // Resolve the live tree identity before reading or mutating its retained state.
            view.push(if node.expanded { "▾ " } else { "▸ " });
        }
        // Resolve the live tree identity before reading or mutating its retained state.
        if !node.name.is_empty() {
            view.push(&node.name);
            view.push(": ");
        }
        view.push(&node.text);
        // Keep only nonempty links entirely inside the bounded native text snapshot.
        if expandable && view.utf16 > start {
            view.links.push(ObjectLink {
                start,
                length: view.utf16 - start,
                node: id,
            });
        }
        // Resolve the live tree identity before reading or mutating its retained state.
        if !node.expanded {
            return;
        }

        // Resolve the live tree identity before reading or mutating its retained state.
        if node.loading {
            view.push("\n");
            view.push(&"  ".repeat(node.depth + 1));
            view.push("Loading properties…");
        }
        // Handle failure before exposing a partial result as usable state.
        if let Some(error) = &node.error {
            view.push("\n");
            view.push(&"  ".repeat(node.depth + 1));
            view.push(error);
        }
        // Resolve the live tree identity before reading or mutating its retained state.
        if let Some(children) = &node.children {
            // Handle failure before exposing a partial result as usable state.
            if children.is_empty() && node.error.is_none() {
                view.push("\n");
                view.push(&"  ".repeat(node.depth + 1));
                view.push("No own properties");
            }
            self.render(children, view);
        }
    }

    // Remove roots.
    pub fn remove_roots(&mut self, roots: &[u64]) {
        let mut remote = HashSet::new();
        let mut pending = roots.to_vec();
        while let Some(id) = pending.pop() {
            // Resolve the live tree identity before reading or mutating its retained state.
            if let Some(node) = self.nodes.remove(&id) {
                remote.extend(node.remote);
                pending.extend(node.children.unwrap_or_default());
            }
        }
        // Avoid sending a remote-release command when the removed rows owned no objects.
        if remote.is_empty() {
            return;
        }
        for id in self.nodes.values().filter_map(|node| node.remote.as_ref()) {
            remote.remove(id);
            // Avoid sending a remote-release command when the removed rows owned no objects.
            if remote.is_empty() {
                break;
            }
        }
        for id in remote {
            self.commands.push(json!({"id":protocol_id(),"method":"Runtime.releaseObject","params":{"objectId":id}}));
        }
    }

    // Release value.
    pub(super) fn release_value(&mut self, value: &Value) {
        // Resolve the stable identity before using a row index or applying a retained reply.
        if let Some(id) = value["objectId"].as_str() {
            self.commands.push(json!({"id":protocol_id(),"method":"Runtime.releaseObject","params":{"objectId":id}}));
        }
    }

    // Release descriptor.
    fn release_descriptor(&mut self, property: &Value) {
        for key in ["value", "get", "set", "symbol"] {
            // Resolve the stable identity before using a row index or applying a retained reply.
            if let Some(value) = property.get(key) {
                self.release_value(value);
            }
        }
    }

    // Release remote handles and discard the cached value graph.
    pub fn clear(&mut self) {
        // All roots are gone: release unique handles without a traversal or survivor scan.
        let remote: HashSet<_> = std::mem::take(&mut self.nodes)
            .into_values()
            .filter_map(|node| node.remote)
            .collect();
        for id in remote {
            self.commands.push(json!({"id":protocol_id(),"method":"Runtime.releaseObject","params":{"objectId":id}}));
        }
    }

    // Forget value nodes whose JavaScript context no longer exists.
    pub fn reset(&mut self) {
        *self = Self {
            next: self.next,
            ..Self::default()
        };
    }

    // Transfer queued commands to the caller without keeping a second copy.
    pub fn drain(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.commands)
    }
}

// Render bounded remote previews without retaining another full object graph.
fn preview(value: &Value) -> String {
    let mut text = String::new();
    let mut remaining = MAX_ROW_CHARS;
    append_remote(&mut text, value, &mut remaining);
    // Primitive values have no property preview; retain only their bounded display text.
    if let Some(properties) = value["preview"]["properties"].as_array() {
        let mut first = true;
        for (name, value) in properties
            .iter()
            .take(4)
            .filter_map(|p| Some((p["name"].as_str()?, p["value"].as_str().unwrap_or("…"))))
        {
            append_text(&mut text, if first { " {" } else { ", " }, &mut remaining);
            append_text(&mut text, name, &mut remaining);
            append_text(&mut text, ": ", &mut remaining);
            append_text(&mut text, value, &mut remaining);
            first = false;
        }
        // Close a preview only when at least one named property was rendered.
        if !first {
            // Chromium can supply a shortened preview even when four or fewer properties are present.
            if properties.len() > 4 || value["preview"]["overflow"] == true {
                append_text(&mut text, ", …", &mut remaining);
            }
            append_text(&mut text, "}", &mut remaining);
        }
    }
    text
}

// Most filters are ASCII URLs/methods: compare bytes without allocating a

// Append remote values directly; a large JSON-by-value result must not allocate a full temporary serialization.
pub(super) fn append_remote(output: &mut String, remote: &Value, remaining: &mut usize) {
    // A full preview needs no more serialization, but its caller still owns remote-handle cleanup.
    if *remaining == 0 {
        return;
    }
    // Primitive strings are already protocol text; other JSON values stream through the same budget.
    if let Some(value) = remote.get("value") {
        // Strings display their contents without JSON quoting or escaping.
        if let Some(value) = value.as_str() {
            append_text(output, value, remaining);
            return;
        }
        let _ = write!(TextBudget { output, remaining }, "{value}");
        return;
    }
    let value = remote["unserializableValue"]
        .as_str()
        .or_else(|| remote["description"].as_str())
        .or_else(|| remote["type"].as_str())
        .unwrap_or("undefined");
    append_text(output, value, remaining);
}

// Stop Display/JSON formatting as soon as the caller's shared character budget is exhausted.
struct TextBudget<'a> {
    output: &'a mut String,
    remaining: &'a mut usize,
}
impl Write for TextBudget<'_> {
    // Report truncation to the formatter so nested JSON stops traversing the remaining value graph.
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let mut consumed = 0;
        for character in value.chars().take(*self.remaining) {
            self.output.push(character);
            *self.remaining -= 1;
            consumed += character.len_utf8();
        }
        // A partial chunk intentionally ends formatting; the output is a text preview, not reparsed JSON.
        if consumed < value.len() {
            return Err(fmt::Error);
        }
        Ok(())
    }
}

// Prefer the exception description, falling back to the protocol error text.
pub(super) fn exception_text(details: &Value) -> &str {
    details["exception"]["description"]
        .as_str()
        .or_else(|| details["text"].as_str())
        .unwrap_or("JavaScript exception")
}

#[cfg(test)]
#[path = "../../tests/objects.rs"]
mod tests;
