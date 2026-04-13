# packages/coding-agent milestone 65: interactive slash-command + model autocomplete source slice

Status: completed 2026-04-13
Target crates:
- `rust/crates/pi-coding-agent-cli`
- `rust/crates/pi-coding-agent-tui`
- `rust/crates/pi-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/core/slash-commands.ts`
- targeted autocomplete setup in `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `packages/tui/src/autocomplete.ts`

Rust files read before or during implementation:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-coding-agent-tui/src/footer.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-tui/src/autocomplete.rs`
- `rust/crates/pi-tui/tests/autocomplete.rs`
- `rust/crates/pi-coding-agent-tui/tests/footer.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

## 2. Behavior inventory summary

TypeScript behavior frozen for this slice:
- interactive prompt autocomplete uses real slash-command sources, not only file/path completion
- `/model` autocomplete arguments come from the current interactive model candidates:
  - scoped models when present
  - otherwise `modelRegistry.getAvailable()`
- `/quit` is a real slash command
- `/model <search>` uses exact model matching before the full selector fallback path

Rust behavior added in this milestone:
- the live Rust interactive runner no longer installs an empty slash-command source into the prompt autocomplete provider
- Rust now wires a real supported-command subset into the prompt autocomplete path:
  - `/model`
  - `/quit`
- `/model` argument suggestions now come from the real interactive model candidates in the Rust app path:
  - scoped models from `--models` when present
  - otherwise the runtime `ModelRegistry::get_available()` set
- the Rust interactive command surface now honestly handles the same supported slash-command subset it advertises:
  - `/quit` exits the interactive shell without sending a prompt to the model
  - `/model <provider/model>` switches the active runtime model by exact match
- footer state now updates immediately after `/model` switches so the live footer matches the selected model and clamped thinking level

Compatibility note:
- this slice intentionally stays narrow
- bare `/model` still does not open the TypeScript selector UI in Rust yet; it reports the current compatibility gap instead
- Rust editor-side command-argument autocomplete triggering still remains narrower than the full TypeScript editor behavior, so the source wiring is frozen through provider tests plus the supported live command path instead of claiming full TS main-editor parity

## 3. Rust design summary

`pi-coding-agent-cli` changes:
- added interactive slash-command construction in the runner for the supported Rust command subset
- added runner-local interactive command helpers for:
  - current model candidate lookup
  - `/quit`
  - exact-match `/model <provider/model>` switching
  - thinking-level clamp reuse for model changes
- interactive shell submit handling now intercepts supported slash commands before dispatching normal prompts to `CodingAgentCore`

`pi-coding-agent-tui` changes:
- added `FooterStateHandle`
- `StartupShellComponent` now exposes footer-state handles so the interactive runner can update live footer state from command callbacks and queue rerenders through an existing `RenderHandle`

`pi-tui` coverage additions:
- `CombinedAutocompleteProvider` command-name and command-argument source behavior is now frozen through direct Rust tests for slash commands

## 4. Files created/modified

Created:
- `migration/packages/coding-agent-milestone-65.md`

Modified:
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-coding-agent-tui/src/footer.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-tui/tests/autocomplete.rs`

## 5. Tests added

New Rust coverage added for:
- `rust/crates/pi-tui/tests/autocomplete.rs`
  - slash-command name filtering
  - slash-command argument completion via a registered completer
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
  - live interactive slash-command autocomplete rendering for supported commands
  - `/quit` exiting without sending a prompt to the provider
  - `/model <provider/model>` switching the live runtime model before the next prompt

## 6. Validation results

Passed:
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-tui --test autocomplete`
- `cd rust && cargo test -p pi-coding-agent-cli --test runner`
- `cd rust && cargo test -p pi-coding-agent-tui --test footer --test startup_shell`
- `cd rust && cargo test -q --workspace`
- `npm run check`

## 7. Open questions

- whether the next interactive command slice should broaden `/model` from exact-match switching to the TypeScript selector fallback path
- whether the Rust editor should next port the missing command-argument autocomplete trigger/update behavior so `/model <prefix>` suggestions appear without relying on the narrower currently-tested explicit paths
- whether additional supported slash commands should land before a broader settings-selector or session-manager slice

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `packages/tui`, `rust/crates/pi-coding-agent-cli`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- now that the Rust interactive prompt has real supported slash-command sources and a matching minimal command surface, close the next directly visible gap in that same area:
  - either port the missing `/model` selector fallback path, or
  - port the remaining editor-side command-argument autocomplete triggering behavior so slash-command arguments behave more like the TypeScript main editor
- keep broader session-manager/resource-loader/theme/widget parity deferred until this interactive command/editor slice is tighter
