Continue the TypeScript-to-Rust migration in `/home/rtb/code/agent/pi-mono` on branch `rust-migration-ai`.

Scope constraints:
- Only migrate:
  - `packages/ai -> rust/crates/pi-ai`
  - `packages/agent -> rust/crates/pi-agent`
  - `packages/coding-agent`
  - `packages/tui`
- Do not touch:
  - `packages/mom`
  - `packages/pods`
  - `packages/web-ui`
- TS remains the source of truth for behavior.
- Do not run runtime apps / agent / tui processes.
- Work incrementally and stop after one milestone with exactly this 8-point report:
  1. Files analyzed
  2. Behavior inventory summary
  3. Rust design summary
  4. Files created/modified
  5. Tests added
  6. Validation results
  7. Open questions
  8. Recommended next step

Repo/worktree rules:
- Read every file you modify in full before editing.
- Use read for file contents, not cat/sed.
- After code changes, run required validation.
- In this environment, `npm run check` is blocked because `biome` is missing; note that explicitly.
- Do not commit unless asked.

Current state after the latest coding-agent-core milestone:
- `rust/crates/pi-coding-agent-core` now has:
  - model resolution (`model_resolver.rs`)
  - `models.json` subset registry (`model_registry.rs`)
  - minimal auth seam (`auth.rs`)
  - uncached config/header resolution (`config_value.rs`)
  - startup bootstrap selection (`bootstrap.rs`)
  - minimal non-interactive runtime (`runtime.rs`)
- `create_coding_agent_core()` now:
  - builds `ModelRegistry`
  - runs `bootstrap_session()`
  - creates a `pi-agent::Agent`
  - uses a registry-backed streamer to resolve auth/headers into `pi-ai::stream_response()`
- Tests added under `rust/crates/pi-coding-agent-core/tests/`:
  - `model_resolver.rs`
  - `model_registry.rs`
  - `bootstrap.rs`
  - `runtime.rs`
- Validation already completed on this milestone:
  - `cd rust && cargo fmt`
  - `cd rust && cargo test -p pi-coding-agent-core`
  - `cd rust && cargo test`
  - `npm run check` fails with `biome: command not found`

Important current gaps:
- built-in model catalog still injected as `Vec<Model>` instead of sourced from Rust `pi-ai`
- coding-agent custom message conversion parity from `packages/coding-agent/src/core/messages.ts` is still missing
- default coding tools (`read`, `bash`, `edit`, `write`) are not wired into the runtime yet
- no auth.json persistence / OAuth / dynamic provider lifecycle yet
- no session-manager/settings/resource-loader integration yet
- no TUI work yet

Recommended next task:
- Stay in `packages/coding-agent / rust/crates/pi-coding-agent-core`
- Port the minimal coding-agent message conversion layer from `packages/coding-agent/src/core/messages.ts`
- Then wire the default coding tools (`read`, `bash`, `edit`, `write`) into the runtime through `pi-agent::AgentTool`
- Keep it non-interactive and session-manager-free for one more milestone

Before editing, ground in:
- `packages/coding-agent/src/core/messages.ts`
- `packages/coding-agent/src/core/sdk.ts`
- `packages/coding-agent/src/core/agent-session-services.ts`
- `rust/crates/pi-coding-agent-core/src/runtime.rs`
- `rust/crates/pi-agent/src/agent.rs`
- `rust/crates/pi-agent/src/tool.rs`
- `migration/packages/coding-agent.md`

Also note:
- Ignore unrelated existing worktree changes in:
  - `rust/crates/pi-agent/src/loop.rs`
  - `rust/crates/pi-agent/src/tool.rs`
  - `rust/crates/pi-agent/tests/agent_loop.rs`
- Ignore unrelated untracked noise:
  - `.codex`
  - `rust/target/`
