# packages/coding-agent milestone 67: scoped-model `/model` live prompt regression coverage

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-coding-agent-cli`
- `rust/crates/pi-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/core/slash-commands.ts`
- targeted `/model` command handling in `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `packages/coding-agent/src/core/model-resolver.ts`
- `packages/coding-agent/src/modes/interactive/components/model-selector.ts`
- `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts`
- `packages/tui/src/autocomplete.ts`
- targeted `/model` autocomplete coverage in `packages/tui/test/editor.test.ts`
- targeted slash-command coverage in `packages/tui/test/autocomplete.test.ts`

Rust files read before or during implementation:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-tui/src/autocomplete.rs`
- `rust/crates/pi-tui/tests/autocomplete.rs`
- `migration/notes/rust-workspace-status.md`
- `migration/packages/coding-agent-milestone-65.md`
- `migration/packages/coding-agent-milestone-66.md`

## 2. Behavior inventory summary

TypeScript behavior frozen for this slice:
- `/model` argument suggestions are sourced from the current interactive model candidates
- when scoped models are active, those scoped candidates are the visible `/model` suggestion set
- selecting a `/model` suggestion applies the canonical `provider/model` value into the prompt while still rendering `label — description` rows in the live autocomplete UI
- the broader non-exact `/model` selector fallback still exists in TypeScript after exact-match lookup, but that separate selector path remains a later Rust gap

Rust behavior frozen in this milestone:
- the Rust live prompt now has explicit end-to-end regression coverage proving that `--models`-scoped interactive sessions only surface scoped `/model` suggestions in the rendered prompt output
- runner coverage now proves that accepting a scoped `/model` autocomplete suggestion and submitting it switches to the scoped target model, not an unscoped sibling with a similar id
- lower-level autocomplete coverage now proves that slash-command argument acceptance writes the canonical `provider/model` value into the prompt text

Compatibility note:
- this milestone intentionally freezes the scoped live-prompt path only
- it does not implement the missing non-exact `/model` selector fallback UI yet

## 3. Rust design summary

No runtime redesign was required.

This slice keeps the existing Rust `CombinedAutocompleteProvider` and interactive runner wiring, and adds focused regression coverage at two levels:
- `pi-tui` unit coverage for slash-command argument completion application
- `pi-coding-agent-cli` end-to-end interactive coverage for scoped-model autocomplete rendering plus accepted-model switching

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-67.md`

Modified:
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-tui/tests/autocomplete.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-tui/tests/autocomplete.rs`
  - slash-command argument completion applying canonical `provider/model` values into `/model ...` prompt text
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - live interactive proof that scoped sessions render only scoped `/model` suggestions in the prompt output
  - live interactive proof that accepting that scoped suggestion and submitting it switches the active model used for the next prompt

## 6. Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-tui --test autocomplete`
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next highest-value interactive `/model` gap is now clearly the missing selector fallback path for non-exact matches
- whether another rendering-only regression should be frozen first, such as `/model` acceptance rendering after completion in a broader set of prompt states
- whether the eventual Rust selector fallback should reuse prompt-side autocomplete primitives, or wait for a more explicit selector/dialog component slice

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `packages/tui`, `rust/crates/pi-coding-agent-cli`, and `rust/crates/pi-tui`:
- now that the scoped `/model` live prompt path is frozen, move to the remaining visible compatibility gap in the same feature area
- best next target: port the missing non-exact `/model` selector fallback behavior
- if you want to keep scope strictly on autocomplete/rendering for one more slice, freeze `/model` completion acceptance rendering in the live prompt before adding selector UI
