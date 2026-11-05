//! Own the Console package's input delegates, history and completion commands independently of workspace actions.
use super::*;
use crate::developer::{history_boundary, indented_newline};

#[derive(Default)]
pub struct ActionIvars {
    pub(super) history: RefCell<crate::developer::CommandHistory>,
    pub(super) suppress_edit: Cell<bool>,
    pub(super) input_height: RefCell<Option<Retained<NSLayoutConstraint>>>,
}
// SAFETY: Retained main-thread target/delegate, with AppKit's void/id actions.
// Delegate callbacks run outside the application's Ui borrows.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[ivars=ActionIvars]
    pub struct ConsoleActions;
    unsafe impl NSObjectProtocol for ConsoleActions {}
    impl ConsoleActions {
        // Submit the current Console expression through its package model.
        #[unsafe(method(runConsole:))]
        fn run(&self, _: &AnyObject) {
            crate::app::evaluate_native_console();
        }
    }
    // SAFETY: The read-only output's delegate uses AppKit's declared link ABI;
    // only our own NSNumber object identities are installed in its text storage.
    unsafe impl NSTextDelegate for ConsoleActions {
        // Notify the Console owner only after a user edit to writable input.
        #[unsafe(method(textDidChange:))]
        fn changed(&self, notification: &NSNotification) {
            // Skip unchanged or programmatic updates so native callbacks do not repeat model work.
            if !self.ivars().suppress_edit.get()
                && let Some(editor) = notification
                    .object()
                    .and_then(|object| object.downcast::<NSTextView>().ok())
                && editor.isEditable()
            {
                self.resize_input(&editor);
                crate::app::native_console_input_changed();
            }
        }
    }
    unsafe impl NSTextViewDelegate for ConsoleActions {
        // Delegate the native text command to this view's editing behavior.
        #[unsafe(method(textView:doCommandBySelector:))]
        unsafe fn command(&self, editor: &NSTextView, selector: objc2::runtime::Sel) -> bool {
            editor.isEditable() && self.handle_command(editor, selector)
        }

        // Resolve only links created by this console before acting on remote objects.
        #[unsafe(method(textView:clickedOnLink:atIndex:))]
        unsafe fn link(&self, _: &NSTextView, link: &AnyObject, _: usize) -> bool {
            // Only our numeric remote-object identities may trigger expansion; external links are consumed.
            if let Some(id) = link.downcast_ref::<objc2_foundation::NSNumber>() {
                crate::app::toggle_native_console_object(id.as_u64());
            }
            true
        }
    }
);
impl ConsoleActions {
    // Measure the laid-out Console input and cap its growth inside the widget.
    pub(super) fn resize_input(&self, editor: &NSTextView) {
        // SAFETY: Both objects belong to this retained, main-thread text view.
        let layout = unsafe { (editor.layoutManager(), editor.textContainer()) };
        // Apply the optional result only while its owning state is available.
        let (Some(layout), Some(container)) = layout else {
            return;
        };
        layout.ensureLayoutForTextContainer(&container);
        let height = (layout.usedRectForTextContainer(&container).size.height + 8.)
            .clamp(crate::ui::controls::ACTION_ROW_HEIGHT, 96.);
        // Use only finite geometry that fits the current viewport before changing native layout.
        if let Some(constraint) = self.ivars().input_height.borrow().as_ref() {
            constraint.setConstant(height);
        }
    }

    // Resolve completion, multiline editing and history before allowing native key handling.
    fn handle_command(&self, editor: &NSTextView, selector: objc2::runtime::Sel) -> bool {
        // Only the focused native input may consume Console editing commands.
        if !is_first_responder(editor) {
            return false;
        }
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if selector == sel!(moveUp:) || selector == sel!(moveDown:) {
            let up = selector == sel!(moveUp:);
            // Arrow keys select an open suggestion before they navigate command history.
            if crate::app::move_native_completion(if up { -1 } else { 1 }) {
                return true;
            }
            let selection = editor.selectedRange();
            let expression = editor.string().to_string();
            // History replaces the draft only when the caret reaches its outer line in this direction.
            if !history_boundary(
                &expression,
                selection.location..selection.location.saturating_add(selection.length),
                up,
            ) {
                return false;
            }
            // Replace input only when history navigation produced a retained command or restored draft.
            if let Some(value) = self.ivars().history.borrow_mut().navigate(up, &expression) {
                self.ivars().suppress_edit.set(true);
                editor.setString(&text(&value));
                editor.setSelectedRange(NSRange::new(editor.string().length(), 0));
                self.resize_input(editor);
                editor.scrollRangeToVisible(editor.selectedRange());
                self.ivars().suppress_edit.set(false);
            }
            crate::app::dismiss_native_completion();
            return true;
        }
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if selector == sel!(insertTab:) || selector == sel!(complete:) {
            // Tab inserts indentation when no active suggestion can replace the current input.
            if !crate::app::accept_native_completion() {
                insert_plain_text(editor, "  ");
            }
            return true;
        }
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if selector == sel!(insertNewline:)
            || selector == sel!(insertNewlineIgnoringFieldEditor:)
            || selector == sel!(insertLineBreak:)
        {
            let shift = editor
                .window()
                .and_then(|window| window.currentEvent())
                .is_some_and(|event| event.modifierFlags().contains(NSEventModifierFlags::Shift));
            // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
            crate::app::dismiss_native_completion();
            // Modified Return inserts a line without submitting the current expression.
            if shift || selector != sel!(insertNewline:) {
                let newline = indented_newline(
                    &editor.string().to_string(),
                    editor.selectedRange().location,
                );
                insert_plain_text(editor, &newline);
                return true;
            }
            crate::app::evaluate_native_console();
            return true;
        }
        // Consume only this declared native command; leave unrelated keys and gestures to AppKit.
        if selector == sel!(cancelOperation:) {
            // Escape dismisses suggestions first; a second Escape can close the widget.
            if !crate::app::dismiss_native_completion() {
                crate::app::close_native_widget(Tool::Console);
            }
            return true;
        }
        false
    }
}

// Apply keyboard-generated text through AppKit editing and undo notifications.
fn insert_plain_text(editor: &NSTextView, value: &str) {
    let range = editor.selectedRange();
    let replacement = text(value);
    // Follow AppKit's editing transaction so delegates, undo and observers see
    // one edit, including indentation inserted by a keyboard command.
    if !editor.shouldChangeTextInRange_replacementString(range, Some(&replacement)) {
        return;
    }
    editor.replaceCharactersInRange_withString(range, &replacement);
    editor.setSelectedRange(NSRange::new(range.location + replacement.length(), 0));
    editor.didChangeText();
    editor.scrollRangeToVisible(editor.selectedRange());
}
