# Enkel module conventions

- Keep each native module independent of browser implementation and CEF. Share only the versioned SDK and build-only checks through normal Cargo dependencies.
- Keep source files at most 600 physical lines, with leading `//!` documentation. Comment function purposes and guards, separate sibling functions, prefer guard clauses and focused callbacks. Refactor instead of weakening checks.
- Keep `src/ui` and `src/developer` flat within each project. Tests remain flat in each project's `tests/`, privately included at the end of their owning source files. Do not publish implementation details for testing.
- Browser WidgetHost owns window/workspace geometry and input routing. Module views own only their internal layout. Use compact native AppKit controls and semantic appearance colors.
- Restrict unsafe to actual native/ABI operations with local safety comments. Never pass Rust layouts across the module interface or unload images containing registered AppKit classes.
- Before publishing, run formatting, strict Clippy and all workspace tests with the locked dependency graph. Do not click or automate the user's browser UI; launch builds for their manual review when requested.
- Official artifacts require Developer ID Application signing and accepted Apple notarization. Never strip quarantine, weaken Gatekeeper/library validation or publish development-signed binaries as releases. Never commit signing credentials.
