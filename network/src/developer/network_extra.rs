//! Network-stack metadata can precede base events; match each redirect hop in order.
use super::*;
const MAX_METADATA: usize = 16 * 1024;
#[derive(Default)]
pub(super) struct Metadata {
    pub expected: Option<bool>,
    pub request: Option<Info>,
    pub response: Option<Info>,
}
#[derive(Default)]
pub(super) struct Info {
    pub headers: String,
    pub cookies: String,
    pub status: Option<u16>,
}
struct Pending {
    id: String,
    response: bool,
    info: Info,
}
impl Pending {
    // Merge extra request or response metadata into the retained row.
    fn apply(&mut self, row: &mut Request) {
        let info = std::mem::take(&mut self.info);
        // Wire codes can refine HTTP state, but must not revive failed, closed or stopped streams.
        if self.response
            && matches!(
                row.status,
                RequestStatus::Pending | RequestStatus::Http(_) | RequestStatus::Unknown
            )
            && let Some(code) = info.status
        {
            row.status = RequestStatus::Http(code);
        }
        // Request and response extra metadata populate different retained fields.
        let slot = if self.response {
            &mut row.extra.response
        } else {
            &mut row.extra.request
        };
        *slot = Some(info);
    }
}
#[derive(Default)]
pub(super) struct Extras(VecDeque<Pending>);
impl Extras {
    // Route incoming protocol data to its owner and ignore replies from stale work.
    pub fn receive(&mut self, p: &Value, response: bool) {
        self.0.push_back(Pending {
            id: field(p, "requestId"),
            response,
            info: Info {
                headers: p["headersText"].as_str().map_or_else(
                    || headers(&p["headers"]),
                    |value| clipped(value, MAX_METADATA),
                ),
                // Keep request and response extras separate until their owning row arrives.
                cookies: if response {
                    received_cookies(p)
                } else {
                    sent_cookies(p)
                },
                status: status_code(&p["statusCode"]),
            },
        });
        // Bound unmatched metadata when a request never materializes.
        if self.0.len() > MAX_REQUESTS {
            self.0.pop_front();
        }
    }

    // Pair out-of-order extra metadata with its request owner.
    pub fn reconcile(&mut self, rows: &mut VecDeque<Request>, id: &str) {
        // There is no deferred metadata to reconcile.
        if self.0.is_empty() {
            return;
        }
        // Index available redirect hops once. Consume metadata FIFO in one pass,
        // rather than searching all requests for every queued metadata event.
        let mut slots = [VecDeque::new(), VecDeque::new()];
        for (index, row) in rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.id == id && row.extra.expected == Some(true))
        {
            // Merge request extras only once into their retained row.
            if row.extra.request.is_none() {
                slots[0].push_back(index);
            }
            // Merge response extras only once into their retained row.
            if row.extra.response.is_none() {
                slots[1].push_back(index);
            }
        }
        self.0.retain_mut(|pending| {
            // Avoid duplicate in-flight work and wait for the reply that already owns this state.
            if pending.id != id {
                return true;
            }
            // Avoid duplicate in-flight work and wait for the reply that already owns this state.
            let Some(index) = slots[usize::from(pending.response)].pop_front() else {
                return true;
            };
            pending.apply(&mut rows[index]);
            false
        });
    }

    // Discard unmatched metadata when network capture is cleared.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    // Release unmatched extra metadata when its request leaves retention.
    pub fn forget(&mut self, id: &str) {
        self.0.retain(|pending| pending.id != id);
    }
}

// Find a header without depending on the server's choice of case.
pub(super) fn header<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value
        .as_object()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))?
        .1
        .as_str()
}

// Format protocol blocking reasons for request details.
fn reasons(value: &Value) -> String {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .filter_map(Value::as_str)
        .map(|value| clipped(value, 128))
        .collect::<Vec<_>>()
        .join(", ")
}

