//! Own expandable CSS sections and their lazily created box diagram.
use super::super::controls::disclosure_button;
use super::*;
use crate::developer::elements::BoxModel;
use crate::ui::controls::attributed;
use objc2_foundation::{NSAttributedString, NSPoint};
use std::rc::Rc;

pub(super) struct Sections {
    pub view: Retained<NSStackView>,
    buttons: [Retained<NSButton>; 3],
    bodies: [Retained<NSView>; 3],
    expanded: [Cell<bool>; 3],
    rules: RefCell<Vec<Rc<StyleRule>>>,
    version: Cell<u64>,
    geometry: Cell<Option<BoxModel>>,
    diagram: RefCell<Option<BoxDiagram>>,
}
impl Sections {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(delegate: &ElementsDelegate) -> Self {
        let buttons = ["Browser default styles", "Parent rules", "Box model"]
            .map(|name| disclosure_button(delegate, name));
        let bodies = std::array::from_fn(|_| NSView::new(mtm()));
        let view = stack(&[], 0.);
        view.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        view.setAlignment(NSLayoutAttribute::Leading);
        view.setDetachesHiddenViews(true);
        for (index, (button, body)) in buttons.iter().zip(&bodies).enumerate() {
            button.setTag(index as isize);
            button.setAccessibilityExpanded(false);
            button.setToolTip(Some(&text([
                "Built-in browser CSS matching this element. Read only.",
                "CSS on ancestor elements. Only inheritable properties pass to this element; Computed shows final values.",
                "Content size, padding, border and margin in pixels.",
            ][index])));
            // CSS sections use a text preview; the box section owns a diagram instead.
            if index < 2 {
                boundary(button);
            }
            view.addArrangedSubview(button);
            view.addArrangedSubview(body);
            body.setHidden(true);
            activate(vec![
                button
                    .widthAnchor()
                    .constraintEqualToAnchor(&view.widthAnchor()),
                button.heightAnchor().constraintEqualToConstant(28.),
                body.widthAnchor()
                    .constraintEqualToAnchor(&view.widthAnchor()),
                body.heightAnchor()
                    .constraintEqualToConstant(if index == 2 { 216. } else { 160. }),
            ]);
        }
        Self {
            view,
            buttons,
            bodies,
            expanded: std::array::from_fn(|_| Cell::new(false)),
            rules: RefCell::new(vec![]),
            version: Cell::new(u64::MAX),
            geometry: Cell::new(None),
            diagram: RefCell::new(None),
        }
    }

    // Refresh expanded CSS and box sections from the current selection.
    pub fn update(&self, model: &Elements, styles: bool) {
        self.view.setHidden(!styles);
        let geometry_changed = self.geometry.replace(model.box_model) != model.box_model;
        self.buttons[2].setEnabled(model.box_model.is_some());
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if geometry_changed && self.expanded[2].get() {
            self.render(2);
        }
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.version.replace(model.styles_revision) == model.styles_revision {
            return;
        }
        self.rules.replace(model.rules.clone());
        let mut counts = [0; 2];
        for rule in &model.rules {
            // Use the rule and stylesheet that own the declaration range being displayed or edited.
            if (1..=2).contains(&rule.category) && !rule.body.is_empty() {
                counts[rule.category - 1] += 1;
            }
        }
        for (index, count) in counts.into_iter().enumerate() {
            self.buttons[index].setTitle(&text(&format!(
                "{} ({count})",
                ["Browser default styles", "Parent rules"][index]
            )));
            self.buttons[index].setEnabled(count > 0);
            // Avoid materializing detail content for collapsed sections.
            if self.expanded[index].get() {
                self.render(index);
            }
        }
    }

    // Expand the chosen detail section and collapse its siblings.
    pub fn toggle(&self, index: usize) {
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(expanded) = self.expanded.get(index) else {
            return;
        };
        let open = !expanded.get();
        // Opening one detail section collapses its siblings to preserve editor space.
        if open {
            for other in (0..self.expanded.len()).filter(|other| *other != index) {
                self.set_expanded(other, false);
            }
        }
        self.set_expanded(index, open);
        // A newly closed section needs no content refresh.
        if !open {
            return;
        }
        self.render(index);
        // Lay out the owning scroll document, not just the footer: expansion changes
        // the parent's height before we reveal the newly materialized content.
        if let Some(document) = self
            .view
            .enclosingScrollView()
            .and_then(|scroll| scroll.documentView())
        {
            document.layoutSubtreeIfNeeded();
        }
        self.bodies[index].scrollRectToVisible(self.bodies[index].bounds());
    }

