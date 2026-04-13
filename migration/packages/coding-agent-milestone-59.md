# packages/coding-agent milestone 59: startup-shell extension-editor mounting slice

Status: completed
Target crate: `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/tui/src/editor-component.ts`
- `packages/tui/src/components/editor.ts`
- `packages/tui/test/editor.test.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
- targeted `packages/coding-agent/src/modes/interactive/interactive-mode.ts` ranges for `showExtensionEditor()` / `hideExtensionEditor()` / editor-container swapping

Rust files read before implementation:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/extension_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/extension_editor.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `migration/packages/coding-agent.md`
- `migration/packages/tui.md`

## 2. Behavior inventory summary

High-value TypeScript behavior frozen in this slice:
- the interactive shell now has an editor-slot swap behavior corresponding to TS `InteractiveMode.showExtensionEditor()` / `hideExtensionEditor()`
- the default prompt remains mounted underneath the richer temporary editor instead of being discarded
- while the extension editor is visible:
  - shell rendering shows the extension editor chrome/body instead of the default prompt widget
  - shell input is routed to the extension editor instead of the hidden prompt
  - submit/cancel from the extension editor restore the default prompt after the callback fires
- when the extension editor closes, the underlying prompt text is preserved and becomes active again

Compatibility note:
- this milestone ports the shell mounting/swap behavior, not the full extension/runtime trigger path from TypeScript interactive mode
- no new `pi-tui` widget behavior was required; the already-migrated multiline editor and extension-editor component were sufficient once the shell gained an editor slot

## 3. Rust design summary

Expanded `pi-coding-agent-tui::StartupShellComponent` with:
- optional mounted `ExtensionEditorComponent`
- shell-level `show_extension_editor(...)`
- shell-level `hide_extension_editor()`
- `is_showing_extension_editor()`
- an internal extension-editor event queue so submit/cancel callbacks can safely request shell teardown and callback delivery during normal input handling
- focus and viewport propagation to the active editor surface (default prompt vs mounted extension editor)

Design choices:
- keep the existing startup-shell prompt as the default editor surface and layer the richer extension editor on top only when requested, matching the TS editor-container swap shape more closely than replacing the shell prompt entirely
- preserve the hidden prompt buffer while the extension editor is mounted so restore behavior is deterministic and TS-aligned
- keep the shell-level public prompt helpers (`set_input_value`, `clear_input`, clipboard insertion helpers) operating on the default prompt buffer even while the extension editor is temporarily mounted

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-59.md`

Modified:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

## 5. Tests added

Extended Rust regression coverage in:
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

New coverage added for:
- mounting an extension editor into the startup shell and hiding the default prompt while it is active
- extension-editor submit restoring the hidden prompt after callback delivery
- extension-editor cancel restoring the hidden prompt after callback delivery

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell`
- `cd rust && cargo test -p pi-coding-agent-tui`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next shell/runtime step should expose a real trigger path from the live interactive runtime into `show_extension_editor(...)`
- whether the shell should eventually gain a more general editor-slot API for both extension editors and extension-provided custom editors, or whether the TypeScript custom-editor path should remain separate
- whether the default shell prompt should stay intentionally narrow for now, with richer multiline editing living only in temporary mounted editors until session/runtime parity is broader

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- wire the new shell-level extension-editor mounting path into one real interactive runtime action or extension-facing hook
- if that runtime trigger is still too blocked by missing extension/session infrastructure, the next best step is the parallel shell-side custom-editor mounting path so both TS temporary editor flows have a Rust equivalent
