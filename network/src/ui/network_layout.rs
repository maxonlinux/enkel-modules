//! Keep Network split gestures inside the widget; frame-owned panes cannot constrain browser window UI.
use super::super::controls::PANE_COLUMNS_WIDTH;
use super::super::panels::DeveloperSplit;
use super::*;
use objc2::Message;

const ACTIVITY_HEIGHT: f64 = 84.;
const REQUEST_HEIGHT: f64 = 50.;
const DETAILS_HEIGHT: f64 = 40.;
const REQUEST_WIDTH: f64 = 240.;
const DETAILS_WIDTH: f64 = 280.;

pub(super) struct LayoutIvars {
    activity: Retained<NSView>,
    requests: Retained<DeveloperSplit>,
    details: Retained<NSView>,
}

// SAFETY: AppKit owns pointer tracking; callbacks only change this widget's retained child views.
define_class!(
    #[unsafe(super(DeveloperSplit))]
    #[thread_kind=MainThreadOnly]
    #[ivars=LayoutIvars]
    pub(super) struct NetworkLayout;
    unsafe impl NSObjectProtocol for NetworkLayout {}
    impl NetworkLayout {
        // Adapt after actual frame changes, including initial attachment and autosave restoration.
        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn layout_panes(&self, old: NSSize) {
            // SAFETY: Preserve the inherited native resize callback before adapting child panes.
            unsafe {
                let _: () = msg_send![super(self), resizeSubviewsWithOldSize:old];
            }
            self.reflow();
        }
    }
    unsafe impl NSSplitViewDelegate for NetworkLayout {
        // Preserve an easy pointer target while the shared divider uses less layout space.
        #[unsafe(method(splitView:effectiveRect:forDrawnRect:ofDividerAtIndex:))]
        fn resize_hit_area(&self, split: &NSSplitView, _: NSRect, drawn: NSRect, _: isize) -> NSRect {
            DeveloperSplit::resize_area(split, drawn)
        }

        // Activity stays fixed on widget resize; stacked details keeps its height while requests absorbs space.
        #[unsafe(method(splitView:shouldAdjustSizeOfSubview:))]
        fn adjusts(&self, split: &NSSplitView, view: &NSView) -> bool {
            self.should_resize_pane(split, view)
        }

        // Pointer constraints apply only to the active divider, never to ancestor view fitting sizes.
        #[unsafe(method(splitView:constrainMinCoordinate:ofSubviewAt:))]
        fn minimum(&self, split: &NSSplitView, _: f64, _: isize) -> f64 {
            // The outer divider reserves usable activity controls above the lower pair.
            if self.is_activity_split(split) {
                return ACTIVITY_HEIGHT;
            }
            // The request pane needs width in columns and height in the stacked arrangement.
            if split.isVertical() { REQUEST_WIDTH } else { REQUEST_HEIGHT }
        }

        // Reserve reachable content on the far side of this divider without limiting the whole window.
        #[unsafe(method(splitView:constrainMaxCoordinate:ofSubviewAt:))]
        fn maximum(&self, split: &NSSplitView, _: f64, _: isize) -> f64 {
            // The outer gesture must reserve the lower panes' required height locally.
            if self.is_activity_split(split) {
                return self.activity_maximum();
            }
            let size = split.bounds().size;
            // Details keeps a nonzero extent along whichever axis the user is resizing.
            if split.isVertical() {
                (size.width - split.dividerThickness() - DETAILS_WIDTH).max(REQUEST_WIDTH)
            } else {
                (size.height - split.dividerThickness() - DETAILS_HEIGHT).max(REQUEST_HEIGHT)
            }
        }

    }
);

