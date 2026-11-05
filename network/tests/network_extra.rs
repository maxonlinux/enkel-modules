//! Test network extra behavior and guard its lifecycle and boundary cases.
use super::*;

// Verify event.
fn event(network: &mut Network, method: &str, params: Value) -> bool {
    network.receive(&json!({"method":format!("Network.{method}"),"params":params}))
}

// Verify request.
fn request(network: &mut Network, id: &str) {
    event(
        network,
        "requestWillBeSent",
        json!({"requestId":id,"type":"Fetch","request":{"url":"https://example.test/data","method":"GET"}}),
    );
}

// Verify extras.
fn extras(network: &mut Network, label: &str) {
    event(
        network,
        "requestWillBeSentExtraInfo",
        json!({"requestId":"chain","headers":{"X-Hop":label},"associatedCookies":[]}),
    );
    event(
        network,
        "responseReceivedExtraInfo",
        json!({"requestId":"chain","headers":{"X-Hop":label},"statusCode":302}),
    );
}

// Verify early metadata preserves wire headers duplicate cookies and block reasons.
#[test]
fn early_metadata_preserves_wire_headers_duplicate_cookies_and_block_reasons() {
    let mut network = Network::default();
    network.set_active(true);
    event(
        &mut network,
        "requestWillBeSentExtraInfo",
        json!({"requestId":"a",
        "headers":{"Cookie":"session=abc","Authorization":"Bearer test"},
        "associatedCookies":[{"cookie":{"name":"session","value":"abc","domain":"example.test","path":"/","httpOnly":true,"secure":true,"sameSite":"Lax"},"blockedReasons":[]},
            {"cookie":{"name":"blocked","value":"value"},"blockedReasons":["SameSiteStrict"]}]}),
    );
    event(
        &mut network,
        "responseReceivedExtraInfo",
        json!({"requestId":"a","statusCode":304,
        "headers":{"Set-Cookie":"ok=1; Path=/\nbad=2; SameSite=None","X-Wire":"yes"},
        "blockedCookies":[{"cookieLine":"bad=2; SameSite=None","blockedReasons":["SameSiteNoneInsecure"]}],
        "exemptedCookies":[{"cookieLine":"ok=1; Path=/","exemptionReason":"UserSetting"}]}),
    );
    request(&mut network, "a");
    event(
        &mut network,
        "responseReceived",
        json!({"requestId":"a","hasExtraInfo":true,"response":{"status":200,"headers":{"X-Provisional":"old"}}}),
    );
    network.select(Some(1));
    let detail = network.details(Detail::Headers);
    for expected in [
        "Authorization: Bearer test",
        "Cookie: session=abc",
        "Set-Cookie: ok=1; Path=/",
        "Set-Cookie: bad=2; SameSite=None",
        "Sent\nsession=abc",
        "httpOnly",
        "SameSite=Lax",
        "Not sent · SameSiteStrict",
        "Blocked · SameSiteNoneInsecure",
        "Allowed by exception · UserSetting",
    ] {
        assert!(detail.contains(expected), "missing {expected}: {detail}");
    }
    assert!(!detail.contains("X-Provisional"));
    assert_eq!(network.rows("")[0].status, RequestStatus::Http(304));
}

// Verify redirect hops match late metadata and skip hops without extra info.
#[test]
fn redirect_hops_match_late_metadata_and_skip_hops_without_extra_info() {
    let mut network = Network::default();
    network.set_active(true);
    request(&mut network, "chain");
    event(
        &mut network,
        "requestWillBeSent",
        json!({"requestId":"chain","redirectHasExtraInfo":true,"redirectResponse":{"status":302},"request":{"url":"https://example.test/cache","method":"GET"}}),
    );
    extras(&mut network, "first");
    event(
        &mut network,
        "requestWillBeSent",
        json!({"requestId":"chain","redirectHasExtraInfo":false,"redirectResponse":{"status":302},"request":{"url":"https://example.test/final","method":"GET"}}),
    );
    extras(&mut network, "final");
    event(
        &mut network,
        "responseReceived",
        json!({"requestId":"chain","hasExtraInfo":true,"response":{"status":200}}),
    );
    for (key, label) in [(1, "first"), (3, "final")] {
        network.select(Some(key));
        let detail = network.details(Detail::Headers);
        assert_eq!(detail.matches(&format!("X-Hop: {label}")).count(), 2);
    }
    network.select(Some(2));
    assert!(
        network
            .details(Detail::Headers)
            .contains("metadata unavailable")
    );
    assert!(!network.details(Detail::Headers).contains("X-Hop"));
    assert!(network.extras.0.is_empty());
}

