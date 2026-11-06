//! Shared developer controls: title/search fields, compact tabs, actions and region boundaries.

pub use super::layout_helpers::stack;
pub use super::shared_controls::icon_button;

pub use super::shared_controls::text_button;
use super::*;

use objc2::AnyThread;

use objc2_foundation::{NSAttributedString, NSDictionary};

use objc2_foundation::{NSMutableAttributedString, NSRange};

// Shared single-line widget actions; Console can grow above this minimum for multiline input.

pub const ACTION_ROW_HEIGHT: f64 = 26.;

pub const ACTION_ROW_BOTTOM: f64 = 4.;

pub const ACTION_ROW_TOP: f64 = 5.;
// Both two-pane editors need this width before placing their content side by side.

pub const PANE_COLUMNS_WIDTH: f64 = 640.;

// Give every bottom action row the same gutter and full-width native separator.
pub fn action_row_separator(host: &NSView, row: &NSView) -> Retained<NSView> {
    let line = NSBox::new(mtm());
    line.setBoxType(NSBoxType::Separator);
    line.setTranslatesAutoresizingMaskIntoConstraints(false);
    host.addSubview(&line);
    activate(vec![
        row.bottomAnchor()
            .constraintEqualToAnchor_constant(&host.bottomAnchor(), -ACTION_ROW_BOTTOM),
        line.leadingAnchor()
            .constraintEqualToAnchor(&host.leadingAnchor()),
        line.trailingAnchor()
            .constraintEqualToAnchor(&host.trailingAnchor()),
        line.bottomAnchor()
            .constraintEqualToAnchor_constant(&row.topAnchor(), -ACTION_ROW_TOP),
        line.heightAnchor().constraintEqualToConstant(0.5),
    ]);
    line.into_super()
}

// Build the Elements overflow action using the common icon-button owner.
pub fn inspector_menu(delegate: &AnyObject) -> Retained<NSButton> {
    let button = icon_button(delegate, "ellipsis", "Element actions", sel!(elementMenu:));
    button.setSymbolConfiguration(Some(&symbol_configuration(18.)));
    button
}

// SAFETY: Native wheel callbacks change only this scroll view's own clip origin.

define_class!(
    #[unsafe(super(NSScrollView))]
    struct TabStrip;
    impl TabStrip {
        // Keep navigation scrollable independently of AppKit's visible-scroller enablement.
        #[unsafe(method(scrollWheel:))]
        fn wheel(&self, event: &NSEvent) {
            self.scroll_tabs(event);
        }
    }
);

impl TabStrip {
    // Apply native wheel and momentum deltas to the bounded horizontal tab document.
    fn scroll_tabs(&self, event: &NSEvent) {
        // A detached or initializing strip has no document to scroll.
        let Some(document) = self.documentView() else {
            return;
        };
        let clip = self.contentView();
        let bounds = clip.bounds();
        let maximum = (document.frame().size.width - bounds.size.width).max(0.);
        // A plain mouse wheel also navigates overflow tabs; trackpads retain their horizontal axis.
        let delta = if event.scrollingDeltaX() != 0. {
            event.scrollingDeltaX()
        } else {
            event.scrollingDeltaY()
        };
        // Precise trackpad deltas are points; traditional wheel deltas use the native line increment.
        let scale = if event.hasPreciseScrollingDeltas() {
            1.
        } else {
            self.horizontalLineScroll()
        };
        clip.scrollToPoint(objc2_foundation::NSPoint::new(
            (bounds.origin.x - delta * scale).clamp(0., maximum),
            bounds.origin.y,
        ));
        self.reflectScrolledClipView(&clip);
    }
}

// Host native tab buttons without a scroll indicator covering their labels or underlines.
pub fn tab_strip(document: &NSView) -> Retained<NSScrollView> {
    let object = TabStrip::alloc(mtm()).set_ivars(());
    // SAFETY: Initialize this native scroll subclass once before attaching its tab document.
    let scroll: Retained<TabStrip> =
        unsafe { msg_send![super(object), initWithFrame:NSRect::ZERO] };
    super::configure_scroll(&scroll);
    scroll.setDocumentView(Some(document));
    scroll.into_super()
}

// SAFETY: Offset only title painting; AppKit keeps the icon and title vertically aligned.

