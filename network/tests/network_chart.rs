//! Verify time-range filtering, raw timestamps and table refresh boundaries.
use super::*;
use crate::developer::network::RequestStatus;

// Verify selection preserves absolute times in both drag directions.
#[test]
fn selection_preserves_absolute_times_in_both_drag_directions() {
    assert_eq!(
        selection_range((100., 2.), 0.25, 0.75),
        Some((100.5, 101.5))
    );
    assert_eq!(
        selection_range((100., 2.), 0.75, 0.25),
        Some((100.5, 101.5))
    );
    assert!(selection_range((0., 0.), 0.1, 0.9).is_none());
    assert!(selection_range((0., 1.), 0.5, 0.5).is_none());
}

// Verify time filter includes overlapping and boundary requests and resets unknown times.
#[test]
fn time_filter_includes_overlapping_and_boundary_requests_and_resets_unknown_times() {
    let span = Some(Span {
        start: 10.,
        end: 12.,
        ongoing: false,
    });
    assert!(overlaps(span, Some((11., 13.))));
    assert!(overlaps(span, Some((12., 13.))));
    assert!(!overlaps(span, Some((13., 14.))));
    assert!(!overlaps(None, Some((11., 13.))));
    assert!(overlaps(None, None));
}

// Verify ticks distinguish short requests and zoomed ranges late in a session.
#[test]
fn ticks_distinguish_short_requests_and_zoomed_ranges_late_in_a_session() {
    assert_eq!(tick(0.0005, 0.001), "500 µs");
    assert_ne!(tick(120.001, 0.001), tick(120.0015, 0.001));
    assert_ne!(tick(0., 0.001), tick(0.0005, 0.001));
}

// Verify axis changes do not reload cells but status and source changes do.
#[test]
fn axis_changes_do_not_reload_cells_but_status_and_source_changes_do() {
    let original = Summary {
        key: 1,
        name: "api".into(),
        method: "GET".into(),
        status: RequestStatus::Http(200),
        kind: "Fetch".into(),
        size: "1 KB".into(),
        time: "2 ms".into(),
        initiator: "app.js:1:1".into(),
        source: "https://example.test/app.js:1:1".into(),
        span: Some(Span {
            start: 0.,
            end: 1.,
            ongoing: false,
        }),
    };
    let mut changed = original.clone();
    changed.span = Some(Span {
        start: 0.,
        end: 0.5,
        ongoing: false,
    });
    assert!(super::super::same_cells(
        std::slice::from_ref(&original),
        std::slice::from_ref(&changed)
    ));
    changed.status = RequestStatus::Http(500);
    assert!(!super::super::same_cells(
        std::slice::from_ref(&original),
        std::slice::from_ref(&changed)
    ));
    changed.status = original.status;
    changed.source = "https://example.test/other/app.js:1:1".into();
    assert!(!super::super::same_cells(&[original], &[changed]));
}