impl NetworkLayout {
    // Build two ordinary native splits; their panes use frames, with no split-controller constraints.
    pub(super) fn new(activity: &NSView, list: &NSView, details: &NSView) -> Retained<Self> {
        let object = DeveloperSplit::alloc(mtm()).set_ivars(());
        // SAFETY: Initialize the allocated inner split exactly once.
        let requests: Retained<DeveloperSplit> = unsafe { msg_send![super(object), init] };
        DeveloperSplit::configure(&requests);
        requests.setAccessibilityLabel(Some(&text("Resize requests and details")));
        requests.addArrangedSubview(list);
        details.setHidden(true);
        let object = Self::alloc(mtm()).set_ivars(LayoutIvars {
            activity: activity.retain(),
            requests,
            details: details.retain(),
        });
        // SAFETY: Initialize the allocated outer split once, before installing its non-retaining delegates.
        let split: Retained<Self> = unsafe { msg_send![super(object), init] };
        DeveloperSplit::configure(&split);
        split.setAccessibilityLabel(Some(&text("Resize activity and requests")));
        split.addArrangedSubview(activity);
        split.addArrangedSubview(&split.ivars().requests);
        // Non-retaining delegates stay alive with their owning layout and its retained child split.
        split.setDelegate(Some(ProtocolObject::from_ref(&*split)));
        split
            .ivars()
            .requests
            .setDelegate(Some(ProtocolObject::from_ref(&*split)));
        split
    }

    // Keep native divider preferences isolated between the test and regular browser profiles.
    pub(super) fn autosave(&self, key: &str) {
        self.setAutosaveName(Some(&text(key)));
        self.ivars()
            .requests
            .setAutosaveName(Some(&text(&format!("{key}.requests"))));
    }

    // Remove the absent pane from native arrangement, so its divider and space disappear together.
    pub(super) fn set_details_visible(&self, visible: bool) {
        let state = self.ivars();
        // An unchanged selection must preserve the user's divider position and responder.
        if state.details.isHidden() != visible {
            return;
        }
        let frame = state.details.frame().size;
        let preferred = if state.requests.isVertical() {
            frame.width
        } else {
            frame.height
        };
        state.details.setHidden(!visible);
        // Only the selected request contributes a details pane to the native split arrangement.
        if visible {
            state.requests.addArrangedSubview(&state.details);
        } else {
            state.details.removeFromSuperview();
        }
        self.reflow();
        // Restore a retained extent or choose a balanced first opening after space has been reconciled.
        if visible {
            self.reveal_details(preferred);
        }
    }

    // Give a newly opened pane usable space, borrowing from activity rather than squeezing requests.
    fn reveal_details(&self, preferred: f64) {
        let state = self.ivars();
        let split = &state.requests;
        let columns = split.isVertical();
        let size = split.bounds().size;
        let activity = !state.activity.isHidden();
        let available = if columns {
            size.width
        } else if activity {
            self.bounds().size.height - self.dividerThickness() - ACTIVITY_HEIGHT
        } else {
            size.height
        };
        let (leading, trailing) = if columns {
            (REQUEST_WIDTH, DETAILS_WIDTH)
        } else {
            (REQUEST_HEIGHT, DETAILS_HEIGHT)
        };
        let content = available - split.dividerThickness();
        let maximum = (content - leading).max(trailing);
        // A never-arranged pane has zero extent; its first opening shares available content evenly.
        let desired = if preferred > 0. {
            preferred
        } else {
            content / 2.
        };
        let desired = desired.clamp(trailing, maximum);
        // Vertical revelation can require reclaiming activity space even when both hard minima already fit.
        if !columns && activity {
            let maximum_activity = self.bounds().size.height
                - self.dividerThickness()
                - leading
                - split.dividerThickness()
                - desired;
            self.setPosition_ofDividerAtIndex(
                state
                    .activity
                    .frame()
                    .size
                    .height
                    .min(maximum_activity)
                    .max(ACTIVITY_HEIGHT),
                0,
            );
        }
        let size = split.bounds().size;
        let extent = if columns { size.width } else { size.height };
        split.setPosition_ofDividerAtIndex(
            (extent - split.dividerThickness() - desired).max(leading),
            0,
        );
        split.setNeedsDisplay(true);
    }

