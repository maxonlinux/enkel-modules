//! Test network timing behavior and guard its lifecycle and boundary cases.
use super::*;

// Verify event.
fn event(network: &mut Network, method: &str, params: Value) {
    network.receive(&json!({"method":format!("Network.{method}"),"params":params}));
}

// Verify request.
fn request(network: &mut Network, id: &str, start: f64, initiator: Value) {
    event(
        network,
        "requestWillBeSent",
        json!({"requestId":id,"timestamp":start,"initiator":initiator,
        "request":{"url":format!("https://example.test/api/{id}"),"method":"GET"}}),
    );
}

// Verify overlapping requests share one axis without rounding the source timestamps.
#[test]
fn overlapping_requests_share_one_axis_without_rounding_the_source_timestamps() {
    let mut network = Network::default();
    network.set_active(true);
    request(&mut network, "a", 10.123456789, json!({}));
    request(&mut network, "b", 10.323456789, json!({}));
    event(
        &mut network,
        "loadingFinished",
        json!({"requestId":"b","timestamp":10.823456789}),
    );
    event(
        &mut network,
        "loadingFinished",
        json!({"requestId":"a","timestamp":11.123456789}),
    );
    let rows = network.rows("");
    let a = rows[0].span.unwrap();
    let b = rows[1].span.unwrap();
    assert_eq!((a.start, a.end), (10.123456789, 11.123456789));
    assert_eq!((b.start, b.end), (10.323456789, 10.823456789));
    assert!(!a.ongoing && !b.ongoing);
    assert_eq!(network.rows[0].started, 10.123456789);
    assert_eq!(network.rows[0].ended, Some(11.123456789));
    assert_eq!(rows[1].time, "500 ms");
}

// Verify ongoing and unknown timing never invent a finish and clear resets the axis.
#[test]
fn ongoing_and_unknown_timing_never_invent_a_finish_and_clear_resets_the_axis() {
    let mut network = Network::default();
    network.set_active(true);
    request(&mut network, "unknown", 0., json!({}));
    request(&mut network, "live", 20., json!({}));
    event(
        &mut network,
        "dataReceived",
        json!({"requestId":"live","timestamp":22}),
    );
    assert!(!network.receive(
        &json!({"method":"Runtime.consoleAPICalled","params":{"timestamp":1790000000000_f64}})
    ));
    assert_eq!(network.observed, 22.);
    let rows = network.rows("");
    assert!(rows[0].span.is_none());
    assert!(rows[1].span.unwrap().ongoing);
    network.select(Some(rows[1].key));
    assert!(network.details(Detail::Info).contains("ongoing"));
    assert_eq!(network.rows[1].ended, None);
    network.clear();
    assert!(network.axis().is_none());
    request(&mut network, "new", 100., json!({}));
    assert_eq!(network.axis().unwrap().start, 100.);
}

// Verify initiators keep a useful async source location and filter by its full url.
#[test]
fn initiators_keep_a_useful_async_source_location_and_filter_by_its_full_url() {
    let mut network = Network::default();
    network.set_active(true);
    request(
        &mut network,
        "script",
        1.,
        json!({"type":"script","stack":{"callFrames":[{"url":""}],"parent":{"callFrames":[
        {"url":"https://example.test/src/app.js","functionName":"loadData","lineNumber":41,"columnNumber":7}]}}}),
    );
    request(
        &mut network,
        "parser",
        2.,
        json!({"type":"parser","url":"https://example.test/index.html","lineNumber":0}),
    );
    let rows = network.rows("src/app.js");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].initiator.as_ref(), "app.js:42:8");
    assert!(rows[0].source.contains("loadData"));
    assert_eq!(network.rows("")[1].initiator.as_ref(), "index.html:1");
    network.select(Some(rows[0].key));
    assert!(
        network
            .details(Detail::Info)
            .contains("https://example.test/src/app.js:42:8")
    );
}
