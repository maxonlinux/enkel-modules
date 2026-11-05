//! Guard inline DOM edits against stale nodes, mutations, Unicode hit offsets and accidental subtree replacement.
use super::*;

// Verify model.
fn model() -> Elements {
    let mut model = Elements {
        active: true,
        ..Elements::default()
    };
    model.nodes.insert(
        4,
        Node {
            id: 4,
            kind: 1,
            name: "div".into(),
            attributes: vec![
                ("id".into(), "sample".into()),
                ("class".into(), "card".into()),
            ],
            children: vec![5],
            count: 1,
            ..Node::default()
        },
    );
    model.nodes.insert(
        5,
        Node {
            id: 5,
            parent: 4,
            kind: 3,
            value: "Hello 🎨".into(),
            ..Node::default()
        },
    );
    model
}

// Verify target.
fn target(model: &Elements, id: i64) -> NodeTarget {
    NodeTarget {
        id,
        epoch: model.epoch,
    }
}

// Verify edit.
fn edit(model: &Elements, id: i64, offset: usize) -> NodeEdit {
    model.prepare_edit(target(model, id), offset).unwrap()
}

// Verify full tag edits update attributes without replacing children.
#[test]
fn full_tag_edits_update_attributes_without_replacing_children() {
    let mut model = model();
    let draft = edit(&model, 4, 20);
    assert_eq!(draft.source(), "<div id=\"sample\" class=\"card\">");
    model.apply_node_edit(
        draft,
        "<div id=\"sample\" class=\"updated\" data-new='yes'>",
    );
    let commands = model.drain();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["method"], "DOM.setAttributesAsText");
    assert_eq!(
        commands[0]["params"],
        json!({"nodeId":4,"text":" id=\"sample\" class=\"updated\" data-new='yes'"})
    );
    assert_eq!(model.nodes[&4].children, vec![5]);
}

// Verify removing an attribute and renaming the tag preserve the subtree.
#[test]
fn removing_an_attribute_and_renaming_the_tag_preserve_the_subtree() {
    let mut model = model();
    let draft = edit(&model, 4, 2);
    model.apply_node_edit(draft, "<section class='card'>");
    let commands = model.drain();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0]["method"], "DOM.removeAttribute");
    assert_eq!(commands[0]["params"], json!({"nodeId":4,"name":"id"}));
    assert_eq!(commands[1]["method"], "DOM.setAttributesAsText");
    assert_eq!(commands[2]["method"], "DOM.setNodeName");
    assert_eq!(model.nodes[&4].children, vec![5]);
}

// Verify full drafts reject attribute mutations and reused node ids.
#[test]
fn full_drafts_reject_attribute_mutations_and_reused_node_ids() {
    let mut model = model();
    let draft = edit(&model, 4, 20);
    model.nodes.get_mut(&4).unwrap().attributes[0].1 = "live".into();
    model.apply_node_edit(draft, "<div class='draft'>");
    assert!(model.drain().is_empty());
    assert!(model.notice.as_deref().unwrap().contains("changed"));
    let draft = edit(&model, 4, 20);
    model.epoch += 1;
    model.apply_node_edit(draft, "<div class='draft'>");
    assert!(model.drain().is_empty());
}

// Verify child mutations and selection changes do not discard the draft.
#[test]
fn child_mutations_and_selection_changes_do_not_discard_the_draft() {
    let mut model = model();
    let draft = edit(&model, 4, 20);
    model.nodes.get_mut(&5).unwrap().value = "live child".into();
    model.selected = Some(5);
    model.apply_node_edit(draft, "<div id='sample' class='new'>");
    assert_eq!(model.drain()[0]["params"]["nodeId"], 4);
    assert_eq!(model.nodes[&5].value, "live child");
    assert_eq!(model.selected, Some(5));
}

// Verify text edits keep the original text node.
#[test]
fn text_edits_keep_the_original_text_node() {
    let mut model = model();
    let draft = edit(&model, 5, 2);
    assert_eq!(draft.source(), "Hello 🎨");
    model.apply_node_edit(draft, "Updated ✨");
    assert_eq!(model.drain()[0]["method"], "DOM.setNodeValue");
}

// Verify unchanged oversized truncated and malformed edits send no commands.
#[test]
fn unchanged_oversized_truncated_and_malformed_edits_send_no_commands() {
    let mut model = model();
    let draft = edit(&model, 4, 20);
    let source = draft.source().to_owned();
    model.apply_node_edit(draft, &source);
    assert!(model.drain().is_empty());
    for source in [
        "x".repeat(65537),
        "<div><span>".into(),
        "<1invalid>".into(),
        "<div x=\"unfinished>".into(),
    ] {
        let draft = edit(&model, 4, 20);
        model.apply_node_edit(draft, &source);
        assert!(model.drain().is_empty());
    }
    model.nodes.get_mut(&4).unwrap().truncated = true;
    assert!(model.prepare_edit(target(&model, 4), 20).is_none());
}

// Verify stale context menus cannot delete the current selection.
#[test]
fn stale_context_menus_cannot_delete_the_current_selection() {
    let mut model = model();
    let old = target(&model, 4);
    model.nodes.remove(&4);
    model.selected = Some(5);
    model.perform_on(old, Action::Delete);
    assert!(model.drain().is_empty());
    model.nodes.insert(
        4,
        Node {
            id: 4,
            kind: 1,
            ..Node::default()
        },
    );
    model.epoch += 1;
    model.perform_on(old, Action::Delete);
    assert!(model.drain().is_empty());
}

// Verify blank space opens the full line with an insertion caret.
#[test]
fn blank_space_opens_the_full_line_with_an_insertion_caret() {
    let mut model = model();
    let draft = edit(&model, 4, usize::MAX);
    assert_eq!(draft.source(), "<div id=\"sample\" class=\"card\" >");
    assert!(draft.selection.is_empty());
    assert_eq!(
        draft.selection.start,
        draft.source().encode_utf16().count() - 1
    );
    model.apply_node_edit(draft, "<div id='sample' class='card' data-added='yes'>");
    assert_eq!(model.drain()[0]["method"], "DOM.setAttributesAsText");
    assert_eq!(model.nodes[&4].children, vec![5]);
}

// Verify late attributes remain selectable and drafts keep unclipped values.
#[test]
fn late_attributes_remain_selectable_and_drafts_keep_unclipped_values() {
    let mut model = model();
    let node = model.nodes.get_mut(&4).unwrap();
    node.attributes = (0..12)
        .map(|index| (format!("data-{index}"), "🎨 & \"".repeat(100)))
        .collect();
    let caption = node.label();
    let offset = caption[..caption.find("data-11=").unwrap()]
        .encode_utf16()
        .count()
        + 2;
    let draft = edit(&model, 4, offset);
    let selected = String::from_utf16(
        &draft.source().encode_utf16().collect::<Vec<_>>()[draft.selection.clone()],
    )
    .unwrap();
    assert_eq!(selected, tag::escape(&"🎨 & \"".repeat(100)));
    assert!(draft.source().contains("data-0="));
    assert!(draft.source().contains("data-11="));
}
