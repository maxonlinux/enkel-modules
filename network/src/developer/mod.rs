//! Own Network protocol lifecycle and state in an independent scoped session.
#![forbid(unsafe_code)]

pub mod network;

use crate::protocol_id;

use serde_json::{Value, json};

#[derive(Default, Clone, Copy)]
pub struct Changes {
    pub data: bool,
}
impl Changes {
    pub const ALL: Self = Self { data: true };

    // Report whether a UI update contains any changed tool state.
    pub fn any(self) -> bool {
        self.data
    }
}
pub struct Session {
    pub changes: Changes,
    target_request: i32,
    attach_request: Option<i32>,
    enable_request: Option<i32>,

    session: Option<String>,
    pub ready: bool,
    pub status: String,

    pub network: network::Network,
}
impl Default for Session {
    // Start with empty collections and no active request or selection.
    fn default() -> Self {
        Self {
            changes: Changes::default(),
            target_request: protocol_id(),
            attach_request: None,
            enable_request: None,

            session: None,
            ready: false,
            status: "Connecting…".into(),

            network: network::Network::default(),
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

        if self.network.receive(&message) {
            self.changes.data = true;
            return self.drain_commands();
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        if id.is_some() && id == self.enable_request {
            self.changes = Changes::ALL;
            return self.enabled(&message);
        }
        // Resolve the stable identity before using a row index or applying a retained reply.

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
        self.network.set_active(true);
        let mut commands = self.drain_commands();
        commands.push(self.command(protocol_id(), "Runtime.disable", json!({})));
        commands
    }

    // Route protocol domain events to their owning retained tool state.
    fn event(&mut self, message: &Value) -> bool {
        let method = message["method"].as_str().unwrap_or_default();
        match method {
            "Inspector.detached" => self.disconnected(),
            _ => return false,
        }
        true
    }

    // Release session-owned state after its protocol target closes.
    fn disconnected(&mut self) {
        self.ready = false;

        self.network.stop_capture();

        self.session = None;

        self.changes = Changes::ALL;
        self.status = "Disconnected — close and reopen developer widgets to reconnect".into();
    }

    // Transfer queued protocol commands after releasing the model borrow.
    fn drain_commands(&mut self) -> Vec<String> {
        let mut commands = Vec::<Value>::new();

        commands.extend(self.network.drain());

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

    // Drain this module's queued commands for its scoped host transport.
    pub fn protocol_commands(&mut self) -> Vec<String> {
        self.drain_commands()
    }

    // Load network body.
    pub fn load_network_body(&mut self) -> Vec<String> {
        // Request body text only after protocol connection completes.
        if self.ready {
            self.network.load_body();
        }
        self.drain_commands()
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
