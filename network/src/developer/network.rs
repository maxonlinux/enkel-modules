//! Bounded request metadata and one explicitly requested response body.
use super::{contains, protocol_id};

pub(crate) use details::elapsed;
use details::{duration, format_size};
use serde_json::{Value, json};
use std::{
    cell::OnceCell,
    collections::{HashMap, VecDeque},
    fmt,
    rc::Rc,
};

#[path = "network_details.rs"]
mod details;
#[path = "network_extra.rs"]
mod extra;
#[path = "network_streams.rs"]
mod streams;

pub use details::Detail;
pub use details::Span;
const MAX_REQUESTS: usize = 300;
const MAX_BODY: usize = 512 * 1024;

// Bound retained text without splitting a Unicode code point.
fn clipped(value: &str, limit: usize) -> String {
    // Bound retained data and per-event work before accepting more content.
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… truncated", &value[..end])
}

// Read an optional protocol string with an empty fallback for display.
fn field(value: &Value, key: &str) -> String {
    clipped(value[key].as_str().unwrap_or_default(), 8192)
}

// Convert the protocol header object into retained name/value pairs.
fn headers(value: &Value) -> String {
    // Reject a missing or mismatched value before the following operation relies on it.
    let Some(headers) = value.as_object() else {
        return String::new();
    };
    let mut output = String::new();
    for (name, value) in headers {
        // Bound retained data and per-event work before accepting more content.
        if output.len() >= 16384 {
            output.push_str("\n… truncated");
            break;
        }
        for line in value.as_str().unwrap_or_default().lines() {
            // Bound retained data and per-event work before accepting more content.
            if output.len() >= 16384 {
                break;
            }
            output.push_str(&clipped(name, 256));
            output.push_str(": ");
            output.push_str(&clipped(line, 8192));
            output.push('\n');
        }
    }
    clipped(&output, 16384)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Http(u16),
    Connecting,
    Open,
    Closed,
    Stopped,
    Failed,
    Unknown,
}
impl RequestStatus {
    // Error appearance follows protocol state, never a translated or formatted label.
    pub fn is_error(self) -> bool {
        matches!(self, Self::Failed | Self::Http(400..))
    }
}
impl fmt::Display for RequestStatus {
    // Keep presentation labels at the final text boundary; HTTP codes remain numeric internally.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Http(code) => return write!(formatter, "{code}"),
            Self::Pending => "Pending",
            Self::Connecting => "Connecting",
            Self::Open => "Open",
            Self::Closed => "Closed",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
            Self::Unknown => "—",
        };
        formatter.write_str(label)
    }
}

// Selected detail data shares one lifetime; changing or evicting this key invalidates all pending/cached content.
struct Selection {
    key: u64,
    payload: OnceCell<Rc<str>>,
    response: Option<Rc<str>>,
    response_request: Option<i32>,
}
#[derive(Clone, PartialEq)]
pub struct Summary {
    pub key: u64,
    pub name: Rc<str>,
    pub method: Rc<str>,
    pub status: RequestStatus,
    pub kind: String,
    pub size: String,
    pub time: String,
    pub initiator: Rc<str>,
    pub source: Rc<str>,
    pub span: Option<Span>,
}
struct Request {
    key: u64,
    id: String,
    name: Rc<str>,
    url: String,
    method: Rc<str>,
    kind: String,
    request_headers: String,
    response_headers: String,
    payload: String,
    status: RequestStatus,
    mime: String,
    started: f64,
    ended: Option<f64>,
    bytes: f64,
    body_available: bool,
    error: String,
    extra: extra::Metadata,
    initiator: details::Initiator,
}
impl Request {
    // Recognize event streams from their protocol type or an HTTP response's MIME type.
    fn is_sse(&self) -> bool {
        self.kind == "EventSource"
            || self
                .mime
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
    }

    // Identify the request types that retain a live message history.
    fn has_messages(&self) -> bool {
        self.kind == "WebSocket" || self.is_sse()
    }