    // Choose the inner split axis and keep activity only when the lower content remains usable.
    fn reflow(&self) {
        let size = self.bounds().size;
        // Initial construction has no geometry; native layout supplies it after attachment.
        if size.width <= 0. || size.height <= 0. {
            return;
        }
        let state = self.ivars();
        let columns = size.width >= PANE_COLUMNS_WIDTH;
        let reoriented = state.requests.isVertical() != columns;
        // Reorient only on a breakpoint crossing; ordinary resizes preserve the current divider.
        if reoriented {
            state.requests.setVertical(columns);
        }
        let show_activity =
            size.height >= ACTIVITY_HEIGHT + self.dividerThickness() + self.content_minimum();
        // Reconcile membership only when available height changes activity visibility.
        if state.activity.isHidden() == show_activity {
            state.activity.setHidden(!show_activity);
            // Activity always precedes the lower pair when it returns to the arrangement.
            if show_activity {
                self.insertArrangedSubview_atIndex(&state.activity, 0);
            } else {
                state.activity.removeFromSuperview();
            }
            self.adjustSubviews();
        }
        // Opening details or changing axis can increase the lower minimum without moving the outer divider.
        if show_activity {
            let height = state.activity.frame().size.height;
            let maximum = size.height - self.dividerThickness() - self.content_minimum();
            let bounded = height.clamp(ACTIVITY_HEIGHT, maximum);
            // Reserve lower-pane space before fitting their divider; tracking constraints alone cannot do this.
            if (bounded - height).abs() >= 0.5 {
                self.setPosition_ofDividerAtIndex(bounded, 0);
            }
        }
        self.fit_requests();
        // Widths do not represent useful heights after an axis change; balance only that transition.
        if reoriented && !state.details.isHidden() {
            self.reveal_details(0.);
        }
        self.setNeedsDisplay(true);
    }

    // Native pane membership determines the lower pair's required height without copied layout state.
    fn content_minimum(&self) -> f64 {
        let state = self.ivars();
        // Side-by-side details shares height with requests; absent details needs no second row.
        if state.requests.isVertical() || state.details.isHidden() {
            return REQUEST_HEIGHT;
        }
        REQUEST_HEIGHT + state.requests.dividerThickness() + DETAILS_HEIGHT
    }

    // Stop activity dragging at the request minimum without consuming the stacked details height.
    fn activity_maximum(&self) -> f64 {
        let state = self.ivars();
        let mut reserved = self.content_minimum();
        // In a stack this gesture owns the table height; the details divider owns details height.
        if !state.requests.isVertical() && !state.details.isHidden() {
            reserved += (state.details.frame().size.height - DETAILS_HEIGHT).max(0.);
        }
        (self.bounds().size.height - self.dividerThickness() - reserved).max(ACTIVITY_HEIGHT)
    }

    // Refill the native request split after membership or axis changes, preserving nonzero pane extents.
    fn fit_requests(&self) {
        let split = &self.ivars().requests;
        split.adjustSubviews();
        let panes = split.arrangedSubviews();
        // One arranged pane has no divider; never ask AppKit to position a nonexistent boundary.
        if panes.len() > 1 {
            let size = split.bounds().size;
            let first = panes.objectAtIndex(0).frame().size;
            let (extent, position, minimum, trailing) = if split.isVertical() {
                (size.width, first.width, REQUEST_WIDTH, DETAILS_WIDTH)
            } else {
                (size.height, first.height, REQUEST_HEIGHT, DETAILS_HEIGHT)
            };
            let maximum = (extent - split.dividerThickness() - trailing).max(minimum);
            let bounded = position.clamp(minimum, maximum);
            // Native proportional resizing may cross a pane minimum; valid user geometry stays unchanged.
            if (bounded - position).abs() >= 0.5 {
                split.setPosition_ofDividerAtIndex(bounded, 0);
            }
        }
        split.setNeedsDisplay(true);
    }

    // Distinguish the two owned native splits by object identity, independently of displayed labels.
    fn is_activity_split(&self, split: &NSSplitView) -> bool {
        std::ptr::eq(split, self as &NSSplitView)
    }

    // Let requests absorb vertical space changes while stacked details keeps its native height.
    fn should_resize_pane(&self, split: &NSSplitView, view: &NSView) -> bool {
        // The outer divider changes the lower region without redistributing activity on window resize.
        if self.is_activity_split(split) {
            return !std::ptr::eq(view, &*self.ivars().activity);
        }
        split.isVertical() || !std::ptr::eq(view, &*self.ivars().details)
    }
}
