# Rust workspace

This workspace contains the in-repo Rust rewrite for the `packages/ai`, `packages/agent`, `packages/coding-agent`, and `packages/tui` portions of pi.

## Scope

The rewrite target is the Rust code under `rust/crates` and `rust/apps`.

Out of scope:

- JavaScript/TypeScript extensions
- any extension runtime, sidecar, or compatibility bridge

## Current extension status

Extensions are removed from the Rust CLI rewrite.

Current behavior:

- the Rust CLI does not load, discover, or execute extensions
- `--extension` / `-e` and `--no-extensions` are rejected
- extension configuration in settings or auto-discovery locations is rejected
- only the TypeScript codebase remains the compatibility reference for extension behavior

`rust/crates/pi-coding-agent-cli/src/rpc_extensions.rs` now exists only as a compile-time stub so the rest of the Rust CLI can build while extension support stays out of scope.
