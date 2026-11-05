//! Format retained request details, source locations and timing without fetching additional data.
use super::*;
use std::fmt::Write;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Detail {
    #[default]
    Info,
    Headers,
    Payload,
    Response,
    Messages,
}
impl Detail {
    pub const ALL: [Self; 5] = [
        Self::Info,
        Self::Headers,
        Self::Payload,
        Self::Response,
        Self::Messages,
    ];

    // Resolve the declared Network detail section by its stable tab index.
    pub fn from_index(index: isize) -> Self {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
            .unwrap_or_default()
    }

    // Return the presentation title for the declared detail section.
    pub fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Headers => "Headers",
            Self::Payload => "Payload",
            Self::Response => "Response",
            Self::Messages => "Messages",
        }
    }
}
impl Network {
    // Report whether the selected request has data for this detail section.
    pub fn detail_available(&self, section: Detail) -> bool {
        // Resolve the stable identity before using a row index or applying a retained reply.
        let Some(row) = self
            .rows
            .iter()
            .find(|row| Some(row.key) == self.selected())
        else {
            return false;
        };
        match section {
            Detail::Messages => row.has_messages(),
            Detail::Payload | Detail::Response => row.kind != "WebSocket",
            _ => true,
        }
    }

    // Format the selected request section without fetching unrelated response data.
    pub fn details(&self, section: Detail) -> Rc<str> {
        // Resolve the stable identity before using a row index or applying a retained reply.
        let Some(row) = self
            .rows
            .iter()
            .find(|row| Some(row.key) == self.selected())
        else {
            return "".into();
        };
        let selection = self.selection.as_ref().expect("Retained selected request");
        match section {
            Detail::Info => format!(
                "{}\n\n{}\n\nInitiator\n{}",
                info(row),
                describe(row, self.axis()),
                row.initiator.description
            )
            .into(),
            Detail::Headers => request_headers(row).into(),
            Detail::Payload => selection
                .payload
                .get_or_init(|| {
                    // Cache the empty state too; an unrelated response cannot change request payload.
                    if row.payload.is_empty() {
                        "No request payload".into()
                    } else {
                        pretty(&row.payload).into()
                    }
                })
                .clone(),
            Detail::Response => self.response_body(row),
            Detail::Messages => self.stream_messages(row).into(),
        }
    }

    // Share the current immutable response preview with the native details cache.
    fn response_body(&self, row: &Request) -> Rc<str> {
        let selection = self.selection.as_ref().expect("Retained selected request");
        // A received preview remains readable after capture stops.
        if let Some(body) = &selection.response {
            return body.clone();
        }
        // A pending response belongs to this selection and is invalidated together with its key.
        if selection.response_request.is_some() {
            return "Loading response…".into();
        }
        // Fetch body text only when capture is active and the protocol can still provide it.
        if row.body_available && self.active {
            return "Load response to preview its body.\nUp to 512 KB; binary content is not displayed.".into();
        }
        "Response body is unavailable or has expired from Chromium’s buffer.".into()
    }
}

// Show wire metadata when available and retain provisional headers as the fallback.
fn request_headers(row: &Request) -> String {
    let mut output = String::new();
    for (label, basic, extra) in [
        ("Request", &row.request_headers, &row.extra.request),
        ("Response", &row.response_headers, &row.extra.response),
    ] {
        let (source, headers, cookies) = extra.as_ref().map_or(
            (
                "provisional; network-stack metadata unavailable",
                basic.as_str(),
                "Cookie details not reported",
            ),
            |info| {
                (
                    "network stack",
                    info.headers.as_str(),
                    info.cookies.as_str(),
                )
            },
        );
        output.push_str(&format!(
            "{label} headers · {source}\n{}\n",
            available(headers.trim_end())
        ));
        // Apply the optional result only while its owning state is available.
        if let Some(code) = extra.as_ref().and_then(|info| info.status) {
            output.push_str(&format!("Wire status  {code}\n"));
        }
        output.push_str(&format!(
            "\n{label} cookies · up to 64 entries\n{}\n\n",
            cookies.trim_end()
        ));
    }
    output
}

// Summarize timing availability and request metadata for the selected detail view.
fn info(row: &Request) -> String {
    let time = duration(row);
    let mut output = format!(
        "{} {}\n\nStatus  {}\nType    {}\nMIME    {}\nTime    {}\nSize    {}",
        row.method,
        row.url,
        row.status,
        row.kind,
        available(&row.mime),
        time,
        format_size(row.bytes)
    );
    // Report which retained timing fields can be displayed.
    if !row.error.is_empty() {
        output.push_str(&format!("\n\n{}", row.error));
    }
    // Show query parameters only for a parsed URL with an actual query.
    if let Ok(url) = url::Url::parse(&row.url)
        && url.query().is_some()
    {
        output.push_str("\n\nQuery parameters\n");
        for (name, value) in url.query_pairs() {
            output.push_str(&format!("{name}: {value}\n"));
        }
    }
    output
}

// Report which retained timing fields can be displayed.
fn available(value: &str) -> &str {
    // Describe an unavailable protocol field instead of rendering an empty value.
    if value.is_empty() { "—" } else { value }
}

// Stop JSON indentation at the preview byte budget instead of allocating the entire expanded document.
struct Preview(String);
impl fmt::Write for Preview {
    // Preserve UTF-8 boundaries and stop the serializer as soon as its next fragment exceeds the budget.
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let mut end = value.len().min(MAX_BODY - self.0.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push_str(&value[..end]);
        // An error terminates serde's formatter; it must not continue traversing an oversized document.
        if end < value.len() {
            return Err(fmt::Error);
        }
        Ok(())
    }
}

