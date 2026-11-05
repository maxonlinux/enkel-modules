//! Own Elements source drafts, full-line DOM editing and native syntax presentation.
use super::tree::{EditMode, ElementsDelegate};
use super::*;
use crate::ui::controls::text_color;
use crate::ui::panels::configure_scroll;
use objc2::AnyThread;
use objc2::Message;
use objc2_foundation::{NSCopying, NSMutableAttributedString, NSRange};

pub struct EditorIvars {
    base: RefCell<Retained<NSString>>,
    apply: Retained<NSButton>,
    hint: Retained<NSTextField>,
    stale: Cell<bool>,
    cancel: fn(),
}
// SAFETY: NSTextView overrides keep their native ABI; editing feedback never borrows app state.
define_class!(
    #[unsafe(super(NSTextView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=EditorIvars]
    pub(super) struct CodeEditor;
    unsafe impl NSObjectProtocol for CodeEditor {}
    impl CodeEditor {
        // Report user edits after updating CSS syntax colors and draft feedback.
        #[unsafe(method(didChangeText))]
        fn changed(&self) {
            // SAFETY: Preserve NSTextView undo and notification handling.
            unsafe {
                let _: () = msg_send![super(self), didChangeText];
            }
            self.feedback();
            highlight_css(self);
        }

        // Consume code-editor commands and preserve native handling for unrelated keys.
        #[unsafe(method(keyDown:))]
        fn key(&self, event: &NSEvent) {
            self.handle_key(event);
        }

        // Refresh focus appearance after the native editor accepts first-responder status.
        #[unsafe(method(becomeFirstResponder))]
        fn focus(&self) -> bool {
            // SAFETY: Forward the native responder boolean exactly.
            let accepted: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            // Refresh focus appearance only if AppKit accepted the responder change.
            if accepted {
                self.focus_border(true);
            }
            accepted
        }

        // Refresh focus appearance after the native editor releases first-responder status.
        #[unsafe(method(resignFirstResponder))]
        fn blur(&self) -> bool {
            // SAFETY: Forward the native responder boolean exactly.
            let accepted: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            // Refresh focus appearance only if AppKit accepted releasing the responder.
            if accepted {
                self.focus_border(false);
            }
            accepted
        }

        // Allow edits only when this code editor owns writable source.
        #[unsafe(method(shouldChangeTextInRange:replacementString:))]
        fn permit_edit(&self, range: NSRange, replacement: Option<&NSString>) -> bool {
            self.string().length().saturating_sub(range.length)
                + replacement.map_or(0, NSString::length)
                <= 65536
        }
    }
);
impl CodeEditor {
    // Construct the owner and initialize the state used by its subsequent operations.
    pub fn new(
        apply: &NSButton,
        hint: &NSTextField,
        cancel: fn(),
    ) -> (Retained<NSView>, Retained<Self>) {
        let object = Self::alloc(mtm()).set_ivars(EditorIvars {
            base: RefCell::new(text("")),
            apply: apply.retain(),
            hint: hint.retain(),
            stale: Cell::new(false),
            cancel,
        });
        // SAFETY: Initialize the allocated text view once with a finite editing viewport.
        let editor: Retained<Self> = unsafe {
            msg_send![super(object),initWithFrame:NSRect::new(objc2_foundation::NSPoint::ZERO,NSSize::new(300.,0.))]
        };
        crate::ui::controls::configure_code_input(&editor);
        editor.setMinSize(NSSize::new(0., 0.));
        editor.setTextContainerInset(NSSize::new(10., 10.));
        editor.setBackgroundColor(&NSColor::textBackgroundColor());
        editor.setTextColor(Some(&NSColor::labelColor()));
        editor.setFont(Some(&crate::ui::controls::code_font(12.)));
        editor.setAccessibilityLabel(Some(&text("Element code editor")));
        // SAFETY: The initialized text view owns its live text container.
        if let Some(container) = unsafe { editor.textContainer() } {
            container.setContainerSize(NSSize::new(300., f64::MAX));
            container.setWidthTracksTextView(true);
            container.setHeightTracksTextView(false);
        }
        let scroll = NSScrollView::new(mtm());
        configure_scroll(&scroll);
        scroll.setHasVerticalScroller(true);
        scroll.setDocumentView(Some(&editor));
        // Keep the scroll view inside the frame: its clip layer must not cover the border.
        let frame = NSBox::new(mtm());
        frame.setBoxType(NSBoxType::Custom);
        frame.setTitlePosition(NSTitlePosition::NoTitle);
        frame.setCornerRadius(6.);
        frame.setBorderWidth(1.);
        frame.setBorderColor(&NSColor::separatorColor());
        frame.setFillColor(&NSColor::textBackgroundColor());
        frame.setContentViewMargins(NSSize::new(2., 2.));
        frame.setContentView(Some(&scroll));
        (frame.into_super(), editor)
    }

