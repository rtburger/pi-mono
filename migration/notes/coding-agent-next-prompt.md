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
- Use `read` for file contents, not cat/sed.
- After code changes, run required validation.
- In this environment, `npm run check` is blocked because `biome` is missing; note that explicitly.
- Do not commit unless asked.

Current state after the latest coding-agent CLI/model milestone:
- `rust/crates/pi-ai` now provides the Rust-backed built-in model catalog used by `rust/apps/pi`.
- `rust/crates/pi-coding-agent-core` now has:
  - CLI model resolution parity slices
  - scoped-model resolution via `resolve_model_scope()`
  - `models.json` subset registry
  - startup bootstrap selection with saved-default-in-scope handling
  - minimal non-interactive runtime over `pi-agent`
- `rust/crates/pi-coding-agent-cli` now has:
  - `--list-models [search]`
  - `--models` scoped-model selection in the non-interactive path
  - `--api-key` override support for explicit `--model` and current first-scoped-model selection
- `rust/crates/pi-tui` now has:
  - `fuzzy_match()`
  - `fuzzy_filter()`
  - tests ported from `packages/tui/test/fuzzy.test.ts`
- Validation already completed on this milestone:
  - `cd rust && cargo fmt`
  - `cd rust && cargo test -p pi-tui`
  - `cd rust && cargo test -p pi-coding-agent-core`
  - `cd rust && cargo test -p pi-coding-agent-cli`
  - `cd rust && cargo test`
  - `npm run check` fails with `biome: command not found`

Important current gaps:
- no xhigh-capability clamping parity yet in the Rust CLI startup path
- no settings-manager/resource-loader/session-manager integration yet
- no JSON session-manager wrapper/header parity yet
- no `blockImages` runtime wrapper yet
- no image auto-resize parity yet
- no broader OAuth/cloud auth parity yet for all providers
- no interactive coding-agent/TUI integration yet beyond fuzzy helpers

Recommended next task:
- Stay in `packages/coding-agent`, `packages/ai`, `rust/crates/pi-coding-agent-core`, and `rust/crates/pi-coding-agent-cli`
- Port the remaining CLI startup model-selection parity that does not require session-manager:
  - xhigh/thinking clamp behavior against model capabilities
  - any missing availability/auth edge cases that affect initial model selection
- Keep TUI and session-manager integration deferred for now

Before editing, ground in:
- `packages/coding-agent/src/main.ts`
- `packages/coding-agent/src/core/model-resolver.ts`
- `packages/ai/src/models.ts`
- `rust/crates/pi-ai/src/models.rs`
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/src/bootstrap.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `migration/packages/coding-agent.md`
- `migration/packages/tui.md`

Also note:
- Ignore unrelated existing worktree changes outside the files touched by this migration slice.
- Ignore `rust/target/` noise.
