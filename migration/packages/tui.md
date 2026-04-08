# packages/tui migration inventory

Status: milestone 1 adds the first Rust `pi-tui` behavior slice needed by coding-agent: fuzzy matching/filtering parity for `--list-models` and future selector search.
Target crate: `rust/crates/pi-tui`

## 1. Files analyzed

TypeScript files read in full for the current slice:
- `packages/tui/README.md`
- `packages/tui/src/index.ts`
- `packages/tui/src/fuzzy.ts`
- `packages/tui/test/fuzzy.test.ts`

Rust files reviewed before implementation:
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`

## 2. Behavior inventory summary

Current TypeScript fuzzy behavior used by coding-agent and selectors:
- `fuzzyMatch(query, text)` lowercases both inputs and matches query characters in order, not necessarily contiguously.
- empty query matches with score `0`.
- query longer than text fails immediately.
- lower score is better.
- scoring rewards:
  - consecutive matches
  - word-boundary matches after whitespace or `- _ . / :`
- scoring penalizes:
  - gaps between matched characters
  - later match positions
- if the direct query does not match, the matcher retries one compatibility case for alphanumeric swaps:
  - `letters + digits` becomes `digits + letters`
  - `digits + letters` becomes `letters + digits`
  - swapped matches incur an additional `+5` score penalty
- `fuzzyFilter(items, query, getText)`:
  - returns all items unchanged when `query.trim()` is empty
  - splits non-empty queries on whitespace into tokens
  - requires every token to match the item text
  - sums token scores and sorts ascending by total score

Observed compatibility target from coding-agent:
- `packages/coding-agent/src/cli/list-models.ts` uses fuzzy filtering only as an inclusion step, then sorts the surviving models by provider/id for display.
- the same fuzzy helpers are also used by TUI autocomplete and selector UIs later, so preserving the scoring/filter rules now avoids diverging search behavior.

## 3. Rust target design

Minimal first slice in `pi-tui`:
- `src/fuzzy.rs`
  - `FuzzyMatch`
  - `fuzzy_match(&str, &str) -> FuzzyMatch`
  - `fuzzy_filter(&[T], &str, get_text) -> Vec<&T>`
- `src/lib.rs`
  - root re-exports for the fuzzy API
  - keep `TuiError` placeholder for the larger TUI rewrite

Design choices:
- keep the slice std-only; no regex or terminal dependencies are needed yet
- return borrowed items from `fuzzy_filter()` to keep the helper generic and allocation-light
- preserve the TS scoring model directly rather than introducing a different ranking abstraction

## 4. Validation plan / coverage

Rust tests should cover the same TS cases now used as the compatibility baseline:
- empty query
- query longer than text
- exact match scoring
- in-order matching requirement
- case insensitivity
- consecutive-match scoring preference
- word-boundary scoring preference
- swapped alphanumeric token support
- filtering + ranking behavior
- custom `get_text` callback behavior

## 5. Known gaps after this slice

Still deferred for the TUI package:
- terminal abstraction
- rendering/diffing
- widgets/components
- key handling/keybindings
- image support
- interactive coding-agent integration

This note intentionally covers only the fuzzy helper slice needed by the current coding-agent CLI milestone.
