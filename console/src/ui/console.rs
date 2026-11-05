//! Console input, output and native action handling.
use super::*;
use crate::module_api::Tool;
use crate::ui::controls::icon_button as widget_button;
use crate::ui::controls::{ACTION_ROW_HEIGHT, action_row_separator};
use objc2::Message;
use objc2_foundation::{NSPoint, NSRange};
#[path = "console_actions.rs"]
mod actions;
use actions::{ActionIvars, ConsoleActions};
#[path = "console_completion.rs"]
pub(in crate::ui) mod completion;
pub struct ConsoleControls {
    actions: Retained<ConsoleActions>,
    output: Retained<NSTextView>,
    scroll: Retained<NSScrollView>,
    pub(crate) input: Retained<NSTextView>,
    input_scroll: Retained<NSScrollView>,
    empty: Retained<NSTextField>,
    run: Retained<NSButton>,
    separator: Retained<NSView>,
    prompt: Retained<NSTextField>,
    completion: completion::CompletionPopup,
}
impl ConsoleControls {
    // Set state.
    pub(crate) fn set_state(&self, ready: bool, output: &str, filtered: bool) {
        // Console input becomes writable only after its protocol target is ready.
        if self.input.isEditable() != ready {
            self.input.setEditable(ready);
            self.run.setEnabled(ready);
        }
        // Distinguish an empty capture from a filter with no matching messages.
        self.empty.setStringValue(&text(if filtered {
            "No matching messages"
        } else if ready {
            // A connected empty capture differs from a missing page connection.

            "No console messages yet"
        } else {
            "Open a page to use Console"
        }));
        self.empty.setHidden(!output.is_empty());
        let current = self.output.string();
        let output_text = text(output);
        // Resolve the current target before applying work that could otherwise affect a stale owner.
        if current.isEqualToString(&output_text) {
            return;
        }
        let visible = self.scroll.documentVisibleRect();
        let follow = current.is_empty()
            || visible.origin.y + visible.size.height >= self.output.bounds().size.height - 24.;
        let selection = self.output.selectedRange();
        self.output.setString(&output_text);
        self.output.setTextColor(Some(&NSColor::textColor()));
        color_console_output(&self.output, output);
        let length = output_text.length();
        // Restore selection only while its entire range still fits the updated output.
        if selection.location.saturating_add(selection.length) <= length {
            self.output.setSelectedRange(selection);
        }
        // Follow new output only if the user was already reading at the bottom.
        if follow {
            self.output.scrollRangeToVisible(NSRange::new(length, 0));
        } else {
            self.scroll.contentView().scrollToPoint(visible.origin);
            self.scroll
                .reflectScrolledClipView(&self.scroll.contentView());
        }
    }

    // Take expression.
    pub fn take_expression(&self) -> String {
        self.completion.dismiss();
        let expression = self.input.string().to_string();
        self.actions
            .ivars()
            .history
            .borrow_mut()
            .record(&expression);
        self.actions.ivars().suppress_edit.set(true);
        self.input.setString(&text(""));
        self.input.setSelectedRange(NSRange::new(0, 0));
        self.actions.resize_input(&self.input);
        self.actions.ivars().suppress_edit.set(false);
        expression
    }

    // Capture text and caret only when Console input owns the field editor.
    pub fn input_snapshot(&self) -> Option<(String, usize)> {
        // Only the focused Console input owns a valid completion caret snapshot.
        if !is_first_responder(&self.input) {
            return None;
        }
        let selection = self.input.selectedRange();
        (selection.length == 0).then(|| (self.input.string().to_string(), selection.location))
    }

    // Set completions.
    pub fn set_completions(
        &self,
        suggestions: Option<&std::rc::Rc<crate::developer::Suggestions>>,
    ) {
        self.completion
            .update(suggestions, self.input_snapshot().as_ref());
    }

    // Hide suggestions and report whether Escape was consumed by the popup.
    pub fn dismiss_completion(&self) -> bool {
        let visible = self.completion.visible();
        self.completion.dismiss();
        visible
    }

    // Move completion.
    pub fn move_completion(&self, delta: isize) -> bool {
        self.completion.move_selection(delta)
    }

    // Apply the selected suggestion only if its text and caret snapshot still match.
    pub fn accept_completion(&self) -> bool {
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some((suggestions, index)) = self.completion.selected() else {
            return false;
        };
        let query = &suggestions.query;
        let unchanged = match self.input_snapshot() {
            Some((expression, caret)) => expression == query.expression && caret == query.caret,
            None => self.input.string().to_string() == query.expression,
        };
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if !unchanged {
            self.dismiss_completion();
            return false;
        }
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some((expression, caret)) = query.apply(&suggestions.names[index]) else {
            return false;
        };
        self.actions.ivars().suppress_edit.set(true);
        self.input.setString(&text(&expression));
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(window) = self.input.window() {
            window.makeFirstResponder(Some(&*self.input));
        }
        self.input.setSelectedRange(NSRange::new(caret, 0));
        self.actions.resize_input(&self.input);
        self.input.scrollRangeToVisible(self.input.selectedRange());
        self.actions.ivars().suppress_edit.set(false);
        self.completion.dismiss();
        true
    }