    // Set expanded.
    fn set_expanded(&self, index: usize, open: bool) {
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.expanded[index].replace(open) == open {
            return;
        }
        self.buttons[index].setImage(
            // Keep disclosure state, content visibility and box-diagram sizing synchronized.
            symbol(if open {
                "chevron.down"
            } else {
                "chevron.right"
            })
            .as_deref(),
        );
        self.buttons[index].setAccessibilityExpanded(open);
        self.bodies[index].setHidden(!open);
        // Keep disclosure state, content visibility and box-diagram sizing synchronized.
        if open {
            return;
        }
        // Release native storage on collapse; only one secondary section is materialized.
        for child in self.bodies[index].subviews() {
            child.removeFromSuperview();
        }
        // Keep disclosure state, content visibility and box-diagram sizing synchronized.
        if index == 2 {
            self.diagram.replace(None);
        }
    }

    // Materialize only the detail content required by the expanded section.
    fn render(&self, index: usize) {
        // The box section renders geometry; CSS sections render rule previews.
        if index == 2 {
            // Reject a missing or mismatched value before the following operation relies on it.
            let Some(model) = self.geometry.get() else {
                return;
            };
            let mut diagram = self.diagram.borrow_mut();
            let diagram = diagram.get_or_insert_with(|| {
                let view = BoxDiagram::new();
                pin_edges(&view.view, &self.bodies[index]);
                view
            });
            diagram.update(model);
            return;
        }
        let source = rule_preview(&self.rules.borrow(), index + 1);
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(scroll) = self.bodies[index].subviews().iter().next()
            && let Some(scroll) = scroll.downcast_ref::<NSScrollView>()
            && let Some(document) = scroll
                .documentView()
                .and_then(|view| view.downcast::<NSTextView>().ok())
        {
            document.setString(&text(&source));
            editor::highlight_css(&document);
            return;
        }
        let (scroll, document) = super::super::panels::text_editor("Expanded CSS rules");
        document.setFont(Some(&crate::ui::controls::code_font(11.)));
        document.setTextContainerInset(NSSize::new(10., 8.));
        document.setString(&text(&source));
        editor::highlight_css(&document);
        pin_edges(&scroll, &self.bodies[index]);
    }
}

// Describe rule ownership and writability before displaying a bounded CSS preview.
fn rule_preview(rules: &[Rc<StyleRule>], category: usize) -> String {
    let description = match category {
        1 => "Built-in browser rules matching this element. Read only.",
        2 => "Ancestor rules. Only inheritable properties pass down; Computed shows final values.",
        _ => "",
    };
    let mut source = String::new();
    for rule in rules
        .iter()
        .filter(|r| r.category == category && !r.body.is_empty())
    {
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if source.is_empty() {
            source.push_str(&format!("/* {description} */\n\n"));
        }
        let code = rule.code();
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if source.len() + code.len() + rule.source.len() + 8 > 65500 {
            source.push_str("\n/* Preview limited to 64 KiB */");
            break;
        }
        source.push_str(&format!("/* {} */\n{code}\n\n", rule.source));
    }
    source
}

pub(super) struct BoxDiagram {
    pub view: Retained<BoxDiagramView>,
}
pub struct DiagramIvars {
    model: Cell<Option<BoxModel>>,
    labels: RefCell<Vec<Retained<NSAttributedString>>>,
    names: [Retained<NSAttributedString>; 3],
}
// SAFETY: Drawing reads typed geometry and cached strings; no page/session callbacks.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=DiagramIvars]
    pub(super) struct BoxDiagramView;
    impl BoxDiagramView {
        // Use top-down coordinates for content laid out from the leading edge.
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }

        // Paint only the content owned by this view in AppKit's current drawing context.
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            self.draw_model();
        }

        // Preserve the native resize callback before updating owned layout.
        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn resized(&self, _: NSSize) {
            self.setNeedsDisplay(true);
        }
    }
);
impl BoxDiagram {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new() -> Self {
        let names = ["margin", "border", "padding"].map(|name| {
            attributed(
                name,
                &NSFont::systemFontOfSize(9.),
                &NSColor::secondaryLabelColor(),
            )
        });
        let object = BoxDiagramView::alloc(mtm()).set_ivars(DiagramIvars {
            model: Cell::new(None),
            labels: RefCell::new(vec![]),
            names,
        });
        // SAFETY: Initialize this allocated main-thread drawing view once.
        let view: Retained<BoxDiagramView> = unsafe { msg_send![super(object), init] };
        view.setClipsToBounds(true);
        Self { view }
    }

