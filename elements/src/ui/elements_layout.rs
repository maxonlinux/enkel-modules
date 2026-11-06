//! Reflow the inspector's internal panes on native resize; workspace geometry stays with WidgetHost.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Columns,
    Stacked,
    Compact,
}

// Choose columns, stacking or tabs from the available inspector geometry.
fn mode(size: NSSize) -> Mode {
    // Use columns when there is enough width for DOM and detail editors.
    if size.width >= super::super::controls::PANE_COLUMNS_WIDTH {
        return Mode::Columns;
    }
    // Use stacking when both panes fit vertically; smaller widgets use compact tabs.
    if size.height >= 360. {
        return Mode::Stacked;
    }
    Mode::Compact
}
pub struct LayoutIvars {
    split: Retained<super::super::panels::DeveloperSplit>,
    dom: Retained<NSScrollView>,
    details: Retained<NSView>,
    tabs: [Retained<NSButton>; 2],
    mode: Cell<Option<Mode>>,
    tree: Cell<bool>,
    more: Retained<NSButton>,
    navigation: Retained<NSView>,
}
// SAFETY: Native view callbacks preserve AppKit ABI and only lay out owned child views.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=LayoutIvars]
    pub(super) struct ElementsLayout;
    unsafe impl NSObjectProtocol for ElementsLayout {}
    impl ElementsLayout {
        // Use top-down coordinates for content laid out from the leading edge.
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }

        // Let AppKit finish layout before choosing the responsive Elements arrangement.
        #[unsafe(method(layout))]
        fn layout_panes(&self) {
            // SAFETY: Preserve NSView layout before updating internal frame-owned panes.
            unsafe {
                let _: () = msg_send![super(self), layout];
            }
            self.reflow();
        }

        // Choose DOM or Styles by the declared compact-tab tag.
        #[unsafe(method(inspectorTab:))]
        fn tab(&self, sender: &NSButton) {
            self.ivars().tree.set(sender.tag() == 0);
            self.reflow();
            // Selecting Styles requests its source as well as revealing the compact pane.
            if sender.tag() == 1 {
                crate::app::element_action(Action::Section(Section::Styles));
            }
        }

        // Paint only the content owned by this view in AppKit's current drawing context.
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            self.draw_tabs();
        }
    }
    unsafe impl NSSplitViewDelegate for ElementsLayout {
        // Use the same invisible pointer padding as Network around the compact visible divider.
        #[unsafe(method(splitView:effectiveRect:forDrawnRect:ofDividerAtIndex:))]
        fn resize_hit_area(&self, split: &NSSplitView, _: NSRect, drawn: NSRect, _: isize) -> NSRect {
            super::super::panels::DeveloperSplit::resize_area(split, drawn)
        }

        // Preserve the native resize callback before updating owned layout.
        #[unsafe(method(splitViewDidResizeSubviews:))]
        fn resized(&self, _: &NSNotification) {
            self.fit_tree();
        }

        // Preserve the user's DOM extent; available height/width belongs to the editor.
        #[unsafe(method(splitView:shouldAdjustSizeOfSubview:))]
        fn adjusts(&self, _: &NSSplitView, view: &NSView) -> bool {
            !std::ptr::eq(view, &*self.ivars().dom as &NSView)
        }

        // Reserve usable DOM space along the active split axis.
        #[unsafe(method(splitView:constrainMinCoordinate:ofSubviewAt:))]
        fn minimum(&self, split: &NSSplitView, _: f64, _: isize) -> f64 {
            // Reserve readable DOM space along the current split axis.
            if split.isVertical() { 180. } else { 90. }
        }

        // Reserve usable detail-editor space along the active split axis.
        #[unsafe(method(splitView:constrainMaxCoordinate:ofSubviewAt:))]
        fn maximum(&self, split: &NSSplitView, _: f64, _: isize) -> f64 {
            let size = split.bounds().size;
            // Reserve readable detail-editor space along the current split axis.
            if split.isVertical() {
                size.width - 240.
            } else {
                size.height - 180.
            }
        }
    }
);
impl ElementsLayout {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(
        dom: &NSScrollView,
        details: &NSView,
        navigation: &NSView,
        delegate: &ElementsDelegate,
    ) -> Retained<Self> {
        let object = super::super::panels::DeveloperSplit::alloc(mtm()).set_ivars(());
        // SAFETY: Initialize an allocated native split view once.
        let split: Retained<super::super::panels::DeveloperSplit> =
            unsafe { msg_send![super(object), init] };
        super::super::panels::DeveloperSplit::configure(&split);
        split.setAccessibilityLabel(Some(&text("Resize DOM and styles")));
        let tabs = ["DOM", "Styles"].map(tab_button);
        let more = inspector_menu(delegate);
        let object = Self::alloc(mtm()).set_ivars(LayoutIvars {
            split,
            dom: dom.retain(),
            details: details.retain(),
            tabs,
            mode: Cell::new(None),
            tree: Cell::new(true),
            navigation: navigation.retain(),
            more,
        });
        // SAFETY: Initialize the allocated view once, then install non-retaining native callbacks.
        let view: Retained<Self> = unsafe { msg_send![super(object), init] };
        view.setClipsToBounds(true);
        // SAFETY: The parent outlives its owned split and buttons; selectors have void/control ABI.
        unsafe {
            view.ivars()
                .split
                .setDelegate(Some(ProtocolObject::from_ref(&*view)));
            for (index, button) in view.ivars().tabs.iter().enumerate() {
                button.setControlSize(NSControlSize::Small);
                button.setTag(index as isize);
                button.setTarget(Some(&view));
                button.setAction(Some(sel!(inspectorTab:)));
                view.addSubview(button);
            }
        }
        view.addSubview(&view.ivars().more);
        view.addSubview(&view.ivars().split);
        view
    }

