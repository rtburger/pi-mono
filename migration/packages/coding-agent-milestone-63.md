# packages/coding-agent milestone 63: runtime `autocompleteMaxVisible` prompt wiring

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-config`
- `rust/crates/pi-coding-agent-cli`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/core/settings-manager.ts`
- targeted `packages/coding-agent/src/modes/interactive/interactive-mode.ts` ranges covering editor construction, runtime settings application, and settings-selector state
- targeted `packages/tui/src/components/editor.ts` ranges covering `autocompleteMaxVisible` defaults/clamping
- `migration/packages/coding-agent-milestone-62.md`

Rust files read before or during implementation:
- `rust/crates/pi-config/src/lib.rs`
- `rust/crates/pi-config/tests/settings.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 2. Behavior inventory summary

TypeScript behavior frozen for this slice:
- `autocompleteMaxVisible` is a top-level settings value loaded from `settings.json`
- the interactive prompt/editor applies that setting at construction time and again when runtime settings are refreshed
- effective editor behavior clamps the value to the editor-supported range `3..=20`
- project settings override global settings

Rust behavior added in this milestone:
- `pi-config::load_runtime_settings()` now loads top-level `autocompleteMaxVisible`
- Rust runtime settings now default that value to `5`, matching the TypeScript default editor behavior
- loaded values are clamped to `3..=20` before reaching the prompt runtime
- project `.pi/settings.json` continues to override global `<agentDir>/settings.json`
- the interactive Rust runner now applies the loaded value to `StartupShellComponent`, so the live prompt dropdown uses the configured maximum visible item count instead of the hard-coded editor default

Compatibility note:
- this milestone wires the config/runtime path only
- it does not add new slash-command/model sources or a full Rust settings-manager/editor-refresh layer
- it keeps the existing prompt dropdown rendering and just makes the visible-item cap configurable through the migrated runtime settings path

## 3. Rust design summary

`pi-config` changes:
- `RuntimeSettings`
  - new `autocomplete_max_visible: usize`
  - explicit `Default` impl preserving the TypeScript default of `5`
- `RawSettings`
  - new top-level `autocomplete_max_visible: Option<f64>` parsed from `autocompleteMaxVisible`
- `apply_settings_file(...)`
  - clamps finite values with the same effective editor range used by TypeScript (`3..=20`)

`pi-coding-agent-cli` changes:
- `run_interactive_command_with_terminal(...)`
  - now calls `shell.set_autocomplete_max_visible(runtime_settings.settings.autocomplete_max_visible)` before installing the live autocomplete provider

Design choice for this slice:
- keep the implementation narrow and honest by reusing the existing startup-shell/editor API rather than inventing a broader settings abstraction for one prompt option

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-63.md`

Modified:
- `rust/crates/pi-config/src/lib.rs`
- `rust/crates/pi-config/tests/settings.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-config/tests/settings.rs`
  - default `autocomplete_max_visible`
  - project override + clamp behavior for `autocompleteMaxVisible`
  - invalid JSON fallback preserving the default value
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - interactive runtime proof that `settings.json` with `autocompleteMaxVisible: 3` limits the live prompt autocomplete dropdown to three visible file suggestions

## 6. Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-config --test settings`
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`
- `cd rust && cargo test -q --workspace`

Pending repo-level validation after code changes:
- `npm run check`

## 7. Open questions

- whether the next interactive autocomplete slice should now populate Rust slash-command/model sources, or wait for broader command-routing parity first
- whether `editorPaddingX` should follow the same narrow config/runtime path next, since it is another prompt-editor setting already used by the TypeScript interactive mode
- whether any additional dropdown presentation polish is worth porting before richer command/model completion sources exist

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `packages/tui`, `rust/crates/pi-config`, `rust/crates/pi-coding-agent-cli`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- keep using the live Rust prompt as the integration surface
- next best step is still one of:
  - wire slash-command/model autocomplete into the Rust prompt once the interactive command surface is broader, or
  - pull the next small prompt-editor setting through the same runtime path if more config parity is needed first
