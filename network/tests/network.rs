//! Test network behavior and guard its lifecycle and boundary cases.
use super::*;

// Verify event.
fn event(method: &str, params: Value) -> Value {
    json!({"method":format!("Network.{method}"),"params":params})
}

// Verify request.
fn request(network: &mut Network, id: &str, url: &str) {
    network.receive(&event("requestWillBeSent",json!({"requestId":id,"timestamp":1,"type":"Fetch","request":{"url":url,"method":"GET","headers":{"Accept":"application/json"}}})));
}

// Verify redirects keep separate rows and bodies are lazy and scoped to selection.
#[test]
fn redirects_keep_separate_rows_and_bodies_are_lazy_and_scoped_to_selection() {
    let mut network = Network::default();
    network.set_active(true);
    network.drain();
    request(&mut network, "a", "https://example.test/old");
    network.receive(&event("requestWillBeSent",json!({"requestId":"a","timestamp":1.1,"redirectResponse":{"status":302},"request":{"url":"https://example.test/new","method":"GET"}})));
    network.receive(&event("responseReceived",json!({"requestId":"a","response":{"status":200,"mimeType":"application/json","headers":{"Content-Type":"application/json"}}})));
    network.receive(&event(
        "loadingFinished",
        json!({"requestId":"a","timestamp":1.2,"encodedDataLength":512}),
    ));
    let rows = network.rows("");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].status, RequestStatus::Http(302));
    assert_eq!(rows[1].time, "100 ms");
    network.select(Some(rows[0].key));
    assert!(!network.can_load_body());
    network.select(Some(rows[1].key));
    assert!(network.can_load_body());
    assert!(network.drain().is_empty());
    network.load_body();
    let command = network.drain().remove(0);
    assert_eq!(command["method"], "Network.getResponseBody");
    network.clear();
    network.receive(&json!({"id":command["id"],"result":{"body":"stale"}}));
    assert!(network.rows("").is_empty());
    assert!(!network.details(Detail::Info).contains("stale"));
}

// Verify capture and body retention are bounded and failures stay visible.
#[test]
fn capture_and_body_retention_are_bounded_and_failures_stay_visible() {
    let mut network = Network::default();
    network.set_active(true);
    network.drain();
    for i in 0..350 {
        request(&mut network, &i.to_string(), "https://example.test/data");
    }
    assert_eq!(network.rows("").len(), MAX_REQUESTS);
    network.receive(&event(
        "loadingFailed",
        json!({"requestId":"349","timestamp":2,"errorText":"net::ERR_FAILED"}),
    ));
    assert_eq!(network.rows("failed").len(), 1);
    network.select(Some(350));
    assert!(!network.can_load_body());
    assert!(network.details(Detail::Info).contains("net::ERR_FAILED"));
    network.receive(&event(
        "loadingFinished",
        json!({"requestId":"348","timestamp":2}),
    ));
    network.select(Some(349));
    network.load_body();
    let command = network.drain().remove(0);
    network.receive(&json!({"id":command["id"],"result":{"body":"🦀".repeat(MAX_BODY)}}));
    assert!(network.details(Detail::Response).len() < MAX_BODY + 32);
    network.select(Some(350));
    assert!(network.selection.as_ref().unwrap().response.is_none());
    network.set_active(false);
    network.drain();
    request(&mut network, "new", "https://example.test/new");
    assert!(network.rows("new").is_empty());
}

// Verify long request names remain available to column resizing and tooltips.
#[test]
fn long_request_names_remain_available_to_column_resizing_and_tooltips() {
    let mut network = Network::default();
    network.set_active(true);
    let name = format!("{}.json", "very-long-request-name-".repeat(20));
    request(
        &mut network,
        "long",
        &format!("https://example.test/{name}"),
    );
    let rows = network.rows("");
    assert_eq!(rows[0].name.as_ref(), name);
    assert!(!rows[0].name.contains('\n'));
    network.select(Some(rows[0].key));
    assert!(network.details(Detail::Info).contains(&name));
}

