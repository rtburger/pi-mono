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
- Do not run runtime apps / agent / tui processes manually.
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
- Do not commit unless asked.

Current state after the latest `pi-tui` multiline editor milestone:
- `rust/crates/pi-ai` already provides the in-scope Rust provider/runtime slices for:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- `rust/crates/pi-agent` already provides:
  - the core execution loop
  - tool execution/hooks
  - queue handling
  - proxy slices
- `rust/crates/pi-coding-agent-core` / `cli` / `tui` already provide:
  - non-interactive runtime + CLI
  - live interactive startup shell path
  - startup/runtime keybinding migration helpers
- `rust/crates/pi-tui` now already has:
  - terminal/input plumbing
  - key parsing/keybindings
  - `Text`, `Spacer`, `TruncatedText`, `Input`
  - overlays/focus/input routing
  - real `ProcessTerminal`
  - resize polling
  - a first multiline `Editor`
  - `word_wrap_line(...)`
- Validation now succeeds with:
  - `cd rust && cargo fmt --all`
  - `cd rust && cargo test -q --workspace`
  - `npm run check`

Important current gaps:
- the new Rust `Editor` is still a narrowed slice, not full TS editor parity
- still missing from Rust editor: autocomplete, undo, kill ring, paste markers, jump mode, richer sticky-column behavior
- the Rust coding-agent interactive path does not yet consume the new multiline editor in a downstream component
- broader markdown/select/settings/image widget parity remains deferred
- broader session-manager/resource-loader/extensions parity remains deferred

Recommended next task:
- Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`
- Consume the new Rust `Editor` in the smallest downstream coding-agent component that genuinely needs multiline editing
- Best candidate:
  - ground in `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
  - then add the Rust-side equivalent component or the smallest shell integration that uses `pi_tui::Editor`
- Keep full main-editor parity deferred until that consumer proves which remaining TS editor behaviors are actually needed next

Before editing, ground in:
- `packages/tui/src/editor-component.ts`
- `packages/tui/src/components/editor.ts`
- `packages/tui/test/editor.test.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `migration/packages/tui.md`
- `migration/packages/coding-agent.md`

Also note:
- Ignore unrelated existing worktree changes outside the files touched by this migration slice.
- Ignore `rust/target/` noise.