    // Resolve the compact table label after MIME metadata can identify a Fetch stream.
    fn label(&self) -> &str {
        // Keep stream-specific handling separate from normal retained request data.
        if self.is_sse() {
            "SSE"
        } else {
            type_label(&self.kind)
        }
    }
}
#[derive(Default)]
pub struct Network {
    rows: VecDeque<Request>,
    next: u64,
    selection: Option<Selection>,
    commands: Vec<Value>,
    messages: VecDeque<streams::Message>,
    extras: extra::Extras,
    streams: HashMap<u64, streams::Stream>,
    control: Option<i32>,
    pub active: bool,
    pub status: String,
    observed: f64,
}
impl Network {
    // Enable or release protocol capture when this tool's active state changes.
    pub fn set_active(&mut self, active: bool) {
        // Skip unchanged capture state and enable or disable domains according to the requested activity.
        if self.active == active {
            return;
        }
        self.active = active;
        // Disable invalidates the engine's captured bodies. Keep only metadata
        // and our single saved preview; never request an obsolete body later.
        if !active {
            self.stop_capture();
        }
        let id = protocol_id();
        self.control = Some(id);
        // Skip unchanged capture state and enable or disable domains according to the requested activity.
        self.status = if active {
            "Recording requests — reload to capture page load"
        } else {
            "Capture stopped"
        }
        .into();
        // Skip unchanged capture state and enable or disable domains according to the requested activity.
        let method = if active {
            "Network.enable"
        } else {
            "Network.disable"
        };
        // Skip unchanged capture state and enable or disable domains according to the requested activity.
        let params = if active {
            json!({"maxTotalBufferSize":8*1024*1024,"maxResourceBufferSize":1024*1024,"maxPostDataSize":16384})
        } else {
            json!({})
        };
        self.commands
            .push(json!({"id":id,"method":method,"params":params}));
    }

    // Disable active network sessions without discarding their retained request rows.
    pub(super) fn stop_capture(&mut self) {
        self.active = false;
        self.control = None;
        self.commands.clear();
        // A stopped capture cannot fulfill its pending response-body request.
        if let Some(selection) = &mut self.selection {
            selection.response_request = None;
        }
        self.extras.clear();
        self.streams.clear();
        for row in &mut self.rows {
            row.body_available = false;
            // Completed requests already have final timing; only live captures need stopping.
            if row.ended.is_some() {
                continue;
            }
            row.error = "Capture stopped before completion".into();
            // A retained stream must stop displaying itself as live after capture releases its parser.
            if row.has_messages() {
                row.status = RequestStatus::Stopped;
            }
        }
    }

    // Route incoming protocol data to its owner and ignore replies from stale work.
    pub fn receive(&mut self, message: &Value) -> bool {
        // Route only the protocol event or reply owned by this handler.
        if let Some(id) = message["id"].as_i64().and_then(|id| i32::try_from(id).ok()) {
            return self.receive_reply(id, message);
        }
        let method = message["method"].as_str().unwrap_or_default();
        // Do not start work after teardown or before this owner is ready.
        if !self.active || !method.starts_with("Network.") {
            return false;
        }
        let p = &message["params"];
        // Apply the optional result only while its owning state is available.
        if let Some(time) = p["timestamp"].as_f64() {
            self.observed = self.observed.max(time);
        }
        let id = field(p, "requestId");
        // Route only the protocol event or reply owned by this handler.
        if method.starts_with("Network.webSocket") {
            return self.socket_event(method, p, id);
        }
        match method {
            "Network.requestWillBeSent" => self.record_request(p, id.clone()),
            "Network.requestWillBeSentExtraInfo" | "Network.responseReceivedExtraInfo" => {
                self.extras
                    .receive(p, method == "Network.responseReceivedExtraInfo");
            }
            "Network.eventSourceMessageReceived" => return self.event_source(p, &id),
            "Network.dataReceived" => return self.stream_chunk(p, &id),
            "Network.responseReceived" | "Network.loadingFinished" | "Network.loadingFailed" => {
                // Resolve the stable identity before using a row index or applying a retained reply.
                let Some(row) = self.rows.iter_mut().rev().find(|row| row.id == id) else {
                    return false;
                };
                // Route only the protocol event or reply owned by this handler.
                if method == "Network.responseReceived" {
                    response(row, &p["response"]);
                    row.extra.expected = p["hasExtraInfo"].as_bool();
                    self.start_stream(&id);
                } else {
                    finish_request(row, p, method.ends_with("Failed"));
                }
            }
            _ => return false,
        }
        self.extras.reconcile(&mut self.rows, &id);
        true
    }

