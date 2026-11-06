# Enkel modules

Official native modules for the Enkel macOS browser. Console, Network and Elements are independent Cargo projects. They are **not compiled into the browser or activated by build features**.

| Project | Responsibility |
| --- | --- |
| `sdk` | Versioned C interface, host services and shared AppKit controls |
| `console` | Console UI, evaluation, completion and remote-object ownership |
| `network` | Request capture, timeline, table and request details |
| `elements` | DOM inspection, editing and styles |
| `build-support` | Build-time source conventions; no runtime component |

Each module builds a `cdylib` with its own code and protocol model. The browser owns its CEF observer, native windows, workspace placement and input routing. Modules receive scoped protocol messages through a versioned C ABI; Rust layouts, allocators and trait objects never cross that boundary. The SDK has no CEF dependency.

A module starts capture only when opened. Closing drops its UI/model and detaches its protocol sessions. AppKit classes prevent safe code unloading, so loaded images remain resident until browser exit; updating a loaded module requires restarting Enkel. There is no WebAssembly VM, separate browser process per module or background update polling.

## Development

macOS 13+, Xcode Command Line Tools and Rust 1.99.1 are required.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo build --locked -p enkel-module-console
```

Every module's build checks the 600-line source limit, purpose comments, function spacing and guard clauses. Native UI and protocol owners use flat files. Tests live in each project's flat `tests/` directory and are included privately by their owning source modules.

## Installation

Open **Enkel > Modules** in the browser, select a repository and install the
packages you want. Modules are downloaded as native bundles; users do not need
Rust, Xcode or an Apple Developer account.

For local source development, quit Enkel normally before replacing loaded code:

```sh
python3 scripts/develop.py --profile /absolute/path/to/browser-profile
```

Use a separate profile when testing. No development flag or signing certificate
is required to use these modules.

## Packages and trust

Each `.enkelmodule` bundle contains its native library, `Info.plist` and a
versioned `module.json` declaring its identity, architecture, host version, ABI
and build digest. Archives use ordinary POSIX `.tar` entries.

The browser checks repository-scoped download URLs, exact archive lengths,
SHA-256, bundle identity, architecture, ABI and native code integrity. Packages
use ad-hoc code seals that require no certificate or paid account. These seals
detect changed files; they do not identify a publisher. Trust comes from the
repository you choose, and modules execute inside the browser process.

Quarantine metadata is preserved. The installer does not disable Gatekeeper,
resign downloaded packages or change system security settings. macOS can still
require its own approval for software imported from outside the catalog.

## Publishing

Create a `vMAJOR.MINOR.PATCH` tag from `main` matching the workspace version,
then run **Publish native modules** in GitHub Actions. It builds both supported
architectures, applies account-free resource seals, verifies the archives and
publishes all six packages before updating `catalog.json`. No Apple signing
credentials or notarization service are required.

The local build command is:

```sh
python3 scripts/release.py --tag v0.2.3 --target aarch64-apple-darwin
```

A release must contain Console, Elements and Network for both supported
architectures. The catalog updater rejects incomplete or modified artifacts.

## License

MIT OR Apache-2.0.
