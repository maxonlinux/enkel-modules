//! Exercise range resizing and translation at timeline boundaries without native event synthesis.
use super::*;

// Compare normalized coordinates without mistaking floating-point rounding for a gesture change.
fn close(actual: (f64, f64), expected: (f64, f64)) {
    assert!(
        (actual.0 - expected.0).abs() < 1e-10,
        "{actual:?} != {expected:?}"
    );
    assert!(
        (actual.1 - expected.1).abs() < 1e-10,
        "{actual:?} != {expected:?}"
    );
}

// Dragging either visible edge changes only that edge, including when it approaches its opposite edge.
#[test]
fn handles_resize_independently_without_crossing() {
    let range = Some((0.25, 0.75));
    close(
        Drag::begin(range, 0.25, 400.).at(0.1).interval(),
        (0.1, 0.75),
    );
    close(
        Drag::begin(range, 0.75, 400.).at(0.9).interval(),
        (0.25, 0.9),
    );
    close(
        Drag::begin(range, 0.25, 400.).at(0.9).interval(),
        (0.7475, 0.75),
    );
    close(
        Drag::begin(range, 0.75, 400.).at(0.1).interval(),
        (0.25, 0.2525),
    );
}

// Moving the selected interior preserves duration and stops at either end of the time axis.
#[test]
fn moving_range_preserves_width_at_both_boundaries() {
    let drag = Drag::begin(Some((0.25, 0.75)), 0.5, 400.);
    close(drag.at(0.6).interval(), (0.35, 0.85));
    close(drag.at(1.).interval(), (0.5, 1.));
    close(drag.at(0.).interval(), (0., 0.5));
    assert!(!drag.selects_request());
}

// Clicking outside the selection starts a new range; a reverse drag normalizes its endpoints.
#[test]
fn outside_selection_starts_a_new_range() {
    let drag = Drag::begin(Some((0.25, 0.75)), 0.9, 400.);
    assert!(drag.selects_request());
    close(drag.at(0.8).interval(), (0.8, 0.9));
    close(Drag::begin(None, 0.8, 400.).at(0.2).interval(), (0.2, 0.8));
}

// Overlapping pointer targets still choose the nearest handle instead of making the right one unreachable.
#[test]
fn narrow_selection_chooses_the_nearest_handle() {
    let drag = Drag::begin(Some((0.5, 0.51)), 0.51, 400.);
    close(drag.at(0.7).interval(), (0.5, 0.7));
    assert!(!drag.selects_request());
    assert!(drag.is_click(400.));
    assert!(!drag.at(0.7).is_click(400.));
}
