//! Own bounded WebSocket and SSE message capture, framing and incremental decoding.
use super::*;
use base64::prelude::{BASE64_STANDARD, Engine as _};
pub(super) const MAX_MESSAGES: usize = 200;
pub(super) const MAX_MESSAGE_BYTES: usize = 4096;
pub(super) enum Kind {
    Socket { sent: bool, opcode: u64 },
    Event { name: String, id: String },
}
impl Kind {
    // Create the presentation label used by this owner.
    fn label(&self) -> String {
        match self {
            Self::Socket { sent, opcode } => {
                // Display the message direction independently of its payload text.
                let direction = if *sent { "↑ Sent" } else { "↓ Received" };
                let kind = match opcode {
                    1 => "Text",
                    2 => "Binary · base64",
                    8 => "Close · base64",
                    9 => "Ping · base64",
                    10 => "Pong · base64",
                    _ => "Data · base64",
                };
                format!("{direction}  {kind}")
            }
            Self::Event { name, id } => format!(
                "↓ SSE  {name}\nID  {}",
                if id.is_empty() { "—" } else { id }
            ),
        }
    }
}
pub(super) struct Message {
    pub key: u64,
    pub timestamp: f64,
    pub kind: Kind,
    pub payload: String,
}
impl Network {
    // Retain a bounded stream-message snapshot under its request owner.
    pub(super) fn push_message(&mut self, message: Message) {
        self.messages.push_back(message);
        // Bound retained stream messages per request.
        if self.messages.len() > MAX_MESSAGES {
            self.messages.pop_front();
        }
    }

    // Render the retained stream messages with direction and truncation feedback.
    pub(super) fn stream_messages(&self, row: &Request) -> String {
        // Explain when no protocol messages have been retained.
        if !row.has_messages() {
            return "Select a WebSocket or SSE request".into();
        }
        let mut output = String::from(
            "Recent messages · newest 200 shared across connections · 4 KB per preview\n",
        );
        let mut any = false;
        for message in self
            .messages
            .iter()
            .filter(|message| message.key == row.key)
        {
            any = true;
            let label = message.kind.label();
            output.push_str(&format!(
                "\n{:.3} s  {label}\n{}\n",
                (message.timestamp - row.started).max(0.),
                message.payload
            ));
        }
        // Explain when the current message filter has no matches.
        if !any {
            output.push_str("\nNo captured messages yet.");
        }
        // Handle failure before exposing a partial result as usable state.
        if let Some(stream) = self.streams.get(&row.key)
            && let Some(error) = &stream.error
        {
            output.push_str(&format!("\n{error}"));
        }
        output
    }
}

#[derive(Default)]
pub(super) struct Parser {
    line: Vec<u8>,
    overflow: bool,
    data: String,
    name: String,
    id: String,
    has_data: bool,
    skip_lf: bool,
    started: bool,
}
pub(super) struct Event {
    pub name: String,
    pub id: String,
    pub data: String,
}
impl Parser {
    // Read the current delimiter without advancing beyond the input.
    pub fn byte(&mut self, byte: u8) -> Option<Event> {
        let skip = self.skip_lf && byte == b'\n';
        self.skip_lf = byte == b'\r';
        // Consume LF after CR as one newline rather than dispatching an empty SSE event.
        if skip {
            return None;
        }
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if matches!(byte, b'\r' | b'\n') {
            return self.finish_line();
        }
        // Stop before indexing or dispatching an operation that has no usable content.
        if self.line.len() < MAX_MESSAGE_BYTES {
            self.line.push(byte);
        } else {
            self.overflow = true;
        }
        None
    }

    // Finish line.
    fn finish_line(&mut self) -> Option<Event> {
        let bytes = std::mem::take(&mut self.line);
        let line = String::from_utf8_lossy(&bytes);
        // Only the first SSE line may contain a UTF-8 byte-order mark.
        let line = if !self.started {
            self.started = true;
            line.trim_start_matches('\u{feff}')
        } else {
            &line
        };
        // An empty line dispatches the accumulated SSE event.
        if line.is_empty() {
            self.line = bytes;
            return self.dispatch();
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                // Multiple data lines join with a newline inside one SSE event.
                if self.has_data {
                    append(&mut self.data, "\n", MAX_MESSAGE_BYTES);
                }
                append(&mut self.data, value, MAX_MESSAGE_BYTES);
                // Discard the remainder of an oversized event before accepting another one.
                if std::mem::take(&mut self.overflow) {
                    self.data = format!(
                        "{}\n… truncated",
                        super::clipped(&self.data, MAX_MESSAGE_BYTES - 32)
                    );
                }
                self.has_data = true;
            }
            "event" => self.name = super::clipped(value, 256),
            "id" if !value.contains('\0') => self.id = super::clipped(value, 512),
            _ => {}
        }
        self.overflow = false;
        self.line = bytes;
        self.line.clear();
        None
    }

    // Emit one completed SSE event and reset its per-event parsing state.
    fn dispatch(&mut self) -> Option<Event> {
        let name = std::mem::take(&mut self.name);
        // SSE comments and metadata without data must not produce a message.
        if !std::mem::take(&mut self.has_data) {
            return None;
        }
        Some(Event {
            // An unnamed SSE event uses the protocol default message type.
            name: if name.is_empty() {
                "message".into()
            } else {
                name
            },
            id: self.id.clone(),
            data: std::mem::take(&mut self.data),
        })
    }
}

