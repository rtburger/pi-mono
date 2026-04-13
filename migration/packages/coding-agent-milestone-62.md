# packages/coding-agent milestone 62: startup-shell autocomplete prompt slice

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-tui`
- `rust/crates/pi-coding-agent-tui`
- `rust/crates/pi-coding-agent-cli`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/tui/src/editor-component.ts`
- `packages/tui/src/autocomplete.ts`
- `packages/tui/src/components/editor.ts`
- `packages/tui/test/autocomplete.test.ts`
- targeted autocomplete/editor coverage in `packages/tui/test/editor.test.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
- targeted autocomplete setup in `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `packages/coding-agent/src/core/slash-commands.ts`
- `migration/notes/rust-workspace-status.md`
- `migration/notes/coding-agent-next-prompt.md`
- `migration/packages/coding-agent-milestone-61.md`
- targeted autocomplete/editor sections in `migration/packages/tui.md`

Rust files read before or during implementation:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 2. Behavior inventory summary

High-value TypeScript behavior frozen in this slice:
- the editor owns autocomplete UI state, selection, completion application, and render-time dropdown output
- `Tab` force-completion is the primary entry point for file/path completion
- active autocomplete should intercept cancel/confirm/navigation before outer shell handlers run
- the shell prompt should not submit while autocomplete is active; submit should first resolve the highlighted completion
- coding-agent’s live prompt uses the current working directory as the completion base for path/file suggestions

Rust behavior added in this milestone:
- `pi-tui::Editor` now has a real autocomplete state instead of `is_showing_autocomplete() -> false`
- the Rust editor now supports a narrowed but useful autocomplete slice:
  - provider injection through a new `AutocompleteProvider` trait
  - prompt/file completion requests on `Tab`
  - dropdown rendering below the editor chrome
  - up/down selection, escape cancel, tab/enter acceptance
  - undoable completion application
- `pi-tui::CombinedAutocompleteProvider` now ports the first Rust provider slice for:
  - direct path completion from the current working directory
  - quoted path continuation
  - `@` attachment-style recursive file lookup with `.git` exclusion
- `StartupShellComponent` now routes submit/interrupt honestly when autocomplete is visible:
  - interrupt cancels autocomplete before shell escape callbacks run
  - submit accepts the highlighted completion before shell-level message submission
- the Rust interactive CLI runner now installs a real prompt autocomplete provider for the live startup shell, grounded in the current CLI working directory

Compatibility note:
- this is still a narrowed autocomplete slice, not full TypeScript parity
- the live Rust shell now has real prompt autocomplete for file/path use cases, but it still does not port the full TypeScript async/debounced autocomplete stack or slash-command/runtime registration surface
- the CLI currently wires the prompt with path/attachment completion only; slash-command completion remains a later runtime slice because Rust interactive command handling is still narrower than TypeScript

## 3. Rust design summary

New public Rust module in `pi-tui`:
- `autocomplete`
  - `AutocompleteItem`
  - `AutocompleteSuggestions`
  - `CompletionResult`
  - `AutocompleteProvider`
  - `SlashCommand`
  - `CombinedAutocompleteProvider`
  - `apply_completion(...)`

Expanded `pi-tui::Editor` with:
- injected autocomplete provider storage
- autocomplete UI state (`Regular` / `Force`)
- max-visible setting plus dropdown rendering
- tab-triggered suggestion requests
- selection movement / cancel / accept handling
- completion application with undo snapshots
- shell-visible `is_showing_autocomplete()` based on real state

Downstream Rust integration changes:
- `pi-coding-agent-tui::CustomEditor`
  - forwards autocomplete provider + max-visible APIs into `pi-tui::Editor`
- `pi-coding-agent-tui::StartupShellComponent`
  - exposes prompt autocomplete configuration methods
  - defers submit to the prompt editor when autocomplete is showing
- `pi-coding-agent-cli::run_interactive_command_with_terminal(...)`
  - installs `CombinedAutocompleteProvider` on the live shell prompt using the interactive cwd

Design choices for this slice:
- keep the first Rust autocomplete provider synchronous and local to `pi-tui` rather than pulling in the TypeScript async/debounce machinery immediately
- wire the shell prompt with concrete file/path value now, but keep slash-command/runtime autocomplete deferred until Rust interactive command handling is broader
- preserve the shell’s prompt ownership model from milestone 61 and only adjust input routing where autocomplete state makes shell-level submit/interrupt interception incorrect

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-62.md`
- `rust/crates/pi-tui/src/autocomplete.rs`
- `rust/crates/pi-tui/tests/autocomplete.rs`

Modified:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-tui/tests/autocomplete.rs`
  - forced directory listing from cwd
  - recursive `@` lookup with `.git` exclusion
  - quoted completion without duplicate closing quotes
- `rust/crates/pi-tui/tests/editor.rs`
  - single-suggestion force-tab auto-apply + undo restoration
  - rendered autocomplete menu navigation + acceptance
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
  - interrupt canceling autocomplete before shell escape callbacks
  - submit accepting autocomplete before shell prompt submission

## 6. Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-tui --test autocomplete`
- `cd rust && cargo test -p pi-tui --test editor`
- `cd rust && cargo test -p pi-coding-agent-tui --test custom_editor`
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell`
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next Rust interactive-runtime slice should wire slash-command/model autocomplete now, or wait until Rust command handling matches more of the TypeScript interactive mode
- whether the prompt autocomplete should eventually grow the TypeScript async/debounce/abort behavior for `@` lookups once larger repos become a measured bottleneck in Rust
- whether autocomplete max-visible should be pulled through the Rust settings/config layer next, or stay at the editor default until more interactive settings parity lands

## 8. Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-coding-agent-cli`:
- use the now-live shell prompt autocomplete to close the next direct interactive gap with real downstream value
- best next candidates:
  - wire slash-command/model autocomplete into the Rust interactive prompt once the corresponding command handling exists, or
  - pull the remaining prompt-editor polish into Rust (`autocompleteMaxVisible` settings wiring, richer dropdown presentation, async cancellation/debounce if needed)
- keep broader session-manager/resource-loader/extensions parity deferred until the prompt/runtime interaction surface is tighter
