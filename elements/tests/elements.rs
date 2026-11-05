//! Verify lazy DOM subscriptions, mutation batching, stale replies and inspector teardown.
use super::*;

// Verify ready.
fn ready() -> Elements {
    let mut model = Elements::default();
    model.set_active(true);
    let enable = model.drain().remove(0);
    model.receive(&json!({"id":enable["id"],"result":{}}));
    let document = model
        .drain()
        .into_iter()
        .find(|c| c["method"] == "DOM.getDocument")
        .unwrap();
    model.receive(&json!({"id":document["id"],"result":{"root":{
        "nodeId":1,"nodeType":9,"nodeName":"#document","children":[{
            "nodeId":2,"nodeType":1,"localName":"html","children":[{
                "nodeId":3,"nodeType":1,"localName":"body","children":[{
                    "nodeId":4,"nodeType":1,"localName":"div","attributes":["id","sample"],"childNodeCount":1
                }]
            }]
        }]
    }}}));
    model.drain();
    model
}

// Verify command.
fn command(model: &mut Elements, method: &str) -> Value {
    model
        .drain()
        .into_iter()
        .find(|c| c["method"] == method)
        .unwrap()
}

// Verify picker toggles off and cancellation removes the page overlay.
#[test]
fn picker_toggles_off_and_cancellation_removes_the_page_overlay() {
    let mut model = ready();
    model.inspect();
    assert!(model.picking);
    assert_eq!(
        command(&mut model, "Overlay.setInspectMode")["params"]["mode"],
        "searchForNode"
    );
    model.inspect();
    let stopped = model.drain();
    assert!(!model.picking);
    assert!(
        stopped
            .iter()
            .any(|c| c["method"] == "Overlay.setInspectMode" && c["params"]["mode"] == "none")
    );
    // Blink validates this object even when disabling inspect mode.
    assert!(
        stopped
            .iter()
            .find(|c| c["method"] == "Overlay.setInspectMode")
            .unwrap()["params"]["highlightConfig"]
            .is_object()
    );
    assert!(
        stopped
            .iter()
            .any(|c| c["method"] == "Overlay.hideHighlight")
    );
    assert!(!model.cancel_inspect());
    assert!(model.drain().is_empty());
    model.inspect();
    model.drain();
    assert!(model.cancel_inspect());
    assert!(!model.picking);
    assert_eq!(model.drain().len(), 2);
}

// Verify picker state waits for chromium and failed cancel stays visible.
#[test]
fn picker_state_waits_for_chromium_and_failed_cancel_stays_visible() {
    let mut model = ready();
    model.inspect();
    let enabled = command(&mut model, "Overlay.setInspectMode");
    assert!(!model.inspecting());
    model.receive(&json!({"id":enabled["id"],"result":{}}));
    assert!(model.inspecting());
    model.cancel_inspect();
    let disabled = command(&mut model, "Overlay.setInspectMode");
    assert!(model.inspecting());
    model.receive(&json!({"id":disabled["id"],"error":{"message":"highlight configuration parameter is missing"}}));
    assert!(model.picking && model.inspecting());
    assert!(
        model
            .notice
            .as_ref()
            .unwrap()
            .contains("highlight configuration")
    );
    model.cancel_inspect();
    let retry = command(&mut model, "Overlay.setInspectMode");
    model.receive(&json!({"id":retry["id"],"result":{}}));
    assert!(!model.picking && !model.inspecting());
}

// Verify rapid picker toggles ignore superseded replies.
#[test]
fn rapid_picker_toggles_ignore_superseded_replies() {
    let mut model = ready();
    model.inspect();
    let first = command(&mut model, "Overlay.setInspectMode");
    model.cancel_inspect();
    let second = command(&mut model, "Overlay.setInspectMode");
    model.inspect();
    let last = command(&mut model, "Overlay.setInspectMode");
    model.receive(&json!({"id":last["id"],"result":{}}));
    model.receive(&json!({"id":second["id"],"result":{}}));
    model.receive(&json!({"id":first["id"],"error":{"message":"stale"}}));
    assert!(model.picking && model.inspecting() && model.notice.is_none());
}

