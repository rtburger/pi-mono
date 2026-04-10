# packages/coding-agent milestone 51: startup-shell status-line slice

Target crates: `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
  - header/startup layout section around `statusContainer`
  - `showStatus(...)` section for status text behavior/context
- `packages/coding-agent/src/modes/interactive/theme/theme.ts`
  - `scrollInfo` / muted text styling surface for nearby interactive text patterns
- existing earlier-grounded interactive keybinding/editor files remained relevant:
  - `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
  - `packages/coding-agent/src/core/keybindings.ts`

Rust files read in full before editing:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/transcript.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/tui.rs`

## 2. Behavior inventory summary

Existing Rust startup-shell behavior already covered before this slice:
- built-in startup header
- transcript rendering + page scrolling
- pending message strip
- prompt input routing
- footer binding + render-handle rerender bridge
- app-level keybinding/action routing
- paste-image and extension-shortcut interception

New TS-grounded behavior added in this slice:
- the Rust startup shell now has a first explicit status-line slot corresponding to the TypeScript `statusContainer` placement in interactive mode
- the status line renders between pending messages and the prompt, preserving the current TS container ordering shape
- transcript viewport budgeting now reserves height for the status line when present, so status text does not push the prompt off-screen or overlap transcript lines
- status text is single-line and width-bounded for the current migration slice
- status text can be cleared without disturbing transcript/pending/footer state

Intentional narrowness of this slice:
- this is the smallest useful status-container port, not the full TS loader/status system
- no spinner/loader animation parity yet
- no theme-aware dim styling parity yet
- no live callback/render-handle binding for status updates yet

## 3. Rust design summary

Implementation stayed intentionally local to `StartupShellComponent`:
- added `status_message: Option<String>` state
- added helper `render_status(width)` using existing `pi_tui::truncate_to_width(...)`
- added public shell API:
  - `set_status_message(...)`
  - `clear_status_message()`
- transcript viewport-height calculation now subtracts rendered status-line height
- final shell render order is now:
  - header
  - transcript
  - pending messages
  - status line
  - prompt input
  - footer

Design choice:
- keep this as plain shell state instead of introducing a separate status component or reactive binding layer yet
- reuse existing text-width/truncation helpers instead of building another rendering abstraction
- defer loader styling/live updates until a later interactive-runtime slice actually needs them

## 4. Files created/modified

Modified:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

Created:
- `migration/packages/coding-agent-milestone-51.md`

## 5. Tests added

Added Rust coverage in `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs` for:
- status-line ordering between pending messages and prompt
- transcript viewport budgeting when a status line is present
- width truncation and clearing behavior for status text

## 6. Validation results

Commands run:
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell`
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-coding-agent-tui`
- `cd rust && cargo test -q --workspace`
- `npm run check`

Results:
- all listed Rust tests passed
- workspace Rust test suite passed
- `npm run check` passed in the current environment

## 7. Open questions

- whether the next status-related slice should port a plain live status callback/binding path first, or jump directly to loader/spinner behavior from TS `statusContainer`
- how much of the eventual status styling should live in `pi-coding-agent-tui` versus a future Rust interactive theme layer
- whether transcript scroll-status/indicator behavior should be added before loader parity, or kept deferred until there is more complete shell/runtime wiring

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- add the next smallest status/live-update slice on top of this new shell status slot
- best next candidate: a render-handle-backed live status callback or a minimal loader/status widget wired into the same shell position
- keep multiline editor parity, full clipboard-image runtime behavior, and broader session/runtime wiring deferred until a few more shell-level interactive paths are frozen
