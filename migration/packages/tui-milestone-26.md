# packages/tui milestone 26: slash-command autocomplete trigger + submit-flow parity

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-tui`
- downstream validation in `rust/crates/pi-coding-agent-cli`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/tui/src/components/editor.ts`
- targeted slash-command autocomplete regressions from `packages/tui/test/editor.test.ts`
- `packages/tui/src/autocomplete.ts`
- downstream interactive usage already in scope from the current migration record:
  - `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
  - `packages/coding-agent/src/core/slash-commands.ts`

Rust files read for this slice:
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-tui/src/autocomplete.rs`
- `rust/crates/pi-tui/src/input.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 2. Behavior inventory summary

New TS-compatible behavior now covered in Rust:
- the multiline Rust `Editor` now auto-triggers slash-command autocomplete while typing instead of requiring an explicit tab-first path
- the migrated auto-trigger slice now follows the current TypeScript editor behavior for slash-command contexts:
  - typing `/` at the start of the first line opens slash-command suggestions
  - typing `[A-Za-z0-9._-]` while in a slash-command context re-triggers suggestions even when the menu is not already visible
  - backspace and forward delete now re-trigger slash-command suggestions when the current text still stays in slash-command context
- command-name vs command-argument enter behavior now matches the current TypeScript split more closely:
  - when the autocomplete prefix starts with `/`, pressing enter accepts the selected slash-command completion and falls through to the normal submit path
  - when the autocomplete prefix is only the command argument text, pressing enter accepts the selected/exact argument completion but does not submit the editor
- this closes the main gap behind the previously narrower Rust interactive prompt behavior where slash-command sources were wired but command-name / command-argument trigger semantics still trailed the TypeScript editor

Downstream interactive effect frozen in Rust tests:
- `/quit` still works as a one-enter interactive command because slash-command-name completion now falls through to submit
- `/model` interactive switching can now be exercised through the prompt autocomplete path by accepting a partial model-id completion first and then submitting the completed slash command

## 3. Rust design summary

Expanded `pi-tui::Editor` with:
- `is_at_start_of_message()` for TS-style slash-menu gating on the first line
- `refresh_autocomplete_after_character_input(...)` for slash-command auto-trigger/update behavior after printable input
- `refresh_autocomplete_after_delete()` for slash-command re-trigger behavior after backspace/forward delete
- `is_slash_autocomplete_char(...)` helper mirroring the current TS trigger character class for slash-command typing
- `accept_autocomplete_selection()` now returns whether the accepted completion should fall through to submit, which is true only for slash-command-name prefixes beginning with `/`

No downstream runtime redesign was required.
The existing `CustomEditor` and interactive shell already consume `pi-tui::Editor`, so the behavior change flows through automatically once the editor parity slice lands.

## 4. Files created/modified

Created:
- `migration/packages/tui-milestone-26.md`

Modified:
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 5. Tests added

New Rust coverage added in `rust/crates/pi-tui/tests/editor.rs` for:
- auto-triggering slash-command suggestions from an initial `/`
- hiding slash-command autocomplete after backspacing the slash away
- auto-triggering command-argument autocomplete in a prefilled slash-command context
- retaining an exact typed `/model` argument on enter while autocomplete is visible
- slash-command-name enter fallthrough to submit for `/quit`

Downstream interactive regression updated in `rust/crates/pi-coding-agent-cli/tests/runner.rs`:
- `/model` switching now exercises the prompt autocomplete path with a partial model-id, enter-to-accept, then enter-to-submit flow

## 6. Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-tui --test editor`
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next autocomplete/editor slice should stay narrowly on slash-command arguments or broaden to the still-deferred `@` attachment auto-trigger behavior from the TypeScript editor
- whether the interactive Rust shell should next freeze more end-to-end slash-command prompt behavior beyond `/model` and `/quit`, or keep that work in `pi-tui::Editor` until another concrete downstream command needs it

## 8. Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-cli`:
- continue with the next editor-level autocomplete parity slice that has direct downstream value, most likely the remaining `@` attachment auto-trigger/update behavior from the TypeScript editor
- keep broader widget parity and the full markdown/select/settings/image stack deferred until another real downstream consumer requires them