// Verify chromium cancel and selection end the same picker session.
#[test]
fn chromium_cancel_and_selection_end_the_same_picker_session() {
    let mut model = ready();
    model.inspect();
    model.drain();
    model.receive(&json!({"method":"Overlay.inspectModeCanceled","params":{}}));
    assert!(!model.picking);
    assert!(
        model
            .drain()
            .iter()
            .any(|c| c["method"] == "Overlay.hideHighlight")
    );
    model.inspect();
    model.drain();
    model.receive(&json!({"method":"Overlay.inspectNodeRequested","params":{"backendNodeId":42}}));
    assert!(!model.picking);
    let reveal = command(&mut model, "DOM.pushNodesByBackendIdsToFrontend");
    assert_eq!(reveal["params"]["backendNodeIds"], json!([42]));
    model.receive(&json!({"id":reveal["id"],"result":{"nodeIds":[4]}}));
    assert_eq!(model.selected, Some(4));
}

// Verify starts only when opened and releases on close.
#[test]
fn starts_only_when_opened_and_releases_on_close() {
    let mut model = ready();
    model.select(4);
    model.force_state("hover");
    model.set_active(false);
    let commands = model.drain();
    assert!(commands.iter().any(|c| c["method"] == "DOM.disable"));
    assert!(commands.iter().any(|c| c["method"] == "CSS.disable"));
    assert!(
        commands
            .iter()
            .any(|c| c["method"] == "Overlay.hideHighlight")
    );
    assert!(!model.active && model.nodes.is_empty() && model.rules.is_empty());
    assert!(model.computed.is_empty() && model.selected.is_none());
    assert!(!model.receive(
        &json!({"method":"DOM.attributeModified","params":{"nodeId":4,"name":"id","value":"late"}})
    ));
}

// Verify expansion is lazy and collapsed branches do not repeat requests.
#[test]
fn expansion_is_lazy_and_collapsed_branches_do_not_repeat_requests() {
    let mut model = ready();
    assert!(!model.nodes[&4].loaded);
    model.toggle(4);
    assert_eq!(
        command(&mut model, "DOM.requestChildNodes")["params"]["depth"],
        1
    );
    model.toggle(4);
    model.toggle(4);
    assert!(model.drain().is_empty());
    model.receive(&json!({"method":"DOM.setChildNodes","params":{"parentId":4,"nodes":[{"nodeId":5,"nodeType":3,"nodeValue":"hello"}]}}));
    assert!(model.nodes[&4].loaded);
    model.toggle(2);
    model.toggle(3);
    assert!(model.rows().iter().any(|row| row.id == 5));
    model.toggle(4);
    model.toggle(4);
    assert!(model.drain().is_empty());
}

// Verify live attribute bursts keep one dirty node and one styles request.
#[test]
fn live_attribute_bursts_keep_one_dirty_node_and_one_styles_request() {
    let mut model = ready();
    model.select(4);
    model.drain();
    model.changed.clear();
    model.structure = false;
    for n in 0..1000 {
        model.receive(&json!({"method":"DOM.attributeModified","params":{"nodeId":4,"name":"class","value":format!("state-{n}")}}));
    }
    assert_eq!(model.changed.len(), 1);
    assert!(!model.structure && model.drain().is_empty());
    assert!(model.nodes[&4].label().contains("state-999"));
    model.flush_details();
    let commands = model.drain();
    assert_eq!(
        commands
            .iter()
            .filter(|c| c["method"] == "CSS.getMatchedStylesForNode")
            .count(),
        1
    );
    model.flush_details();
    assert!(model.drain().is_empty());
}

// Verify reselecting a node does not accept its previous selection reply.
#[test]
fn reselecting_a_node_does_not_accept_its_previous_selection_reply() {
    let mut model = ready();
    model.section(Section::Computed);
    model.select(4);
    model.flush_details();
    let old = command(&mut model, "CSS.getComputedStyleForNode");
    model.select(3);
    model.select(4);
    model.flush_details();
    model.drain();
    model.receive(
        &json!({"id":old["id"],"result":{"computedStyle":[{"name":"color","value":"stale"}]}}),
    );
    assert!(model.computed.is_empty());
}

// Verify late search results are discarded even after close and reopen.
#[test]
fn late_search_results_are_discarded_even_after_close_and_reopen() {
    let mut model = ready();
    model.search("div");
    let old = command(&mut model, "DOM.performSearch");
    model.set_active(false);
    model.drain();
    model.set_active(true);
    model.drain();
    model.receive(&json!({"id":old["id"],"result":{"searchId":"old-search","resultCount":100}}));
    assert_eq!(
        command(&mut model, "DOM.discardSearchResults")["params"]["searchId"],
        "old-search"
    );
    assert_eq!(model.search_count, 0);
}

