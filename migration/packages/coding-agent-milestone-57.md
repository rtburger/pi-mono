# packages/coding-agent milestone 57: `CustomEditor` wrapper slice

Status: completed
Target crates: `rust/crates/pi-coding-agent-tui`, `rust/crates/pi-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (editor construction, handler wiring, custom-editor swapping ranges)
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`

Rust files read before implementation:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`

## 2. Behavior inventory summary

High-value TypeScript behavior frozen in this slice:
- `CustomEditor` is a thin wrapper over the shared multiline `Editor`
- extension shortcuts run before app-level keybindings and normal editor input
- `app.clipboard.pasteImage` is intercepted before editor input and does not mutate editor text directly
- `app.interrupt` prefers a dynamic escape handler, then a registered app action, and otherwise falls through to normal editor handling
- `app.exit` is special-cased only when the editor is empty; otherwise the same key should still reach normal editor delete-forward behavior
- other app-level actions run before editor text handling

## 3. Rust design summary

Implemented a new Rust component:
- `pi_coding_agent_tui::CustomEditor`

Design choices:
- reuse the existing migrated `pi_tui::Editor` directly instead of forking editor logic into coding-agent
- keep the first slice focused on input-routing parity only; no startup-shell integration yet
- expose only the editor surface currently needed by downstream coding-agent wiring:
  - text get/set/insert
  - history
  - padding
  - submit/change callbacks
  - app-action registration
  - dynamic escape/ctrl-d/paste-image/extension-shortcut hooks
- keep external-editor flow deferred to a later component/runtime slice; the generic `on_action("app.editor.external", ...)` path is enough for this wrapper milestone

## 4. Files created/modified

Created:
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/custom_editor.rs`
- `migration/packages/coding-agent-milestone-57.md`

Modified:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`

## 5. Tests added

New Rust regression file:
- `rust/crates/pi-coding-agent-tui/tests/custom_editor.rs`

Coverage added for:
- extension shortcut consume/fall-through behavior
- paste-image binding interception
- interrupt binding preferring the dynamic escape callback
- empty-editor exit handling with non-empty delete-forward fall-through
- generic app-action interception before editor text handling

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-coding-agent-tui --test custom_editor`
- `cd rust && cargo test -p pi-coding-agent-tui`
- `cd rust && cargo test -q --workspace`
- `cd rust && cargo fmt --all`
- `npm run check`

## 7. Open questions

- whether the next consumer should be the startup-shell input path or a dedicated Rust `ExtensionEditorComponent`
- whether the Rust interactive runtime should switch from `StartupShellComponent`'s current `Input` widget to `CustomEditor` directly, or keep the startup shell simple and introduce a separate richer editor surface first
- how much of the TypeScript custom-editor surface beyond this routing slice is actually needed before session/resource-loader-backed interactive parity work

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- use the new Rust `CustomEditor` in the next real consumer instead of leaving it isolated
- the smallest honest follow-up is likely either:
  - a Rust `ExtensionEditorComponent` wrapper using `CustomEditor`/`Editor`, or
  - replacing the startup-shell `Input` widget with `CustomEditor` where multiline editing is now the limiting gap