    // Match protocol replies to the retained request that owns their result.
    fn receive_reply(&mut self, id: i32, message: &Value) -> bool {
        // Keep stream-specific handling separate from normal retained request data.
        if self.stream_reply(id, message) {
            return true;
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        if self.control == Some(id) {
            self.control = None;
            // Merge a request-start event into its stable session/request identity.
            if let Some(error) = message.get("error") {
                self.status = format!("Network unavailable: {}", field(error, "message"));
                self.stop_capture();
            }
            return true;
        }
        // Only the currently selected request can own an outstanding response preview.
        let Some(selection) = self
            .selection
            .as_mut()
            .filter(|selection| selection.response_request == Some(id))
        else {
            return false;
        };
        selection.response_request = None;
        // A protocol failure needs an explanation instead of an empty response preview.
        let body = if let Some(error) = message.get("error") {
            format!("Body unavailable: {}", field(error, "message"))
        } else if message["result"]["base64Encoded"].as_bool() == Some(true) {
            "Binary response — preview not loaded".into()
        } else {
            clipped(
                message["result"]["body"].as_str().unwrap_or_default(),
                MAX_BODY,
            )
        };
        selection.response = Some(details::pretty(&body).into());
        true
    }

    // Merge a request-start event into its stable session/request identity.
    fn record_request(&mut self, p: &Value, id: String) {
        // A redirect completes the preceding response while retaining the shared request identity.
        if let Some(redirect) = p.get("redirectResponse")
            && let Some(row) = self.rows.iter_mut().rev().find(|row| row.id == id)
        {
            response(row, redirect);
            row.extra.expected = p["redirectHasExtraInfo"].as_bool();
            row.ended = p["timestamp"].as_f64();
            row.body_available = false;
        }
        self.next += 1;
        let request = &p["request"];
        let url = field(request, "url");
        self.rows.push_back(Request {
            key: self.next,
            id,
            name: request_name(&url).into(),
            url,
            method: field(request, "method").into(),
            kind: field(p, "type"),
            request_headers: headers(&request["headers"]),
            response_headers: String::new(),
            payload: clipped(request["postData"].as_str().unwrap_or_default(), 16384),
            status: RequestStatus::Pending,
            mime: String::new(),
            started: p["timestamp"].as_f64().unwrap_or(0.),
            ended: None,
            bytes: 0.,
            body_available: false,
            error: String::new(),
            extra: extra::Metadata::default(),
            initiator: details::Initiator::new(&p["initiator"]),
        });
        while self.rows.len() > MAX_REQUESTS {
            // Reject a missing or mismatched value before the following operation relies on it.
            let Some(old) = self.rows.pop_front() else {
                break;
            };
            self.messages.retain(|message| message.key != old.key);
            self.streams.remove(&old.key);
            // Resolve the stable identity before using a row index or applying a retained reply.
            if !self.rows.iter().any(|row| row.id == old.id) {
                self.extras.forget(&old.id);
            }
            // Resolve the stable identity before using a row index or applying a retained reply.
            if self.selected() == Some(old.key) {
                self.selection = None;
            }
        }
    }

    // Build the filtered request snapshot from stable retained request identities.
    pub fn rows(&self, query: &str) -> Vec<Summary> {
        let query = query.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|row| {
                query.is_empty()
                    || [
                        row.url.as_str(),
                        &row.method,
                        &row.kind,
                        row.label(),
                        &row.initiator.description,
                    ]
                    .iter()
                    .any(|value| contains(value, &query))
                    || contains(&row.status.to_string(), &query)
            })
            .map(|row| Summary {
                key: row.key,
                name: row.name.clone(),
                method: row.method.clone(),
                status: row.status,
                kind: row.label().into(),
                // Completed requests and retained streams remain visible after their active capture ends.
                size: if row.ended.is_some() || row.has_messages() {
                    format_size(row.bytes)
                } else {
                    "—".into()
                },
                time: duration(row),
                initiator: row.initiator.label.clone(),
                source: row.initiator.description.clone(),
                span: details::span(row, self.observed),
            })
            .collect()
    }

