//! Share native action buttons and stacks between browser chrome, settings and installed widget packages.
use super::*;

// Build an icon action with the shared pointer target and accessibility label.
pub fn icon_button(
    delegate: &AnyObject,
    icon: &str,
    tip: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let button = text_button(delegate, "", action);
    button.setImage(symbol(icon).as_deref());
    button.setSymbolConfiguration(Some(&symbol_configuration(13.)));
    button.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    button.setToolTip(Some(&text(tip)));
    button.setAccessibilityLabel(Some(&text(tip)));
    button
}

// Create one compact native action with the declared retained target.
pub fn text_button(
    delegate: &AnyObject,
    title: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    // SAFETY: These fixed selectors accept a control sender; the widget retains the delegate.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&text(title), Some(delegate), Some(action), mtm())
    };
    button.setBordered(false);
    button.setControlSize(NSControlSize::Small);
    button
}