// Verify stopping capture invalidates bodies including inflight replies.
#[test]
fn stopping_capture_invalidates_bodies_including_inflight_replies() {
    let mut network = Network::default();
    network.set_active(true);
    network.drain();
    request(&mut network, "one", "https://example.test/one");
    network.receive(&event(
        "loadingFinished",
        json!({"requestId":"one","timestamp":2}),
    ));
    network.select(Some(1));
    network.load_body();
    let body = network.drain().remove(0);
    network.set_active(false);
    network.set_active(true);
    assert!(!network.can_load_body());
    assert!(!network.receive(&json!({"id":body["id"],"result":{"body":"obsolete"}})));
    assert!(!network.details(Detail::Response).contains("obsolete"));
    request(&mut network, "two", "https://example.test/two");
    network.receive(&event(
        "loadingFinished",
        json!({"requestId":"two","timestamp":3}),
    ));
    network.select(Some(2));
    assert!(network.can_load_body());
    let enable = network.drain().pop().unwrap();
    network.receive(&json!({"id":enable["id"],"error":{"message":"unavailable"}}));
    assert!(!network.active && !network.can_load_body());
}

// Verify request sections keep headers payload and formatted response separate.
#[test]
fn request_sections_keep_headers_payload_and_formatted_response_separate() {
    let mut network = Network::default();
    network.set_active(true);
    network.drain();
    network.receive(&event(
        "requestWillBeSent",
        json!({
            "requestId":"post", "timestamp":1, "type":"Fetch",
            "request":{"url":"https://example.test/data?q=hello%20world&tag=a&tag=b",
                "method":"POST", "headers":{"X-Request":"sent"}, "postData":"{\"count\":2}"}
        }),
    ));
    network.receive(&event("responseReceived", json!({"requestId":"post",
        "response":{"status":200,"mimeType":"application/json","headers":{"X-Response":"received"}}})));
    network.receive(&event(
        "loadingFinished",
        json!({"requestId":"post","timestamp":1.25}),
    ));
    network.select(Some(1));
    let info = network.details(Detail::Info);
    assert!(info.contains("q: hello world\ntag: a\ntag: b"));
    assert!(!info.contains("X-Request"));
    let headers = network.details(Detail::Headers);
    assert!(headers.contains("X-Request: sent") && headers.contains("X-Response: received"));
    assert!(
        network
            .details(Detail::Payload)
            .contains("\n  \"count\": 2\n")
    );
    assert!(network.drain().is_empty());
    network.load_body();
    let command = network.drain().remove(0);
    network.receive(&json!({"id":command["id"],"result":{"body":"{\"ok\":true,\"items\":[1,2]}"}}));
    let response = network.details(Detail::Response);
    assert!(response.contains("\n  \"items\": [\n"));
    assert!(!response.contains("X-Request") && !response.contains("count"));
}

// Verify socket.
fn socket(network: &mut Network, id: &str) {
    network.receive(&event(
        "webSocketCreated",
        json!({"requestId":id,"url":format!("wss://example.test/{id}")}),
    ));
    network.receive(&event(
        "webSocketWillSendHandshakeRequest",
        json!({"requestId":id,"timestamp":10,
        "request":{"headers":{"Upgrade":"websocket"}}}),
    ));
    network.receive(&event(
        "webSocketHandshakeResponseReceived",
        json!({"requestId":id,"timestamp":10.1,
        "response":{"status":101,"headers":{"Sec-WebSocket-Protocol":"test"}}}),
    ));
}

// Verify frame.
fn frame(network: &mut Network, id: &str, sent: bool, opcode: u64, payload: &str) -> bool {
    network.receive(&event(if sent {"webSocketFrameSent"} else {"webSocketFrameReceived"},
        json!({"requestId":id,"timestamp":10.25,"response":{"opcode":opcode,"payloadData":payload}})))
}

