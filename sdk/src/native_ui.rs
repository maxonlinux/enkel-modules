//! Assemble package-owned native widgets using shared AppKit primitives, without importing the browser's window or workspace implementation.
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSRect, NSSize, NSString};

#[path = "ui_primitives.rs"]
mod primitives;
pub use primitives::*;
#[path = "ui_layout.rs"]
mod layout_helpers;
pub use layout_helpers::*;
#[path = "ui_controls.rs"]
pub mod controls;
#[path = "ui_buttons.rs"]
mod shared_controls;

// Reuse native table cells across reloads instead of rebuilding views and constraints.
pub fn text_cell(
    table: &NSTableView,
    key: &NSString,
    padding: f64,
) -> (Retained<NSTableCellView>, Retained<NSTextField>) {
    // SAFETY: No nib owner; this identifier is assigned only to our cells.
    let reused = unsafe { table.makeViewWithIdentifier_owner(key, None) }
        .and_then(|view| view.downcast::<NSTableCellView>().ok());
    // Native views and outlets may be absent during reparenting or teardown; use only live objects.
    if let Some(cell) = reused {
        // SAFETY: The cell's subview hierarchy retains its text field.
        if let Some(title) = unsafe { cell.textField() } {
            return (cell, title);
        }
    }
    let cell = NSTableCellView::new(mtm());
    cell.setIdentifier(Some(key));
    let title = NSTextField::labelWithString(&text(""), mtm());
    title.setFont(Some(&NSFont::systemFontOfSize(11.)));
    title.setMaximumNumberOfLines(1);
    // Native views and outlets may be absent during reparenting or teardown; use only live objects.
    if let Some(cell) = title.cell() {
        cell.setUsesSingleLineMode(true);
        cell.setScrollable(false);
    }
    title.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    title.setContentCompressionResistancePriority_forOrientation(
        250.,
        NSLayoutConstraintOrientation::Horizontal,
    );
    title.setTranslatesAutoresizingMaskIntoConstraints(false);
    cell.addSubview(&title);
    // SAFETY: addSubview above keeps this unretained outlet alive.
    unsafe {
        cell.setTextField(Some(&title));
    }
    activate(vec![
        title
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&cell.leadingAnchor(), padding),
        title
            .trailingAnchor()
            .constraintEqualToAnchor_constant(&cell.trailingAnchor(), -padding),
        title
            .centerYAnchor()
            .constraintEqualToAnchor(&cell.centerYAnchor()),
    ]);
    (cell, title)
}

// Determine whether first responder.
pub fn is_first_responder(editor: &NSTextView) -> bool {
    editor
        .window()
        .filter(|window| window.isKeyWindow())
        .and_then(|window| window.firstResponder())
        .is_some_and(|responder| {
            std::ptr::eq(
                &*responder as *const _ as *const (),
                editor as *const _ as *const (),
            )
        })
}

// Create the shared read-only code view and its independent scroll container.
pub fn text_editor(accessibility: &str) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    let detail_scroll = NSTextView::scrollableTextView(mtm());
    let details = detail_scroll
        .documentView()
        .unwrap()
        .downcast::<NSTextView>()
        .unwrap();
    configure_scroll(&detail_scroll);
    details.setEditable(false);
    details.setSelectable(true);
    details.setRichText(false);
    details.setUsesFindBar(true);
    details.setIncrementalSearchingEnabled(true);
    details.setDrawsBackground(false);
    details.setTextColor(Some(&NSColor::textColor()));
    details.setTextContainerInset(NSSize::new(4., 6.));
    details.setAccessibilityLabel(Some(&text(accessibility)));
    // SAFETY: The retained text view owns its container throughout this main-thread configuration.
    let container = unsafe { details.textContainer() };
    // Configure wrapping only after AppKit supplied the text container.
    if let Some(container) = container {
        container.setContainerSize(NSSize::new(500., f64::MAX));
        container.setWidthTracksTextView(true);
        container.setHeightTracksTextView(false);
        container.setLineFragmentPadding(0.);
    }
    details.setFont(Some(&controls::code_font(11.)));
    (detail_scroll, details)
}
