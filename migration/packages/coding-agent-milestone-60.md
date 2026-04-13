# packages/coding-agent milestone 60: startup-shell `app.editor.external` prompt-edit slice

Status: completed
Target crate: `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
- targeted `packages/coding-agent/src/modes/interactive/interactive-mode.ts` ranges for:
  - default-editor extension-shortcut wiring
  - `showExtensionEditor()` / `hideExtensionEditor()`
  - `openExternalEditor()` on the main interactive editor

Rust files read before implementation:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/extension_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/interactive_binding.rs`
- `rust/crates/pi-coding-agent-tui/tests/extension_editor.rs`
- `migration/packages/coding-agent.md`
- `migration/packages/coding-agent-milestone-57.md`
- `migration/packages/coding-agent-milestone-58.md`
- `migration/packages/coding-agent-milestone-59.md`

## 2. Behavior inventory summary

High-value TypeScript behavior frozen in this slice:
- the interactive coding-agent flow exposes an editor action on `app.editor.external`
- richer temporary editor flows in TS are mounted by swapping the active editor surface while preserving the hidden default editor state underneath
- app-level action handlers should still be able to override built-in behavior

Rust behavior added in this milestone:
- the startup shell now gives `app.editor.external` a real built-in behavior instead of leaving the keybinding inert in the live interactive path
- pressing the external-editor action on the shell prompt now mounts the already-migrated `ExtensionEditorComponent` with the current prompt text as prefill
- submitting that temporary editor restores the hidden prompt and replaces its buffer with the edited text
- cancel restores the hidden prompt unchanged
- registered `app.editor.external` handlers still override the built-in shell behavior

Compatibility note:
- this is an honest migration step, not full TS main-editor parity
- TypeScript opens the configured system editor directly from the main multiline editor; the current Rust shell still uses the narrower single-line prompt widget
- Rust therefore routes the shell action through the already-migrated extension editor as the first real multiline prompt-edit path, while keeping override hooks intact

## 3. Rust design summary

Expanded `pi_coding_agent_tui::StartupShellComponent` with:
- built-in prompt-edit routing for `app.editor.external`
- internal prompt-restore tracking so extension-editor submit can write back into the hidden shell prompt buffer
- a dedicated built-in title for the temporary prompt editor (`Edit message`)

Design choices:
- keep the change local to `StartupShellComponent` instead of widening `InteractiveCoreBinding` or the CLI runner
- reuse the existing shell-level extension-editor mounting path rather than introducing another temporary editor abstraction
- preserve action-handler precedence so future runtime hooks can still replace the built-in behavior

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-60.md`

Modified:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/interactive_binding.rs`

## 5. Tests added

Extended Rust regression coverage in:
- `rust/crates/pi-coding-agent-tui/tests/interactive_binding.rs`

New coverage added for:
- using `app.editor.external` in the live shell to mount the extension editor, edit prompt text, restore the prompt, and then submit the edited message through the bound runtime
- verifying that a registered `app.editor.external` action handler still overrides the new built-in shell behavior

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-coding-agent-tui --test interactive_binding`
- `cd rust && cargo test -p pi-coding-agent-tui`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the shell should eventually stop routing prompt editing through a temporary extension editor once the main Rust prompt becomes multiline/custom-editor-backed
- whether the mounted shell extension editor should gain a real external-editor host bridge in the live interactive runtime so its own `app.editor.external` path can safely stop/start the TUI
- how multiline prompt content should be represented once the current single-line shell prompt is no longer the limiting widget

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `packages/tui`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- move the live shell prompt itself toward the richer editor path so prompt editing no longer has to round-trip through a temporary mounted editor
- the next honest slice is either:
  - replacing the shell prompt widget with `CustomEditor`/`Editor`, or
  - wiring a real external-editor host bridge into the mounted shell extension editor before deeper prompt-editor parity work