// Verify websocket messages keep direction binary encoding and handshake without fetching bodies.
#[test]
fn websocket_messages_keep_direction_binary_encoding_and_handshake_without_fetching_bodies() {
    let mut network = Network::default();
    network.set_active(true);
    network.drain();
    socket(&mut network, "ws");
    assert_eq!(network.rows("")[0].status, RequestStatus::Open);
    assert_eq!(network.rows("")[0].time, "Live");
    assert!(frame(&mut network, "ws", true, 1, "hello"));
    frame(&mut network, "ws", false, 1, "{\"ok\":true}");
    frame(&mut network, "ws", false, 2, "AQID");
    network.select(Some(1));
    assert!(network.detail_available(Detail::Messages));
    let messages = network.details(Detail::Messages);
    assert!(messages.contains("0.250 s  ↑ Sent  Text\nhello"));
    assert!(messages.contains("↓ Received  Binary · base64\nAQID"));
    assert_eq!(network.rows("")[0].size, "19 B");
    assert!(
        network
            .details(Detail::Headers)
            .contains("Sec-WebSocket-Protocol: test")
    );
    assert!(!network.can_load_body());
    network.load_body();
    assert!(network.drain().is_empty());
    network.receive(&event(
        "webSocketClosed",
        json!({"requestId":"ws","timestamp":10.5}),
    ));
    assert_eq!(network.rows("")[0].status, RequestStatus::Closed);
    assert_eq!(network.rows("")[0].time, "500 ms");
    request(&mut network, "http", "https://example.test/api");
    network.select(Some(2));
    assert!(!network.detail_available(Detail::Messages));
    assert!(!network.details(Detail::Messages).contains("hello"));
}

// Verify websocket history is bounded across connections and evicted with the request.
#[test]
fn websocket_history_is_bounded_across_connections_and_evicted_with_the_request() {
    let mut network = Network::default();
    network.set_active(true);
    socket(&mut network, "one");
    socket(&mut network, "two");
    for i in 0..240 {
        frame(
            &mut network,
            if i % 2 == 0 { "one" } else { "two" },
            false,
            1,
            &format!("message-{i}"),
        );
    }
    assert_eq!(network.messages.len(), 200);
    frame(&mut network, "one", false, 1, &"🦀".repeat(4096));
    // The preview remains valid UTF-8 and is bounded even for multibyte input.
    assert!(network.details(Detail::Messages).is_empty());
    network.select(Some(1));
    let preview = network.details(Detail::Messages);
    assert!(preview.contains("truncated"));
    assert!(preview.len() < 16384);
    for i in 0..300 {
        request(
            &mut network,
            &format!("http-{i}"),
            "https://example.test/api",
        );
    }
    assert!(network.messages.is_empty());
    assert!(!frame(&mut network, "one", false, 1, "late"));
    network.clear();
    assert!(network.messages.is_empty());
}

// Verify websocket errors survive closing and stopped capture ignores messages.
#[test]
fn websocket_errors_survive_closing_and_stopped_capture_ignores_messages() {
    let mut network = Network::default();
    network.set_active(true);
    socket(&mut network, "failed");
    network.receive(&event(
        "webSocketFrameError",
        json!({"requestId":"failed","timestamp":10.4,"errorMessage":"Connection reset"}),
    ));
    network.receive(&event(
        "webSocketClosed",
        json!({"requestId":"failed","timestamp":10.5}),
    ));
    network.select(Some(1));
    assert_eq!(network.rows("")[0].status, RequestStatus::Failed);
    assert!(network.details(Detail::Info).contains("Connection reset"));
    socket(&mut network, "live");
    network.set_active(false);
    assert_eq!(network.rows("")[1].status, RequestStatus::Stopped);
    assert_eq!(network.rows("")[1].time, "—");
    assert!(!frame(&mut network, "live", false, 1, "not captured"));
    assert!(network.messages.is_empty());
}

// Verify switching and deselecting requests invalidates pending response previews.
#[test]
fn switching_and_deselecting_requests_invalidates_pending_response_previews() {
    let mut network = Network::default();
    network.set_active(true);
    network.drain();
    for id in ["one", "two"] {
        request(&mut network, id, &format!("https://example.test/{id}"));
        network.receive(&event(
            "loadingFinished",
            json!({"requestId":id,"timestamp":2}),
        ));
    }
    network.select(Some(1));
    network.load_body();
    let first = network.drain().remove(0);
    network.select(Some(2));
    assert!(network.can_load_body());
    network.load_body();
    let second = network.drain().remove(0);
    assert!(!network.receive(&json!({"id":first["id"],"result":{"body":"obsolete"}})));
    network.receive(&json!({"id":second["id"],"result":{"body":"current"}}));
    assert_eq!(network.details(Detail::Response).as_ref(), "current");
    network.select(None);
    assert!(network.details(Detail::Response).is_empty());
    assert!(!network.can_load_body());
}