// Verify failed cors requests keep wire status and invalid cookie diagnostics.
#[test]
fn failed_cors_requests_keep_wire_status_and_invalid_cookie_diagnostics() {
    let mut network = Network::default();
    network.set_active(true);
    request(&mut network, "cors");
    event(
        &mut network,
        "loadingFailed",
        json!({"requestId":"cors","timestamp":2,"errorText":"CORS error"}),
    );
    event(
        &mut network,
        "responseReceivedExtraInfo",
        json!({"requestId":"cors","statusCode":403,
        "headersText":"HTTP/1.1 403 Forbidden\r\nX-Raw: yes\r\n\r\n","headers":{},
        "blockedCookies":[{"cookieLine":"invalid cookie","blockedReasons":["SyntaxError"]}]}),
    );
    network.select(Some(1));
    assert_eq!(network.rows("")[0].status, RequestStatus::Failed);
    let detail = network.details(Detail::Headers);
    assert!(detail.contains("Wire status  403") && detail.contains("X-Raw: yes"));
    assert!(detail.contains("Blocked · SyntaxError\ninvalid cookie"));
}

// Verify unmatched metadata is bounded and clear or stop releases it.
#[test]
fn unmatched_metadata_is_bounded_and_clear_or_stop_releases_it() {
    let mut network = Network::default();
    network.set_active(true);
    for i in 0..350 {
        event(
            &mut network,
            "requestWillBeSentExtraInfo",
            json!({"requestId":i.to_string(),"headers":{"X-Large":"x".repeat(32768)}}),
        );
    }
    assert_eq!(network.extras.0.len(), MAX_REQUESTS);
    assert!(
        network
            .extras
            .0
            .iter()
            .all(|pending| pending.info.headers.len() < MAX_METADATA + 32)
    );
    network.clear();
    assert!(network.extras.0.is_empty());
    extras(&mut network, "orphan");
    network.set_active(false);
    assert!(network.extras.0.is_empty());
    assert!(!event(
        &mut network,
        "responseReceivedExtraInfo",
        json!({"requestId":"late","headers":{}})
    ));
}

// Verify interleaved metadata is consumed only by its own request.
#[test]
fn interleaved_metadata_is_consumed_only_by_its_own_request() {
    let mut network = Network::default();
    network.set_active(true);
    for id in ["a", "b"] {
        event(
            &mut network,
            "responseReceivedExtraInfo",
            json!({"requestId":id,"statusCode":200,"headers":{"X-Request":id}}),
        );
        request(&mut network, id);
    }
    for (id, key, other) in [("b", 2, "a"), ("a", 1, "b")] {
        event(
            &mut network,
            "responseReceived",
            json!({"requestId":id,"hasExtraInfo":true,"response":{"status":200}}),
        );
        network.select(Some(key));
        let headers = network.details(Detail::Headers);
        assert!(headers.contains(&format!("X-Request: {id}")));
        assert!(!headers.contains(&format!("X-Request: {other}")));
    }
}

// Late network-stack headers add diagnostic data without reopening a stream that already closed.
#[test]
fn late_wire_status_does_not_revive_a_closed_event_stream() {
    let mut network = Network::default();
    network.set_active(true);
    request(&mut network, "stream");
    event(
        &mut network,
        "responseReceived",
        json!({"requestId":"stream","hasExtraInfo":true,
        "response":{"status":200,"mimeType":"text/event-stream"}}),
    );
    event(
        &mut network,
        "loadingFailed",
        json!({"requestId":"stream","timestamp":2,"canceled":true}),
    );
    event(
        &mut network,
        "responseReceivedExtraInfo",
        json!({"requestId":"stream","statusCode":200,"headers":{"Content-Type":"text/event-stream"}}),
    );
    network.select(Some(1));
    assert_eq!(network.rows("")[0].status, RequestStatus::Closed);
    assert!(
        network
            .details(Detail::Headers)
            .contains("Wire status  200")
    );
}