    // Refresh box-model labels only when the retained measurements change.
    pub fn update(&self, model: BoxModel) {
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if self.view.ivars().model.replace(Some(model)) == Some(model) {
            return;
        }
        let font = NSFont::systemFontOfSize(10.);
        let color = NSColor::labelColor();
        let mut labels = self.view.ivars().labels.borrow_mut();
        labels.clear();
        labels.extend(
            [model.margin, model.border, model.padding]
                .into_iter()
                .flatten()
                .map(|value| attributed(&number(value), &font, &color)),
        );
        labels.push(attributed(
            &format!(
                "{} × {} px",
                number(model.content[0]),
                number(model.content[1])
            ),
            &font,
            &color,
        ));
        self.view.setAccessibilityLabel(Some(&text(&format!(
            "Box model: content {} by {} pixels; margin {:?}; border {:?}; padding {:?}",
            number(model.content[0]),
            number(model.content[1]),
            model.margin,
            model.border,
            model.padding
        ))));
        self.view.setNeedsDisplay(true);
    }
}
impl BoxDiagramView {
    // Draw model.
    fn draw_model(&self) {
        let labels = self.ivars().labels.borrow();
        // All thirteen box labels must be present before indexed drawing begins.
        if labels.len() != 13 {
            return;
        }
        let bounds = self.bounds().size;
        let width = (bounds.width - 24.).clamp(0., 360.);
        let mut rect = NSRect::new(
            NSPoint::new((bounds.width - width) / 2., 12.),
            NSSize::new(width, 192.),
        );
        let colors = [
            NSColor::systemOrangeColor(),
            NSColor::systemYellowColor(),
            NSColor::systemGreenColor(),
            NSColor::systemBlueColor(),
        ];
        for (index, color) in colors.iter().enumerate() {
            color.colorWithAlphaComponent(0.12).setFill();
            NSRectFill(rect);
            NSColor::separatorColor().setStroke();
            let outline = NSBezierPath::bezierPathWithRect(rect);
            outline.setLineWidth(0.5);
            outline.stroke();
            // The content label belongs in the center; surrounding measurements label the box rings.
            if index == 3 {
                draw_centered(&labels[12], rect);
                break;
            }
            self.ivars().names[index]
                .drawAtPoint(NSPoint::new(rect.origin.x + 5., rect.origin.y + 3.));
            let x = rect.origin.x;
            let y = rect.origin.y;
            let w = rect.size.width;
            let h = rect.size.height;
            let areas = [
                NSRect::new(NSPoint::new(x, y), NSSize::new(w, 24.)),
                NSRect::new(NSPoint::new(x + w - 32., y), NSSize::new(32., h)),
                NSRect::new(NSPoint::new(x, y + h - 24.), NSSize::new(w, 24.)),
                NSRect::new(NSPoint::new(x, y), NSSize::new(32., h)),
            ];
            for (value, area) in labels[index * 4..index * 4 + 4].iter().zip(areas) {
                draw_centered(value, area);
            }
            rect = NSRect::new(
                NSPoint::new(rect.origin.x + 32., rect.origin.y + 24.),
                NSSize::new(rect.size.width - 64., rect.size.height - 48.),
            );
        }
    }
}

// Draw centered.
fn draw_centered(value: &NSAttributedString, rect: NSRect) {
    let size = value.size();
    value.drawAtPoint(NSPoint::new(
        rect.origin.x + (rect.size.width - size.width) / 2.,
        rect.origin.y + (rect.size.height - size.height) / 2.,
    ));
}

// Keep box-model measurements compact while retaining useful fractional precision.
fn number(value: f64) -> String {
    // Display near-integer measurements without an unnecessary decimal place.
    if value.fract().abs() < 0.05 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
#[path = "../../tests/elements_sections.rs"]
mod tests;
