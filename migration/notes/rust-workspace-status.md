# Rust workspace status

Date: 2026-04-10

## Workspace scaffold

The Rust workspace under `rust/` already exists and matches the requested migration layout:

- `rust/Cargo.toml`
- `rust/apps/pi`
- `rust/crates/pi-core`
- `rust/crates/pi-config`
- `rust/crates/pi-events`
- `rust/crates/pi-test-harness`
- `rust/crates/pi-ai`
- `rust/crates/pi-agent`
- `rust/crates/pi-tui`
- `rust/crates/pi-coding-agent-core`
- `rust/crates/pi-coding-agent-tools`
- `rust/crates/pi-coding-agent-cli`
- `rust/crates/pi-coding-agent-tui`

This is not a blank scaffold anymore. The workspace is already a working partial rewrite.

## Current implementation snapshot

### Shared crates

- `pi-core`: minimal shared error/result surface
- `pi-config`: narrow settings loading slice currently used for `images.blockImages`
- `pi-events`: shared normalized message/model/event types used across AI, agent, coding-agent, and TUI-adjacent crates
- `pi-test-harness`: still effectively placeholder-level and not yet the full long-term shared scenario harness described in the migration plan

### `pi-ai`

Current Rust `pi-ai` already provides:
- provider registry and built-in registration
- normalized assistant event stream surface
- built-in model catalog loaded from TypeScript `packages/ai/src/models.generated.ts`
- provider/model helpers (`built_in_models`, `get_model`, `get_models`, `get_providers`, `models_are_equal`, `supports_xhigh`)
- overflow detection helpers
- faux provider for deterministic tests
- real in-scope provider/runtime slices for:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- Codex multi-transport support including SSE, WebSocket, retry, and session-scoped WebSocket reuse
- narrowed `stream_simple()` / `complete_simple()` parity for the in-scope providers
- payload-hook request overrides now preserve arbitrary JSON fields through final provider dispatch

This is already a substantial compatibility layer, not scaffolding.

### `pi-agent`

Current Rust `pi-agent` already provides:
- normalized agent state/context types
- low-level prompt/continue loop
- stateful `Agent` wrapper
- tool registration and execution flow
- before/after tool hooks
- tool-argument validation/coercion slice
- queue handling for steering/follow-up messages
- proxy streaming support with request/response wire-shape reconstruction

### `pi-coding-agent-*`

Current Rust coding-agent crates already provide a real non-interactive application slice:
- `pi-coding-agent-core`
  - model catalog/bootstrap/resolution
  - auth source layering (`override -> auth.json -> env`)
  - startup and request-time OAuth refresh
  - runtime wrapper over `pi-agent` + `pi-ai`
  - coding-agent message conversion
  - settings-backed image blocking
- `pi-coding-agent-tools`
  - `read`, `bash`, `edit`, `write`
  - image resize helpers used by the non-interactive file path
- `pi-coding-agent-cli`
  - argument parsing
  - `--list-models`
  - scoped `--models`
  - print/json non-interactive runner
  - `@file` preprocessing
- `rust/apps/pi`
  - thin binary entrypoint over the Rust CLI/core path

### `pi-tui` and `pi-coding-agent-tui`

Current Rust TUI work is also materially beyond scaffolding.

`pi-tui` already includes:
- fuzzy matching
- keybinding registry
- raw key parsing
- stdin buffering
- terminal protocol/control handling
- terminal image capability/cell-size state
- width/truncation/wrapping/slicing helpers
- minimal container/overlay rendering
- cursor-marker extraction
- focus and input routing
- queued terminal-callback bridge
- first widget slices (`Text`, `TruncatedText`, `Spacer`, `Input`)
- overlay/render handles exported from the crate root

`pi-coding-agent-tui` already includes coding-agent-specific presentation components such as:
- transcript/user/assistant/tool-execution renderers
- footer and keybinding hint helpers
- startup header/shell components
- clipboard image helpers
- migrated app keybinding config/migration helpers

## Existing package migration notes

The package inventories already exist and show the migration is well past initial scaffolding:
- `migration/packages/ai.md`
- `migration/packages/agent.md`
- `migration/packages/coding-agent.md`
- `migration/packages/tui.md`

## Remaining high-level gaps

The rewrite is not finished. The biggest remaining gaps from the current notes are:

- `pi-ai`
  - broader `streamSimple()` / `completeSimple()` parity beyond the narrowed current slice
- `pi-agent`
  - fuller parity with the TypeScript `Agent` property surface and partial-JSON proxy/tool reconstruction edges
- `pi-coding-agent-*`
  - session-manager/resource-loader/extensions/interactive-mode parity
- `pi-tui`
  - broader widget coverage and additional renderer parity still needed before full interactive coding-agent integration
- `pi-test-harness`
  - not yet the full shared Rust migration harness envisioned in the target plan

## Immediate constraint going forward

Per the requested migration order, new behavior work should still stay disciplined:
1. confirm workspace/shared crate status
2. finish/close remaining `pi-ai` gaps that block downstream fidelity
3. only then continue broader `pi-agent`, `pi-coding-agent`, and `pi-tui` parity work

## Validation snapshot

Validated on 2026-04-10:

- `cd rust && cargo test -q --workspace` passed
