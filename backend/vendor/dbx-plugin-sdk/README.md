# dbx-plugin-sdk (vendored)

Vendored copy of the DBX plugin Rust SDK, source:
<https://github.com/t8y2/dbx/tree/main/plugins/sdk/rust/dbx-plugin-sdk>
(version 0.1.0, Apache-2.0, as of 2026-10-19).

Why vendored instead of a crates.io / git dependency: the crate is not yet
published on crates.io, and a git dependency would pull the whole DBX
repository into the build graph. The DBX CLI generates
`dbx-plugin-sdk = { version = "0.1.0" }` — **switch this path dependency back
to a version dependency before submitting to the DBX Store** once the crate
is published.

The SDK only speaks the sidecar protocol (JSON-RPC over stdio); it contains
no QSDB logic, so reviewing the vendored copy against upstream is cheap.
