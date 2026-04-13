# packages/ai milestone 24: Anthropic `streamSimple()` thinking-disable parity

Status: completed 2026-04-13
Target crate: `rust/crates/pi-ai`

## Files analyzed

TypeScript files read for this slice:
- `packages/ai/src/providers/anthropic.ts`
- `packages/ai/src/types.ts`
- `packages/ai/src/providers/simple-options.ts`
- `packages/ai/test/anthropic-thinking-disable.test.ts`

Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_params.rs`
- `rust/crates/pi-ai/tests/simple_stream.rs`
- `rust/crates/pi-ai/Cargo.toml`

## Behavior inventory summary

TypeScript behavior grounded for this milestone:
- `streamSimple(model, context, options)` must disable Anthropic thinking when `options.reasoning` is omitted.
- That behavior applies to both Anthropic reasoning model families currently in scope:
  - budget-based reasoning models such as `claude-sonnet-4-5`
  - adaptive reasoning models such as `claude-opus-4-6`
- When thinking is disabled through the simple API path:
  - request payload includes `thinking: { type: "disabled" }`
  - request payload omits `output_config`
- The TypeScript payload-freeze test captures payload before the HTTP request fails, so the compatibility target is the preflight payload shape, not a successful network round trip.

## Rust design summary

No runtime API redesign was needed.

Observed Rust mapping already matched the TypeScript intent:
- `pi-ai/src/lib.rs`
  - `map_simple_stream_options(...)` forwards omitted `reasoning` as `reasoning_effort: None`
- `pi-ai/src/anthropic_messages.rs`
  - `anthropic_options_from_stream_options(...)` maps `None` reasoning effort to `thinking_enabled: Some(false)` for reasoning-capable Anthropic models
  - `build_thinking_config(...)` converts that into `AnthropicThinkingConfig::Disabled`
  - adaptive-only `output_config` is emitted only when thinking is explicitly enabled

This milestone therefore freezes parity at the higher-level `complete_simple()` / `streamSimple()` surface rather than changing runtime logic.

## Files created or modified

Created:
- `migration/packages/ai-milestone-24.md`

Modified:
- `rust/crates/pi-ai/tests/simple_stream.rs`

## Tests added

Added two TS-derived Rust tests in `rust/crates/pi-ai/tests/simple_stream.rs`:
- `simple_anthropic_disables_thinking_for_budget_reasoning_models_by_default`
- `simple_anthropic_disables_thinking_for_adaptive_reasoning_models_by_default`

Both tests capture the payload via the Rust payload-hook surface before the HTTP request fails, mirroring the TypeScript payload test strategy.

## Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-ai --test simple_stream`
- `cd rust && cargo test -p pi-ai`

Repo-wide TypeScript validation still pending for this slice at the monorepo level:
- `npm run check`

## Open questions

- None for this narrowed slice.
- Broader Anthropic simple-path parity still needs an explicit audit against any remaining TS-only simple API behavior outside this disabled-thinking path.

## Recommended next step

Stay on `packages/ai` and continue freezing remaining in-scope `streamSimple()` compatibility behavior that is still only indirectly covered in Rust, then move to the next highest-value `pi-agent` parity gap once the AI simple-path surface is better locked down.
