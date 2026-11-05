//! Test console sessions, bounded histories, update routing and tool teardown.

use super::*;
impl Session {
    // Verify receive.
    fn receive(&mut self, body: &str) -> Vec<String> {
        self.receive_bytes(body.as_bytes())
    }

    // Verify text.
    fn text(&self) -> String {
        self.text_matching("")
    }

    // Verify text matching.
    fn text_matching(&self, query: &str) -> String {
        self.view(query).text
    }
}

// Verify connected.
fn connected() -> Session {
    let mut console = Session::default();
    let attach: Value = serde_json::from_str(
        &console.receive(
            &json!({"id":console.target_request,
        "result":{"targetInfo":{"targetId":"page"}}})
            .to_string(),
        )[0],
    )
    .unwrap();
    let enable: Value = serde_json::from_str(
        &console.receive(
            &json!({"id":attach["id"],
        "result":{"sessionId":"native"}})
            .to_string(),
        )[0],
    )
    .unwrap();
    console.receive(&json!({"id":enable["id"],"sessionId":"native","result":{}}).to_string());
    console
}

// Verify package model isolation and invalidation; the browser transport owns scoped-session detachment.
#[test]
fn console_uses_its_own_session_and_invalidates_on_detach() {
    let mut console = connected();
    let event = |session| {
        json!({"sessionId":session,"method":"Runtime.consoleAPICalled","params":{"type":"log","args":[{"value":"hello"}]}}).to_string()
    };
    console.receive(&event("classic"));
    assert!(console.text().is_empty());
    console.receive(&event("native"));
    assert_eq!(console.text(), "[log] hello");
    console.receive(r#"{"sessionId":"native","method":"Inspector.detached"}"#);
    assert!(!console.ready);
    assert!(console.evaluations.is_empty());
}

// Verify evaluation tracks only its reply.
#[test]
fn evaluation_tracks_only_its_reply() {
    let mut console = connected();
    let request: Value = serde_json::from_str(&console.evaluate("2+2").unwrap()).unwrap();
    assert_eq!(request["sessionId"], "native");
    let reply = json!({"id":request["id"],"sessionId":"native","result":{"result":{"value":4}}});
    assert!(console.receive(&reply.to_string()).is_empty());
    assert_eq!(console.text(), "> 2+2\n4");

    assert!(console.receive(&reply.to_string()).is_empty());
}

// Verify object results remain inspectable until clear and use their session.
#[test]
fn object_results_remain_inspectable_until_clear_and_use_their_session() {
    let mut console = connected();
    let request: Value = serde_json::from_str(&console.evaluate("sample").unwrap()).unwrap();
    assert!(console.receive(&json!({"id":request["id"],"sessionId":"native", "result":{"result":{"objectId":"result","description":"Object"}}}).to_string()).is_empty());
    let root = console.view("").links[0].node;
    let properties: Value = serde_json::from_str(&console.toggle_object(root)[0]).unwrap();
    assert_eq!(properties["sessionId"], "native");
    console.receive(&json!({"id":properties["id"],"sessionId":"native","result":{"result":[{"name":"count","value":{"value":42}}]}}).to_string());
    assert!(console.view("").text.contains("count: 42"));
    let cleanup = console
        .clear_commands()
        .iter()
        .map(|command| serde_json::from_str::<Value>(command).unwrap())
        .collect::<Vec<_>>();
    assert!(
        !cleanup
            .iter()
            .any(|command| command["method"] == "Runtime.releaseObject")
    );
    assert!(
        cleanup
            .iter()
            .any(|command| command["method"] == "Runtime.discardConsoleEntries")
    );
    assert!(
        cleanup
            .iter()
            .any(|command| command["method"] == "Runtime.releaseObjectGroup")
    );
    assert!(console.view("").text.is_empty());
}

// Verify failed connections and bounded logs.
#[test]
fn failed_connections_and_bounded_logs() {
    let mut console = Session::default();
    assert!(console.evaluate("1").is_none());
    console.receive(&json!({"id":console.target_request,"error":{"message":"closed"}}).to_string());
    assert!(!console.ready);
    for _ in 0..600 {
        console.push("🦀".repeat(5000));
    }
    assert_eq!(console.rows.len(), MAX_ROWS);
    assert_eq!(console.rows[0].text.chars().count(), MAX_ROW_CHARS);
}

// Verify unresolved promises are bounded and errors remain readable.
#[test]
fn unresolved_promises_are_bounded_and_errors_remain_readable() {
    let mut console = connected();
    for _ in 0..MAX_EVALUATIONS {
        assert!(console.evaluate("new Promise(() => {})").is_some());
    }
    assert!(console.evaluate("1").is_none());
    assert!(console.text().contains("Too many pending expressions"));
    console.receive(
        &json!({"sessionId":"native","method":"Runtime.exceptionThrown",
        "params":{"exceptionDetails":{"exception":{"description":"Error: check"}}}})
        .to_string(),
    );
    assert!(console.text().ends_with("Error: check"));
    assert!(!console.text().contains("Error: Error:"));
    console.receive(r#"{"sessionId":"native","method":"Inspector.detached"}"#);
    assert!(console.evaluations.is_empty());
}

// Verify filtering keeps matching exception stacks together.
#[test]
fn filtering_keeps_matching_exception_stacks_together() {
    let mut console = connected();
    console.push("[log] unrelated");
    console.push("Error: failed request\n    at check.js:10");
    assert_eq!(
        console.text_matching("FAILED"),
        "Error: failed request\n    at check.js:10"
    );
    assert_eq!(console.text_matching("missing"), "");
    assert!(console.text_matching(" ").contains("unrelated"));
}

// Verify byte transport rejects invalid utf8 without losing the session.
#[test]
fn byte_transport_rejects_invalid_utf8_without_losing_the_session() {
    let mut console = connected();
    assert!(
        console
            .receive_bytes(b"{\"sessionId\":\"native\",\"value\":\"\xff\"}")
            .is_empty()
    );
    assert!(!console.changes.any());
    assert!(console.ready);
    console.receive(r#"{"sessionId":"native","method":"Runtime.consoleAPICalled","params":{"args":[{"value":"🦀"}]}}"#);
    assert_eq!(console.text(), "[log] 🦀");
}

// Clearing a not-yet-connected module must not send cleanup commands into Chromium's root session.
#[test]
fn disconnected_clear_stays_local() {
    let mut session = Session::default();
    session.push("local");
    assert!(session.clear_commands().is_empty());
    assert!(session.view("").text.is_empty());
}
