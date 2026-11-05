//! Share constraint-owned native layout builders between browser windows and multi-pane module controls.
use super::*;

// Attach a child to all parent edges without mixing frames and constraints.
pub fn pin_edges(child: &NSView, parent: &NSView) {
    child.setTranslatesAutoresizingMaskIntoConstraints(false);
    parent.addSubview(child);
    activate(vec![
        child
            .leadingAnchor()
            .constraintEqualToAnchor(&parent.leadingAnchor()),
        child
            .trailingAnchor()
            .constraintEqualToAnchor(&parent.trailingAnchor()),
        child
            .topAnchor()
            .constraintEqualToAnchor(&parent.topAnchor()),
        child
            .bottomAnchor()
            .constraintEqualToAnchor(&parent.bottomAnchor()),
    ]);
}

// Arrange native controls with the shared spacing and alignment conventions.
pub fn stack(views: &[&NSView], spacing: f64) -> Retained<NSStackView> {
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(views), mtm());
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    stack.setAlignment(NSLayoutAttribute::CenterY);
    stack.setDistribution(NSStackViewDistribution::Fill);
    stack.setSpacing(spacing);
    stack
}
