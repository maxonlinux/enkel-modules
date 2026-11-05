//! Own package-local protocol state and native controls; the host owns windows, placement and CEF transport.
use crate::{
    developer::Session,
    module_api::{event, notice},
    ui,
};
use enkel_module_sdk::appkit::{Host, Module};
use objc2::{Message, rc::Retained};

use objc2_app_kit::NSView;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

thread_local! { static CURRENT: RefCell<Weak<Runtime>> = const { RefCell::new(Weak::new()) }; }

// Borrow the current package only while its host instance remains alive.
pub(crate) fn current<T>(run: impl FnOnce(&Runtime) -> T) -> Option<T> {
    let runtime = CURRENT.with(|current| current.borrow().upgrade())?;
    Some(run(&runtime))
}

pub(crate) struct Runtime {
    pub(crate) model: RefCell<Session>,
    pub(crate) content: ui::console::ConsoleControls,
    pub(crate) host: Host,
    _parent: Retained<NSView>,
    query: RefCell<String>,
    pub(crate) actions: RefCell<Vec<crate::app::Command>>,
    actions_scheduled: Cell<bool>,
    render_scheduled: Cell<bool>,
    pub(crate) edit_generation: Cell<u64>,
    status: RefCell<(String, bool)>,
}
impl Runtime {
    // Construct package-owned controls inside the retained host surface without taking over its geometry.
    fn create(host: Host, parent: &NSView, _primary: &objc2_app_kit::NSButton) -> Rc<Self> {
        let _main_thread = ui::mtm();
        let parent = parent.retain();
        let model = Session::default();

        let content = ui::console::ConsoleControls::new(&parent);
        let runtime = Rc::new(Self {
            host,
            _parent: parent,
            content,
            model: RefCell::new(model),
            query: RefCell::new(String::new()),
            actions: RefCell::new(Vec::new()),
            actions_scheduled: Cell::new(false),
            render_scheduled: Cell::new(false),
            edit_generation: Cell::new(0),
            status: RefCell::new((String::new(), false)),
        });
        CURRENT.with(|current| current.replace(Rc::downgrade(&runtime)));
        runtime
    }

    // Queue native actions outside model borrows; one host task drains the complete input burst.
    pub(crate) fn enqueue(&self, command: crate::app::Command) {
        self.actions.borrow_mut().push(command);
        // A previously queued wake already owns this action batch.
        if !self.actions_scheduled.replace(true) {
            self.host.schedule(event::ACTIONS, 0, 0);
        }
    }

    // Coalesce protocol bursts before converting snapshots into native text and table cells.
    fn request_render(&self) {
        // A scheduled render already owns all accumulated model changes.
        if !self.render_scheduled.replace(true) {
            self.host.schedule(event::RENDER, 0, 16);
        }
    }

    // Route host events into this package; unrelated tools never share mutable protocol models.
    pub(crate) fn event(&self, kind: u32, _token: u64, bytes: &[u8]) -> bool {
        match kind {
            event::CONNECT => {
                self.reset("");
                self.host.send(vec![self.model.borrow().connect()]);
            }
            event::RESET => {
                self.reset(std::str::from_utf8(bytes).unwrap_or("Open a page to connect"))
            }
            event::PROTOCOL => {
                let mut model = self.model.borrow_mut();
                let commands = model.receive_bytes(bytes);
                let changed = model.changes.any();
                drop(model);
                self.host.send(commands);
                // A protocol message can belong to another attached inspector and require no native changes.
                if changed {
                    self.request_render();
                }
            }
            event::ACTIONS => {
                self.actions_scheduled.set(false);
                let actions = self.actions.take();
                for command in actions {
                    self.perform(command);
                }
                self.render();
            }
            event::RENDER => {
                self.render_scheduled.set(false);
                self.render();
            }
            event::CLEAR => {
                self.perform(crate::app::Command::Clear);
                self.render();
            }
            event::FILTER => {
                self.query
                    .replace(String::from_utf8_lossy(bytes).into_owned());

                self.render();
            }

            event::DISMISS => {
                return crate::app::dismiss_native_completion();
            }
            event::LAYOUT => {
                self.content.position_popup(&self._parent);
            }
            event::FOCUS => {
                let console = &self.content;
                let focus: &NSView = &console.input;
                // Newly mounted content receives focus only after its native window is available.
                if let Some(window) = focus.window() {
                    window.makeFirstResponder(Some(focus));
                }
            }

            event::COMPLETE => self.complete(_token),
            _ => return false,
        }
        true
    }

    // Invalidate every outstanding UI/model generation when the host changes or closes the inspected page.
    fn reset(&self, status: &str) {
        self.edit_generation.set(self.edit_generation.get() + 1);
        self.actions.borrow_mut().clear();
        let mut model = Session::default();

        // A closed/empty target needs an explicit state rather than an endless Connecting label.
        if !status.is_empty() {
            model.status = status.into();
        }
        self.model.replace(model);
        self.render();
    }

    // Convert only this widget's model into native presentation and release its commands after the model borrow.
    pub(crate) fn render(&self) {
        let mut model = self.model.borrow_mut();

        let status = &model.status;

        // Unchanged connection labels need no cross-package notification or native layout invalidation.
        if self.status.borrow().0 != *status || self.status.borrow().1 != model.ready {
            self.host.notify(notice::STATUS, status, model.ready);
            self.status.replace((status.clone(), model.ready));
        }
        let console = &self.content;

        let query = self.query.borrow();
        let view = model.view(&query);
        console.set_state(model.ready, &view.text, !query.is_empty());
        console.set_objects(&view);
        console.set_completions(model.completion.suggestions.as_ref());

        let commands = model.protocol_commands();
        drop(model);
        self.host.send(commands);
    }

    // Run completion only for the edit generation that scheduled the delayed request.
    fn complete(&self, token: u64) {
        // Edits, target changes and canceled suggestions invalidate previously queued completion work.
        if token != self.edit_generation.get() {
            return;
        }

        if let Some((expression, caret)) = self.content.input_snapshot() {
            let commands = self.model.borrow_mut().complete(&expression, caret);
            self.host.send(commands);
        }
    }
}

impl Module for Runtime {
    // Mount this project's native widget without taking ownership of the browser's workspace geometry.
    fn mount(host: Host, parent: &NSView, primary: &objc2_app_kit::NSButton) -> Rc<Self> {
        Self::create(host, parent, primary)
    }

    // Deliver an event to this project only; no other widget state is compiled into this runtime.
    fn event(&self, kind: u32, token: u64, bytes: &[u8]) -> bool {
        self.event(kind, token, bytes)
    }
}
