# packages/tui milestone 27: quoted `@"..."` attachment continuation in the live prompt

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-tui`
- downstream validation in `rust/crates/pi-coding-agent-cli`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/tui/src/autocomplete.ts`
- targeted autocomplete trigger/update ranges in `packages/tui/src/components/editor.ts`
- quoted attachment coverage in `packages/tui/test/autocomplete.test.ts`
- targeted attachment-autocomplete ranges in `packages/tui/test/editor.test.ts`
- `migration/packages/tui-milestone-26.md`
- `migration/packages/coding-agent-milestone-65.md`
- `migration/notes/coding-agent-next-prompt.md`

Rust files read before or during implementation:
- `rust/crates/pi-tui/src/autocomplete.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/autocomplete.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- targeted interactive setup in `rust/crates/pi-coding-agent-cli/src/runner.rs`

## 2. Behavior inventory summary

Grounded TypeScript behavior for this slice:
- `CombinedAutocompleteProvider` already supports quoted attachment prefixes such as `@"my folder/` and quoted completion application without duplicating the closing quote
- the editor/live prompt path should keep updating attachment suggestions while the user continues editing a quoted attachment token
- the remaining parity risk was specifically the transition from an accepted quoted attachment-directory completion back into live prompt editing, where the Rust editor still treated attachment context too narrowly once spaces existed inside the quoted token

Rust behavior added in this milestone:
- the Rust editor now recognizes quoted attachment-editing context with the same token shape already used by the quoted attachment provider path
- after accepting a quoted directory attachment completion such as `@"my folder/"`, continuing to type deeper path text inside the quotes now reopens and updates autocomplete instead of silently falling back to plain text editing
- if a quoted attachment query temporarily produces no matches, backspacing back into a matching quoted attachment prefix now re-triggers autocomplete
- downstream interactive proof now exists in the Rust CLI runner, so the live startup prompt path is frozen for this quoted attachment continuation case instead of only the unit-level editor path

Compatibility note:
- this milestone stays narrow
- it does not port the broader TypeScript async/debounced autocomplete request stack
- it only closes the next visible quoted attachment continuation gap in the already-live Rust prompt/editor path

## 3. Rust design summary

`pi-tui::Editor` changes:
- added a focused quoted-attachment context matcher used by the existing autocomplete refresh hooks
- reused the existing character-input and delete refresh flow instead of adding a second prompt-specific path

Design choice:
- keep the fix inside `pi-tui::Editor`, because `pi-coding-agent-tui::CustomEditor` and the interactive CLI already consume that editor directly
- validate the slice end-to-end through the existing interactive runner instead of widening the shell/runtime surface

## 4. Files created/modified

Created:
- `migration/packages/tui-milestone-27.md`

Modified:
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-tui/tests/editor.rs`
  - continuing autocomplete after accepting a quoted attachment directory completion
  - backspacing back into a matching quoted attachment prefix after a no-match state
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - live interactive prompt proof that quoted attachment continuation renders updated suggestions after accepting a quoted directory completion and typing deeper path text

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-tui --test editor`
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`

Pending repo-level validation after this milestone:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next remaining editor parity gap in this same area is the broader quoted-attachment debounce/cancellation behavior from the full TypeScript async editor path, or whether the next concrete regression is now elsewhere in slash-command/model autocomplete rendering
- whether another downstream interactive prompt regression should be frozen through `pi-coding-agent-tui` component tests before broadening the runner surface further

## 8. Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-cli`:
- with quoted attachment continuation now covered in the live Rust prompt path, move to the next still-visible interactive parity gap under the same scope
- best next candidates:
  - the next unported slash-command/model autocomplete rendering mismatch in the live prompt, or
  - the next editor-side interactive regression around selection/rendering that already has a concrete TypeScript test or manual repro