// Append only within the SSE buffer limit before accepting more bytes.
fn append(output: &mut String, value: &str, limit: usize) {
    // Bound retained data and per-event work before accepting more content.
    if output.len() <= limit {
        output.push_str(&super::clipped(value, limit - output.len()));
    }
}

impl Network {
    // Route WebSocket lifecycle and frame events into their retained request.
    pub(super) fn socket_event(&mut self, method: &str, p: &Value, id: String) -> bool {
        // Route only the protocol event or reply owned by this handler.
        if method == "Network.webSocketCreated" {
            return self.socket_created(p, id);
        }
        // Keep stream-specific handling separate from normal retained request data.
        let Some(row) = self
            .rows
            .iter_mut()
            .rev()
            .find(|row| row.id == id && row.kind == "WebSocket")
        else {
            return false;
        };
        match method {
            "Network.webSocketWillSendHandshakeRequest" => {
                row.started = p["timestamp"].as_f64().unwrap_or(0.);
                row.request_headers = headers(&p["request"]["headers"]);
            }
            "Network.webSocketHandshakeResponseReceived" => {
                response(row, &p["response"]);
                // HTTP 101 identifies the successfully upgraded WebSocket request.
                if row.status == RequestStatus::Http(101) {
                    row.status = RequestStatus::Open;
                } else {
                    row.error = format!("WebSocket handshake failed: HTTP {}", row.status);
                }
            }
            "Network.webSocketClosed" => {
                row.ended = p["timestamp"].as_f64();
                // Create the request row that will own subsequent WebSocket messages.
                if row.error.is_empty() {
                    row.status = RequestStatus::Closed;
                }
            }
            "Network.webSocketFrameError" => {
                row.status = RequestStatus::Failed;
                row.error = field(p, "errorMessage");
            }
            "Network.webSocketFrameSent" | "Network.webSocketFrameReceived" => {
                let frame = &p["response"];
                let opcode = frame["opcode"].as_u64().unwrap_or(0);
                let payload = frame["payloadData"].as_str().unwrap_or_default();
                row.bytes += payload_size(opcode, payload) as f64;
                let message = Message {
                    key: row.key,
                    timestamp: p["timestamp"].as_f64().unwrap_or(row.started),
                    kind: Kind::Socket {
                        sent: method.ends_with("Sent"),
                        opcode,
                    },
                    payload: clipped(payload, MAX_MESSAGE_BYTES),
                };
                self.push_message(message);
            }
            _ => return false,
        }
        true
    }

    // Create the request row that will own subsequent WebSocket messages.
    fn socket_created(&mut self, p: &Value, id: String) -> bool {
        // Resolve the stable identity before using a row index or applying a retained reply.
        if self.rows.iter().any(|row| row.id == id) {
            return false;
        }
        self.record_request(
            &json!({"timestamp":0,"type":"WebSocket",
            "request":{"url":p["url"],"method":"GET"}}),
            id,
        );
        // Apply the optional result only while its owning state is available.
        if let Some(row) = self.rows.back_mut() {
            row.status = RequestStatus::Connecting;
            row.mime = "WebSocket".into();
        }
        true
    }
}

// Measure text payloads directly and decoded binary frames when needed.
fn payload_size(opcode: u64, payload: &str) -> usize {
    // Text frames already expose decoded text; binary frame length needs base64 decoding.
    if opcode == 1 {
        return payload.len();
    }
    let padding = payload
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    (payload.len() / 4 * 3).saturating_sub(padding)
}

const MAX_PENDING: usize = 64 * 1024;
pub(super) struct Stream {
    parser: Parser,
    pending: Option<i32>,
    chunks: VecDeque<(f64, String)>,
    queued: usize,
    pub error: Option<String>,
}
impl Stream {
    // Decode a bounded protocol stream payload into displayable text.
    fn decode(&mut self, key: u64, time: f64, data: &str) -> VecDeque<Message> {
        let mut messages = VecDeque::new();
        // Decode base64 stream chunks without exceeding retained-data limits.
        if self.error.is_some() {
            return messages;
        }
        // Decode in base64-aligned blocks, including large initial bufferedData.
        let mut buffer = [0; 6144];
        for block in data.as_bytes().chunks(8192) {
            // Decode base64 stream chunks without exceeding retained-data limits.
            let Ok(length) = BASE64_STANDARD.decode_slice(block, &mut buffer) else {
                self.error = Some("SSE preview unavailable: invalid stream encoding".into());
                break;
            };
            self.decode_bytes(key, time, &buffer[..length], &mut messages);
        }
        messages
    }

