//! Verify Console history and indentation at native UTF-16 caret boundaries.
use super::*;

// Verify multiline history only replaces input at outer lines.
#[test]
fn multiline_history_only_replaces_input_at_outer_lines() {
    let code = "🦀\n  value\nend";
    assert!(history_boundary(code, 2..2, true));
    assert!(!history_boundary(code, 5..5, true));
    assert!(!history_boundary(code, 5..5, false));
    assert!(history_boundary(code, 13..13, false));
    assert!(!history_boundary(code, 0..2, true));
}

// Verify newline keeps current indent at a utf16 caret.
#[test]
fn newline_keeps_current_indent_at_a_utf16_caret() {
    assert_eq!(indented_newline("  let value = '🦀'", 18), "\n  ");
    assert_eq!(indented_newline("  old\n\tnew", 10), "\n\t");
    assert_eq!(indented_newline("code", 0), "\n");
}
