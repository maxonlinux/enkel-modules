//! Translate package-native actions into deferred model commands without borrowing the browser's application state.

use crate::developer::elements::{Action, NodeEdit, NodeTarget};

use crate::runtime::{Runtime, current};

pub(crate) enum Command {
    Clear,

    Element(Action),

    ElementOn(NodeTarget, Action),

    Apply,

    Reload,

    Section(usize),

    Rule(usize),

    CancelPick,
}

// Enqueue one native action; a closing package has no current runtime to mutate.
fn enqueue(command: Command) {
    current(|runtime| runtime.enqueue(command));
}

// Ask the host to close the package's existing workspace widget after the native callback returns.

// Apply the declared DOM/CSS action after native delegates release their editor borrows.
pub(crate) fn element_action(action: Action) {
    enqueue(Command::Element(action));
}

// Preserve a context menu's original document generation and node identity until dispatch.
pub(crate) fn element_node_action(target: NodeTarget, action: Action) {
    enqueue(Command::ElementOn(target, action));
}

// Snapshot the complete edit source from this package's live DOM model.
pub(crate) fn element_edit(target: NodeTarget, offset: usize) -> Option<NodeEdit> {
    current(|runtime| runtime.model.borrow().elements.prepare_edit(target, offset)).flatten()
}

// Keep the Rust draft inside its originating package; only protocol bytes cross the host boundary.
pub(crate) fn commit_element_edit(edit: NodeEdit, source: String) {
    enqueue(Command::Element(Action::EditNode(edit, source)));
}

// Submit the current selected-rule editor through its enabled native Apply action.
pub(crate) fn apply_element_edit() {
    enqueue(Command::Apply);
}

// Cancel or reload the current local source draft without altering another module.
pub(crate) fn reload_element_draft() {
    enqueue(Command::Reload);
}

// Toggle the selected package-owned detail group.
pub(crate) fn toggle_element_section(index: usize) {
    enqueue(Command::Section(index));
}

// Preserve the index selected in this package's matched-rule list.
pub(crate) fn select_element_rule(index: usize) {
    enqueue(Command::Rule(index));
}

impl Runtime {
    // Mutate the package model and send commands only after its RefCell borrow is released.
    pub(crate) fn perform(&self, command: Command) {
        let mut model = self.model.borrow_mut();
        let elements = &self.content;
        let mut commands = Vec::new();
        match command {
            Command::Clear => model.elements.refresh(),

            Command::Element(action) => {
                // Selecting a detail section must also reveal its pane in compact layouts.
                if matches!(action, Action::Section(_)) {
                    elements.show_details();
                }
                model.elements.perform(action);
            }

            Command::ElementOn(target, action) => {
                // Context-menu navigation follows the same compact-pane behavior as ordinary selection.
                if matches!(action, Action::Section(_)) {
                    elements.show_details();
                }
                model.elements.perform_on(target, action);
            }

            Command::Apply => {
                // Native Apply may have become disabled after a newer stylesheet revision arrived.
                if let Some(action) = elements.apply_edit() {
                    model.elements.perform(action);
                    commands = model.protocol_commands();
                    mark_submission(elements, &commands);
                }
            }

            Command::Reload => {
                model.elements.notice = None;
                // The loaded package determines which detail controls own the draft.
                {
                    elements.reload_draft();
                }
            }

            Command::Section(index) => {
                // Only the Elements package owns foldable style groups.
                {
                    elements.toggle_section(index);
                }
            }

            Command::Rule(index) => {
                // Rule selection changes native presentation while retaining the existing protocol snapshot.
                {
                    elements.select_rule(index);
                }
            }

            Command::CancelPick => {
                model.elements.cancel_inspect();
            }
        }
        commands.extend(model.protocol_commands());
        drop(model);
        self.host.send(commands);
    }
}

// Mark a draft submitted only when model validation produced at least one actual mutation command.
fn mark_submission(elements: &crate::ui::elements::ElementsView, commands: &[String]) {
    // Validation failures keep the edited draft and its dirty state available for correction.
    if commands.is_empty() {
        return;
    }
    elements.submitted();
}
