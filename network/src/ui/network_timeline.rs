//! Bounded network activity drawing and time-range filtering; no timers or engine sampling.
use super::range::Drag;
use super::*;
use crate::developer::network::Span;
use objc2_foundation::NSPoint;
#[derive(PartialEq)]
struct Bar {
    key: u64,
    span: Span,
    lane: usize,
}
pub struct TimelineIvars {
    bars: RefCell<Vec<Bar>>,
    selected: Cell<Option<u64>>,
    axis: Cell<(f64, f64)>,
    interval: Cell<Option<(f64, f64)>>,
    gesture: Cell<Option<Drag>>,
    labels: [Retained<NSTextField>; 4],
    reset: Retained<NSButton>,
}
// SAFETY: AppKit owns main-thread drawing and input; retained controls are children.
define_class!(
    #[unsafe(super(NSView))]
    #[ivars=TimelineIvars]
    pub(super) struct Timeline;
    impl Timeline {
        // Use top-down coordinates for content laid out from the leading edge.
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }

        // Paint only the content owned by this view in AppKit's current drawing context.
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            self.draw_activity();
        }

        // Preserve the native resize callback before updating owned layout.
        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn resized(&self, old: NSSize) {
            // SAFETY: Preserve NSView's resize callback and its exact ABI.
            unsafe {
                let _: () = msg_send![super(self),resizeSubviewsWithOldSize:old];
            }
            self.layout_controls();
            self.setNeedsDisplay(true);
        }

        // Clear the selected time interval and restore the complete retained axis.
        #[unsafe(method(clearRange:))]
        fn reset(&self, _: &AnyObject) {
            self.set_interval(None);
        }

        // Begin the owned pointer gesture from the current native event.
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: &NSEvent) {
            self.begin(event);
        }

        // Compute bounded gesture geometry while keeping the opposite edges fixed.
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, event: &NSEvent) {
            self.drag(event);
        }

        // Match native pointer feedback to the visible range handles and movable selected interval.
        #[unsafe(method(resetCursorRects))]
        fn cursors(&self) {
            self.range_cursors();
        }

        // Finish the gesture and persist its final placement.
        #[unsafe(method(mouseUp:))]
        fn up(&self, event: &NSEvent) {
            self.finish(event);
        }
    }
);
impl Timeline {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new() -> Retained<Self> {
        let labels = std::array::from_fn(|_| {
            let field = label("");
            field.setFont(Some(&NSFont::systemFontOfSize(10.)));
            field.setTextColor(Some(&NSColor::secondaryLabelColor()));
            field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            field
        });
        let object = Self::alloc(mtm()).set_ivars(TimelineIvars {
            bars: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            axis: Cell::new((0., 0.)),
            interval: Cell::new(None),
            gesture: Cell::new(None),
            labels,
            reset: range_button(),
        });
        // SAFETY: Initialize this allocated NSView after its ivars exactly once.
        let view: Retained<Self> = unsafe {
            msg_send![super(object),initWithFrame:NSRect::new(NSPoint::ZERO,NSSize::new(500.,88.))]
        };
        for label in &view.ivars().labels {
            view.addSubview(label);
        }
        let button = &view.ivars().reset;
        // SAFETY: The fixed selector is declared on this retained view.
        unsafe {
            button.setTarget(Some(&view));
        }
        view.addSubview(button);
        let title = &view.ivars().labels[0];
        title.setTranslatesAutoresizingMaskIntoConstraints(false);
        button.setTranslatesAutoresizingMaskIntoConstraints(false);
        // Native text baselines align different font sizes without visual offset constants.
        activate(vec![
            title
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&view.leadingAnchor(), 12.),
            title
                .firstBaselineAnchor()
                .constraintEqualToAnchor(&button.firstBaselineAnchor()),
            title
                .trailingAnchor()
                .constraintLessThanOrEqualToAnchor_constant(&button.leadingAnchor(), -8.),
            button
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&view.trailingAnchor(), -12.),
            button
                .topAnchor()
                .constraintEqualToAnchor(&view.topAnchor()),
            button.widthAnchor().constraintEqualToConstant(92.),
            button.heightAnchor().constraintEqualToConstant(20.),
        ]);
        view.setToolTip(Some(&text("Drag to select a time range. Drag either handle to resize it or its interior to move it. Click a request bar to inspect it. Double-click or Clear range resets the filter.")));
        view.setAccessibilityElement(true);
        view
    }

    // Refresh chart geometry and retain a still-valid selected interval.
    pub fn update(&self, rows: &[Summary], selected: Option<u64>, axis: (f64, f64)) {
        let unchanged = self.ivars().axis.get() == axis
            && self
                .ivars()
                .bars
                .borrow()
                .iter()
                .map(|bar| (bar.key, bar.span))
                .eq(rows.iter().filter_map(|row| Some((row.key, row.span?))));
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if unchanged {
            // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
            if self.ivars().selected.replace(selected) != selected {
                self.setNeedsDisplay(true);
            }
            return;
        }
        // Reuse bounded storage; changing selection alone never repacks bars.
        let mut bars = self.ivars().bars.borrow_mut();
        bars.clear();
        let mut ends = [f64::NEG_INFINITY; 6];
        for row in rows {
            // Reject a missing or mismatched value before the following operation relies on it.
            let Some(span) = row.span else {
                continue;
            };
            let lane = ends
                .iter()
                .position(|end| *end <= span.start)
                .unwrap_or_else(|| {
                    ends.iter()
                        .enumerate()
                        .min_by(|a, b| a.1.total_cmp(b.1))
                        .unwrap()
                        .0
                });
            ends[lane] = ends[lane].max(span.end);
            bars.push(Bar {
                key: row.key,
                span,
                lane,
            });
        }
        drop(bars);
        self.ivars().selected.set(selected);
        self.ivars().axis.set(axis);
        // Clear an interval that no longer overlaps any retained request timing.
        if rows.is_empty()
            || self
                .ivars()
                .interval
                .get()
                .is_some_and(|(start, end)| end < axis.0 || start > axis.0 + axis.1)
        {
            self.ivars().interval.set(None);
        }
        self.layout_controls();
        self.setNeedsDisplay(true);
    }

    // Read the retained timeline selection without changing its bounds.
    pub fn interval(&self) -> Option<(f64, f64)> {
        self.ivars().interval.get()
    }

    // Set interval.
    fn set_interval(&self, interval: Option<(f64, f64)>) {
        self.ivars().interval.set(interval);
        self.layout_controls();
        self.setNeedsDisplay(true);
        // Range edges moved, so native resize cursor rectangles must follow the new selection.
        if let Some(window) = self.window() {
            window.invalidateCursorRectsForView(self);
        }
        // Commit once on mouse-up; pointer movement only repaints this view.
        crate::app::refresh_native_network();
    }

    // Position the reset action inside the timeline-owned header.
    fn layout_controls(&self) {
        let (origin, duration) = self.ivars().axis.get();
        let (start, end) = (origin, origin + duration);
        let bounds = self.bounds().size;
        let labels = &self.ivars().labels;
        let title = self.interval().map_or_else(
            || "Activity · drag to filter".into(),
            |(start, end)| {
                format!(
                    "{} – {} · filtered",
                    tick(start - origin, duration),
                    tick(end - origin, duration)
                )
            },
        );
        labels[0].setStringValue(&text(&title));
        let reset = &self.ivars().reset;
        reset.setHidden(self.interval().is_none());
        for (index, time) in [start, (start + end) / 2., end].into_iter().enumerate() {
            let field = &labels[index + 1];
            field.setStringValue(&text(&tick(time - origin, end - start)));
            field.setAlignment(
                [
                    NSTextAlignment::Left,
                    NSTextAlignment::Center,
                    NSTextAlignment::Right,
                ][index],
            );
            field.setFrame(NSRect::new(
                NSPoint::new(
                    12. + index as f64 * (bounds.width - 124.) / 2.,
                    bounds.height - 18.,
                ),
                NSSize::new(100., 16.),
            ));
        }
        self.setAccessibilityLabel(Some(&text(&format!(
            "Network activity, {} requests. {} to {}",
            self.ivars().bars.borrow().len(),
            labels[1].stringValue(),
            labels[3].stringValue()
        ))));
    }

    // Reserve label margins around the interactive request timeline.
    fn plot(&self) -> NSRect {
        NSRect::new(
            NSPoint::new(12., 24.),
            NSSize::new(
                (self.bounds().size.width - 24.).max(1.),
                (self.bounds().size.height - 48.).max(1.),
            ),
        )
    }

    // Map a request timing span into the timeline plot.
    fn rect(&self, bar: &Bar, plot: NSRect) -> Option<NSRect> {
        let (left, duration) = self.ivars().axis.get();
        // Zero-duration requests still need a visible timeline marker.
        if duration <= 0. {
            return None;
        }
        let right = left + duration;
        let (start, end) = (bar.span.start, bar.span.end);
        // Skip bars completely outside the selected time interval.
        if end < left || start > right {
            return None;
        }
        let x = ((start - left) / (right - left)).clamp(0., 1.) * plot.size.width;
        let end = ((end - left) / (right - left)).clamp(0., 1.) * plot.size.width;
        let lane = plot.size.height / 6.;
        Some(NSRect::new(
            NSPoint::new(
                plot.origin.x + x.min(plot.size.width - 2.),
                plot.origin.y + (bar.lane as f64 + 0.2) * lane,
            ),
            NSSize::new((end - x).max(2.).min(plot.size.width), (lane * 0.6).max(2.)),
        ))
    }

    // Draw activity.
    fn draw_activity(&self) {
        let plot = self.plot();
        NSColor::windowBackgroundColor().setFill();
        let frame = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(plot, 4., 4.);
        frame.fill();
        NSColor::separatorColor().setStroke();
        frame.setLineWidth(1.);
        frame.stroke();
        self.draw_interval(plot);
        NSColor::separatorColor()
            .colorWithAlphaComponent(0.4)
            .setFill();
        for fraction in [0.25, 0.5, 0.75] {
            NSRectFill(NSRect::new(
                NSPoint::new(plot.origin.x + fraction * plot.size.width, plot.origin.y),
                NSSize::new(0.5, plot.size.height),
            ));
        }
        let bars = self.ivars().bars.borrow();
        // Draw selection last so a busy lane cannot cover the selected request.
        // Batch unselected bars into two paths instead of allocating a path
        // and color for every request on every repaint.
        let complete = NSBezierPath::bezierPath();
        let ongoing = NSBezierPath::bezierPath();
        for bar in bars
            .iter()
            .filter(|bar| Some(bar.key) != self.ivars().selected.get())
        {
            // A request outside the plotted time axis has no bar to paint.
            let Some(rect) = self.rect(bar, plot) else {
                continue;
            };
            // An ongoing request extends to the current axis end until completion arrives.
            let path = if bar.span.ongoing {
                &ongoing
            } else {
                &complete
            };
            path.appendBezierPathWithRoundedRect_xRadius_yRadius(rect, 1., 1.);
        }
        for (path, alpha) in [(&complete, 0.45), (&ongoing, 0.25)] {
            NSColor::secondaryLabelColor()
                .colorWithAlphaComponent(alpha)
                .setFill();
            path.fill();
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        if let Some(bar) = bars
            .iter()
            .find(|bar| Some(bar.key) == self.ivars().selected.get())
        {
            self.draw_selected(bar, plot);
        }
    }

    // Draw interval.
    fn draw_interval(&self, plot: NSRect) {
        let interval = self
            .ivars()
            .gesture
            .get()
            .map(Drag::interval)
            .or_else(|| self.range_fractions());
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some((start, end)) = interval else {
            return;
        };
        let left = plot.origin.x + start.min(end).clamp(0., 1.) * plot.size.width;
        let right = plot.origin.x + start.max(end).clamp(0., 1.) * plot.size.width;
        NSColor::controlAccentColor()
            .colorWithAlphaComponent(0.08)
            .setFill();
        NSRectFill(NSRect::new(
            NSPoint::new(left, plot.origin.y),
            NSSize::new(right - left, plot.size.height),
        ));
        NSColor::controlAccentColor()
            .colorWithAlphaComponent(0.75)
            .setFill();
        for x in [left, right] {
            NSRectFill(NSRect::new(
                NSPoint::new(x, plot.origin.y),
                NSSize::new(1., plot.size.height),
            ));
            let grip = NSRect::new(
                NSPoint::new(x - 2., plot.origin.y + plot.size.height / 2. - 5.),
                NSSize::new(4., 10.),
            );
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(grip, 2., 2.).fill();
        }
    }

    // Draw selected.
    fn draw_selected(&self, bar: &Bar, plot: NSRect) {
        // The selected request can lie outside the current axis after retention changes.
        let Some(rect) = self.rect(bar, plot) else {
            return;
        };
        NSColor::controlAccentColor()
            // An ongoing selected request extends to the current axis end.
            .colorWithAlphaComponent(if bar.span.ongoing { 0.4 } else { 0.85 })
            .setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 1., 1.).fill();
    }

    // Map a pointer position into a bounded fraction of the plotted time axis.
    fn fraction(&self, event: &NSEvent) -> f64 {
        let plot = self.plot();
        ((self.convertPoint_fromView(event.locationInWindow(), None).x - plot.origin.x)
            / plot.size.width)
            .clamp(0., 1.)
    }

    // Capture the pointer origin and request identity for a timeline gesture.
    fn begin(&self, event: &NSEvent) {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        let plot = self.plot();
        // Ignore pointer presses outside the plotted time area.
        if point.y < plot.origin.y || point.y > plot.origin.y + plot.size.height {
            return;
        }
        // Double-click restores the complete time axis instead of beginning another selection.
        if event.clickCount() == 2 {
            self.set_interval(None);
            return;
        }
        let fraction = self.fraction(event);
        self.ivars().gesture.set(Some(Drag::begin(
            self.range_fractions(),
            fraction,
            plot.size.width,
        )));
    }

    // Preview the bounded selection without rebuilding the request table on each pointer movement.
    fn drag(&self, event: &NSEvent) {
        // A drag outside the plot or after a cancelled press owns no timeline gesture.
        if let Some(drag) = self.ivars().gesture.get() {
            self.ivars()
                .gesture
                .set(Some(drag.at(self.fraction(event))));
            self.setNeedsDisplay(true);
        }
    }

    // Normalize retained times before using them for handle hit testing or pointer feedback.
    fn range_fractions(&self) -> Option<(f64, f64)> {
        let (origin, duration) = self.ivars().axis.get();
        // An empty or instantaneous axis has no meaningful range to resize.
        if duration <= 0. {
            return None;
        }
        self.interval().map(|(start, end)| {
            (
                ((start - origin) / duration).clamp(0., 1.),
                ((end - origin) / duration).clamp(0., 1.),
            )
        })
    }

    // Keep handle targets wider than their paint while making interior translation discoverable.
    fn range_cursors(&self) {
        // No selection means the plot only offers request clicks and new range drags.
        let Some((start, end)) = self.range_fractions() else {
            return;
        };
        let plot = self.plot();
        let left = plot.origin.x + start * plot.size.width;
        let right = plot.origin.x + end * plot.size.width;
        self.addCursorRect_cursor(
            NSRect::new(
                NSPoint::new(left, plot.origin.y),
                NSSize::new(right - left, plot.size.height),
            ),
            &NSCursor::openHandCursor(),
        );
        let cursor = enkel_module_sdk::appkit::resize_cursor(NSCursorFrameResizePosition::Right);
        for x in [left, right] {
            self.addCursorRect_cursor(
                NSRect::new(
                    NSPoint::new(x - 6., plot.origin.y),
                    NSSize::new(12., plot.size.height),
                ),
                &cursor,
            );
        }
    }

    // Resolve the timeline row under the pointer to its stable request key.
    fn key_at(&self, point: NSPoint) -> Option<u64> {
        let plot = self.plot();
        self.ivars()
            .bars
            .borrow()
            .iter()
            .rev()
            .find(|bar| {
                self.rect(bar, plot).is_some_and(|rect| {
                    point.x >= rect.origin.x - 2.
                        && point.x <= rect.origin.x + rect.size.width + 2.
                        && point.y >= rect.origin.y
                        && point.y < rect.origin.y + rect.size.height + 1.
                })
            })
            .map(|bar| bar.key)
    }

    // Commit the completed timeline drag and discard transient gesture state.
    fn finish(&self, event: &NSEvent) {
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(drag) = self.ivars().gesture.take() else {
            return;
        };
        let drag = drag.at(self.fraction(event));
        // Treat a short pointer movement as a request click rather than a time-range selection.
        if drag.is_click(self.plot().size.width) {
            // A click on an existing handle or interval keeps its filter instead of selecting a request.
            if !drag.selects_request() {
                self.setNeedsDisplay(true);
                return;
            }
            let key = self.key_at(self.convertPoint_fromView(event.locationInWindow(), None));
            // Resolve the stable identity before using a row index or applying a retained reply.
            if let Some(key) = key {
                // A bar outside the current range must still reveal its table row.
                self.ivars().interval.set(None);
                self.layout_controls();
                crate::app::select_native_request(Some(key));
            } else {
                self.set_interval(None);
            }
            self.setNeedsDisplay(true);
            return;
        }
        let (start, end) = drag.interval();
        self.set_interval(selection_range(self.ivars().axis.get(), start, end));
        self.setNeedsDisplay(true);
    }
}

