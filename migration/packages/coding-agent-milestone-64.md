# packages/coding-agent milestone 64: runtime `editorPaddingX` prompt wiring

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-config`
- `rust/crates/pi-coding-agent-cli`
- `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/core/settings-manager.ts`
- targeted `packages/coding-agent/src/modes/interactive/interactive-mode.ts` ranges covering prompt editor construction and runtime settings refresh
- targeted `packages/coding-agent/src/modes/interactive/components/settings-selector.ts` ranges covering the `editor-padding` setting
- `migration/packages/coding-agent-milestone-63.md`
- `migration/notes/coding-agent-next-prompt.md`

Rust files read before or during implementation:
- `rust/crates/pi-config/src/lib.rs`
- `rust/crates/pi-config/tests/settings.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- targeted `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- targeted `rust/crates/pi-tui/src/editor.rs`

## 2. Behavior inventory summary

TypeScript behavior frozen for this slice:
- `editorPaddingX` is a top-level settings value loaded from `settings.json`
- the interactive coding-agent prompt applies that value when constructing the default editor
- the same setting is re-applied when runtime settings refresh
- effective values are clamped to `0..=3`
- project settings override global settings

Rust behavior added in this milestone:
- `pi-config::load_runtime_settings()` now loads top-level `editorPaddingX`
- Rust runtime settings now default that value to `0`, matching the TypeScript default prompt editor padding
- loaded values are clamped to `0..=3` before reaching the prompt runtime
- project `.pi/settings.json` continues to override global `<agentDir>/settings.json`
- the interactive Rust runner now applies the loaded value to `StartupShellComponent`, so the live prompt editor uses configured horizontal padding instead of the hard-coded zero-padding default

Compatibility note:
- this milestone only ports the config/runtime path for the prompt editor
- it does not add a Rust interactive settings selector yet
- it keeps the existing shell/editor rendering and just makes the input padding configurable through the migrated runtime settings path

## 3. Rust design summary

`pi-config` changes:
- `RuntimeSettings`
  - new `editor_padding_x: usize`
  - explicit default of `0`
- `RawSettings`
  - new top-level `editor_padding_x: Option<f64>` parsed from `editorPaddingX`
- `apply_settings_file(...)`
  - clamps finite values with the same effective range used by TypeScript (`0..=3`)

`pi-coding-agent-tui` changes:
- `StartupShellComponent`
  - new `set_input_padding_x(...)` passthrough into the wrapped multiline editor

`pi-coding-agent-cli` changes:
- `run_interactive_command_with_terminal(...)`
  - now calls `shell.set_input_padding_x(runtime_settings.settings.editor_padding_x)` before starting the live shell

Design choice for this slice:
- keep the implementation narrow and reuse the existing startup-shell/editor boundary instead of introducing a broader Rust settings manager for a single prompt-editor option

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-64.md`

Modified:
- `rust/crates/pi-config/src/lib.rs`
- `rust/crates/pi-config/tests/settings.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-config/tests/settings.rs`
  - default `editor_padding_x`
  - project override + clamp behavior for `editorPaddingX`
  - invalid JSON fallback preserving the default value
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - interactive runtime proof that `settings.json` with `editorPaddingX: 3` reaches the live prompt and renders padded editor content

## 6. Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-config --test settings`
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next prompt/runtime parity slice should now move from prompt layout settings to interactive command/model autocomplete sources
- whether a Rust settings selector is worth starting before more of the interactive command surface exists
- whether additional prompt-editor presentation settings should wait until there is a single shared runtime refresh path instead of one-off runner wiring

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `packages/tui`, `rust/crates/pi-config`, `rust/crates/pi-coding-agent-cli`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- keep the prompt/runtime scope narrow
- next best step is now to wire actual slash-command/model autocomplete sources into the Rust interactive prompt once the command surface is ready
- if config parity is still the priority, only pull another setting through the same runtime path when it already exists in the Rust editor/shell and can be validated as a similarly small vertical slice