    // Reveal the selected node's editor when compact tabs hide its pane.
    pub fn show_details(&self) {
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.ivars().tree.replace(false) && self.ivars().mode.get() == Some(Mode::Compact) {
            self.reflow();
        }
    }

    // Choose the layout from actual widget bounds while retaining both pane owners.
    pub fn reflow(&self) {
        let state = self.ivars();
        let size = self.bounds().size;
        // Wait for nonzero widget bounds before choosing the responsive arrangement.
        if size.width <= 0. || size.height <= 0. {
            return;
        }
        let next = mode(size);
        let changed = state.mode.replace(Some(next)) != Some(next);
        for button in &state.tabs {
            button.setHidden(next != Mode::Compact);
        }
        state.more.setHidden(next != Mode::Compact);
        state.split.setHidden(next == Mode::Compact);
        state.navigation.setHidden(next == Mode::Compact);
        // Follow the current placement owner so native frames and workspace constraints do not conflict.
        if next == Mode::Compact {
            self.compact(size);
            self.fit_tree();
            return;
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if changed {
            state.dom.removeFromSuperview();
            state.details.removeFromSuperview();
            state.split.addSubview(&state.dom);
            state.split.addSubview(&state.details);
            state.split.setVertical(next == Mode::Columns);
        }
        state.dom.setHidden(false);
        state.details.setHidden(false);
        // Update the owned split frame only when its container geometry changed.
        if state.split.frame() != self.bounds() {
            state.split.setFrame(self.bounds());
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if changed {
            // Reparenting from Compact can leave the old split bounds unchanged.
            // Refill both panes before positioning the divider; its second pane
            // otherwise keeps the compact fitting height instead of the available space.
            state.split.adjustSubviews();
            // Follow the current placement owner so native frames and workspace constraints do not conflict.
            let extent = if next == Mode::Columns {
                size.width * 0.44
            } else {
                (size.height * 0.36).clamp(90., 200.)
            };
            state.split.setPosition_ofDividerAtIndex(extent, 0);
            // Refit the retained DOM table after its viewport changes.
            if let Some(table) = state
                .dom
                .documentView()
                .and_then(|view| view.downcast::<NSTableView>().ok())
            {
                table.scrollRowToVisible(table.selectedRow());
            }
        }
        self.fit_tree();
        self.setNeedsDisplay(true);
    }

    // Keep long captions reachable while ensuring the column fills its viewport.
    fn fit_tree(&self) {
        let scroll = &self.ivars().dom;
        let viewport = scroll.contentView().bounds().size.width;
        // Do not relayout an unmeasurable or unchanged DOM viewport.
        let Some(table) = scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTableView>().ok())
        else {
            return;
        };
        let width = tree::content_width(&table).max(viewport);
        // Do not relayout an unmeasurable or unchanged DOM viewport.
        if viewport <= 0. || table.frame().size.width == width {
            return;
        }
        // The document keeps the complete caption reachable with horizontal scrolling.
        table.setFrameSize(NSSize::new(width, table.frame().size.height));
        // Follow the current placement owner so native frames and workspace constraints do not conflict.
        if let Some(column) = table.tableColumns().iter().next() {
            column.setWidth(width);
        }
    }

    // Present one pane at a time when both panes cannot fit in the widget.
    fn compact(&self, size: NSSize) {
        let state = self.ivars();
        let tree = state.tree.get();
        state.more.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(size.width - 32., 0.),
            NSSize::new(28., 32.),
        ));
        // Present the chosen compact pane while retaining both native pane owners.
        let (shown, hidden) = if tree {
            (&*state.dom as &NSView, &*state.details)
        } else {
            (&*state.details, &*state.dom as &NSView)
        };
        // Keep presentation changes separate from the retained tool state and session lifetime.
        if !parent_is(shown, self) {
            shown.removeFromSuperview();
            self.addSubview(shown);
        }
        hidden.setHidden(true);
        shown.setHidden(false);
        shown.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(0., 32.),
            NSSize::new(size.width, (size.height - 32.).max(0.)),
        ));
        let mut x = 0.;
        for (index, button) in state.tabs.iter().enumerate() {
            let size = button.intrinsicContentSize();
            button.setFrame(NSRect::new(objc2_foundation::NSPoint::new(x, 0.), size));
            x += size.width;
            set_tab(button, tree == (index == 0));
        }
        self.setNeedsDisplay(true);
    }

    // Draw the compact navigation boundary only in the tabbed layout.
    fn draw_tabs(&self) {
        // Follow the current placement owner so native frames and workspace constraints do not conflict.
        if self.ivars().mode.get() != Some(Mode::Compact) {
            return;
        }
        NSColor::separatorColor().setFill();
        NSRectFill(NSRect::new(
            objc2_foundation::NSPoint::new(0., 31.),
            NSSize::new(self.bounds().size.width, 1.),
        ));
    }
}

// Check native parent identity before reparenting a retained pane.
fn parent_is(view: &NSView, parent: &NSView) -> bool {
    // SAFETY: The main-thread layout owns both live views throughout this query.
    unsafe {
        view.superview()
            .is_some_and(|value| std::ptr::eq(&*value, parent))
    }
}
#[cfg(test)]
#[path = "../../tests/elements_layout.rs"]
mod tests;