define_class!(
    #[unsafe(super(NSButtonCell))]
    #[thread_kind=MainThreadOnly]
    #[ivars=()]
    struct DisclosureCell;
    impl DisclosureCell {
        // Draw title.
        #[unsafe(method(drawTitle:withFrame:inView:))]
        fn draw_title(&self, title: &NSAttributedString, mut frame: NSRect, view: &NSView) -> NSRect {
            frame.size.width = (frame.origin.x + frame.size.width - 18.).max(0.);
            frame.origin.x = 18.;
            // SAFETY: Preserve native title painting and its exact return/signature.
            unsafe { msg_send![super(self),drawTitle:title,withFrame:frame,inView:view] }
        }
    }
);

// Create a native disclosure cell with the shared section typography.
pub fn disclosure_button(delegate: &AnyObject, title: &str) -> Retained<NSButton> {
    let object = DisclosureCell::alloc(mtm()).set_ivars(());
    // SAFETY: Initialize the custom native cell once; its button retains it.
    let cell: Retained<DisclosureCell> =
        unsafe { msg_send![super(object),initTextCell:&*text(title)] };
    let button = NSButton::new(mtm());
    button.setCell(Some(&cell));
    button.setBordered(false);
    button.setControlSize(NSControlSize::Small);
    button.setFont(Some(&NSFont::systemFontOfSize(12.)));
    button.setAlignment(NSTextAlignment::Left);
    button.setImagePosition(NSCellImagePosition::ImageLeft);
    button.setImage(symbol("chevron.right").as_deref());
    // SAFETY: The owning widget retains this fixed disclosure action's delegate.
    unsafe {
        button.setTarget(Some(delegate));
        button.setAction(Some(sel!(elementDisclosure:)));
    }
    button
}

pub struct TabIvars {
    titles: [Retained<NSAttributedString>; 3],
    size: NSSize,
}
// SAFETY: Native NSButton owns input/accessibility; cached titles share one centered geometry.

define_class!(
    #[unsafe(super(NSButton))]
    #[thread_kind=MainThreadOnly]
    #[ivars=TabIvars]
    struct InspectorTab;
    impl InspectorTab {
        // Paint only the content owned by this view in AppKit's current drawing context.
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            self.draw_title();
        }

        // Expose the cached caption extent to native layout without remeasuring text.
        #[unsafe(method(intrinsicContentSize))]
        fn size(&self) -> NSSize {
            self.ivars().size
        }
    }
);

impl InspectorTab {
    // Draw title.
    fn draw_title(&self) {
        let active = self.state() == NSControlStateValueOn;
        // Disabled tabs share the same muted presentation in every widget.
        let index = if self.isEnabled() {
            usize::from(active)
        } else {
            2
        };
        let title = &self.ivars().titles[index];
        let bounds = self.bounds();
        let size = title.size();
        let x = (bounds.size.width - size.width) / 2.;
        title.drawAtPoint(objc2_foundation::NSPoint::new(
            x,
            (bounds.size.height - size.height) / 2.,
        ));
        // Inactive headers do not need the active focus marker.
        if !active {
            return;
        }
        // Place the header marker correctly for either native coordinate orientation.
        let y = if self.isFlipped() {
            bounds.size.height - 2.
        } else {
            0.
        };
        NSColor::controlAccentColor().setFill();
        NSRectFill(NSRect::new(
            objc2_foundation::NSPoint::new(x, y),
            NSSize::new(size.width, 2.),
        ));
    }
}

// Build the shared compact tab appearance without encoding behavior in its title.
pub fn tab_button(title: &str) -> Retained<NSButton> {
    let font = NSFont::systemFontOfSize(12.);
    let titles = [
        NSColor::secondaryLabelColor(),
        NSColor::labelColor(),
        NSColor::disabledControlTextColor(),
    ]
    .map(|color| attributed(title, &font, &color));
    // Both frame-based and Auto Layout tab rows use the same eight-point text insets.
    let size = NSSize::new(titles[0].size().width + 16., 32.);
    let object = InspectorTab::alloc(mtm()).set_ivars(TabIvars { titles, size });
    // SAFETY: Initialize the allocated native button once before configuring it.
    let button: Retained<InspectorTab> =
        unsafe { msg_send![super(object),initWithFrame:NSRect::ZERO] };
    button.setTitle(&text(title));
    button.setBordered(false);
    button.setFont(Some(&font));
    button.into_super()
}

// Apply selection appearance to the tab whose declared target is active.
pub fn set_tab(button: &NSButton, active: bool) {
    // Derive the native button state from the declared active tab.
    let state = if active {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    };
    // Skip unchanged native state to avoid redundant redraws and callbacks.
    if button.state() == state {
        return;
    }
    button.setState(state);
    NSView::setNeedsDisplay(button, true);
}

