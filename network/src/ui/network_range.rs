//! Own normalized timeline gestures independently of AppKit drawing and request filtering.
#![forbid(unsafe_code)]
#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    New,
    Start,
    End,
    Move,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    part: Part,
    initial: (f64, f64),
    anchor: f64,
    current: f64,
    minimum: f64,
}

impl Drag {
    // Prefer the nearest existing handle, then the selected interior, before starting a new range.
    pub(super) fn begin(interval: Option<(f64, f64)>, anchor: f64, width: f64) -> Self {
        let initial = interval.unwrap_or((anchor, anchor));
        let left = (anchor - initial.0).abs() * width;
        let right = (anchor - initial.1).abs() * width;
        // A missing selection always starts a new interval; handle hit targets are six points wide per side.
        let part = if interval.is_none() {
            Part::New
        } else if left.min(right) <= 6. {
            // Choose the nearest handle even when the selection is narrower than their combined hit targets.
            if left <= right {
                Part::Start
            } else {
                Part::End
            }
        } else if anchor > initial.0 && anchor < initial.1 {
            Part::Move
        } else {
            Part::New
        };
        Self {
            part,
            initial,
            anchor,
            current: anchor,
            minimum: 1. / width.max(1.),
        }
    }

    // Update only the pointer snapshot so previews and commits use the same bounded geometry.
    pub(super) fn at(mut self, position: f64) -> Self {
        self.current = position.clamp(0., 1.);
        self
    }

    // Preserve range width during translation and keep resize handles from crossing or disappearing.
    pub(super) fn interval(self) -> (f64, f64) {
        let (start, end) = self.initial;
        match self.part {
            Part::New => (self.anchor.min(self.current), self.anchor.max(self.current)),
            Part::Start => (self.current.min((end - self.minimum).max(0.)), end),
            Part::End => (start, self.current.max((start + self.minimum).min(1.))),
            Part::Move => {
                let delta = (self.current - self.anchor).clamp(-start, 1. - end);
                (start + delta, end + delta)
            }
        }
    }

    // Request selection is a click only for a new gesture; an existing range remains selected on a click.
    pub(super) fn selects_request(self) -> bool {
        self.part == Part::New
    }

    // Apply a point-based click threshold independently of the timeline's duration or scale.
    pub(super) fn is_click(self, width: f64) -> bool {
        (self.current - self.anchor).abs() * width <= 5.
    }
}

#[cfg(test)]
#[path = "../../tests/network_range.rs"]
mod tests;
