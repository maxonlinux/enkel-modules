//! Guard the approved wide, stacked and compact inspector layouts at their boundaries.
use super::*;

// Verify inspector reflows for all three approved sizes.
#[test]
fn inspector_reflows_for_all_three_approved_sizes() {
    assert!(mode(NSSize::new(820., 440.)) == Mode::Columns);
    assert!(mode(NSSize::new(380., 570.)) == Mode::Stacked);
    assert!(mode(NSSize::new(380., 280.)) == Mode::Compact);
    assert!(mode(NSSize::new(639., 360.)) == Mode::Stacked);
    assert!(mode(NSSize::new(639., 359.)) == Mode::Compact);
    assert!(mode(NSSize::new(640., 280.)) == Mode::Columns);
}