    // Replace the committed editor source while preserving unchanged drafts.
    pub fn set_base(&self, source: &str, editable: bool) {
        self.ivars().base.replace(text(source));
        self.setEditable(editable);
        self.cancel();
    }

    // Accept the already-native submitted draft without converting it through another UTF-8 buffer.
    pub fn submitted(&self) {
        // Retaining alone can alias mutable text storage; the committed base needs an immutable native snapshot.
        self.ivars().base.replace(self.string().copy());
        self.ivars().stale.set(false);
        self.feedback();
    }

    // Compare the current editor text with its committed base.
    pub fn dirty(&self) -> bool {
        !self.string().isEqualToString(&self.ivars().base.borrow())
    }

    // Expose whether incoming source has superseded the current draft.
    pub fn is_stale(&self) -> bool {
        self.ivars().stale.get()
    }

    // Restore the committed editor base and clear stale draft feedback.
    pub fn cancel(&self) {
        let source = self.ivars().base.borrow().clone();
        self.ivars().stale.set(false);
        // Restoring the same text needs no syntax pass or selection reset.
        if !self.string().isEqualToString(&source) {
            self.setString(&source);
            highlight_css(self);
            self.setSelectedRange(NSRange::new(0, 0));
            self.scrollRangeToVisible(NSRange::new(0, 0));
        }
        self.feedback();
    }

    // Mark the incoming base as newer without discarding the local draft.
    pub fn stale(&self) {
        self.ivars().stale.set(true);
        self.feedback();
    }

    // Present whether the draft is modified or superseded by incoming source.
    fn feedback(&self) {
        let dirty = self.dirty();
        self.ivars()
            .apply
            .setEnabled(dirty && self.isEditable() && !self.ivars().stale.get());
        // Explain that incoming source superseded the draft without discarding user edits.
        let message = if self.ivars().stale.get() {
            "Styles changed · Esc reload"
        } else if !self.isEditable() {
            "Read only"
        } else if dirty {
            "⌘ + Enter apply · Esc cancel"
        } else {
            "Edit CSS · ⌘ + Enter apply"
        };
        self.ivars().hint.setStringValue(&text(message));
    }

    // Consume only the declared editing keys and leave native navigation intact.
    fn handle_key(&self, event: &NSEvent) {
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if event.keyCode() == 36
            && event
                .modifierFlags()
                .contains(NSEventModifierFlags::Command)
        {
            // Submit through the same enabled-state check used by the Apply button.
            if self.ivars().apply.isEnabled() {
                // SAFETY: The retained Apply button owns its declared action and target.
                unsafe {
                    self.ivars().apply.performClick(None);
                }
            }
            return;
        }
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if event.keyCode() == 53 {
            (self.ivars().cancel)();
            return;
        }
        // SAFETY: Ordinary text and navigation keys retain NSTextView behavior.
        unsafe {
            let _: () = msg_send![super(self),keyDown:event];
        }
    }

    // Update editor focus appearance independently of its text contents.
    fn focus_border(&self, focused: bool) {
        // Do not start work after teardown or before this owner is ready.
        let Some(scroll) = self.enclosingScrollView() else {
            return;
        };
        // Reflect native editor focus in its shared border color.
        let color = if focused {
            NSColor::controlAccentColor()
        } else {
            NSColor::separatorColor()
        };
        // SAFETY: The editor's enclosing scroll is retained inside its dedicated frame.
        let frame = unsafe { scroll.superview() };
        // Update only the editor-owned frame, leaving unrelated native ancestors untouched.
        if let Some(frame) = frame.and_then(|view| view.downcast::<NSBox>().ok()) {
            frame.setBorderColor(&color);
        }
    }
}

