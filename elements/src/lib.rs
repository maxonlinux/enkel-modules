//! Export the independent Elements AppKit module; all controls and protocol state are owned by this project.
mod app;
mod developer;
mod runtime;
mod ui;
pub use enkel_module_sdk as module_api;
pub(crate) use enkel_module_sdk::appkit::next_id as protocol_id;
enkel_module_sdk::export_module!(module_api::Tool::Elements, runtime::Runtime);