    // Change selection as one unit so old response replies and formatted payloads cannot attach to another row.
    pub fn select(&mut self, key: Option<u64>) {
        // Native selection notifications may repeat the current key without changing its detail state.
        if self.selected() == key {
            return;
        }
        self.selection = self
            .rows
            .iter()
            .find(|row| Some(row.key) == key)
            .map(|row| Selection {
                key: row.key,
                payload: OnceCell::new(),
                response: None,
                response_request: None,
            });
    }

    // Expose only the stable selected key, keeping detail caches and pending requests private to their owner.
    pub fn selected(&self) -> Option<u64> {
        self.selection.as_ref().map(|selection| selection.key)
    }

    // A response may be fetched once while its selected request still belongs to an active capture.
    pub fn can_load_body(&self) -> bool {
        // No selected row can own a response-body request.
        let Some(selection) = &self.selection else {
            return false;
        };
        self.active
            && selection.response.is_none()
            && selection.response_request.is_none()
            && self
                .rows
                .iter()
                .find(|row| row.key == selection.key)
                .is_some_and(|row| row.body_available && row.kind != "WebSocket")
    }

    // Queue the response fetch under the selection that will receive or discard its reply.
    pub fn load_body(&mut self) {
        // Duplicate, expired and inactive fetches must not allocate another response preview.
        if !self.can_load_body() {
            return;
        }
        let selection = self
            .selection
            .as_mut()
            .expect("Validated response selection");
        let row = self
            .rows
            .iter()
            .find(|row| row.key == selection.key)
            .expect("Retained selected request");
        let id = protocol_id();
        selection.response_request = Some(id);
        self.commands.push(
            json!({"id":id,"method":"Network.getResponseBody","params":{"requestId":row.id}}),
        );
    }

    // Discard captured requests and their retained body and stream data.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.messages.clear();
        self.extras.clear();
        self.streams.clear();
        self.selection = None;
        self.observed = 0.;
    }

    // Transfer queued commands to the caller without keeping a second copy.
    pub fn drain(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.commands)
    }
}

// Use the last path component as the request label, with its origin as the root-path fallback.
fn request_name(value: &str) -> String {
    url::Url::parse(value)
        .ok()
        .map(|url| {
            let path = url.path();
            let name = path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or_else(|| url.host_str().unwrap_or("request"));
            name.to_owned()
        })
        .unwrap_or_else(|| value.into())
}

// Describe HTTP or streaming request state for the request list.
fn type_label(kind: &str) -> &str {
    match kind {
        "WebSocket" => "WS",
        "EventSource" => "SSE",
        "Document" => "Doc",
        "Stylesheet" => "CSS",
        "Script" => "JS",
        _ => kind,
    }
}

// Accept integer-valued protocol numbers without rounding malformed status codes into valid HTTP state.
fn status_code(value: &Value) -> Option<u16> {
    value
        .as_f64()
        .filter(|code| code.fract() == 0. && (0. ..=f64::from(u16::MAX)).contains(code))
        .map(|code| code as u16)
}

// Merge response status and headers into the request that owns them.
fn response(row: &mut Request, value: &Value) {
    row.status = status_code(&value["status"]).map_or(RequestStatus::Unknown, RequestStatus::Http);
    row.mime = field(value, "mimeType");
    row.response_headers = headers(&value["headers"]);
}

// Complete transfer timing and preserve the distinction between failure and an explicitly closed stream.
fn finish_request(row: &mut Request, p: &Value, failed: bool) {
    row.ended = p["timestamp"].as_f64();
    row.body_available = !failed;
    // Handle failure before exposing a partial result as usable state.
    if !failed {
        row.bytes = p["encodedDataLength"].as_f64().unwrap_or(0.);
        return;
    }
    // Keep stream-specific handling separate from normal retained request data.
    if row.is_sse() && p["canceled"].as_bool() == Some(true) {
        row.status = RequestStatus::Closed;
        return;
    }
    row.status = RequestStatus::Failed;
    row.extra.expected.get_or_insert(true);
    row.error = field(p, "errorText");
    let blocked = field(p, "blockedReason");
    // Append blocking details only when the protocol supplied a reason.
    if !blocked.is_empty() {
        row.error.push_str(&format!(" ({blocked})"));
    }
}

#[cfg(test)]
#[path = "../../tests/network.rs"]
mod tests;