// Format a cookie from retained protocol fields for readable details.
fn cookie(value: &Value) -> String {
    let mut line = format!(
        "{}={}; Domain={}; Path={}",
        field(value, "name"),
        clipped(value["value"].as_str().unwrap_or_default(), 2048),
        field(value, "domain"),
        field(value, "path")
    );
    for key in ["secure", "httpOnly"] {
        // Resolve the stable identity before using a row index or applying a retained reply.
        if value[key].as_bool() == Some(true) {
            line.push_str(&format!("; {key}"));
        }
    }
    // Include SameSite only when the protocol supplied it.
    if let Some(same_site) = value["sameSite"].as_str() {
        line.push_str(&format!("; SameSite={same_site}"));
    }
    // Session cookies have no persistent expiry timestamp to display.
    if value["session"].as_bool() == Some(true) {
        line.push_str("; Session");
    }
    // Resolve the stable identity before using a row index or applying a retained reply.
    if let Some(partition) = value.get("partitionKey") {
        line.push_str(&format!(
            "; Partition={}",
            clipped(&partition.to_string(), 512)
        ));
    }
    clipped(&line, 4096)
}

// Describe included and excluded request cookies with their blocking reasons.
fn sent_cookies(p: &Value) -> String {
    let mut output = String::new();
    for entry in p["associatedCookies"]
        .as_array()
        .into_iter()
        .flatten()
        .take(64)
    {
        // Bound retained data and per-event work before accepting more content.
        if output.len() >= MAX_METADATA {
            break;
        }
        let blocked = reasons(&entry["blockedReasons"]);
        // Apply the optional result only while its owning state is available.
        let state = if let Some(exemption) = entry["exemptionReason"].as_str() {
            format!("Sent · exception: {}", clipped(exemption, 128))
        } else if blocked.is_empty() {
            "Sent".into()
        } else {
            format!("Not sent · {blocked}")
        };
        output.push_str(&format!("{state}\n{}\n\n", cookie(&entry["cookie"])));
    }
    // Explain the empty cookie result instead of rendering a blank detail section.
    if output.is_empty() {
        return "No associated cookies reported".into();
    }
    clipped(&output, MAX_METADATA)
}

// Describe response cookies without duplicating Set-Cookie entries.
fn received_cookies(p: &Value) -> String {
    let mut output = String::new();
    let mut count = 0;
    let blocked = p["blockedCookies"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let exempted = p["exemptedCookies"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut blocked_by_line = HashMap::new();
    let mut exempted_by_line = HashMap::new();
    for (entries, index) in [
        (blocked, &mut blocked_by_line),
        (exempted, &mut exempted_by_line),
    ] {
        for entry in entries {
            // Only descriptors with a wire header can match a received Set-Cookie line; preserve the first match.
            if let Some(line) = entry["cookieLine"].as_str() {
                index.entry(line).or_insert(entry);
            }
        }
    }
    let mut received = std::collections::HashSet::new();
    for line in header(&p["headers"], "set-cookie")
        .unwrap_or_default()
        .lines()
        .take(64)
    {
        // Bound retained data and per-event work before accepting more content.
        if output.len() >= MAX_METADATA {
            break;
        }
        received.insert(line);
        let blocked = blocked_by_line.get(line);
        let exempted = exempted_by_line.get(line);
        // Apply the optional result only while its owning state is available.
        let state = if let Some(entry) = blocked {
            format!("Blocked · {}", reasons(&entry["blockedReasons"]))
        } else if let Some(entry) = exempted {
            format!("Allowed by exception · {}", field(entry, "exemptionReason"))
        } else {
            "Received · storage not confirmed".into()
        };
        output.push_str(&format!("{state}\n{}\n\n", clipped(line, 4096)));
        count += 1;
    }
    // Invalid Set-Cookie lines may exist only in blockedCookies, not in headers.
    for entry in blocked.iter().take(64) {
        // Bound retained data and per-event work before accepting more content.
        if output.len() >= MAX_METADATA || count >= 64 {
            break;
        }
        let line = entry["cookieLine"].as_str().unwrap_or_default();
        // Avoid duplicating cookies already represented in Set-Cookie headers.
        if received.contains(line) {
            continue;
        }
        output.push_str(&format!(
            "Blocked · {}\n{}\n\n",
            reasons(&entry["blockedReasons"]),
            clipped(line, 4096)
        ));
        count += 1;
    }
    // Explain the empty cookie result instead of rendering a blank detail section.
    if output.is_empty() {
        return "No Set-Cookie headers reported".into();
    }
    clipped(&output, MAX_METADATA)
}
#[cfg(test)]
#[path = "../../tests/network_extra.rs"]
mod tests;