// Verify compact type labels can be used in the request filter.
#[test]
fn compact_type_labels_can_be_used_in_the_request_filter() {
    let mut network = Network::default();
    network.set_active(true);
    network.receive(&event(
        "requestWillBeSent",
        json!({"requestId":"style","type":"Stylesheet",
        "request":{"url":"https://example.test/asset","method":"GET"}}),
    ));
    let rows = network.rows("CSS");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "CSS");
}

// Keep immutable request text shared while a response can still enable another detail section.
#[test]
fn cached_payload_does_not_freeze_request_metadata_or_stream_availability() {
    let mut network = Network::default();
    network.set_active(true);
    network.receive(&event(
        "requestWillBeSent",
        json!({"requestId":"stream","type":"Fetch","timestamp":1,
        "request":{"url":"https://example.test/events","method":"POST","postData":"{\"topic\":\"news\"}"},
        "initiator":{"type":"script","url":"https://example.test/app.js","lineNumber":2}}),
    ));
    network.select(Some(1));
    let before = network.rows("");
    let payload = network.details(Detail::Payload);
    let cached = Rc::downgrade(&payload);
    assert_eq!(payload.as_ref(), "{\n  \"topic\": \"news\"\n}");
    assert!(!network.detail_available(Detail::Messages));
    network.receive(&event(
        "responseReceived",
        json!({"requestId":"stream","response":{"status":200,"mimeType":"text/event-stream"}}),
    ));
    let after = network.rows("");
    assert!(Rc::ptr_eq(&before[0].name, &after[0].name));
    assert!(Rc::ptr_eq(&before[0].method, &after[0].method));
    assert!(Rc::ptr_eq(&before[0].initiator, &after[0].initiator));
    assert!(Rc::ptr_eq(&before[0].source, &after[0].source));
    assert!(network.detail_available(Detail::Messages));
    assert!(Rc::ptr_eq(&payload, &network.details(Detail::Payload)));
    drop(payload);
    network.select(None);
    assert!(cached.upgrade().is_none());
}

// Malformed protocol numbers cannot silently round into a valid HTTP response or error state.
#[test]
fn response_status_remains_numeric_until_presented() {
    let mut network = Network::default();
    network.set_active(true);
    for (index, (code, expected)) in [
        (json!(200.0), RequestStatus::Http(200)),
        (json!(404), RequestStatus::Http(404)),
        (json!(200.5), RequestStatus::Unknown),
        (json!(-1), RequestStatus::Unknown),
        (json!(65536), RequestStatus::Unknown),
        (Value::Null, RequestStatus::Unknown),
    ]
    .into_iter()
    .enumerate()
    {
        let id = index.to_string();
        request(&mut network, &id, "https://example.test/api");
        network.receive(&event(
            "responseReceived",
            json!({"requestId":id,"response":{"status":code}}),
        ));
        assert_eq!(network.rows("")[index].status, expected);
    }
    let errors: Vec<_> = network
        .rows("")
        .into_iter()
        .filter(|row| row.status.is_error())
        .map(|row| row.key)
        .collect();
    assert_eq!(errors, [2]);
    assert_eq!(network.rows("404")[0].key, 2);
}

// Readable indentation must obey the preview limit even when compact input fits in the capture buffer.
#[test]
fn pretty_response_expansion_preserves_the_byte_limit_and_unicode() {
    let value = json!({"items":vec!["🦀"; 60000]});
    let compact = value.to_string();
    let expanded = serde_json::to_string_pretty(&value).unwrap();
    assert!(compact.len() < MAX_BODY && expanded.len() > MAX_BODY);
    let mut network = Network::default();
    network.set_active(true);
    network.drain();
    request(&mut network, "large", "https://example.test/large");
    network.receive(&event(
        "loadingFinished",
        json!({"requestId":"large","timestamp":2}),
    ));
    network.select(Some(1));
    network.load_body();
    let command = network.drain().remove(0);
    assert!(network.receive(&json!({"id":command["id"],"result":{"body":compact}})));
    let response = network.details(Detail::Response);
    assert_eq!(response.as_ref(), clipped(&expanded, MAX_BODY));
    assert!(Rc::ptr_eq(&response, &network.details(Detail::Response)));
}
