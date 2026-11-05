//! Test network streams behavior and guard its lifecycle and boundary cases.
use super::*;

// Verify event.
fn event(network: &mut Network, method: &str, params: Value) -> bool {
    network.receive(&json!({"method":format!("Network.{method}"),"params":params}))
}

// Verify start.
fn start(network: &mut Network, kind: &str) -> Vec<Value> {
    network.set_active(true);
    network.drain();
    event(
        network,
        "requestWillBeSent",
        json!({"requestId":"sse","type":kind,"timestamp":1,"request":{"url":"https://example.test/events","method":"GET"}}),
    );
    event(
        network,
        "responseReceived",
        json!({"requestId":"sse","hasExtraInfo":false,"response":{"status":200,"mimeType":"text/event-stream; charset=utf-8"}}),
    );
    network.select(Some(1));
    network.drain()
}

// Verify chunk.
fn chunk(network: &mut Network, data: &[u8]) -> bool {
    event(
        network,
        "dataReceived",
        json!({"requestId":"sse","timestamp":2,"encodedDataLength":data.len(),"data":BASE64_STANDARD.encode(data)}),
    )
}

// Verify event source uses native events without streaming or body fetches.
#[test]
fn event_source_uses_native_events_without_streaming_or_body_fetches() {
    let mut network = Network::default();
    assert!(start(&mut network, "EventSource").is_empty());
    assert!(network.detail_available(Detail::Messages));
    for i in 0..250 {
        event(
            &mut network,
            "eventSourceMessageReceived",
            json!({"requestId":"sse","timestamp":2,"eventName":"update","eventId":i.to_string(),"data":format!("event-{i}")}),
        );
    }
    assert_eq!(network.messages.len(), MAX_MESSAGES);
    let detail = network.details(Detail::Messages);
    assert!(!detail.contains("ID  0\n"));
    assert!(detail.contains("ID  249\nevent-249"));
    assert_eq!(network.rows("SSE")[0].time, "Live");
    network.set_active(false);
    assert_eq!(network.rows("")[0].status, RequestStatus::Stopped);
    assert_eq!(network.rows("")[0].time, "—");
    assert!(!event(
        &mut network,
        "eventSourceMessageReceived",
        json!({"requestId":"sse","data":"late"})
    ));
    network.clear();
    assert!(network.messages.is_empty());
}

// Verify closing event source is not reported as a network failure.
#[test]
fn closing_event_source_is_not_reported_as_a_network_failure() {
    let mut network = Network::default();
    start(&mut network, "EventSource");
    event(
        &mut network,
        "loadingFailed",
        json!({"requestId":"sse","timestamp":3,"canceled":true,"errorText":"net::ERR_ABORTED"}),
    );
    assert_eq!(network.rows("")[0].status, RequestStatus::Closed);
    assert!(!network.details(Detail::Info).contains("ERR_ABORTED"));
}

// Verify fetch sse combines initial buffer and early chunks in order without utf8 damage.
#[test]
fn fetch_sse_combines_initial_buffer_and_early_chunks_in_order_without_utf8_damage() {
    let mut network = Network::default();
    let command = start(&mut network, "Fetch").remove(0);
    assert_eq!(command["method"], "Network.streamResourceContent");
    let data = "data: 🦀\r\n\r\nid: 2\nevent: next\ndata: second\n\n".as_bytes();
    chunk(&mut network, &data[8..]);
    network.receive(
        &json!({"id":command["id"],"result":{"bufferedData":BASE64_STANDARD.encode(&data[..8])}}),
    );
    let detail = network.details(Detail::Messages);
    assert!(detail.contains("↓ SSE  message\nID  —\n🦀"));
    assert!(detail.contains("↓ SSE  next\nID  2\nsecond"));
    assert!(detail.find("🦀").unwrap() < detail.find("second").unwrap());
    assert_eq!(network.rows("sse").len(), 1);
    assert!(network.detail_available(Detail::Payload));
    assert!(network.drain().is_empty());
}

// Verify large initial sse buffer preserves unicode order and the history limit.
#[test]
fn large_initial_sse_buffer_preserves_unicode_order_and_the_history_limit() {
    let mut network = Network::default();
    let command = start(&mut network, "Fetch").remove(0);
    let data: String = (0..1000)
        .map(|id| format!("id: {id}\ndata: 🦀-{id}\n\n"))
        .collect();
    network.receive(
        &json!({"id":command["id"],"result":{"bufferedData":BASE64_STANDARD.encode(data)}}),
    );
    assert!(network.streams[&1].error.is_none());
    assert_eq!(network.messages.len(), MAX_MESSAGES);
    assert_eq!(network.messages.front().unwrap().payload, "🦀-800");
    assert_eq!(network.messages.back().unwrap().payload, "🦀-999");
}

// Verify clearing a stream rejects pending replies and limits preinitialization data.
#[test]
fn clearing_a_stream_rejects_pending_replies_and_limits_preinitialization_data() {
    let mut network = Network::default();
    let command = start(&mut network, "Fetch").remove(0);
    for _ in 0..300 {
        event(&mut network, "dataReceived", json!({"requestId":"sse"}));
    }
    assert!(network.streams[&1].chunks.is_empty());
    chunk(&mut network, &vec![b'x'; MAX_PENDING]);
    assert!(network.streams[&1].error.is_some());
    assert!(network.streams[&1].queued <= MAX_PENDING);
    network.clear();
    assert!(network.streams.is_empty());
    assert!(!network.receive(&json!({"id":command["id"],"result":{"bufferedData":""}})));
    assert!(!chunk(&mut network, b"data: late\n\n"));
}

// Verify streaming errors are visible and eviction releases parser state.
#[test]
fn streaming_errors_are_visible_and_eviction_releases_parser_state() {
    let mut network = Network::default();
    let command = start(&mut network, "Fetch").remove(0);
    network.receive(&json!({"id":command["id"],"error":{"message":"not supported"}}));
    assert!(network.details(Detail::Messages).contains("not supported"));
    for i in 0..300 {
        event(
            &mut network,
            "requestWillBeSent",
            json!({"requestId":i.to_string(),"request":{"url":"https://example.test/data"}}),
        );
    }
    assert!(network.streams.is_empty());
    assert!(network.messages.is_empty());
}
