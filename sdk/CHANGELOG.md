# Enkel module SDK

## 0.3.0

- Add the Downloads capability and its read-only transfer snapshot event. The native ABI remains 1. Downloads needs Enkel 0.2.1 or later in the 0.2 series.
- Existing Console, Network and Elements binaries retain their ABI and module IDs.

Use a tagged SDK dependency, such as `enkel-module-sdk = { git = "https://github.com/maxonlinux/enkel-modules.git", tag = "sdk-v0.3.0", features = ["appkit"] }`. Each module declares its own release version and the host range it needs. Increase the ABI only for incompatible changes to the native tables or calling contract.