// Verify removing a selected subtree releases nodes and style tracking.
#[test]
fn removing_a_selected_subtree_releases_nodes_and_style_tracking() {
    let mut model = ready();
    model.toggle(4);
    model.receive(&json!({"method":"DOM.setChildNodes","params":{"parentId":4,"nodes":[{"nodeId":5,"nodeType":1,"localName":"span"}]}}));
    model.select(5);
    model.drain();
    model.receive(&json!({"method":"DOM.childNodeRemoved","params":{"parentNodeId":3,"nodeId":4}}));
    assert!(!model.nodes.contains_key(&4) && !model.nodes.contains_key(&5));
    assert!(model.selected.is_none());
    assert!(
        model
            .drain()
            .iter()
            .any(|c| c["method"] == "CSS.trackComputedStyleUpdatesForNode"
                && c["params"].get("nodeId").is_none())
    );
}

// Verify computed updates do not refetch matched rules.
#[test]
fn computed_updates_do_not_refetch_matched_rules() {
    let mut model = ready();
    model.section(Section::Computed);
    model.select(4);
    model.flush_details();
    let commands = model.drain();
    for c in commands {
        model.receive(&json!({"id":c["id"],"result":{}}));
    }
    model.receive(&json!({"method":"CSS.computedStyleUpdated","params":{"nodeId":4}}));
    model.flush_details();
    let commands = model.drain();
    assert!(
        commands
            .iter()
            .any(|c| c["method"] == "CSS.getComputedStyleForNode")
    );
    assert!(
        !commands
            .iter()
            .any(|c| c["method"] == "CSS.getMatchedStylesForNode")
    );
}

// Verify computed properties are requested only while visible and late data is dropped.
#[test]
fn computed_properties_are_requested_only_while_visible_and_late_data_is_dropped() {
    let mut model = ready();
    model.select(4);
    model.flush_details();
    let initial = model.drain();
    assert!(
        !initial
            .iter()
            .any(|c| c["method"] == "CSS.getComputedStyleForNode")
    );
    assert!(initial.iter().any(|c| c["method"] == "DOM.getBoxModel"));
    for c in initial {
        model.receive(&json!({"id":c["id"],"result":{}}));
    }
    model.section(Section::Computed);
    model.flush_details();
    let commands = model.drain();
    let request = commands
        .iter()
        .find(|c| c["method"] == "CSS.getComputedStyleForNode")
        .unwrap();
    for c in commands.iter().filter(|c| c["method"] == "DOM.getBoxModel") {
        model.receive(&json!({"id":c["id"],"result":{}}));
    }
    model.section(Section::Styles);
    let properties: Vec<_> = (0..1500)
        .map(|i| json!({"name":format!("--property-{i}"),"value":"retained"}))
        .collect();
    model.receive(&json!({"id":request["id"],"result":{"computedStyle":properties}}));
    assert!(model.computed.is_empty());
    model.section(Section::Computed);
    model.flush_details();
    let current = command(&mut model, "CSS.getComputedStyleForNode");
    model.receive(
        &json!({"id":current["id"],"result":{"computedStyle":[{"name":"color","value":"red"}]}}),
    );
    assert_eq!(model.computed.len(), 1);
    model.section(Section::Styles);
    assert!(model.computed.is_empty());
}

// Verify css edits use one rule range and keep undo boundary.
#[test]
fn css_edits_use_one_rule_range_and_keep_undo_boundary() {
    let mut model = ready();
    model.select(4);
    model.flush_details();
    let request = command(&mut model, "CSS.getMatchedStylesForNode");
    let range = json!({"startLine":0,"startColumn":0,"endLine":0,"endColumn":10});
    model.receive(&json!({"id":request["id"],"result":{"matchedCSSRules":[{"rule":{"selectorList":{"text":".card"},"style":{"styleSheetId":"sheet","range":range,"cssText":"color: red;"}}}]}}));
    model.edit_rule(0, model.styles_revision, ".card {\ncolor: blue;\n}");
    let commands = model.drain();
    assert_eq!(commands[0]["method"], "CSS.setStyleTexts");
    assert_eq!(commands[0]["params"]["edits"][0]["range"], range);
    assert_eq!(
        commands[0]["params"]["edits"][0]["text"],
        "\ncolor: blue;\n"
    );
    model.receive(&json!({"id":commands[0]["id"],"result":{}}));
    assert_eq!(
        command(&mut model, "DOM.markUndoableState")["method"],
        "DOM.markUndoableState"
    );
}