    // Update numeric object links in one native attribute batch without reading Console text back into Rust.
    pub fn set_objects(&self, view: &crate::developer::ObjectView) {
        // SAFETY: The widget owns this initialized main-thread output view and its live text storage.
        let Some(storage) = (unsafe { self.output.textStorage() }) else {
            return;
        };
        storage.beginEditing();
        // SAFETY: The immutable link key applies to the full native string installed by set_state.
        unsafe {
            storage.removeAttribute_range(
                NSLinkAttributeName,
                NSRange::new(0, self.output.string().length()),
            );
        }
        for link in &view.links {
            let node = objc2_foundation::NSNumber::new_u64(link.node);
            let range = NSRange::new(link.start, link.length);
            // SAFETY: AppKit accepts an arbitrary link object; the bounded snapshot owns UTF-16 ranges in this exact text.
            unsafe {
                storage.addAttribute_value_range(NSLinkAttributeName, &node, range);
            }
        }
        storage.endEditing();
    }

    // Construct the owner and initialize the state used by its subsequent operations.
    pub(crate) fn new(view: &NSView) -> Self {
        let object = ConsoleActions::alloc(mtm()).set_ivars(ActionIvars::default());
        // SAFETY: Initialize this Console-owned native delegate before attaching its input and output views.
        let actions: Retained<ConsoleActions> = unsafe { msg_send![super(object), init] };
        let actions = &*actions;
        let input_scroll = console_scroll();
        let input = new_console_input(actions);
        input_scroll.setDocumentView(Some(&input));
        let input_height = input_scroll
            .heightAnchor()
            .constraintEqualToConstant(ACTION_ROW_HEIGHT);
        actions
            .ivars()
            .input_height
            .replace(Some(input_height.clone()));
        let prompt = label("›");
        prompt.setFont(input.font().as_deref());
        prompt.setTextColor(Some(&NSColor::controlAccentColor()));
        let run = widget_button(
            actions,
            "return",
            "Run expression (Return)",
            sel!(runConsole:),
        );
        let scroll = console_scroll();
        let output = new_console_output(actions);
        scroll.setDocumentView(Some(&output));
        let empty = label("No console messages yet");
        empty.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        for child in [
            &*scroll as &NSView,
            &*empty,
            &*prompt,
            &*input_scroll,
            &*run,
        ] {
            child.setTranslatesAutoresizingMaskIntoConstraints(false);
            view.addSubview(child);
        }
        let separator = action_row_separator(view, &input_scroll);
        let completion = completion::CompletionPopup::new(view, &input_scroll, &input);
        let controls = Self {
            actions: actions.retain(),
            output,
            scroll,
            input,
            input_scroll,
            empty,
            run,
            separator,
            prompt,
            completion,
        };
        controls.layout(view, input_height);
        controls
    }

    // Reposition the transient completion list after the host finishes laying out the widget.
    pub(crate) fn position_popup(&self, host: &NSView) {
        self.completion.anchor.position(host);
    }

    // Resolve pending constraints before querying or presenting child geometry.
    fn layout(&self, view: &NSView, input_height: Retained<NSLayoutConstraint>) {
        let Self {
            scroll,
            empty,
            separator,
            prompt,
            input_scroll,
            run,
            ..
        } = self;
        let font = self.input.font().expect("Configured Console font");
        // Use the editor's native first-line metrics for both adjacent controls.
        // SAFETY: This retained text view owns its main-thread layout manager.
        let (baseline, line_height) = unsafe { self.input.layoutManager() }
            .map(|layout| {
                (
                    layout.defaultBaselineOffsetForFont(&font),
                    layout.defaultLineHeightForFont(&font),
                )
            })
            .unwrap_or((font.ascender(), font.ascender() - font.descender()));
        let inset = self.input.textContainerInset().height;
        activate(vec![
            scroll
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&view.leadingAnchor(), 8.),
            scroll
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&view.trailingAnchor(), -8.),
            scroll
                .topAnchor()
                .constraintEqualToAnchor_constant(&view.topAnchor(), 94.),
            scroll
                .bottomAnchor()
                .constraintEqualToAnchor_constant(&separator.topAnchor(), -4.),
            empty
                .centerXAnchor()
                .constraintEqualToAnchor(&scroll.centerXAnchor()),
            empty
                .centerYAnchor()
                .constraintEqualToAnchor(&scroll.centerYAnchor()),
            prompt
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&view.leadingAnchor(), 14.),
            prompt
                .firstBaselineAnchor()
                .constraintEqualToAnchor_constant(&input_scroll.topAnchor(), inset + baseline),
            input_scroll
                .leadingAnchor()
                .constraintEqualToAnchor_constant(&prompt.trailingAnchor(), 8.),
            input_scroll
                .trailingAnchor()
                .constraintEqualToAnchor_constant(&run.leadingAnchor(), -6.),
            input_height,
            run.trailingAnchor()
                .constraintEqualToAnchor_constant(&view.trailingAnchor(), -12.),
            run.centerYAnchor().constraintEqualToAnchor_constant(
                &input_scroll.topAnchor(),
                inset + line_height / 2.,
            ),
            run.widthAnchor().constraintEqualToConstant(28.),
            run.heightAnchor()
                .constraintEqualToConstant(ACTION_ROW_HEIGHT),
        ]);
    }
}

