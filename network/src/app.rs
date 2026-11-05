//! Translate package-native actions into deferred model commands without borrowing the browser's application state.

use crate::module_api::event;

use crate::runtime::{Runtime, current};

pub(crate) enum Command {
    Clear,

    SelectRequest(Option<u64>),

    Body,

    Refresh,
}

// Enqueue one native action; a closing package has no current runtime to mutate.
fn enqueue(command: Command) {
    current(|runtime| runtime.enqueue(command));
}

// Ask the host to close the package's existing workspace widget after the native callback returns.

// Open the package-owned embedded finder without opening another window.
pub(crate) fn find_network_details() {
    current(|runtime| {
        runtime.event(event::FIND, 0, &[]);
    });
}

// Preserve the requested stable identity across native table selection callbacks.
pub(crate) fn select_native_request(key: Option<u64>) {
    enqueue(Command::SelectRequest(key));
}

// Fetch one selected response through the package's scoped protocol session.
pub(crate) fn load_native_response_body() {
    enqueue(Command::Body);
}

// Refresh the selected native section after a local tab or range change.
pub(crate) fn refresh_native_network() {
    enqueue(Command::Refresh);
}

impl Runtime {
    // Mutate the package model and send commands only after its RefCell borrow is released.
    pub(crate) fn perform(&self, command: Command) {
        let mut model = self.model.borrow_mut();
        let mut commands = Vec::new();
        match command {
            Command::Clear => model.network.clear(),

            Command::SelectRequest(key) => model.network.select(key),

            Command::Body => commands = model.load_network_body(),

            Command::Refresh => {}
        }
        commands.extend(model.protocol_commands());
        drop(model);
        self.host.send(commands);
    }
}