    // Decode base64 stream chunks without exceeding retained-data limits.
    fn decode_bytes(
        &mut self,
        key: u64,
        time: f64,
        bytes: &[u8],
        messages: &mut VecDeque<Message>,
    ) {
        for event in bytes.iter().filter_map(|byte| self.parser.byte(*byte)) {
            messages.push_back(Message {
                key,
                timestamp: time,
                kind: Kind::Event {
                    name: event.name,
                    id: event.id,
                },
                payload: event.data,
            });
            // Bound decoded messages before retaining more streamed content.
            if messages.len() > MAX_MESSAGES {
                messages.pop_front();
            }
        }
    }
}
impl Network {
    // Start stream.
    pub(super) fn start_stream(&mut self, id: &str) {
        // Resolve the stable identity before using a row index or applying a retained reply.
        let Some(row) = self.rows.iter().rev().find(|row| row.id == id) else {
            return;
        };
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if !row.is_sse() || row.kind == "EventSource" || self.streams.contains_key(&row.key) {
            return;
        }
        let command = protocol_id();
        self.streams.insert(
            row.key,
            Stream {
                parser: Parser::default(),
                pending: Some(command),
                chunks: VecDeque::new(),
                queued: 0,
                error: None,
            },
        );
        self.commands.push(json!({"id":command,"method":"Network.streamResourceContent","params":{"requestId":id}}));
    }

    // Translate a protocol EventSource message into the common stream record.
    pub(super) fn event_source(&mut self, p: &Value, id: &str) -> bool {
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        let Some(row) = self
            .rows
            .iter()
            .rev()
            .find(|row| row.id == id && row.kind == "EventSource")
        else {
            return false;
        };
        let message = Message {
            key: row.key,
            timestamp: p["timestamp"].as_f64().unwrap_or(row.started),
            kind: Kind::Event {
                name: clipped(
                    p["eventName"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .unwrap_or("message"),
                    256,
                ),
                id: clipped(p["eventId"].as_str().unwrap_or_default(), 512),
            },
            payload: clipped(p["data"].as_str().unwrap_or_default(), MAX_MESSAGE_BYTES),
        };
        self.push_message(message);
        true
    }

    // Consume decoded response bytes through the request's retained SSE parser.
    pub(super) fn stream_chunk(&mut self, p: &Value, id: &str) -> bool {
        // Keep stream-specific handling separate from normal retained request data.
        let Some(row) = self
            .rows
            .iter_mut()
            .rev()
            .find(|row| row.id == id && row.is_sse())
        else {
            return false;
        };
        row.bytes += p["encodedDataLength"].as_f64().unwrap_or(0.);
        // Keep stream-specific handling separate from normal retained request data.
        let Some(stream) = self.streams.get_mut(&row.key) else {
            return false;
        };
        let data = p["data"].as_str().unwrap_or_default();
        // Match a streamed response-body reply to its owning request.
        if data.is_empty() || stream.error.is_some() {
            return false;
        }
        let time = p["timestamp"].as_f64().unwrap_or(row.started);
        // Avoid duplicate in-flight work and wait for the reply that already owns this state.
        if stream.pending.is_some() {
            // Bound retained data and per-event work before accepting more content.
            if stream.queued + data.len() <= MAX_PENDING {
                stream.queued += data.len();
                stream.chunks.push_back((time, data.into()));
            } else {
                stream.error = Some("SSE preview incomplete: initial queue limit exceeded".into());
            }
            return true;
        }
        let messages = stream.decode(row.key, time, data);
        for message in messages {
            self.push_message(message);
        }
        true
    }

    // Match a streamed response-body reply to its owning request.
    pub(super) fn stream_reply(&mut self, id: i32, message: &Value) -> bool {
        // Avoid duplicate in-flight work and wait for the reply that already owns this state.
        let Some((&key, stream)) = self
            .streams
            .iter_mut()
            .find(|(_, stream)| stream.pending == Some(id))
        else {
            return false;
        };
        stream.pending = None;
        // Handle failure before exposing a partial result as usable state.
        if let Some(error) = message.get("error") {
            stream.chunks.clear();
            stream.queued = 0;
            stream.error = Some(format!(
                "SSE preview unavailable: {}",
                field(error, "message")
            ));
            return true;
        }
        let time = self
            .rows
            .iter()
            .find(|row| row.key == key)
            .map_or(0., |row| row.started);
        let mut messages = stream.decode(
            key,
            time,
            message["result"]["bufferedData"]
                .as_str()
                .unwrap_or_default(),
        );
        for (time, data) in std::mem::take(&mut stream.chunks) {
            messages.extend(stream.decode(key, time, &data));
            while messages.len() > MAX_MESSAGES {
                messages.pop_front();
            }
        }
        stream.queued = 0;
        for message in messages {
            self.push_message(message);
        }
        true
    }
}

#[cfg(test)]
#[path = "../../tests/sse.rs"]
mod sse_tests;

#[cfg(test)]
#[path = "../../tests/network_streams.rs"]
mod streams_tests;
