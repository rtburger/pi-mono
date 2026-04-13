# packages/coding-agent milestone 69: `app.model.select` interactive selector wiring

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-coding-agent-cli`
- `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/core/keybindings.ts`
- `packages/coding-agent/src/modes/interactive/components/model-selector.ts`
- `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts`
- targeted model-action and selector ranges in `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `migration/packages/coding-agent.md`
- `migration/packages/coding-agent-milestone-68.md`
- `migration/notes/rust-workspace-status.md`

Rust files read before or during implementation:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/model_selector.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

## 2. Behavior inventory summary

Grounded TypeScript behavior for this slice:
- `app.model.select` is a real app action with a default `ctrl+l` binding
- interactive mode wires that action directly to `showModelSelector()`
- that keybinding opens the same selector component used by the `/model` command path
- selector display hides the prompt while preserving prompt buffer state underneath
- model selection updates the active runtime model and footer state; cancel restores the prompt with no model change

Rust behavior added in this milestone:
- the Rust interactive CLI now wires `app.model.select` to the same `show_interactive_model_selector(...)` path already used by the `/model` slash-command fallback
- pressing the configured keybinding opens the selector even when no `/model` command was typed
- selector completion still runs through the existing model-switch path, so footer/status updates and thinking-level clamping stay consistent with the slash-command path
- if a request is currently streaming, the keybinding now surfaces the same status message already used by the `/model` command path instead of opening a conflicting selector

Compatibility note:
- this milestone closes the advertised keybinding entry point only
- broader selector parity such as TS all/scoped toggles and persisted default-scope editing remains deferred to the separate scoped-model selector flow

## 3. Rust design summary

`pi-coding-agent-tui` changes:
- added a shell-aware registered action path via `StartupShellComponent::on_action_with_shell(...)`
- kept the existing `on_action(...)` API as a wrapper, so existing action-handler callsites did not need to change
- internally, registered action invocation now temporarily removes and reinserts the handler so a callback can safely mutate the shell, including opening modal prompt-side UI like the model selector

`pi-coding-agent-cli` changes:
- `install_interactive_submit_handler(...)` now also registers `app.model.select`
- that handler reuses the existing selector helper and existing model-switch callback path instead of creating a second model-selection implementation

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-69.md`

Modified:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
  - shell-aware registered action handlers can open the model selector and preserve the hidden prompt buffer
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - end-to-end interactive proof that the default `app.model.select` keybinding opens the selector, allows selecting a different model, and uses that model for the next prompt

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell -p pi-coding-agent-cli --test runner`
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next interactive model-control parity slice should wire `app.model.cycleForward` / `app.model.cycleBackward`, since those keybindings are still advertised in the startup header
- whether the next narrower regression freeze in this same area should be the live prompt `/model` autocomplete-acceptance rendering transition end-to-end before broadening selector behavior again
- whether `app.model.select` should eventually open a richer TS-style selector surface with scope toggling rather than the intentionally narrowed current-candidate selector

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-cli`, and `rust/crates/pi-coding-agent-tui`:
- next highest-value parity gap in the same area is the remaining advertised model-control bindings, especially `app.model.cycleForward` / `app.model.cycleBackward`
- if you want to stay narrower before adding more selector behavior, freeze the live prompt `/model` autocomplete-acceptance rendering transition end-to-end in the Rust interactive path