// Apply syntax colors in one native editing batch without untyped attribute-key/value calls.
pub(super) fn highlight_css(view: &NSTextView) {
    // SAFETY: Every caller owns this initialized main-thread text view and its live storage.
    let Some(storage) = (unsafe { view.textStorage() }) else {
        return;
    };
    let native_source = view.string();
    let source = native_source.to_string();
    let mut offset = 0;
    storage.beginEditing();
    text_color(
        &storage,
        &NSColor::labelColor(),
        NSRange::new(0, native_source.length()),
    );
    for line in source.split_inclusive('\n') {
        let length = line.encode_utf16().count();
        let range = NSRange::new(offset, length);
        // Comment text must not be parsed as a property declaration.
        if line.trim_start().starts_with("/*") {
            text_color(&storage, &NSColor::secondaryLabelColor(), range);
        } else if let Some(selector) = line.trim_end().strip_suffix('{') {
            // Measure the actual selector, including a final line without a trailing newline.
            text_color(
                &storage,
                &NSColor::systemBrownColor(),
                NSRange::new(offset, selector.encode_utf16().count()),
            );
        } else if let Some(colon) = line.find(':') {
            let property = line[..colon].encode_utf16().count();
            text_color(
                &storage,
                &NSColor::systemBlueColor(),
                NSRange::new(offset, property),
            );
            // The colon is ASCII: one UTF-16 unit separates property and value.
            let value = property + 1;
            // Hexadecimal literals use the same accent as other explicit code identifiers.
            let color = if line[colon + 1..].trim_start().starts_with('#') {
                NSColor::systemBlueColor()
            } else {
                NSColor::systemPurpleColor()
            };
            text_color(
                &storage,
                &color,
                NSRange::new(offset + value, length - value),
            );
        }
        offset += length;
    }
    storage.endEditing();
}

// Color only materialized cells. UTF-16 runs keep non-ASCII text aligned with AppKit ranges.
pub(super) fn dom_caption(source: &str) -> Retained<NSMutableAttributedString> {
    let value = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &text(source),
    );
    let colors = [
        NSColor::labelColor(),
        NSColor::systemBlueColor(),
        NSColor::systemBrownColor(),
    ];
    let (mut tag, mut name, mut quote) = (false, false, None);
    let (mut start, mut offset, mut previous) = (0, 0, 0);
    for ch in source.chars() {
        let kind = match ch {
            _ if quote.is_some() => {
                // Validate source boundaries and delimiters before slicing or interpreting editable text.
                if quote == Some(ch) {
                    quote = None;
                }
                2
            }
            '<' => {
                tag = true;
                name = true;
                1
            }
            '>' => {
                tag = false;
                1
            }
            '\'' | '"' if tag => {
                quote = Some(ch);
                2
            }
            _ if ch.is_whitespace() => {
                name = false;
                0
            }
            _ if tag && name => 1,
            '=' if tag => 0,
            _ if tag => 2,
            _ => 0,
        };
        // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
        if kind != previous {
            text_color(
                &value,
                &colors[previous],
                NSRange::new(start, offset - start),
            );
            start = offset;
            previous = kind;
        }
        offset += ch.len_utf16();
    }
    text_color(
        &value,
        &colors[previous],
        NSRange::new(start, offset - start),
    );
    value
}