// Render a bounded JSON preview; already bounded non-JSON input keeps its original text.
pub(super) fn pretty(value: &str) -> String {
    // Truncated bodies and non-JSON responses remain useful without a failed formatting attempt.
    let Ok(parsed) = serde_json::from_str::<Value>(value) else {
        return value.into();
    };
    let mut output = Preview(String::with_capacity(value.len().min(MAX_BODY)));
    // The writer's only error means the preview is full, rather than a protocol or parsing failure.
    if write!(output, "{parsed:#}").is_err() {
        output.0.push_str("\n… truncated");
    }
    output.0
}

pub(super) struct Initiator {
    pub label: Rc<str>,
    pub description: Rc<str>,
}
impl Initiator {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(value: &Value) -> Self {
        let mut stack = &value["stack"];
        for _ in 0..4 {
            let frame = stack["callFrames"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|frame| frame["url"].as_str().is_some_and(|url| !url.is_empty()));
            // Prefer the first source-bearing stack frame before searching its asynchronous parent.
            if let Some(frame) = frame {
                return location(frame, "Script");
            }
            stack = &stack["parent"];
        }
        let kind = value["type"]
            .as_str()
            .filter(|kind| !kind.is_empty())
            .unwrap_or("Unknown");
        location(value, kind)
    }
}

// Format a source position for the request initiator display.
fn location(value: &Value, kind: &str) -> Initiator {
    let url = value["url"].as_str().filter(|url| !url.is_empty());
    // Reject a missing or mismatched value before the following operation relies on it.
    let Some(url) = url else {
        return Initiator {
            label: clipped(kind, 64).into(),
            description: clipped(kind, 64).into(),
        };
    };
    let mut position = String::new();
    for key in ["lineNumber", "columnNumber"] {
        // Resolve the stable identity before using a row index or applying a retained reply.
        if let Some(index) = value[key].as_u64() {
            position.push_str(&format!(":{}", index.saturating_add(1)));
        }
    }
    let name = request_name(url);
    let function = field(value, "functionName");
    Initiator {
        label: clipped(&format!("{name}{position}"), 512).into(),
        description: clipped(&format!("{kind}\n{url}{position}\n{function}"), 4096).into(),
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
    pub ongoing: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Axis {
    pub start: f64,
    pub duration: f64,
}
impl Network {
    // Compute a stable time axis spanning the retained requests.
    pub(super) fn axis(&self) -> Option<Axis> {
        let mut start = f64::INFINITY;
        let mut end = self.observed;
        for row in &self.rows {
            // Exclude requests without a known start time from the timeline origin.
            if row.started > 0. {
                start = start.min(row.started);
            }
            end = end.max(row.ended.unwrap_or(end));
        }
        start.is_finite().then_some(Axis {
            start,
            duration: (end - start).max(0.001),
        })
    }

    // Compute the visible timeline interval from retained request timing.
    pub fn timeline_bounds(&self) -> (f64, f64) {
        self.axis()
            .map_or((0., 0.), |axis| (axis.start, axis.duration))
    }
}

// Map request timing into a bounded duration interval.
pub(super) fn span(row: &Request, observed: f64) -> Option<Span> {
    (row.started > 0.).then_some(Span {
        start: row.started,
        end: row.ended.unwrap_or(observed).max(row.started),
        ongoing: row.ended.is_none(),
    })
}

// Explain the retained timing phases and unavailable protocol fields.
pub(super) fn describe(row: &Request, axis: Option<Axis>) -> String {
    // Reject a missing or mismatched value before the following operation relies on it.
    let Some(axis) = axis.filter(|_| row.started > 0.) else {
        return "Timing not reported".into();
    };
    let start = row.started - axis.start;
    let end = row
        .ended
        .map_or("ongoing · through latest observed event".into(), |end| {
            format!("+{:.6} s", (end - axis.start).max(start))
        });
    format!(
        "Start  +{start:.6} s\nFinish {end}\nDuration {}\nRelative to earliest retained request",
        duration(row)
    )
}

// Format transferred bytes using a compact readable unit for table cells and details.
pub(super) fn format_size(bytes: f64) -> String {
    let bytes = bytes.max(0.);
    // Keep sub-kilobyte payloads readable without rounding them down to zero KB.
    if bytes < 1024. {
        return format!("{bytes:.0} B");
    }
    // Switch to megabytes only after a full megabyte has been transferred.
    if bytes < 1024. * 1024. {
        return format!("{:.1} KB", bytes / 1024.);
    }
    format!("{:.1} MB", bytes / (1024. * 1024.))
}

// Describe a request's completed or ongoing timing span.
pub(super) fn duration(row: &Request) -> String {
    // Apply the optional result only while its owning state is available.
    if let Some(end) = row.ended {
        return elapsed((end - row.started).max(0.));
    }
    // An upgraded or connecting socket has a live duration until its close timestamp arrives.
    if row.kind == "WebSocket"
        && matches!(row.status, RequestStatus::Open | RequestStatus::Connecting)
    {
        return "Live".into();
    }
    // An accepted event stream stays live until capture stops or the connection ends.
    if row.is_sse() && row.status == RequestStatus::Http(200) {
        return "Live".into();
    }
    "—".into()
}

// Choose readable units for request durations.
pub(crate) fn elapsed(seconds: f64) -> String {
    // Choose readable duration units without hiding short request timings.
    if seconds < 1. {
        return format!("{:.0} ms", seconds * 1000.);
    }
    // Choose readable duration units without hiding short request timings.
    if seconds < 60. {
        return format!("{seconds:.2} s");
    }
    // Choose readable duration units without hiding short request timings.
    if seconds < 3600. {
        return format!("{:.1} min", seconds / 60.);
    }
    format!("{:.1} h", seconds / 3600.)
}

#[cfg(test)]
#[path = "../../tests/network_timing.rs"]
mod timing_tests;
