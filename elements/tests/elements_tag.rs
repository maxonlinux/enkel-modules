//! Verify complete-tag selection, whitespace carets and draft-only navigation with Unicode and quoting.
use super::*;

// Verify selected.
fn selected(source: &str, range: Range<usize>) -> String {
    String::from_utf16(&source.encode_utf16().collect::<Vec<_>>()[range]).unwrap()
}

// Verify click selects only the attribute value inside the full line.
#[test]
fn click_selects_only_the_attribute_value_inside_the_full_line() {
    let source = "<div id=\"sample\" title=\"🎨 &quot;quoted&quot;\">";
    let offset = source[..source.find("title").unwrap()]
        .encode_utf16()
        .count()
        + 2;
    let range = hit_selection(source, source, offset).unwrap();
    assert_eq!(selected(source, range), "🎨 &quot;quoted&quot;");
    assert_eq!(
        selected(source, hit_selection(source, source, 2).unwrap()),
        "div"
    );
}

// Verify spaces and the closing edge are carets not attribute selections.
#[test]
fn spaces_and_the_closing_edge_are_carets_not_attribute_selections() {
    let source = "<div id=\"one\"  class=\"two\">";
    let between = source.find("  ").unwrap();
    assert_eq!(
        hit_selection(source, source, between),
        Some(between..between)
    );
    assert_eq!(
        hit_selection(source, source, between + 1),
        Some(between + 1..between + 1)
    );
    let end = source.len() - 1;
    assert_eq!(hit_selection(source, source, usize::MAX), Some(end..end));
}

// Verify shortened preview maps to full escaped values and later attributes.
#[test]
fn shortened_preview_maps_to_full_escaped_values_and_later_attributes() {
    let attrs = vec![
        ("title".into(), "🎨".repeat(300)),
        ("class".into(), "card".into()),
    ];
    let preview = source("div", &attrs, true);
    let full = source("div", &attrs, false);
    assert_eq!(
        selected(&full, hit_selection(&preview, &full, 7).unwrap()),
        "🎨".repeat(300)
    );
    let hit = preview[..preview.find("class").unwrap()]
        .encode_utf16()
        .count();
    assert_eq!(
        selected(&full, hit_selection(&preview, &full, hit).unwrap()),
        "card"
    );
}

// Verify tab navigates the edited draft then opens a new attribute slot.
#[test]
fn tab_navigates_the_edited_draft_then_opens_a_new_attribute_slot() {
    let mut source = "<div id=\"🎨\" class='card'>".to_owned();
    let fields = Tag::parse(&source).unwrap().fields(&source);
    let next = navigate(&mut source, fields[1].clone(), false).unwrap();
    assert_eq!(selected(&source, next.clone()), "card");
    let end = navigate(&mut source, next.clone(), false).unwrap();
    assert!(source.ends_with("card' >"));
    assert!(end.is_empty());
    assert_eq!(navigate(&mut source, end, true), Some(next));
    assert!(navigate(&mut source, fields[0].clone(), true).is_none());
    let tail = source.len() - 1;
    source.insert_str(tail, "data-new=\"fresh\"");
    let fields = Tag::parse(&source).unwrap().fields(&source);
    assert_eq!(selected(&source, fields.last().unwrap().clone()), "fresh");
}

// Verify malformed or multiple tags are rejected before any dom command.
#[test]
fn malformed_or_multiple_tags_are_rejected_before_any_dom_command() {
    for source in [
        "<div title=\"unfinished>",
        "<div><span>",
        "<div x=\"a\"y=\"b\">",
        "<div x=>",
        "div",
        "<div /foo>",
    ] {
        assert!(Tag::parse(source).is_none(), "{source}");
    }
    assert!(Tag::parse("<div disabled data-one='>' data-two=unquoted>").is_some());
}

// Verify attributes escape quotes entities and control characters without changing values.
#[test]
fn attributes_escape_quotes_entities_and_control_characters_without_changing_values() {
    assert_eq!(escape("\"&<\n\r\t"), "&quot;&amp;&lt;&#10;&#13;&#9;");
    assert!(Tag::parse("<br/>").is_some());
    assert!(Tag::parse("<input disabled />").is_some());
}
