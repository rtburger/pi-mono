# packages/coding-agent milestone 68: non-exact `/model` selector fallback slice

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-coding-agent-cli`
- `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `packages/coding-agent/src/modes/interactive/components/model-selector.ts`
- `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts`
- `packages/coding-agent/src/core/slash-commands.ts`
- `packages/coding-agent/src/core/model-resolver.ts`
- targeted `/model` editor behavior in `packages/tui/test/editor.test.ts`
- `migration/packages/coding-agent-milestone-65.md`
- `migration/packages/coding-agent-milestone-66.md`
- `migration/packages/coding-agent-milestone-67.md`

Rust files read before or during implementation:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-tui/src/input.rs`
- `rust/crates/pi-tui/src/fuzzy.rs`
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/tests/model_resolver.rs`

## 2. Behavior inventory summary

TypeScript behavior frozen for this slice:
- `/model <search>` checks for an exact model reference match first
- when that exact match fails, interactive mode falls back to opening the model selector prefilled with the typed search term instead of erroring immediately
- bare `/model` opens the selector with an empty search
- selector filtering uses the interactive model candidate set rather than an unrelated global list

Rust behavior added in this milestone:
- the Rust interactive shell no longer stops at `No exact model match` for non-exact `/model` searches
- bare `/model` and non-exact `/model <search>` now open a real Rust-side selector component inside the interactive shell
- the selector is prefilled with the submitted search term, so a user can type `/model beta`, dismiss autocomplete, submit, then press Enter once more to select the filtered result
- selector choice switches the live runtime model and updates footer/status state the same way as the existing exact-match path
- live regression coverage now proves the selector is rendered and the selected model is used for the next prompt

Compatibility note:
- this ports the visible fallback behavior, but still keeps the Rust slice intentionally narrower than TypeScript
- the new Rust selector currently uses the current interactive candidate set only:
  - scoped models when `--models` is active
  - otherwise `ModelRegistry::get_available()`
- it does not yet port the full TypeScript selector extras such as all/scoped scope toggling or saved-default persistence
- `app.model.select` keybinding wiring remains a separate later gap; this milestone closes the `/model` slash-command fallback path

## 3. Rust design summary

New `pi-coding-agent-tui` slice:
- added a focused `ModelSelectorComponent` with:
  - search input
  - fuzzy filtering over model id/provider text
  - selectable rows
  - configurable navigation/confirm/cancel bindings

`StartupShellComponent` changes:
- submit callbacks can now optionally receive `&mut StartupShellComponent`, letting the CLI runner open shell-owned UI state from submit handling without introducing a larger runtime redesign
- the shell now hosts a model selector mode alongside the existing prompt and extension-editor modes
- selector events are funneled through the shell and then invoke external callbacks after the selector is hidden, matching the existing extension-editor event style
- transcript height budgeting now respects whichever prompt-side component is currently active, so the selector height is accounted for in viewport calculations

`pi-coding-agent-cli` changes:
- interactive `/model` handling now does:
  - exact-match switch when possible
  - otherwise open selector fallback with the submitted search text
- model selection from that fallback path reuses the existing runtime model-switch logic and footer/thinking clamp updates

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-68.md`
- `rust/crates/pi-coding-agent-tui/src/model_selector.rs`

Modified:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
  - showing the model selector while preserving the hidden prompt buffer
  - cancelling the selector and restoring prompt visibility/state
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - end-to-end interactive proof that a non-exact `/model` submission opens the selector, selects the filtered model, and uses it for the next prompt

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell -p pi-coding-agent-cli --test runner`
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next `/model` parity slice should now wire `app.model.select` to the same selector instead of leaving the command path as the only selector entry point
- whether the Rust selector should stay intentionally narrow around current interactive candidates, or pick up the TypeScript all/scoped toggle next
- whether another live prompt regression should still be frozen in the same area, such as exact completion acceptance rendering after a visible `/model` suggestion row is accepted and then submitted

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-cli`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- now that the non-exact `/model` fallback path exists, wire the same selector to the existing `app.model.select` action so the advertised keybinding reaches the migrated UI
- if you want to keep scope even tighter first, freeze the remaining live prompt `/model` acceptance rendering transition end-to-end before broadening selector features further