// Create a compact output scroller with overlay indicators.
fn console_scroll() -> Retained<NSScrollView> {
    let scroll = NSScrollView::new(mtm());
    scroll.setHasVerticalScroller(true);
    configure_scroll(&scroll);
    scroll
}

// Construct console input.
fn new_console_input(actions: &ConsoleActions) -> Retained<NSTextView> {
    let input = NSTextView::initWithFrame(
        NSTextView::alloc(mtm()),
        NSRect::new(NSPoint::ZERO, NSSize::new(440., ACTION_ROW_HEIGHT)),
    );
    input.setAccessibilityLabel(Some(&text("JavaScript expression")));
    crate::ui::controls::configure_code_input(&input);
    input.setImportsGraphics(false);
    input.setDrawsBackground(false);
    input.setTextColor(Some(&NSColor::textColor()));
    input.setInsertionPointColor(Some(&NSColor::textColor()));
    input.setTextContainerInset(NSSize::new(0., 4.));
    input.setMinSize(NSSize::new(0., ACTION_ROW_HEIGHT));
    input.setAutomaticTextCompletionEnabled(false);
    // SAFETY: The initialized main-thread text view retains its native text container.
    if let Some(container) = unsafe { input.textContainer() } {
        container.setWidthTracksTextView(true);
        container.setLineFragmentPadding(0.);
    }
    input.setFont(Some(&crate::ui::controls::code_font(12.)));
    input.setDelegate(Some(ProtocolObject::from_ref(actions)));
    input
}

// Construct console output.
fn new_console_output(actions: &ConsoleActions) -> Retained<NSTextView> {
    let output = NSTextView::initWithFrame(
        NSTextView::alloc(mtm()),
        NSRect::new(NSPoint::ZERO, NSSize::new(520., 200.)),
    );
    output.setEditable(false);
    output.setSelectable(true);
    output.setRichText(true);
    output.setDisplaysLinkToolTips(false);
    // SAFETY: These attributes have the documented NSColor/NSNumber value
    // types. Disclosure triangles supply the affordance, without styling
    // object rows like external web links.
    unsafe {
        let color = NSColor::textColor();
        let underline = objc2_foundation::NSNumber::new_isize(0);
        let attributes = objc2_foundation::NSDictionary::<NSString, AnyObject>::from_slices(
            &[
                NSForegroundColorAttributeName,
                NSUnderlineStyleAttributeName,
            ],
            &[&*color as &AnyObject, &*underline as &AnyObject],
        );
        output.setLinkTextAttributes(Some(&attributes));
    }
    output.setTextColor(Some(&NSColor::textColor()));
    output.setDrawsBackground(false);
    output.setTextContainerInset(NSSize::new(6., 8.));
    output.setVerticallyResizable(true);
    output.setHorizontallyResizable(false);
    output.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    output.setFont(Some(&crate::ui::controls::code_font(12.)));
    // SAFETY: The initialized text view owns its live text container.
    if let Some(container) = unsafe { output.textContainer() } {
        container.setWidthTracksTextView(true);
    }
    output.setAccessibilityLabel(Some(&text("Console output")));
    output.setDelegate(Some(ProtocolObject::from_ref(actions)));
    output
}

// Apply severity colors without changing Console text or selection offsets.
fn color_console_output(output_view: &NSTextView, output: &str) {
    let mut location = 0;
    for line in output.split_inclusive('\n') {
        let length = line.encode_utf16().count();
        // Console severity controls color without changing the retained output text.
        let color = if line.starts_with("[error]")
            || line
                .split(':')
                .next()
                .is_some_and(|prefix| prefix.ends_with("Error"))
        {
            Some(NSColor::systemRedColor())
        } else if line.starts_with("[warning]") || line.starts_with("[warn]") {
            Some(NSColor::systemOrangeColor())
        } else if line.starts_with("> ") {
            Some(NSColor::secondaryLabelColor())
        } else {
            None
        };
        // Apply the optional result only while its owning state is available.
        if let Some(color) = color {
            output_view.setTextColor_range(Some(&color), NSRange::new(location, length));
        }
        location += length;
    }
}

impl Drop for ConsoleControls {
    // Native views may survive their controller during window teardown; sever their unretained delegate/action pointers first.
    fn drop(&mut self) {
        self.input.setDelegate(None);
        self.output.setDelegate(None);
        // SAFETY: Clearing an unretained action target is valid before releasing its retained delegate.
        unsafe {
            self.run.setTarget(None);
        }
        self.completion.dismiss();
    }
}
