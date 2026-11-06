//! Share native control, text and scroll primitives between the browser shell and independently loaded widget packages.
use super::*;

// Bridge a Rust string to the immutable string required by AppKit.
pub fn text(value: &str) -> Retained<NSString> {
    NSString::from_str(value)
}

// Require the UI thread before accessing thread-bound native objects.
pub fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("AppKit main thread")
}

// Install the declared native constraints together so the layout has one owner.
pub fn activate(constraints: Vec<Retained<NSLayoutConstraint>>) {
    NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&constraints));
}

// Load and size one system symbol for the shared native controls.
pub fn symbol(name: &str) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&text(name), None)?;
    image.setSize(NSSize::new(14., 14.));
    Some(image)
}

// Create the presentation label used by this owner.
pub fn label(value: &str) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&text(value), mtm());
    field.setFont(Some(&NSFont::systemFontOfSize(13.)));
    field
}

// Use the shared system weight and size for toolbar symbols.
pub fn symbol_configuration(size: f64) -> Retained<NSImageSymbolConfiguration> {
    // SAFETY: NSFontWeightRegular is an immutable AppKit scalar, initialized
    // by the loaded framework before any controls are created.
    let weight = unsafe { NSFontWeightRegular };
    NSImageSymbolConfiguration::configurationWithPointSize_weight(size, weight)
}

// Resolve the semantic editor color to a plain component fill so native tables cannot substitute a wallpaper-tinted material.
pub fn plain_text_background() -> Retained<NSColor> {
    NSColor::textBackgroundColor()
        .colorUsingType(NSColorType::ComponentBased)
        .unwrap_or_else(NSColor::textBackgroundColor)
}

// Use overlay indicators and clip content inside its owned scrolling region.
pub fn configure_scroll(scroll: &NSScrollView) {
    // AppKit's clip views must not paint scrolled rows over headers or adjacent panes.
    scroll.setClipsToBounds(true);
    scroll.contentView().setClipsToBounds(true);
    // Internal constraints already reserve headers; AppKit must not add window-titlebar insets on detach.
    scroll.setAutomaticallyAdjustsContentInsets(false);
    scroll.setAutohidesScrollers(true);
    scroll.setScrollerStyle(NSScrollerStyle::Overlay);
    scroll.setBorderType(NSBorderType::NoBorder);
    scroll.setDrawsBackground(false);
}
