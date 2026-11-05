//! Preserve stable DOM row identity through mutation bursts and branch expansion.
use super::*;

// Verify rows.
fn rows(ids: &[i64]) -> Vec<DomRow> {
    ids.iter()
        .map(|id| DomRow {
            id: *id,
            depth: 0,
            expandable: true,
            expanded: false,
            label: format!("node {id}"),
        })
        .collect()
}

// Verify text and attribute bursts never replace native rows.
#[test]
fn text_and_attribute_bursts_never_replace_native_rows() {
    let old = rows(&[1, 2, 3, 4]);
    let mut next = old.clone();
    for version in 0..1000 {
        next[2].label = format!("<div data-version=\"{version}\">");
        let (removed, inserted) = changed_range(&old, &next);
        assert!(removed.is_empty() && inserted.is_empty());
    }
}

// Verify subtree edits keep parent and unrelated rows alive.
#[test]
fn subtree_edits_keep_parent_and_unrelated_rows_alive() {
    assert_eq!(
        changed_range(&rows(&[1, 2, 3, 4]), &rows(&[1, 2, 8, 9, 3, 4])),
        (2..2, 2..4)
    );
    assert_eq!(
        changed_range(&rows(&[1, 2, 8, 9, 3, 4]), &rows(&[1, 2, 3, 4])),
        (2..4, 2..2)
    );
    assert_eq!(
        changed_range(&rows(&[1, 2, 8, 3, 4]), &rows(&[1, 2, 9, 3, 4])),
        (2..3, 2..3)
    );
    assert_eq!(changed_range(&[], &rows(&[1])), (0..0, 0..1));
    assert_eq!(changed_range(&rows(&[1]), &[]), (0..1, 0..0));
}
