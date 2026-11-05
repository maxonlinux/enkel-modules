//! Test sse behavior and guard its lifecycle and boundary cases.
use super::*;

// Verify parse.
fn parse(parser: &mut Parser, bytes: &[u8]) -> Vec<Event> {
    bytes.iter().filter_map(|byte| parser.byte(*byte)).collect()
}

// Verify framing survives every byte boundary and preserves ids and multiline data.
#[test]
fn framing_survives_every_byte_boundary_and_preserves_ids_and_multiline_data() {
    let input = "\u{feff}: keepalive\r\nid: 42\r\nevent: update\r\ndata: 🦀\r\ndata: second\r\n\r\ndata:\n\nid: bad\0id\ndata: last\n\n";
    for split in 0..=input.len() {
        let mut parser = Parser::default();
        let mut events = parse(&mut parser, &input.as_bytes()[..split]);
        events.extend(parse(&mut parser, &input.as_bytes()[split..]));
        assert_eq!(events.len(), 3);
        assert_eq!(
            (&*events[0].name, &*events[0].id, &*events[0].data),
            ("update", "42", "🦀\nsecond")
        );
        assert_eq!(
            (&*events[1].name, &*events[1].id, &*events[1].data),
            ("message", "42", "")
        );
        assert_eq!(events[2].id, "42");
    }
}

// Verify incomplete events are not dispatched and id can be reset.
#[test]
fn incomplete_events_are_not_dispatched_and_id_can_be_reset() {
    let mut parser = Parser::default();
    assert!(parse(&mut parser, b"id: previous\n\nid:\ndata: incomplete").is_empty());
    let events = parse(&mut parser, b"\n\n");
    assert_eq!(events[0].id, "");
    assert_eq!(events[0].data, "incomplete");
}

// Verify oversized lines and multiline events remain bounded without losing framing.
#[test]
fn oversized_lines_and_multiline_events_remain_bounded_without_losing_framing() {
    let mut parser = Parser::default();
    assert!(
        parse(
            &mut parser,
            format!("data: {}", "🦀".repeat(65536)).as_bytes()
        )
        .is_empty()
    );
    assert!(parser.line.len() <= MAX_MESSAGE_BYTES);
    let events = parse(&mut parser, b"\n\ndata: next\n\n");
    assert!(events[0].data.contains("truncated"));
    assert!(events[0].data.len() < MAX_MESSAGE_BYTES + 32);
    assert_eq!(events[1].data, "next");
    for _ in 0..1000 {
        parse(&mut parser, b"data: abcdefghijklmnopqrstuvwxyz\n");
    }
    assert!(parser.data.len() < MAX_MESSAGE_BYTES + 32);
    assert!(parser.data.contains("truncated"));
}