impl ElementsDelegate {
    // Open one full-line editor and select the value under the original click.
    pub(super) fn begin_edit(&self, table: &NSTableView, row: isize, mode: EditMode) {
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        let Some(cell) = table
            .viewAtColumn_row_makeIfNecessary(0, row, false)
            .and_then(|view| view.downcast::<NSTableCellView>().ok())
        else {
            return;
        };
        // SAFETY: The retained cell owns this text-field outlet for the whole edit.
        let Some(field) = (unsafe { cell.textField() }) else {
            return;
        };
        // Preserve the active native editor instead of stealing or resetting its responder.
        if field.currentEditor().is_some() {
            return;
        }
        // Reject work from an earlier document or selection; protocol IDs may be reused.
        let Some(target) = self
            .ivars()
            .rows
            .borrow()
            .get(row as usize)
            .map(|node| NodeTarget {
                id: node.id,
                epoch: self.ivars().epoch.get(),
            })
        else {
            return;
        };
        field.setEditable(true);
        field.setSelectable(true);
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
        }
        // SAFETY: Native selectText: accepts an unused optional sender on this live field.
        unsafe {
            field.selectText(None);
        }
        // Preserve the active native editor instead of stealing or resetting its responder.
        let Some(editor) = field
            .currentEditor()
            .and_then(|view| view.downcast::<NSTextView>().ok())
        else {
            self.restore_field(&field);
            return;
        };
        // The field owns only markup; its native leading constraint owns tree indentation.
        let offset = NSApplication::sharedApplication(mtm())
            .currentEvent()
            .filter(|_| matches!(mode, EditMode::Hit))
            .map_or(1, |event| {
                let point = editor.convertPoint_fromView(event.locationInWindow(), None);
                editor.characterIndexForInsertionAtPoint(point)
            });
        // The explicit Add attribute command opens an insertion slot; double-click uses the hit range.
        let offset = if matches!(mode, EditMode::Attribute) {
            usize::MAX
        } else {
            offset
        };
        // Resolve the current target before applying work that could otherwise affect a stale owner.
        let Some(edit) = crate::app::element_edit(target, offset) else {
            field.abortEditing();
            self.restore_field(&field);
            return;
        };
        let source = text(edit.source());
        let selection = edit.selection.clone();
        self.ivars().edit.replace(Some(edit));
        field.setStringValue(&source);
        editor.setString(&source);
        // Reuse the native field editor without its opaque background or a second focus outline.
        editor.setDrawsBackground(false);
        editor.setTextColor(Some(&NSColor::alternateSelectedControlTextColor()));
        // Keep the entire opening tag in one editor; only the clicked value is selected.
        select_range(&editor, selection);
    }

    // Keep Tab navigation inside the draft and prevent Escape from committing it.
    pub(super) fn inline_command(
        &self,
        control: &NSControl,
        editor: &NSTextView,
        command: objc2::runtime::Sel,
    ) -> bool {
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if command == sel!(cancelOperation:) {
            // Take the draft before AppKit ends editing, so Esc cannot accidentally commit it.
            self.ivars().edit.borrow_mut().take();
            control.abortEditing();
            // Apply the optional result only while its owning state is available.
            if let Some(field) = control.downcast_ref::<NSTextField>() {
                self.restore_field(field);
            }
            return true;
        }
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if command != sel!(insertTab:) && command != sel!(insertBacktab:) {
            return false;
        }
        let draft = self.ivars().edit.borrow();
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(edit) = draft.as_ref() else {
            return false;
        };
        let mut source = editor.string().to_string();
        let range = editor.selectedRange();
        let selection = range.location..range.location + range.length;
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        let Some(next) = edit.navigate(&mut source, selection, command == sel!(insertBacktab:))
        else {
            return true;
        };
        // Tab changes the current draft and selection only; Enter/focus loss commits once.
        if let Some(field) = control.downcast_ref::<NSTextField>() {
            field.setStringValue(&text(&source));
        }
        editor.setString(&text(&source));
        select_range(editor, next);
        true
    }

    // Return the reused field to its read-only DOM presentation after editing.
    pub(super) fn restore_field(&self, field: &NSTextField) {
        field.setEditable(false);
        field.setSelectable(false);
        // Native views and outlets may be absent during reparenting or teardown; use only live objects.
        if let Some(cell) = field.cell() {
            cell.setScrollable(false);
        }
        // Resolve the stable identity before using a row index or applying a retained reply.
        if let Some(row) = self
            .ivars()
            .rows
            .borrow()
            .iter()
            .find(|row| row.id == field.tag() as i64)
        {
            super::tree::set_caption(field, &row.label);
        }
    }

    // Capture the native draft once and defer protocol work past the callback.
    pub(super) fn finish_edit(&self, note: &NSNotification) {
        // Reject a missing or mismatched value before the following operation relies on it.
        let Some(field) = note
            .object()
            .and_then(|object| object.downcast::<NSTextField>().ok())
        else {
            return;
        };
        let edit = self.ivars().edit.borrow_mut().take();
        let source = edit.as_ref().map(|_| field.stringValue().to_string());
        self.restore_field(&field);
        // Validate source boundaries and delimiters before slicing or interpreting editable text.
        if let Some((edit, source)) = edit.zip(source) {
            crate::app::commit_element_edit(edit, source);
        }
    }
}

// Set the intended native selection and reveal it in the active editor.
fn select_range(editor: &NSTextView, range: std::ops::Range<usize>) {
    let range = NSRange::new(range.start, range.end - range.start);
    editor.setSelectedRange(range);
    editor.scrollRangeToVisible(range);
}
