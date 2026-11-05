//! Protect CSS formatting from corrupting quoted, functional or commented declarations.
use super::*;

// Verify original css source is borrowed and property reconstruction is only a fallback.
#[test]
fn original_css_source_is_borrowed_and_property_reconstruction_is_only_a_fallback() {
    let source =
        json!({"cssText":"color: red;","cssProperties":[{"name":"ignored","value":"blue"}]});
    let raw = style_text(&source);
    assert!(matches!(raw, Cow::Borrowed(_)));
    assert_eq!(raw.as_ptr(), source["cssText"].as_str().unwrap().as_ptr());
    assert_eq!(raw, "color: red;");
    assert_eq!(
        style_text(&json!({"cssProperties":[{"name":"color","value":"red"}]})),
        "color: red;"
    );
}

// Verify css formatting preserves strings functions and commented declarations.
#[test]
fn css_formatting_preserves_strings_functions_and_commented_declarations() {
    let source = r#"content: "a;b"; background: url("data:image/svg+xml;a;b"); /* color:red; */ --x: {a;b}; padding:24px;"#;
    let formatted = format_declarations(source);
    assert!(formatted.contains("\npadding: 24px;"));
    assert!(formatted.contains(r#"content: "a;b";"#));
    assert!(formatted.contains(r#"url("data:image/svg+xml;a;b")"#));
    assert!(formatted.contains("/* color:red; */ --x: {a;b};"));
}

// Verify box geometry retains fractional content and independent sides.
#[test]
fn box_geometry_retains_fractional_content_and_independent_sides() {
    let mut model = Elements::default();
    model.set_box(&json!({
        "content":[20.5,30.25,260.75,30.25,260.75,90.75,20.5,90.75],
        "padding":[10.,10.,280.,10.,280.,110.,10.,110.],
        "border":[8.,7.,284.,7.,284.,115.,8.,115.],
        "margin":[0.,0.,290.,0.,290.,120.,0.,120.]
    }));
    let geometry = model.box_model.unwrap();
    assert_eq!(geometry.content, [240.25, 60.5]);
    assert_eq!(geometry.margin, [7., 6., 5., 8.]);
    assert_eq!(geometry.border, [3., 4., 5., 2.]);
    assert_eq!(geometry.padding, [20.25, 19.25, 19.25, 10.5]);
}

// Verify matched selectors exclude unrelated branches without parsing selector text.
#[test]
fn matched_selectors_exclude_unrelated_branches_without_parsing_selector_text() {
    let entry = json!({"matchingSelectors":[1,2],"rule":{"selectorList":{
        "text":"article, div, :is(main, aside)",
        "selectors":[{"text":"article"},{"text":"div"},{"text":":is(main, aside)"}]
    }}});
    assert_eq!(matched_selector(&entry), "div, :is(main, aside)");
    assert_eq!(
        matched_selector(&json!({"rule":{"selectorList":{"text":"body"}}})),
        "body"
    );
}

// Verify unchanged style replies preserve rule identity and edit revisions.
#[test]
fn unchanged_style_replies_preserve_rule_identity_and_edit_revisions() {
    let mut model = Elements::default();
    let result = json!({"inlineStyle":{"cssText":"color: red;"}});
    model.set_styles(&result);
    let version = model.styles_revision;
    let rule = model.rules[0].clone();
    model.set_styles(&result);
    assert_eq!(model.styles_revision, version);
    assert!(Rc::ptr_eq(&rule, &model.rules[0]));
    model.set_styles(&json!({"inlineStyle":{"cssText":"color: blue;"}}));
    assert!(model.styles_revision > version);
}
