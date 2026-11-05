//! Define the versioned C boundary between the browser workspace and separately installed native widget packages.
use serde::{Deserialize, Serialize};
use std::ffi::c_void;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    #[default]
    Console = 0,
    Network = 1,
    Elements = 2,
}
impl Tool {
    pub const ALL: [Self; 3] = [Self::Console, Self::Network, Self::Elements];

    // Persist package identities independently of presentation names and native action tags.
    pub fn id(self) -> &'static str {
        match self {
            Self::Console => "dev.enkel.console",
            Self::Network => "dev.enkel.network",
            Self::Elements => "dev.enkel.elements",
        }
    }

    // Validate a package's declared widget capability before exposing it to the workspace.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }

    /// Return the module's presentation name without using labels as command identifiers.
    pub fn title(self) -> &'static str {
        match self {
            Self::Console => "Console",
            Self::Network => "Network",
            Self::Elements => "Elements",
        }
    }

    /// Describe the actual built-in capability in the module list.
    pub fn description(self) -> &'static str {
        match self {
            Self::Console => "Logs and JavaScript",
            Self::Elements => "DOM and CSS",
            Self::Network => "Requests and streams",
        }
    }

    /// Declare each module's native system symbol as presentation data.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Console => "terminal",
            Self::Elements => "chevron.left.forwardslash.chevron.right",
            Self::Network => "network",
        }
    }
}

pub const ABI_VERSION: u32 = 1;
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
    pub data: *const u8,
    pub len: usize,
}
impl Bytes {
    // Borrow bytes only for the duration of one synchronous ABI call; ownership stays with the sender.
    pub fn new(value: &[u8]) -> Self {
        Self {
            data: value.as_ptr(),
            len: value.len(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostHeader {
    pub version: u32,
    pub size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
    pub version: u32,
    pub size: usize,
    pub context: u64,
    pub profile_key: u64,
    pub resize_cursor: extern "C" fn(u64) -> *mut c_void,
    pub next_id: extern "C" fn() -> i32,
    pub send: unsafe extern "C" fn(u64, *const Bytes, usize),
    pub schedule: extern "C" fn(u64, u32, u64, u32),
    pub notify: unsafe extern "C" fn(u64, u32, Bytes, bool),
}

#[repr(C)]
pub struct ModuleHeader {
    pub version: u32,
    pub size: usize,
    pub tool: u32,
}
#[repr(C)]
pub struct ModuleApi {
    pub header: ModuleHeader,
    pub create: unsafe extern "C" fn(*const HostApi, *mut c_void, *mut c_void) -> *mut c_void,
    pub destroy: unsafe extern "C" fn(*mut c_void),
    pub event: unsafe extern "C" fn(*mut c_void, u32, u64, Bytes) -> bool,
}

// Event numbers are ABI data; displayed labels and Rust enum layouts never cross the package boundary.
pub mod event {
    pub const CONNECT: u32 = 1;
    pub const RESET: u32 = 2;
    pub const PROTOCOL: u32 = 3;
    pub const RENDER: u32 = 4;
    pub const ACTIONS: u32 = 5;
    pub const COMPLETE: u32 = 6;
    pub const CLEAR: u32 = 7;
    pub const FILTER: u32 = 8;
    pub const FIND: u32 = 9;
    pub const PICK: u32 = 10;
    pub const INSPECT: u32 = 11;
    pub const CANCEL_PICK: u32 = 12;
    pub const DISMISS: u32 = 13;
    pub const LAYOUT: u32 = 14;
    pub const FOCUS: u32 = 15;
}
pub mod notice {
    pub const STATUS: u32 = 1;
    pub const CLOSE: u32 = 2;
    pub const COPY: u32 = 3;
    pub const FAILURE: u32 = 4;
}

#[cfg(feature = "appkit")]
pub mod appkit;
