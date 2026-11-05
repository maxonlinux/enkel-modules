//! Enforce this independent project's source conventions through the shared build-only checker.
// Check the owning project before compiling any native code.
fn main() {
    enkel_module_build::check(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
}