// Region boundaries belong to their controls and resize through internal Auto Layout.
pub fn boundary(view: &NSView) {
    let line = NSBox::new(mtm());
    line.setBoxType(NSBoxType::Separator);
    line.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.addSubview(&line);
    activate(vec![
        line.leadingAnchor()
            .constraintEqualToAnchor(&view.leadingAnchor()),
        line.trailingAnchor()
            .constraintEqualToAnchor(&view.trailingAnchor()),
        line.bottomAnchor()
            .constraintEqualToAnchor(&view.bottomAnchor()),
        line.heightAnchor().constraintEqualToConstant(0.5),
    ]);
}

// Safe native text boundary shared by tab titles and the numeric box diagram.
pub fn attributed(value: &str, font: &NSFont, color: &NSColor) -> Retained<NSAttributedString> {
    // SAFETY: AppKit's font/color keys receive their exact documented object types.
    unsafe {
        let attributes = NSDictionary::<NSString, AnyObject>::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[font as &AnyObject, color as &AnyObject],
        );
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &text(value),
            Some(&attributes),
        )
    }
}

// Apply a typed foreground color to attributed text, including plain-text editors with syntax decoration.
pub fn text_color(value: &NSMutableAttributedString, color: &NSColor, range: NSRange) {
    // SAFETY: The immutable foreground-color key receives exactly its documented NSColor value type.
    unsafe {
        value.addAttribute_value_range(NSForegroundColorAttributeName, color, range);
    }
}

// Use the shared native monospaced font for code and measured DOM captions.
pub fn code_font(size: f64) -> Retained<NSFont> {
    // SAFETY: Read AppKit's immutable regular font-weight constant at this one boundary.
    NSFont::monospacedSystemFontOfSize_weight(size, unsafe { NSFontWeightRegular })
}

// Shared code-entry behavior: no prose substitutions or independent horizontal document growth.
pub fn configure_code_input(view: &NSTextView) {
    view.setRichText(false);
    view.setAllowsUndo(true);
    view.setAutomaticQuoteSubstitutionEnabled(false);
    view.setAutomaticDashSubstitutionEnabled(false);
    view.setAutomaticTextReplacementEnabled(false);
    view.setAutomaticSpellingCorrectionEnabled(false);
    view.setContinuousSpellCheckingEnabled(false);
    view.setVerticallyResizable(true);
    view.setHorizontallyResizable(false);
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    view.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
}

// SAFETY: Preserve NSSplitView tracking and sizing; the shared drawing selectors use AppKit's NSRect and retained NSColor ABI.

define_class!(
    #[unsafe(super(NSSplitView))]
    pub struct DeveloperSplit;
    impl DeveloperSplit {
        // Match the browser's shared pane boundary: composite a neutral semantic stroke over an opaque native surface.
        #[unsafe(method(drawDividerInRect:))]
        fn draw_divider(&self, rect: NSRect) {
            NSColor::windowBackgroundColor().setFill();
            NSRectFill(rect);
            self.dividerColor().setFill();
            // Semantic strokes contain alpha; SourceOver preserves the opaque surface instead of copying transparent RGBA over it.
            NSRectFillUsingOperation(rect, NSCompositingOperation::SourceOver);
        }

        // A thin divider must lighten Dark Aqua and darken Aqua, rather than use NSSplitView's default white-pane contrast.
        #[unsafe(method_id(dividerColor))]
        fn divider_color(&self) -> Retained<NSColor> { NSColor::quaternaryLabelColor() }
    }
);

impl DeveloperSplit {
    // All widget boundaries share a thin native divider; layouts own their orientation, minima and accessibility labels.
    pub fn configure(split: &NSSplitView) {
        split.setDividerStyle(NSSplitViewDividerStyle::Thin);
        split.setClipsToBounds(true);
    }

    // Keep a 16 pt native pointer target around the compact divider without reserving extra space.
    pub fn resize_area(split: &NSSplitView, mut drawn: NSRect) -> NSRect {
        // Extend only across the divider so the full pane boundary remains draggable.
        if split.isVertical() {
            drawn.origin.x -= (16. - drawn.size.width) / 2.;
            drawn.size.width = 16.;
            return drawn;
        }
        drawn.origin.y -= (16. - drawn.size.height) / 2.;
        drawn.size.height = 16.;
        drawn
    }
}
