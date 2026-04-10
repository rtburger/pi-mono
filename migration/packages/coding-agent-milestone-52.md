# packages/coding-agent milestone 52: startup-shell live status-handle slice

Target crates: `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
  - startup layout around `statusContainer`
  - `showStatus(...)` update path
  - nearby `requestRender()` usage for visible status changes
- `packages/coding-agent/src/modes/interactive/theme/theme.ts`
  - nearby muted status/scroll text styling surface
- previously relevant interactive grounding kept in scope:
  - `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
  - `packages/coding-agent/src/core/keybindings.ts`

Rust files read in full before editing:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/tui.rs`

## 2. Behavior inventory summary

Before this slice, Rust already had:
- a shell-level status line slot in `StartupShellComponent`
- transcript viewport budgeting for that status line
- queued rerender support in `pi-tui` via `RenderHandle`
- a live footer rerender bridge using that queued render path

New TS-grounded behavior added in this slice:
- the Rust startup shell now has a live status-update handle analogous to the existing interactive TS pattern where status content can change independently of prompt ownership
- status updates no longer require direct mutable access to the shell after it has been moved into the TUI/component tree
- shell status updates can now optionally queue rerenders through the existing `RenderHandle`, matching the broader Rust queued-event architecture already used for footer updates
- status clear behavior also supports the same queued rerender path

Compatibility note for this slice:
- this ports the shell ownership/update shape, not the full TS loader/status subsystem yet
- updates still become visible through Rust’s queued-event drain path, not an immediate runtime event loop

## 3. Rust design summary

New public shell-facing type in `pi-coding-agent-tui`:
- `StatusHandle`
  - `set_message(...)`
  - `clear()`

Implementation changes in `StartupShellComponent`:
- status storage moved from plain `Option<String>` to shared `Arc<Mutex<Option<String>>>`
- existing shell methods remain available:
  - `set_status_message(...)`
  - `clear_status_message()`
- new handle constructors:
  - `status_handle()`
  - `status_handle_with_render_handle(render_handle)`

Design choices:
- reuse the already-ported `pi_tui::RenderHandle` instead of inventing another callback queue
- keep status rendering as the existing plain string-based shell slot from milestone 51
- avoid introducing a new generic status widget or loader abstraction until a later interactive-runtime slice actually needs one

## 4. Files created/modified

Modified:
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

Created:
- `migration/packages/coding-agent-milestone-52.md`

## 5. Tests added

Added Rust coverage in `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs` for:
- updating shell status through a handle after the shell has already been moved into the TUI tree
- queued rerender behavior for status set/clear via `RenderHandle`

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

- whether the next status-related slice should add a minimal live loader/spinner on top of `StatusHandle`, or first add a richer shell-level runtime status callback path
- whether status styling should stay plain until a Rust interactive theme lands, or gain a small local muted/accent styling contract before that
- whether shell status and footer live-update APIs should eventually share a more generic reactive binding helper, or remain separate until more interactive runtime wiring exists

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- build the next smallest live status/runtime slice on top of `StatusHandle`
- best next candidate: a minimal shell-level loader/status component that can update through the new handle and queue redraws through `RenderHandle`
- keep multiline editor parity, full clipboard-image runtime behavior, and broader session/runtime wiring deferred until a few more shell-level interactive paths are frozen
