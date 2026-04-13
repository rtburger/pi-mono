# packages/coding-agent milestone 66: live prompt `/model` autocomplete rendering regression

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-coding-agent-cli`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/tui/src/autocomplete.ts`
- targeted slash-command autocomplete handling in `packages/tui/src/components/editor.ts`
- targeted interactive prompt autocomplete wiring in `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `packages/coding-agent/src/core/slash-commands.ts`
- `migration/packages/coding-agent-milestone-65.md`
- `migration/packages/tui-milestone-26.md`
- `migration/packages/tui-milestone-27.md`

Rust files read before or during implementation:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-tui/src/autocomplete.rs`
- `rust/crates/pi-tui/src/editor.rs`

## 2. Behavior inventory summary

TypeScript behavior frozen for this slice:
- prompt autocomplete should transition from slash-command suggestions to `/model` argument suggestions while the user keeps typing in the same live prompt
- the rendered `/model` suggestions show model ids as labels and providers as descriptions
- this is visible in the live prompt, not only inside provider/editor unit coverage

Rust behavior frozen in this milestone:
- the existing Rust prompt path is now covered end-to-end for the `/model <prefix>` render case
- runner coverage now proves that typing into `/model` argument context renders the filtered model suggestion row with the expected `label — description` shape in the live prompt output

Compatibility note:
- this milestone is intentionally narrow
- it does not broaden `/model` command handling beyond the already-supported exact-match switch path
- it freezes the live render path that was still missing from end-to-end regression coverage after the earlier slash-command source and quoted-attachment slices

## 3. Rust design summary

No runtime redesign was required.

This slice keeps the existing Rust prompt/editor/autocomplete wiring and adds an end-to-end runner regression around the already-ported `CombinedAutocompleteProvider` + `Editor` + startup-shell path so future prompt work cannot silently regress the `/model` autocomplete render transition.

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-66.md`

Modified:
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - live interactive prompt proof that typing `/model <prefix>` renders the matching model suggestion row with provider description in the terminal output

## 6. Validation results

Passed:
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`

Pending repo-level validation after this milestone:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next visible `/model` gap is now the unsupported selector fallback path for non-exact matches
- whether the next live prompt autocomplete slice should freeze another render transition in the same area, such as scoped-model-only suggestions or `/model` completion acceptance rendering

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `packages/tui`, `rust/crates/pi-coding-agent-cli`, and `rust/crates/pi-tui`:
- now that the live prompt `/model <prefix>` render path is frozen end-to-end, move to the next still-visible interactive command gap in the same area
- best next candidate: the `/model` selector fallback path or the next scoped-model-specific live prompt regression
