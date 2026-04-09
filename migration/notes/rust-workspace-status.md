# Rust workspace status

Date: 2026-04-09

## Workspace scaffold

Existing Rust workspace under `rust/` is already in place and matches the intended migration layout:

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

## Current implementation snapshot

### `pi-ai`
Current Rust implementation is not blank scaffolding. It already includes:
- normalized event/message types via `pi-events`
- provider registry
- built-in model catalog parsing from `packages/ai/src/models.generated.ts`
- env API-key lookup coverage for many providers
- faux provider test slice
- OpenAI Responses provider slice with SSE parsing, request shaping, replay/tool-result handling, and HTTP tests

Current built-in registration still only wires `openai-responses` as the real provider path.

### `pi-agent`
Current Rust implementation already includes:
- conversation state
- agent loop
- sequential tool execution
- queue handling
- before/after tool hooks
- custom message conversion hooks
- stateful `Agent` wrapper

### `pi-coding-agent-*`
Current Rust implementation already includes:
- model registry/resolver/bootstrap slices
- auth source layering (`auth.json`, env, override)
- non-interactive CLI runner
- read/bash/edit/write tool slices
- `--list-models`, scoped `--models`, startup thinking clamp, image blocking, settings-backed `blockImages`
- request-time OAuth refresh in the non-interactive runtime path

### `pi-tui` and `pi-coding-agent-tui`
Current Rust TUI work is also beyond scaffolding. It already includes:
- fuzzy matching
- keybinding registry
- raw key parsing
- stdin buffering
- terminal protocol/control slice
- width/truncation/wrapping helpers
- slicing/segment helpers
- minimal container/overlay rendering
- cursor-marker extraction
- focus/input-routing slice
- terminal callback queue bridge
- additional coding-agent presentation components in `pi-coding-agent-tui`

## Existing package migration notes

Current package notes already exist:
- `migration/packages/ai.md`
- `migration/packages/agent.md`
- `migration/packages/coding-agent.md`
- `migration/packages/tui.md`

These notes show the migration has already advanced well past initial scaffolding.

## Immediate constraint going forward

Even though downstream crates already exist, further migration work should still prioritize `packages/ai` / `rust/crates/pi-ai` before expanding `agent`, `coding-agent`, or `tui` behavior further.

## Validation snapshot

`cd rust && cargo test -q --workspace` passes.
