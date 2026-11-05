//! Keep expanded CSS previews scoped, bounded and safe for multibyte text.
use super::*;

// Verify disclosure previews are scoped and bounded without splitting unicode.
#[test]
fn disclosure_previews_are_scoped_and_bounded_without_splitting_unicode() {
    let rules: Vec<_> = (0..128)
        .map(|index| {
            let mut rule = StyleRule::default();
            rule.selector = ".card".into();
            rule.source = "app.css:18".into();
            rule.body = "content: \"🎨\";".repeat(1024);
            rule.category = index % 3;
            Rc::new(rule)
        })
        .collect();
    let value = rule_preview(&rules, 1);
    assert!(value.len() <= 65536);
    assert!(value.contains("🎨"));
    assert!(value.ends_with("/* Preview limited to 64 KiB */"));
    assert!(rule_preview(&rules, 4).is_empty());
}
