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

This is not a blank scaffold. It is already a working partial rewrite with passing Rust tests.

## Current implementation snapshot

### Shared crates

- `pi-core`: minimal shared error/result surface
- `pi-config`: runtime settings loading for image behavior and thinking budgets
- `pi-events`: shared normalized model/message/event types used across AI, agent, coding-agent, and TUI layers
- `pi-test-harness`: still much thinner than the long-term migration target; most behavior is currently frozen through per-crate tests instead of a fully shared scenario harness

### `pi-ai`

Current Rust `pi-ai` already provides real in-scope provider support for the requested migration scope:
- built-in model catalog sourced from TypeScript `packages/ai/src/models.generated.ts`
- model/provider helpers:
  - `built_in_models()`
  - `get_model()`
  - `get_models()`
  - `get_providers()`
  - `models_are_equal()`
  - `supports_xhigh()`
- normalized assistant event stream surface
- faux provider for deterministic migration tests
- real provider/runtime slices for:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- `stream_response()` / `complete()`
- narrowed but substantial `stream_simple()` / `complete_simple()` parity for the in-scope providers
- request option support including:
  - abort signaling
  - session id
  - cache retention
  - API key override
  - headers
  - metadata
  - payload hook / request mutation
  - temperature
  - max tokens
  - reasoning effort / summary
  - thinking budgets on the simple path
- OpenAI Codex multi-transport support, including SSE and WebSocket slices already covered by Rust tests
- overflow helpers and Unicode sanitization slices exercised by Rust tests

This crate is already beyond scaffolding and is the most complete migration layer today.

### `pi-agent`

Current Rust `pi-agent` already provides:
- normalized agent state/context/message model
- low-level prompt / continue loop
- stateful `Agent` wrapper
- queue handling for steering and follow-up messages
- tool registration and execution flow
- before/after tool hooks
- sequential tool execution with streamed tool updates
- default-streamer support for reasoning budget forwarding into `pi-ai`
- proxy stream reconstruction support and proxy tests

The main remaining gaps here are fuller TypeScript `Agent` surface parity and deeper proxy/tool reconstruction edge handling, not basic orchestration.

### `pi-coding-agent-*`

Current Rust coding-agent crates already provide a real application slice, not just utilities.

`pi-coding-agent-core` already includes:
- model registry and model resolution
- default model tables for the in-scope providers
- bootstrap logic for CLI model selection, scoped models, defaults, restored sessions, and thinking-level clamping
- auth source layering (`override -> auth.json -> env`)
- startup/request-time OAuth refresh entrypoints
- runtime wrapper over `pi-agent` + `pi-ai`
- coding-agent message conversion for custom message types already migrated
- footer-data and skill-block slices
- runtime settings hooks for image blocking / auto-resize flags and thinking budgets

`pi-coding-agent-tools` already includes:
- `read`
- `bash`
- `edit`
- `write`
- image resize helpers used by coding-agent paths

`pi-coding-agent-cli` already includes:
- argument parsing
- `--list-models`
- scoped `--models`
- non-interactive print/json runner
- `@file` preprocessing for text and images
- runtime API-key override behavior
- a live interactive runner path separate from the buffered non-interactive runner

`rust/apps/pi` is already wired to the Rust crates and now chooses between:
- buffered non-interactive command execution
- live interactive startup through the Rust TUI path

### `pi-tui` and `pi-coding-agent-tui`

Current Rust TUI work is materially beyond the early-widget stage.

`pi-tui` already includes:
- fuzzy matching/filtering
- configurable keybinding registry
- raw key parsing and matching
- stdin buffering
- terminal abstraction and process-backed terminal support
- terminal image capability / cell-size helpers
- width, truncation, wrapping, and ANSI-aware text helpers
- `Text`, `TruncatedText`, `Spacer`, `Input`, and a first multiline `Editor`
- `word_wrap_line(...)` for the first Rust multiline-editor slice
- `Tui`, `Container`, overlays, render handles, focus/input routing
- live resize callback support in the real process-backed terminal path

`pi-coding-agent-tui` already includes coding-agent-specific presentation/runtime helpers such as:
- transcript-related components
- user / assistant / tool-execution renderers
- footer and keybinding hint helpers
- startup header / startup shell
- interactive runtime binding via `InteractiveCoreBinding`
- keybinding migration helpers
- clipboard-image support slices

There is already an honest Rust interactive app path using the migrated startup shell and live TUI plumbing.

## Existing package migration notes

The package migration notes already exist and are ahead of initial inventory/scaffolding work:
- `migration/packages/ai.md`
- `migration/packages/agent.md`
- `migration/packages/coding-agent.md`
- `migration/packages/tui.md`

Those notes should be treated as the detailed migration record for package-by-package parity work. This status note is only the cross-workspace summary.

## Remaining high-level gaps

The rewrite is still incomplete. The biggest remaining gaps visible from the current Rust code and migration notes are:

- `pi-ai`
  - broader parity for the full TypeScript provider/config surface outside the intentionally narrowed in-scope providers and options
  - more compatibility coverage for remaining edge cases that are still only documented in the TS suite
- `pi-agent`
  - fuller TypeScript `Agent` wrapper surface parity
  - deeper proxy/tool reconstruction parity in edge cases
- `pi-coding-agent-*`
  - broader session-manager/resource-loader/extensions parity
  - more of the TypeScript interactive runtime behavior above the current startup-shell path
  - richer transcript/navigation/runtime UX parity
- `pi-tui`
  - multiline/custom editor parity
  - richer widgets still missing or partial (`Box`, autocomplete, markdown, select/settings/image widgets, broader renderer parity)
  - broader theme/image/widget parity
- `pi-test-harness`
  - still not the full shared Rust scenario harness envisioned in the migration plan

## Immediate constraint going forward

The workspace is no longer in a “create scaffolding” phase. New work should be treated as package-level parity closing work against the TypeScript implementation, with small TS-grounded milestones and tests.

The next practical blocker is no longer workspace creation; it is closing the highest-value remaining interactive and runtime gaps without overbuilding.

## Validation snapshot

Validated on 2026-04-10:

- `cd rust && cargo test -q --workspace` passed