// A named action is discoverable without guessing the meaning of a reset icon.
fn range_button() -> Retained<NSButton> {
    // SAFETY: Timeline installs itself as target and implements clearRange:.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &text("Clear range"),
            None,
            Some(sel!(clearRange:)),
            mtm(),
        )
    };
    button.setBezelStyle(NSBezelStyle::Push);
    button.setControlSize(NSControlSize::Small);
    button.setToolTip(Some(&text("Show all requests")));
    button
}

// Normalize drag endpoints into the request timeline selection.
fn selection_range((origin, duration): (f64, f64), start: f64, end: f64) -> Option<(f64, f64)> {
    (duration > 0. && start != end).then_some((
        origin + start.min(end) * duration,
        origin + start.max(end) * duration,
    ))
}

// Check request visibility against the selected timeline interval.
pub(super) fn overlaps(span: Option<Span>, interval: Option<(f64, f64)>) -> bool {
    // Reject a missing or mismatched value before the following operation relies on it.
    let Some((start, end)) = interval else {
        return true;
    };
    span.is_some_and(|span| span.start <= end && span.end >= start)
}

// Format axis labels at a readable unit for the current time span.
fn tick(seconds: f64, span: f64) -> String {
    // Choose label precision that remains readable at the current time scale.
    if seconds < 0.001 {
        return format!("{:.0} µs", seconds * 1e6);
    }
    // Choose label precision that remains readable at the current time scale.
    if span < 0.01 {
        return format!("{seconds:.6} s");
    }
    // Choose label precision that remains readable at the current time scale.
    if span < 1. {
        return format!("{seconds:.3} s");
    }
    crate::developer::network::elapsed(seconds)
}
#[cfg(test)]
#[path = "../../tests/network_chart.rs"]
mod tests;
