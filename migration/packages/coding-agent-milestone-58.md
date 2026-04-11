# packages/coding-agent milestone 58: `ExtensionEditorComponent` slice

Status: completed
Target crate: `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` relevant custom-editor/extension-editor wiring ranges

Rust files read before implementation:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/keybinding_hints.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/src/tui.rs`

## 2. Behavior inventory summary

High-value TypeScript behavior frozen in this slice:
- there is now a concrete downstream Rust consumer of the migrated multiline editor path
- the component shows extension-editor chrome around a multiline editor:
  - top/bottom border
  - title line
  - editor body
  - hint line for submit/newline/cancel and optional external editor
- cancel handling intercepts `tui.select.cancel` before editor input
- submit flows through the wrapped multiline editor behavior
- external-editor key handling is intercepted before editor input and does not mutate text
- configured keybinding overrides apply to cancel and external-editor actions

Compatibility note:
- the current Rust slice preserves the extension-editor component/input-routing behavior, but not the full TypeScript external-process workflow (`spawnSync`, temporary file, `tui.stop()` / `tui.start()` around the external editor)
- instead, Rust exposes an `on_external_editor` callback hook and consumes the binding even when no callback is installed, keeping the component behavior stable without inventing a process-runtime abstraction prematurely

## 3. Rust design summary

Implemented a new Rust component:
- `pi_coding_agent_tui::ExtensionEditorComponent`

Design choices:
- use the new Rust `CustomEditor` as the editor body so this slice is a real downstream consumer of the prior milestone
- keep the component self-contained instead of pulling overlay/runtime/editor-manager abstractions forward
- keep viewport handling explicit and narrow: outer chrome reserves space, inner editor gets the remaining height
- keep external-editor execution delegated via callback rather than coupling the component to a top-level `Tui`/terminal lifecycle in this milestone

## 4. Files created/modified

Created:
- `rust/crates/pi-coding-agent-tui/src/extension_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/extension_editor.rs`
- `migration/packages/coding-agent-milestone-58.md`

Modified:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`

## 5. Tests added

New Rust regression file:
- `rust/crates/pi-coding-agent-tui/tests/extension_editor.rs`

Coverage added for:
- title/prefill/hint rendering
- submit callback routing through the wrapped multiline editor
- cancel interception preserving editor text
- external-editor interception with callback
- external-editor interception without callback
- configured keybinding overrides for cancel and external editor

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-coding-agent-tui --test extension_editor`
- `cd rust && cargo test -p pi-coding-agent-tui`
- `cd rust && cargo test -q --workspace`
- `cd rust && cargo fmt --all`
- `npm run check`

## 7. Open questions

- whether the next higher-value consumer is wiring this component into the interactive runtime or replacing the startup-shell single-line input path entirely
- whether the external-editor callback should remain component-local or be lifted into a shared interactive runtime helper once a real consumer exists
- whether the startup-shell path should stay intentionally simple while richer editor flows live in separate components

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- wire one of the migrated richer editor components into the actual Rust interactive runtime
- the most honest next step is likely replacing the startup-shell single-line `Input` path with a multiline editor-backed component, or introducing an overlay/selector path that actually mounts `ExtensionEditorComponent`