// Verify large dom and text updates remain bounded.
#[test]
fn large_dom_and_text_updates_remain_bounded() {
    let mut model = ready();
    let nodes = (10..(MAX_NODES as i64 + 30))
        .map(|id| json!({"nodeId":id,"nodeType":3,"nodeValue":"content"}))
        .collect::<Vec<_>>();
    model.receive(&json!({"method":"DOM.setChildNodes","params":{"parentId":4,"nodes":nodes}}));
    assert!(model.nodes.len() <= MAX_NODES && model.retained_bytes <= MAX_DOM_BYTES);
    let text = "x".repeat(MAX_TEXT);
    for id in 10..(MAX_NODES as i64) {
        model.receive(&json!({"method":"DOM.characterDataModified","params":{"nodeId":id,"characterData":text}}));
    }
    assert!(model.retained_bytes <= MAX_DOM_BYTES);
}

// Verify stale rule ranges cannot overwrite a newer stylesheet.
#[test]
fn stale_rule_ranges_cannot_overwrite_a_newer_stylesheet() {
    let mut model = ready();
    model.select(4);
    model.set_styles(&json!({"inlineStyle":{"cssText":"color:red;"}}));
    let revision = model.styles_revision;
    model.set_styles(&json!({"inlineStyle":{"cssText":"color:green;"}}));
    model.drain();
    model.edit_rule(0, revision, "element.style {color:blue;}");
    assert!(model.drain().is_empty());
    assert!(model.status.contains("Styles changed"));
}

// Verify markup bursts are coalesced and do not duplicate inflight requests.
#[test]
fn markup_bursts_are_coalesced_and_do_not_duplicate_inflight_requests() {
    let mut model = ready();
    model.select(4);
    model.section(Section::Html);
    model.flush_details();
    let request = command(&mut model, "DOM.getOuterHTML");
    for n in 0..1000 {
        model.receive(&json!({"method":"DOM.attributeModified","params":{"nodeId":4,"name":"data-version","value":n.to_string()}}));
    }
    model.flush_details();
    assert!(
        !model
            .drain()
            .iter()
            .any(|c| c["method"] == "DOM.getOuterHTML")
    );
    model.receive(&json!({"id":request["id"],"result":{"outerHTML":"<div>latest</div>"}}));
    model.flush_details();
    assert_eq!(
        model
            .drain()
            .iter()
            .filter(|c| c["method"] == "DOM.getOuterHTML")
            .count(),
        1
    );
}

// Verify a missing layout box does not turn into a styles error.
#[test]
fn a_missing_layout_box_does_not_turn_into_a_styles_error() {
    let mut model = ready();
    model.select(4);
    model.flush_details();
    let request = command(&mut model, "DOM.getBoxModel");
    model.receive(&json!({"id":request["id"],"error":{"message":"Could not compute box model."}}));
    assert!(model.box_model.is_none() && model.notice.is_none());
    assert_eq!(model.selected, Some(4));
}

// Verify mutations inside collapsed branches do not rebuild visible tree.
#[test]
fn mutations_inside_collapsed_branches_do_not_rebuild_visible_tree() {
    let mut model = ready();
    model.reveal(4);
    model.receive(&json!({"method":"DOM.setChildNodes","params":{"parentId":4,"nodes":[{"nodeId":5,"nodeType":3,"nodeValue":"before"}]}}));
    model.structure = false;
    model.changed.clear();
    model.receive(&json!({"method":"DOM.childNodeRemoved","params":{"parentNodeId":4,"nodeId":5}}));
    model.receive(&json!({"method":"DOM.childNodeInserted","params":{"parentNodeId":4,"previousNodeId":0,"node":{"nodeId":6,"nodeType":3,"nodeValue":"after"}}}));
    assert!(!model.structure);
    assert!(model.changed.contains(&4));
    model.toggle(4);
    assert!(model.structure);
    assert!(model.rows().iter().any(|row| row.id == 6));
}
