# Rust workspace grounding

Date: 2026-04-14

## Purpose

Confirm what already exists under `rust/` before continuing the TypeScript-to-Rust migration.

## Files analyzed

- `README.md`
- `packages/ai/README.md`
- `packages/agent/README.md`
- `packages/coding-agent/README.md`
- `packages/tui/README.md`
- `rust/Cargo.toml`
- `rust/apps/pi/Cargo.toml`
- `rust/apps/pi/src/main.rs`
- `rust/crates/pi-core/Cargo.toml`
- `rust/crates/pi-core/src/lib.rs`
- `rust/crates/pi-config/Cargo.toml`
- `rust/crates/pi-config/src/lib.rs`
- `rust/crates/pi-events/Cargo.toml`
- `rust/crates/pi-events/src/lib.rs`
- `rust/crates/pi-test-harness/Cargo.toml`
- `rust/crates/pi-test-harness/src/lib.rs`
- `rust/crates/pi-ai/Cargo.toml`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/models.rs`
- `rust/crates/pi-ai/tests/models.rs`
- `rust/crates/pi-agent/Cargo.toml`
- `rust/crates/pi-agent/src/lib.rs`
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-core/Cargo.toml`
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-tools/Cargo.toml`
- `rust/crates/pi-coding-agent-tools/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/Cargo.toml`
- `rust/crates/pi-coding-agent-cli/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`

Validation command run:

- `cd rust && cargo test --workspace`

## What already exists

### Workspace scaffolding

The requested Rust workspace layout already exists and matches the intended split:

- shared crates
  - `pi-core`
  - `pi-config`
  - `pi-events`
  - `pi-test-harness`
- migrated package crates
  - `pi-ai`
  - `pi-agent`
  - `pi-tui`
  - `pi-coding-agent-core`
  - `pi-coding-agent-tools`
  - `pi-coding-agent-cli`
  - `pi-coding-agent-tui`
- app
  - `apps/pi`

So Step 1 from the migration order is not blank work anymore; it is already scaffolded and partially implemented.

### Shared crates

- `pi-core`
  - only contains `PiResult` and a minimal `PiError::NotImplemented`
  - still far from the requested “shared types, ids, utilities, errors” role
- `pi-config`
  - implemented, but narrow
  - currently loads only a small runtime settings subset from global/project `settings.json`
  - covers image settings, thinking budgets, editor padding, autocomplete list size, and warning collection
- `pi-events`
  - already acts as the shared message/event schema layer
  - defines messages, content blocks, usage, models, context, and assistant stream events
  - still uses broad string aliases for `Api` and `Provider`, so typing can be tightened later
- `pi-test-harness`
  - still a placeholder only
  - no real mock provider/scenario harness yet

### `pi-ai`

Already implemented as a real crate, not just a stub.

Current visible scope from the crate surface:

- providers/modules present
  - `anthropic_messages`
  - `openai_responses`
  - `openai_completions`
  - `openai_codex_responses`
  - faux provider support in `lib.rs`
- core features present
  - provider registry
  - streaming + complete APIs
  - simple options mapping
  - payload hooks
  - prompt-cache simulation in faux provider
  - env API key lookup
  - model catalog loading from the TypeScript `models.generated.ts`
- current model/provider filter
  - only `anthropic`, `openai`, and `openai-codex` are exposed from the Rust model catalog
  - this aligns with the migration scope you specified for providers in Rust

### `pi-agent`

Already implemented as a real crate.

Current visible scope from the crate surface:

- `Agent` wrapper
- low-level loop APIs (`agent_loop`, `agent_loop_continue`)
- tool execution hooks
- proxy streamer support
- typed agent state/messages/tools exports
- transport/thinking/payload hook pass-through from `pi-ai`

### `pi-tui`

Already implemented as a real crate, but only for a subset of the TypeScript TUI package.

Currently exported:

- autocomplete
- editor
- fuzzy matching
- input
- keybindings
- keys
- spacer
- stdin buffer
- terminal abstraction
- terminal image capability detection
- text / truncated text
- base TUI/container/render types
- width/wrapping helpers

Not visible from the current Rust public surface yet:

- markdown
- select list
- settings list
- loader / cancellable loader
- box/image components matching TS package structure

That fits the migration strategy of “only build what coding-agent actually needs first”, but it is not a full parity port of `packages/tui` yet.

### `pi-coding-agent-*`

The TypeScript `packages/coding-agent` split already exists in Rust:

- `pi-coding-agent-core`
  - auth
  - bootstrap
  - footer data
  - model registry/resolver
  - runtime
  - message conversion helpers
  - skill block parsing
- `pi-coding-agent-tools`
  - bash, read, write, edit
  - image resize helpers
  - truncation/path helpers
  - `create_coding_tools*` helpers
- `pi-coding-agent-cli`
  - args
  - auth overlays
  - file processing
  - initial message building
  - print mode
  - top-level command runner
- `pi-coding-agent-tui`
  - assistant/user/tool-rendering widgets
  - footer
  - startup shell/header
  - model selector
  - interactive binding
  - keybinding migration helpers

### App entrypoint

`rust/apps/pi` already wires together:

- auth refresh
- keybinding migration on startup
- stdin handling
- CLI parsing
- interactive vs non-interactive dispatch
- model catalog loading

## Current validation status

`cargo test --workspace` is green.

Recent `pi-ai` validation added on top of the existing suite:

- `rust/crates/pi-ai/tests/openai_codex_fixture_compat.rs`
- `rust/crates/pi-ai/tests/fixtures/openai_codex_sse_completed_terminal.sse`
- `rust/crates/pi-ai/tests/cross_provider_handoff.rs` validating Anthropic -> OpenAI Codex and OpenAI Responses -> Anthropic request-shape replay

Those lock in the TypeScript Codex behavior that the Rust stream must terminate on `response.completed` even if `[DONE]` is delayed, and the cross-provider request-shape replay used by the handoff regression.

## Immediate compatibility observations

1. The Rust rewrite is already much further along than just scaffolding.
2. The provider scope in Rust is already narrowed to the requested migration scope for `pi-ai`.
3. The shared crates are uneven:
   - `pi-events` is useful already
   - `pi-config` is partial but real
   - `pi-core` and `pi-test-harness` are still underbuilt
4. `pi-tui` is intentionally partial, which is fine for the migration order, but it is not yet a full replacement for `packages/tui`.
5. We still have not completed the required package-by-package TypeScript inventory notes. That remains the next mandatory migration step.

## Recommended next step

Continue `packages/ai` Step 3 with another exact fixture-driven parity check:

- pin Anthropic Claude Code OAuth tool-name round-trip behavior from the TypeScript suite
- keep the Rust workspace green after each small compatibility slice
- then choose the next `pi-ai` fixture target before moving on to `packages/agent`
