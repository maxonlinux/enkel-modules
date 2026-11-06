# Enkel module conventions

- Keep each native module independent of browser implementation and CEF. Share only the versioned SDK and build-only checks through normal Cargo dependencies.
- Keep source files at most 600 physical lines, with leading `//!` documentation. Comment function purposes and guards, separate sibling functions, prefer guard clauses and focused callbacks. Refactor instead of weakening checks.
- Keep `src/ui` and `src/developer` flat within each project. Tests remain flat in each project's `tests/`, privately included at the end of their owning source files. Do not publish implementation details for testing.
- Browser WidgetHost owns window/workspace geometry and input routing. Module views own only their internal layout. Use compact native AppKit controls and semantic appearance colors.
- Restrict unsafe to actual native/ABI operations with local safety comments. Never pass Rust layouts across the module interface or unload images containing registered AppKit classes.
- Before publishing, run formatting, strict Clippy and all workspace tests with the locked dependency graph. Do not click or automate the user's browser UI; launch builds for their manual review when requested.
- Native modules use account-free ad-hoc resource seals, validated package metadata and repository-scoped archive hashes. Do not require Apple Developer credentials or a development launch flag. Preserve quarantine metadata and native macOS enforcement; never change Gatekeeper or library-validation settings. Do not describe an ad-hoc seal as publisher authentication. Never commit signing credentials.
