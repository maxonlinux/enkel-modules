//! Own Console protocol lifecycle and state in an independent scoped session.
#![forbid(unsafe_code)]

use objects::{append_remote, exception_text};

mod completion;

pub use completion::CommandHistory;

#[path = "console_log.rs"]
mod log;

mod objects;
use crate::protocol_id;

pub use completion::Suggestions;

pub(crate) use completion::{history_boundary, indented_newline};

pub use objects::ObjectView;
use serde_json::{Value, json};

use std::collections::{HashSet, VecDeque};

const MAX_ROWS: usize = 500;

const MAX_ROW_CHARS: usize = 4096;

const MAX_EVALUATIONS: usize = 64;

struct Row {
    text: String,
    roots: Vec<u64>,
}
#[derive(Default, Clone, Copy)]
pub struct Changes {
    pub data: bool,
    pub completion: bool,
}
impl Changes {
    pub const ALL: Self = Self {
        data: true,
        completion: true,
    };

    // Report whether a UI update contains any changed tool state.
    pub fn any(self) -> bool {
        self.data || self.completion
    }
}
pub struct Session {
    pub changes: Changes,
    target_request: i32,
    attach_request: Option<i32>,
    enable_request: Option<i32>,

    evaluations: HashSet<i32>,

    abandoned_evaluations: HashSet<i32>,
    session: Option<String>,
    pub ready: bool,
    pub status: String,

    rows: VecDeque<Row>,

    objects: objects::Objects,

    pub completion: completion::Completion,
}
impl Default for Session {
    // Start with empty collections and no active request or selection.
    fn default() -> Self {
        Self {
            changes: Changes::default(),
            target_request: protocol_id(),
            attach_request: None,
            enable_request: None,

            evaluations: HashSet::new(),

            abandoned_evaluations: HashSet::new(),
            session: None,
            ready: false,
            status: "Connecting…".into(),

            rows: VecDeque::new(),

            objects: objects::Objects::default(),

            completion: completion::Completion::default(),
        }
    }
}
impl Session {
    // Enable the protocol domains needed by the currently active tools.
    pub fn connect(&self) -> String {
        json!({"id":self.target_request,"method":"Target.getTargetInfo"}).to_string()
    }

    // Queue a protocol command with the reply metadata needed to validate it.
    fn command(&self, id: i32, method: &str, params: Value) -> String {
        let mut message = json!({"id":id,"method":method,"params":params});
        // Apply the optional result only while its owning state is available.
        if let Some(session) = &self.session {
            message["sessionId"] = session.clone().into();
        }
        message.to_string()
    }

    // Queue the current Console expression and retain its reply identity.
    pub fn evaluate(&mut self, expression: &str) -> Option<String> {
        // Do not start work after teardown or before this owner is ready.
        if !self.ready || expression.trim().is_empty() {
            return None;
        }
        // Bound retained data and per-event work before accepting more content.
        if self.evaluations.len() >= MAX_EVALUATIONS {
            self.push(
                "Too many pending expressions — wait or close and reopen all developer widgets",
            );
            return None;
        }
        let id = protocol_id();
        self.evaluations.insert(id);
        self.push(format!("> {expression}"));
        Some(self.command(
            id,
            "Runtime.evaluate",
            json!({"expression":expression,
            "includeCommandLineAPI":true,"awaitPromise":true,"replMode":true,"generatePreview":true,
            "objectGroup":"enkel-native-console"}),
        ))
    }

