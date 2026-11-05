//! Test objects behavior and guard its lifecycle and boundary cases.
use super::*;

// Verify clearing a large object history releases each shared handle once.
#[test]
fn clearing_a_large_object_history_releases_each_shared_handle_once() {
    let mut objects = Objects::default();
    for i in 0..1000 {
        objects.capture(
            "",
            &json!({"objectId":format!("remote-{}",i%20),"description":"Object"}),
            0,
        );
    }
    objects.clear();
    let released = objects.drain();
    assert_eq!(released.len(), 20);
    assert_eq!(
        released
            .iter()
            .map(|c| c["params"]["objectId"].as_str().unwrap())
            .collect::<HashSet<_>>()
            .len(),
        20
    );
    assert!(objects.nodes.is_empty());
    assert_eq!(objects.nodes.capacity(), 0);
    objects.clear();
    assert!(objects.drain().is_empty());
}

// Verify expands nested objects without invoking getters and releases handles.
#[test]
fn expands_nested_objects_without_invoking_getters_and_releases_handles() {
    let mut objects = Objects::default();
    let root = objects
        .capture("🦀", &json!({"objectId":"root","description":"Object"}), 0)
        .unwrap();
    objects.toggle(root);
    let request = objects.drain().remove(0);
    assert_eq!(request["method"], "Runtime.getProperties");
    objects.receive(&json!({"id":request["id"],"result":{"result":[
        {"name":"child","value":{"objectId":"child","description":"Object"}},
        {"name":"count","value":{"value":42}},
        {"name":"danger","get":{"type":"function","objectId":"getter"}}
    ]}}));
    assert!(
        objects
            .drain()
            .iter()
            .all(|command| command["method"] == "Runtime.releaseObject")
    );
    let mut view = ObjectView::default();
    objects.render(&[root], &mut view);
    assert!(view.text.contains("count: 42"));
    assert!(view.text.contains("Getter — not evaluated"));
    let child = view.links[1].node;
    let slice = view
        .text
        .encode_utf16()
        .skip(view.links[1].start)
        .take(view.links[1].length)
        .collect::<Vec<_>>();
    assert!(String::from_utf16(&slice).unwrap().contains("child"));
    objects.toggle(child);
    assert_eq!(objects.drain()[0]["params"]["objectId"], "child");
    objects.remove_roots(&[root]);
    let released = objects.drain();
    assert!(
        released
            .iter()
            .any(|command| command["params"]["objectId"] == "root")
    );
    assert!(
        released
            .iter()
            .any(|command| command["params"]["objectId"] == "child")
    );
}

// Verify late property reply after clear is discarded and released.
#[test]
fn late_property_reply_after_clear_is_discarded_and_released() {
    let mut objects = Objects::default();
    let root = objects
        .capture("", &json!({"objectId":"root","description":"Object"}), 0)
        .unwrap();
    objects.toggle(root);
    let request = objects.drain().remove(0);
    objects.clear();
    objects.drain();
    objects.receive(&json!({"id":request["id"],"result":{"result":[{"name":"child","value":{"objectId":"late"}}]}}));
    assert_eq!(objects.drain()[0]["params"]["objectId"], "late");
    assert!(objects.nodes.is_empty());
}

// Verify removing one root keeps a shared remote handle alive.
#[test]
fn removing_one_root_keeps_a_shared_remote_handle_alive() {
    let mut objects = Objects::default();
    let remote = json!({"objectId":"shared", "description":"Object"});
    let first = objects.capture("first", &remote, 0).unwrap();
    let second = objects.capture("second", &remote, 0).unwrap();
    objects.remove_roots(&[first]);
    assert!(objects.drain().is_empty());
    objects.remove_roots(&[second]);
    let releases = objects.drain();
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0]["params"]["objectId"], "shared");
}

// Formatting a JSON-by-value result must stop at a Unicode boundary before building a full preview string.
#[test]
fn json_value_previews_stream_within_the_shared_character_budget() {
    let remote = json!({"value":{"text":"🦀\"\\\n".repeat(MAX_ROW_CHARS * 2),"tail":[1,2,3]}});
    let rendered = remote["value"].to_string();
    for limit in [0, 1, 7, MAX_ROW_CHARS, rendered.chars().count() + 1] {
        let mut output = String::from("prefix: ");
        let mut remaining = limit;
        append_remote(&mut output, &remote, &mut remaining);
        let expected: String = rendered.chars().take(limit).collect();
        assert_eq!(output, format!("prefix: {expected}"));
        assert_eq!(remaining, limit.saturating_sub(rendered.chars().count()));
    }
}
