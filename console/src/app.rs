//! Translate package-native actions into deferred model commands without borrowing the browser's application state.

use crate::module_api::event;

use crate::{
    module_api::Tool,
    runtime::{Runtime, current},
};

pub(crate) enum Command {
    Clear,

    Evaluate,

    Object(u64),

    Input,

    CancelCompletion,
}

// Enqueue one native action; a closing package has no current runtime to mutate.
fn enqueue(command: Command) {
    current(|runtime| runtime.enqueue(command));
}

// Ask the host to close the package's existing workspace widget after the native callback returns.
pub(crate) fn close_native_widget(_: Tool) {
    current(|runtime| {
        runtime
            .host
            .notify(crate::module_api::notice::CLOSE, "", false)
    });
}

// Submit the Console draft through the package-owned protocol model.
pub(crate) fn evaluate_native_console() {
    enqueue(Command::Evaluate);
}

// Expand the remote object associated with a native Console link.
pub(crate) fn toggle_native_console_object(node: u64) {
    enqueue(Command::Object(node));
}

// Coalesce edited input before scheduling a generation-checked completion request.
pub(crate) fn native_console_input_changed() {
    enqueue(Command::Input);
}

// Consume Escape when the package's transient popup was visible and cancel pending completion work.
pub(crate) fn dismiss_native_completion() -> bool {
    current(|runtime| {
        runtime.enqueue(Command::CancelCompletion);
        runtime.content.dismiss_completion()
    })
    .unwrap_or(false)
}

// Keep completion-list navigation synchronous so AppKit knows whether to move the caret instead.
pub(crate) fn move_native_completion(delta: isize) -> bool {
    current(|runtime| runtime.content.move_completion(delta)).unwrap_or(false)
}

// Apply a current completion locally and defer only the protocol-model cancellation.
pub(crate) fn accept_native_completion() -> bool {
    current(|runtime| {
        let accepted = runtime.content.accept_completion();
        // Replacing the draft invalidates all requests built for its previous text and caret.
        if accepted {
            runtime.enqueue(Command::CancelCompletion);
        }
        accepted
    })
    .unwrap_or(false)
}

impl Runtime {
    // Mutate the package model and send commands only after its RefCell borrow is released.
    pub(crate) fn perform(&self, command: Command) {
        let mut model = self.model.borrow_mut();
        let console = &self.content;
        let mut commands = Vec::new();
        match command {
            Command::Clear => commands = model.clear_commands(),

            Command::Evaluate => {
                // A disconnected Console cannot consume or discard the user's current input draft.
                if model.ready {
                    self.edit_generation.set(self.edit_generation.get() + 1);
                    model.cancel_completion();
                    commands.extend(model.evaluate(&console.take_expression()));
                }
            }

            Command::Object(node) => commands = model.toggle_object(node),

            Command::Input | Command::CancelCompletion => {
                self.edit_generation.set(self.edit_generation.get() + 1);
                model.cancel_completion();
                // Popup views are package-owned and can be dismissed without a browser-state borrow.
                {
                    console.dismiss_completion();
                }
                // Only changed input schedules another request; Escape and accepted completions cancel it.
                if matches!(command, Command::Input) {
                    self.host
                        .schedule(event::COMPLETE, self.edit_generation.get(), 140);
                }
            }
        }
        commands.extend(model.protocol_commands());
        drop(model);
        self.host.send(commands);
    }
}
