//! Test completion behavior and guard its lifecycle and boundary cases.
use super::*;

// Verify identifier completion preserves suffix and unicode offsets.
#[test]
fn identifier_completion_preserves_suffix_and_unicode_offsets() {
    let query = Query::parse("'🦀'; document.qu + 1", 17).unwrap();
    assert_eq!(query.receiver, "document");
    assert_eq!(query.prefix, "qu");
    assert_eq!(
        query.apply("querySelector").unwrap().0,
        "'🦀'; document.querySelector + 1"
    );
    assert!(Query::parse("fn().prop", 9).is_none());
    assert!(Query::parse("items[0].prop", 13).is_none());
    assert!(Query::parse("123.p", 5).is_none());
    assert!(Query::parse("console.log('window.lo", 22).is_none());
}

// Verify stale completion replies release handles and cannot replace new results.
#[test]
fn stale_completion_replies_release_handles_and_cannot_replace_new_results() {
    let mut completion = Completion::default();
    completion.start("document.qu", 11);
    let old = completion.drain().remove(0);
    assert_eq!(old["params"]["throwOnSideEffect"], true);
    completion.start("window.lo", 9);
    let new = completion.drain().remove(0);
    completion.receive(&json!({"id":old["id"],"result":{"result":{"objectId":"old"}}}));
    assert!(completion.suggestions.is_none());
    assert_eq!(
        completion.drain()[0]["method"],
        "Runtime.releaseObjectGroup"
    );
    completion.receive(&json!({"id":new["id"],"result":{"result":{"objectId":"new"}}}));
    let properties = completion.drain().remove(0);
    assert_eq!(properties["method"], "Runtime.getProperties");
    completion.receive(&json!({"id":properties["id"],"result":{"result":[{"name":"location"},{"name":"localStorage"},{"name":"alert"}]}}));
    assert_eq!(
        completion.suggestions.unwrap().names,
        ["localStorage", "location"]
    );
}

// Verify global completion combines lexical names and property names.
#[test]
fn global_completion_combines_lexical_names_and_property_names() {
    let mut completion = Completion::default();
    completion.start("my", 2);
    let requests = completion.drain();
    completion.receive(&json!({"id":requests[1]["id"],"result":{"names":["myLocal"]}}));
    assert!(completion.suggestions.is_none());
    completion.receive(&json!({"id":requests[0]["id"],"result":{"result":{"objectId":"window"}}}));
    let properties = completion.drain().remove(0);
    completion.receive(
        &json!({"id":properties["id"],"result":{"result":[{"name":"myGlobal"},{"name":"my"}]}}),
    );
    assert_eq!(
        completion.suggestions.unwrap().names,
        ["my", "myGlobal", "myLocal"]
    );
}

// Verify large completion replies keep only the first 40 matches in either order.
#[test]
fn large_completion_replies_keep_only_the_first_40_matches_in_either_order() {
    for lexical_first in [false, true] {
        let mut completion = Completion::default();
        completion.start("my", 2);
        let requests = completion.drain();
        completion
            .receive(&json!({"id":requests[0]["id"],"result":{"result":{"objectId":"window"}}}));
        let properties = completion.drain().remove(0);
        let lexical = json!({"id":requests[1]["id"],"result":{"names":["my000", "my001", "my050", "unrelated"]}});
        let properties = json!({"id":properties["id"],"result":{"result":
            (2..2000).rev().flat_map(|i| [json!({"name":format!("my{i:03}")}), json!({"name":format!("unrelated{i}")})]).collect::<Vec<_>>()}});
        let replies = if lexical_first {
            [&lexical, &properties]
        } else {
            [&properties, &lexical]
        };
        completion.receive(replies[0]);
        assert_eq!(
            completion.batch.as_ref().unwrap().names.len(),
            if lexical_first { 3 } else { 40 }
        );
        completion.receive(replies[1]);
        assert_eq!(
            completion.suggestions.unwrap().names,
            (0..40).map(|i| format!("my{i:03}")).collect::<Vec<_>>()
        );
    }
}

// Verify non ascii input never splits a utf8 character.
#[test]
fn non_ascii_input_never_splits_a_utf8_character() {
    assert!(Query::parse("в", 1).is_none());
    for value in ["ввод", "🦀", "变量", "ввод; document.qu", "🦀; document.qu"] {
        for caret in 0..=value.encode_utf16().count() {
            let _ = Query::parse(value, caret);
        }
    }
    let query = Query::parse("ввод; document.qu", 17).unwrap();
    assert_eq!(
        query.apply("querySelector").unwrap().0,
        "ввод; document.querySelector"
    );
}

// Verify history preserves drafts skips duplicates and has a bound.
#[test]
fn history_preserves_drafts_skips_duplicates_and_has_a_bound() {
    let mut history = CommandHistory::default();
    assert!(history.navigate(true, "draft").is_none());
    history.record("1+1");
    history.record("1+1");
    history.record("2+2");
    assert_eq!(history.navigate(true, "unfinished"), Some("2+2".into()));
    assert_eq!(history.navigate(true, "2+2"), Some("1+1".into()));
    assert_eq!(history.navigate(true, "1+1"), Some("1+1".into()));
    assert_eq!(history.navigate(false, "1+1"), Some("2+2".into()));
    assert_eq!(history.navigate(false, "2+2"), Some("unfinished".into()));
    for i in 0..150 {
        history.record(&i.to_string());
    }
    assert_eq!(history.entries.len(), 100);
}
