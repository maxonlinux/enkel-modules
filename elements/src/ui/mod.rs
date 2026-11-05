//! Assemble package-owned native widgets using shared AppKit primitives, without importing the browser's window or workspace implementation.
use objc2::{
    DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::{NSIndexSet, NSNotification, NSRect, NSSize, NSString};
use std::cell::{Cell, RefCell};

use std::collections::HashMap;

use enkel_module_sdk::appkit::ui::*;
pub(crate) use enkel_module_sdk::appkit::ui::{controls, mtm};

pub(crate) mod elements;

mod panels {
    pub(super) use enkel_module_sdk::appkit::ui::controls::DeveloperSplit;
    pub(super) use enkel_module_sdk::appkit::ui::{configure_scroll, text_editor};
}
