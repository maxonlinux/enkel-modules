//! Verify source-style checks against macro callbacks, expressions and terminating guard clauses.
use super::*;

/// Keep an empty line between a function and the next function's attached comment.
#[test]
fn adjacent_functions_need_a_blank_line() {
    let invalid = "// First.\nfn first() {}\n// Second.\nfn second() {}";
    assert!(
        violations(invalid)
            .iter()
            .any(|error| error.contains("blank line"))
    );
    let valid = "// First.\nfn first() {}\n\n// Second.\nfn second() {}";
    assert!(violations(valid).is_empty());
}

/// Attributes and native macro delimiters must not hide dense callback declarations.
#[test]
fn macro_functions_follow_the_same_spacing_and_comment_rules() {
    let source = "define_class!(impl View { #[unsafe(method(first:))] fn first() {} #[unsafe(method(second:))] fn second() {} });";
    let errors = violations(source);
    assert!(errors.iter().any(|error| error.contains("blank line")));
    assert!(errors.iter().any(|error| error.contains("purpose comment")));
}

/// An explicit return makes a following else unnecessary, including in native macros.
#[test]
fn terminating_guards_cannot_keep_an_else() {
    let source = "// Check availability.\nfn check() { if missing { return; } else { update(); } }";
    assert!(
        violations(source)
            .iter()
            .any(|error| error.contains("return early"))
    );
    assert!(
        violations("// Choose a size.\nfn size() -> usize { if compact { 1 } else { 2 } }")
            .is_empty()
    );
}

/// Function-pointer fields in action tables are data and need no declaration metadata.
#[test]
fn bare_function_types_are_not_mistaken_for_functions() {
    assert!(violations("enum Command { Perform(fn() -> Action) }").is_empty());
}

/// Guard checks leave small optional updates and expression-valued choices alone.
#[test]
fn substantial_function_wrappers_need_an_early_guard() {
    let source = "// Update the view.\nfn update() { if ready { first(); second(); third(); } }";
    assert!(
        violations(source)
            .iter()
            .any(|error| error.contains("early guard"))
    );
    assert!(
        violations("// Update the view.\nfn update() { if ready { update_view(); } }").is_empty()
    );
}