    // Decode one bounded protocol message before dispatching its domain updates.
    pub fn receive_bytes(&mut self, body: &[u8]) -> Vec<String> {
        self.changes = Changes::default();
        // Reject a missing or mismatched value before the following operation relies on it.
        let Ok(message) = serde_json::from_slice::<Value>(body) else {
            return vec![];
        };
        let id = message["id"].as_i64().and_then(|id| i32::try_from(id).ok());
        // Resolve the current target before applying work that could otherwise affect a stale owner.
        if id == Some(self.target_request) {
            self.changes = Changes::ALL;
            return self.attach_target(&message);
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        if id.is_some() && id == self.attach_request {
            self.changes = Changes::ALL;
            return self.attached(&message);
        }
        // Each tool consumes only our attached session, never another inspector.
        if self.session.is_none() || message["sessionId"].as_str() != self.session.as_deref() {
            return vec![];
        }
        // Let the owning tool consume its protocol message before trying unrelated handlers.

        if self.completion.receive(&message) {
            self.changes.completion = true;
            return self.drain_commands();
        }
        // Let the owning tool consume its protocol message before trying unrelated handlers.

        if self.objects.receive(&message) {
            self.changes.data = true;
            return self.drain_commands();
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        if id.is_some() && id == self.enable_request {
            self.changes = Changes::ALL;
            return self.enabled(&message);
        }
        // Resolve the stable identity before using a row index or applying a retained reply.

        if let Some(id) = id.filter(|id| self.evaluations.remove(id)) {
            self.changes.data = true;
            return self.evaluated(id, &message);
        }
        self.changes.data = self.event(&message);
        self.drain_commands()
    }

    // Attach relevant child targets to capture worker and frame activity.
    fn attach_target(&mut self, message: &Value) -> Vec<String> {
        // Route only the protocol event or reply owned by this handler.
        let Some(target) = message["result"]["targetInfo"]["targetId"].as_str() else {
            self.status = "Could not connect to this page".into();
            return vec![];
        };
        let id = protocol_id();
        self.attach_request = Some(id);
        vec![json!({"id":id,"method":"Target.attachToTarget","params":{"targetId":target,"flatten":true}}).to_string()]
    }

    // Initialize protocol capture for a newly attached child session.
    fn attached(&mut self, message: &Value) -> Vec<String> {
        self.attach_request = None;
        // Route only the protocol event or reply owned by this handler.
        let Some(session) = message["result"]["sessionId"].as_str() else {
            self.status = "This page does not support a developer session".into();
            return vec![];
        };
        self.session = Some(session.into());
        let id = protocol_id();
        self.enable_request = Some(id);
        vec![self.command(id, "Runtime.enable", json!({}))]
    }

    // Start queued tool work after the required protocol domains are ready.
    fn enabled(&mut self, message: &Value) -> Vec<String> {
        self.enable_request = None;
        self.ready = message.get("error").is_none();
        self.status = if self.ready {
            "Connected · current page"
        } else {
            "Connection failed"
        }
        .into();
        // Failed handshakes must not enable capture or enqueue domain work for an unattached page.
        if !self.ready {
            return vec![];
        }
        self.drain_commands()
    }

    // Match an evaluation reply to the expression that requested it.
    fn evaluated(&mut self, id: i32, message: &Value) -> Vec<String> {
        // Resolve the stable identity before using a row index or applying a retained reply.
        if self.abandoned_evaluations.remove(&id) {
            self.objects.release_value(&message["result"]["result"]);
            self.objects
                .release_value(&message["result"]["exceptionDetails"]["exception"]);
            return self.drain_commands();
        }
        // Route protocol domain events to their owning retained tool state.
        if let Some(error) = message.get("error") {
            self.push(format!(
                "Error: {}",
                error["message"].as_str().unwrap_or("Evaluation failed")
            ));
        } else if let Some(exception) = message["result"].get("exceptionDetails") {
            self.push(exception_text(exception));
            self.objects.release_value(&exception["exception"]);
        } else {
            self.push_values("", std::slice::from_ref(&message["result"]["result"]));
        }
        self.drain_commands()
    }

    // Route protocol domain events to their owning retained tool state.
    fn event(&mut self, message: &Value) -> bool {
        let method = message["method"].as_str().unwrap_or_default();
        match method {
            "Runtime.consoleAPICalled" => self.logged(&message["params"]),

            "Runtime.exceptionThrown" => self.exception(&message["params"]["exceptionDetails"]),

            "Runtime.executionContextsCleared" => self.contexts_cleared(),
            "Inspector.detached" => self.disconnected(),
            _ => return false,
        }
        true
    }

    // Retain a Console event with its level, source and remote-object ownership.
    fn logged(&mut self, params: &Value) {
        let kind = params["type"].as_str().unwrap_or("log");
        // The Console clear event discards rows and their remote ownership.
        if kind == "clear" {
            self.clear();
            return;
        }
        self.push_values(
            &format!("[{kind}]"),
            params["args"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default(),
        );
    }

    // Retain exception text and source information for Console display.
    fn exception(&mut self, exception: &Value) {
        self.push(exception_text(exception));
        self.objects.release_value(&exception["exception"]);
    }

    // Invalidate remote objects and completions when JavaScript contexts disappear.
    fn contexts_cleared(&mut self) {
        self.reset_context();
        self.push("— Page context changed —");
    }

    // Reset context.
    fn reset_context(&mut self) {
        for row in self.rows.iter_mut().filter(|row| !row.roots.is_empty()) {
            let mut view = ObjectView::with_text(std::mem::take(&mut row.text));
            self.objects.render(&row.roots, &mut view);
            row.text = view.text;
            row.roots.clear();
        }
        self.objects.reset();
        self.completion.reset();
        self.changes.completion = true;
    }

    // Release session-owned state after its protocol target closes.
    fn disconnected(&mut self) {
        self.ready = false;

        self.session = None;

        self.evaluations.clear();

        self.abandoned_evaluations.clear();

        self.reset_context();
        self.changes = Changes::ALL;
        self.status = "Disconnected — close and reopen developer widgets to reconnect".into();
    }

    // Transfer queued protocol commands after releasing the model borrow.
    fn drain_commands(&mut self) -> Vec<String> {
        let mut commands = Vec::<Value>::new();

        commands.extend(self.objects.drain());
        commands.extend(self.completion.drain());

        commands
            .into_iter()
            .map(|mut command| {
                // Apply the optional result only while its owning state is available.
                if let Some(session) = &self.session {
                    command["sessionId"] = session.clone().into();
                }
                command.to_string()
            })
            .collect()
    }

    // Start bounded completion for the current native expression and caret.
    pub fn complete(&mut self, expression: &str, caret: usize) -> Vec<String> {
        // Dispatch to the widget that owns this operation rather than inferring behavior from labels.
        if self.ready {
            self.completion.start(expression, caret);
        }
        self.drain_commands()
    }

    // Drain this module's queued commands for its scoped host transport.
    pub fn protocol_commands(&mut self) -> Vec<String> {
        self.drain_commands()
    }

    // Cancel completion.
    pub fn cancel_completion(&mut self) {
        self.completion.cancel();
    }

    // Toggle object.
    pub fn toggle_object(&mut self, node: u64) -> Vec<String> {
        // Request remote properties only after protocol connection completes.
        if self.ready {
            self.objects.toggle(node);
        }
        self.drain_commands()
    }
}

// Enforce the shared Unicode-character budget while appending, before allocating a large preview.
fn append_text(output: &mut String, value: &str, remaining: &mut usize) {
    for character in value.chars().take(*remaining) {
        output.push(character);
        *remaining -= 1;
    }
}

// lowercase copy for every field. Unicode filters retain case-insensitive matching.
pub(super) fn contains(value: &str, query: &str) -> bool {
    // Empty filters match everything; windows(0) would panic on the ASCII fast path.
    if query.is_empty() {
        return true;
    }
    // ASCII queries can use case-insensitive byte matching without allocating folded strings.
    if query.is_ascii() {
        return value
            .as_bytes()
            .windows(query.len())
            .any(|part| part.eq_ignore_ascii_case(query.as_bytes()));
    }
    value.to_lowercase().contains(query)
}

#[cfg(test)]
#[path = "../../tests/session.rs"]
mod tests;
