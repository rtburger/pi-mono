# packages/coding-agent milestone 61: startup-shell multiline prompt editor slice

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-coding-agent-tui`
- supporting API slice in `rust/crates/pi-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
- targeted ranges in `packages/coding-agent/src/modes/interactive/interactive-mode.ts` for:
  - default editor construction
  - autocomplete hookup
  - app action registration
  - submit handling
- `packages/tui/src/editor-component.ts`
- targeted grounding in `packages/tui/src/components/editor.ts`
- `migration/packages/coding-agent-milestone-60.md`
- `migration/packages/tui.md`

Rust files read before or during implementation:
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/interactive_binding.rs`
- `rust/crates/pi-coding-agent-tui/tests/assistant_message.rs`
- `rust/crates/pi-coding-agent-tui/tests/branch_summary.rs`
- `rust/crates/pi-coding-agent-tui/tests/compaction_summary.rs`
- `rust/crates/pi-coding-agent-tui/tests/custom_message.rs`
- `rust/crates/pi-coding-agent-tui/tests/skill_invocation.rs`
- `rust/crates/pi-coding-agent-tui/tests/tool_execution.rs`
- `rust/crates/pi-coding-agent-tui/tests/user_message.rs`

## 2. Behavior inventory summary

High-value TypeScript behavior frozen in this slice:
- the real interactive coding-agent path uses a multiline custom editor as the main prompt surface, not a one-line prompt widget
- app-level prompt editing should inherit the richer editor behavior already present in the editor implementation instead of routing all advanced prompt editing through temporary editor surfaces
- prompt edits restored from temporary editor flows must preserve cursor placement semantics when the prompt is edited programmatically

Rust behavior added in this milestone:
- `StartupShellComponent` now uses `CustomEditor` for the live prompt instead of `pi_tui::Input`
- the live shell prompt now inherits the already-migrated multiline editor behavior in the Rust interactive path, including:
  - multiline insertion through editor keybindings
  - undo/kill-ring/jump/paste-marker behavior already implemented in `pi-tui::Editor`
  - multiline rendering and transcript-height budgeting based on the real editor surface
- shell prompt editing no longer depends on the temporary extension editor for multiline-only behavior; the shell prompt itself is now editor-backed
- prompt restoration after the shell-mounted extension editor now maps the old flat cursor API onto the multiline editor cursor shape so programmatic cursor placement still works for clipboard/image insertion and restored prompt text
- shell `app.interrupt` handling now leaves room for future autocomplete cancellation semantics by delegating back into the editor when autocomplete is active

Compatibility note:
- this is still a narrowed startup-shell slice, not full TS interactive-mode parity
- the Rust startup shell now uses the real editor surface, but it still does not yet wire the full TS autocomplete/session/runtime stack into that prompt
- the shell prompt therefore gains the multiline editor behavior first, while richer main-editor runtime integrations remain later work

## 3. Rust design summary

Implementation changes:
- `pi-tui::Editor`
  - added public `set_cursor(EditorCursor)` with TS-compatible clamping behavior for downstream programmatic prompt restoration
- `pi-coding-agent-tui::CustomEditor`
  - added `set_cursor(...)` forwarding into `pi-tui::Editor`
- `pi-coding-agent-tui::StartupShellComponent`
  - replaced the `Input` field with `CustomEditor`
  - moved shell prompt getters/setters/insert-at-cursor helpers onto the multiline editor
  - added flat-offset -> `EditorCursor` mapping helper for existing shell APIs that still expose a single `usize` cursor offset
  - updated interrupt handling so shell-level escape logic no longer blocks future editor-side autocomplete cancel behavior

Design choices:
- keep the shell’s public prompt helper methods (`input_value`, `set_input_value`, `set_input_cursor`, `insert_input_text_at_cursor`) stable so clipboard-image and extension-editor slices do not need a wider API redesign yet
- land the multiline prompt consumer in `StartupShellComponent` first rather than introducing another prompt abstraction
- preserve the current shell action routing and only swap the prompt surface itself in this milestone

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-61.md`

Modified:
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/interactive_binding.rs`
- `rust/crates/pi-coding-agent-tui/tests/assistant_message.rs`
- `rust/crates/pi-coding-agent-tui/tests/branch_summary.rs`
- `rust/crates/pi-coding-agent-tui/tests/compaction_summary.rs`
- `rust/crates/pi-coding-agent-tui/tests/custom_message.rs`
- `rust/crates/pi-coding-agent-tui/tests/skill_invocation.rs`
- `rust/crates/pi-coding-agent-tui/tests/tool_execution.rs`
- `rust/crates/pi-coding-agent-tui/tests/user_message.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-tui/tests/editor.rs`
  - `public_set_cursor_clamps_to_existing_line_and_column`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
  - `startup_shell_supports_multiline_prompt_editing_via_custom_editor_bindings`

Existing shell/transcript interaction tests were updated to validate ordering and viewport behavior against the new multiline editor-backed shell prompt.

## 6. Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-tui --test editor`
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell`
- `cd rust && cargo test -p pi-coding-agent-tui --test interactive_binding`
- `cd rust && cargo test -p pi-tui && cargo test -p pi-coding-agent-tui`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the startup shell should eventually style the prompt surface closer to the TS main editor presentation instead of exposing the raw editor chrome directly
- whether the next shell step should be autocomplete/provider wiring now that the shell prompt is editor-backed
- whether more shell-level app actions should move down into `CustomEditor` registration once the prompt surface is closer to the full TS interactive-mode editor stack

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `packages/tui`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- now that the live startup shell prompt is editor-backed, move to the next direct downstream editor gap with high value:
  - wire autocomplete into the shell prompt, or
  - port the next missing custom-editor/main-editor behavior that the shell can now actually exercise end-to-end
- keep broader session-manager/resource-loader parity and the full theme/markdown/image stack deferred until the editor/runtime interaction gaps are tighter
