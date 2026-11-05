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

## Distribution and trust

`catalog.json` contains **only published, signed and notarized releases**. Its initially empty state is intentional: development signatures must not be presented as official installable releases.

Each `.enkelmodule` bundle contains `Contents/MacOS/Module`, its signed `Info.plist`, and `Contents/Resources/module.json` declaring capability, version, host requirement, architecture, ABI and build digest. Published assets are uncompressed POSIX `.tar` archives; symlinks, sparse files and extended archive records are excluded.

The browser checks release length and SHA-256, metadata compatibility, the configured official Apple Team ID, **Developer ID Application** certificate type, code signature and notarization before invoking `NSBundle`. Quarantine remains intact. An Apple Development or Apple Distribution signature is insufficient for official public distribution.

Notarization tickets for code bundles are verified online with `codesign --check-notarization`; generic code bundles cannot be treated as notarized applications merely by running `spctl --type execute`. See [Apple's notarization guidance](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

## Release setup

The `release` GitHub environment needs these variables:

- `APPLE_TEAM_ID`: official ten-character publisher Team ID.
- `DEVELOPER_ID_APPLICATION`: full Developer ID Application signing identity.

Store credentials as GitHub **environment secrets**, never in source or chat:

- `DEVELOPER_ID_P12_BASE64`, `DEVELOPER_ID_P12_PASSWORD`
- `RELEASE_KEYCHAIN_PASSWORD`
- `NOTARY_API_KEY_BASE64`, `NOTARY_KEY_ID`, `NOTARY_ISSUER_ID`

Use a dedicated signing certificate and least-privilege notarization key. Protect the release environment with required reviewers and restrict deployment refs to reviewed release tags. The workflow imports credentials into an ephemeral keychain and removes it even after failure. Follow [GitHub's signing guidance](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications) and [Apple's Developer ID instructions](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/).

Set the same official Team ID in the browser build using `ENKEL_MODULE_TEAM_ID` or its ignored local `.module-team` file. This does not change the browser's stable development signing identity. A hardened distribution browser and its native modules must use a compatible signing team/library-validation policy.

After updating the workspace version, create a reviewed `vMAJOR.MINOR.PATCH` tag on `main` and run **Publish notarized modules**. The workflow checks code, builds both supported architectures, signs every bundle, waits for Apple acceptance, verifies the tickets, uploads release assets, then atomically updates the catalog. Failed or incomplete builds cannot produce a catalog entry. The local equivalent is:

```sh
python3 scripts/release.py --tag v0.2.0 --target aarch64-apple-darwin \
  --identity 'Developer ID Application: Your Name (TEAMID)' \
  --team YOURTEAMID --notary-profile enkel-modules
```

## License

MIT OR Apache-2.0.
