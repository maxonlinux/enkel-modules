//! Own native ABI lifetimes, host services and panic containment shared by independently compiled module projects.
use crate::{ABI_VERSION, Bytes, HostApi, HostHeader, MAX_MESSAGE};
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSButton, NSCursor, NSCursorFrameResizePosition, NSView};
use std::{
    cell::Cell,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::atomic::{AtomicI32, Ordering},
};
thread_local! { static CURRENT_HOST: Cell<Option<HostApi>> = const { Cell::new(None) }; }
static MODEL_IDS: AtomicI32 = AtomicI32::new(1);

#[derive(Clone, Copy)]
pub struct Host(HostApi);
impl Host {
    // Borrow host callbacks only after the factory validated their complete versioned table.
    fn new(api: HostApi) -> Self {
        CURRENT_HOST.with(|host| host.set(Some(api)));
        Self(api)
    }

    // Coalesce native work on the browser's existing event loop without creating polling timers or another process.
    pub fn schedule(self, kind: u32, token: u64, delay: u32) {
        (self.0.schedule)(self.0.context, kind, token, delay);
    }

    // Transfer protocol bytes synchronously while retaining the command strings for the entire callback.
    pub fn send(self, commands: Vec<String>) {
        // Empty batches require no cross-library allocation or scheduled transport work.
        if commands.is_empty() {
            return;
        }
        let borrowed: Vec<_> = commands
            .iter()
            .map(|value| Bytes::new(value.as_bytes()))
            .collect();
        // SAFETY: Every borrowed buffer and the descriptor array stay alive until the host returns.
        unsafe {
            (self.0.send)(self.0.context, borrowed.as_ptr(), borrowed.len());
        }
    }

    // Report a presentation change without sending a Rust string layout across the native library boundary.
    pub fn notify(self, kind: u32, message: &str, flag: bool) {
        // SAFETY: The host copies initialized UTF-8 bytes synchronously and never retains this slice pointer.
        unsafe {
            (self.0.notify)(self.0.context, kind, Bytes::new(message.as_bytes()), flag);
        }
    }
}

// Allocate IDs through the host; model-only tests use their own process-local monotonic namespace.
pub fn next_id() -> i32 {
    CURRENT_HOST.with(|host| {
        host.get().map_or_else(
            || MODEL_IDS.fetch_add(1, Ordering::Relaxed),
            |host| (host.next_id)(),
        )
    })
}

// Namespace native autosave keys without giving module views access to browser profile files.
pub fn profile_key() -> u64 {
    CURRENT_HOST.with(|host| host.get().map_or(0, |host| host.profile_key))
}

// Consume the +1 native cursor returned by the browser's single macOS compatibility boundary.
pub fn resize_cursor(position: NSCursorFrameResizePosition) -> Retained<NSCursor> {
    let pointer =
        CURRENT_HOST.with(|host| host.get().map(|host| (host.resize_cursor)(position as u64)));
    let Some(pointer) = pointer else {
        return NSCursor::arrowCursor();
    };
    // SAFETY: The host transfers one retained NSCursor; no object layout or ownership is inferred by the module.
    unsafe { Retained::from_raw(pointer.cast()) }.unwrap_or_else(NSCursor::arrowCursor)
}

pub trait Module: Sized {
    // Mount package-owned controls inside the host's retained AppKit surface.
    fn mount(host: Host, parent: &NSView, primary: &NSButton) -> Rc<Self>;

    // Handle a synchronous host event without retaining its borrowed input bytes.
    fn event(&self, kind: u32, token: u64, bytes: &[u8]) -> bool;
}

/// Construct a package runtime through its validated native descriptor.
///
/// # Safety
/// The host must supply a readable HostHeader prefix and live main-thread NSView/NSButton objects retained until destruction.
pub unsafe extern "C" fn create<T: Module>(
    host: *const HostApi,
    parent: *mut c_void,
    primary: *mut c_void,
) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        // Reject incomplete inputs before interpreting either a native object or an older callback table.
        if host.is_null()
            || parent.is_null()
            || primary.is_null()
            || MainThreadMarker::new().is_none()
        {
            return std::ptr::null_mut();
        }
        // SAFETY: Supported descriptors always begin with this readable version/size prefix.
        let header = unsafe { &*host.cast::<HostHeader>() };
        // The complete table extent must match before any callback slot is read.
        if header.version != ABI_VERSION || header.size != std::mem::size_of::<HostApi>() {
            return std::ptr::null_mut();
        }
        // SAFETY: The prefix confirms the complete table; host native objects remain retained throughout this synchronous mount.
        let (host, parent, primary) = unsafe {
            (
                *host,
                &*parent.cast::<NSView>(),
                &*primary.cast::<NSButton>(),
            )
        };
        Rc::into_raw(T::mount(Host::new(host), parent, primary))
            .cast_mut()
            .cast()
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Release all module-owned controls and state while host AppKit objects are still alive.
///
/// # Safety
/// The non-null context must be returned by create::<T> and destroyed exactly once after host event delivery stops.
pub unsafe extern "C" fn destroy<T: Module>(context: *mut c_void) {
    // A failed factory does not own an instance to release.
    if context.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Final host ownership consumes precisely the strong reference transferred by create.
        drop(unsafe { Rc::from_raw(context.cast::<T>()) });
    }));
}

/// Dispatch a bounded native input buffer without unwinding into the caller.
///
/// # Safety
/// Context must be a live create::<T> result; nonempty bytes must remain initialized and readable until this call returns.
pub unsafe extern "C" fn event<T: Module>(
    context: *mut c_void,
    kind: u32,
    token: u64,
    bytes: Bytes,
) -> bool {
    // A malformed or oversized event cannot be interpreted as a Rust reference or slice.
    if context.is_null() || bytes.len > MAX_MESSAGE || (bytes.len > 0 && bytes.data.is_null()) {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let input = if bytes.len == 0 {
            &[]
        } else {
            // SAFETY: The caller retains this checked, initialized buffer throughout the synchronous event.
            unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) }
        };
        // SAFETY: Host event delivery retains this typed runtime until the call returns.
        unsafe { &*context.cast::<T>() }.event(kind, token, input)
    }))
    .unwrap_or(false)
}

// Publish one immutable C descriptor while keeping all Rust module implementation types inside their own library.
#[macro_export]
macro_rules! export_module {
    ($tool:expr, $runtime:ty) => {
        static API: $crate::ModuleApi = $crate::ModuleApi {
            header: $crate::ModuleHeader {
                version: $crate::ABI_VERSION,
                size: std::mem::size_of::<$crate::ModuleApi>(),
                tool: $tool as u32,
            },
            create: $crate::appkit::create::<$runtime>,
            destroy: $crate::appkit::destroy::<$runtime>,
            event: $crate::appkit::event::<$runtime>,
        };
        // Return the package's versioned interface after NSBundle has loaded its native image.
        #[unsafe(no_mangle)]
        pub extern "C" fn enkel_module_v1() -> *const $crate::ModuleApi {
            &API
        }
    };
}

#[path = "native_ui.rs"]
pub mod ui;
