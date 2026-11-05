//! Verify independent session readiness, domain activation and isolation from other inspectors.
use super::*;

// Complete only this module's attach handshake and retain commands sent after Runtime becomes ready.
fn connected() -> (Session, Vec<Value>) {
    let mut session = Session::default();
    let commands = session.receive_bytes(
        json!({"id": session.target_request, "result":{"targetInfo":{"targetId":"page"}}})
            .to_string()
            .as_bytes(),
    );
    let attach: Value = serde_json::from_str(&commands[0]).unwrap();
    let commands = session.receive_bytes(
        json!({"id":attach["id"],"result":{"sessionId":"module"}})
            .to_string()
            .as_bytes(),
    );
    let enable: Value = serde_json::from_str(&commands[0]).unwrap();
    let commands = session.receive_bytes(
        json!({"id":enable["id"],"sessionId":"module","result":{}})
            .to_string()
            .as_bytes(),
    );
    (
        session,
        commands
            .iter()
            .map(|body| serde_json::from_str(body).unwrap())
            .collect(),
    )
}

// Readiness must be set before domain commands are queued, including when Console has never been opened.
#[test]
fn independent_module_becomes_ready_and_scopes_all_domain_commands() {
    let (session, commands) = connected();
    assert!(session.ready);
    assert!(
        commands
            .iter()
            .any(|command| command["method"] == "Network.enable")
    );
    assert!(
        commands
            .iter()
            .all(|command| command["sessionId"] == "module")
    );
}

// Another inspector's detach notification must not disconnect this module; its own detach invalidates subsequent events.
#[test]
fn detach_is_scoped_to_the_owning_session() {
    let (mut session, _) = connected();
    session.receive_bytes(br#"{"sessionId":"classic","method":"Inspector.detached"}"#);
    assert!(session.ready);
    session.receive_bytes(br#"{"sessionId":"module","method":"Inspector.detached"}"#);
    assert!(!session.ready);
    assert!(session.session.is_none());
}
