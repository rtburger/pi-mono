# packages/coding-agent milestone 53: clipboard-image shell runtime slice

Target crates: `rust/crates/pi-coding-agent-tui`, `rust/crates/pi-tui`

## 1. Files analyzed

TypeScript grounding read for this slice:
- `packages/coding-agent/src/utils/clipboard-image.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (clipboard-image paste handler)
- `packages/coding-agent/test/clipboard-image.test.ts`
- `packages/coding-agent/test/clipboard-image-bmp-conversion.test.ts`
- previously relevant interactive routing grounding kept in scope:
  - `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
  - `packages/coding-agent/src/core/keybindings.ts`

Rust files read in full before editing:
- `rust/crates/pi-tui/src/input.rs`
- `rust/crates/pi-tui/tests/input.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

## 2. Behavior inventory summary

TypeScript behavior grounded for this slice:
- interactive paste-image handling is shell/editor-level behavior, not a model/provider behavior
- the current TS runtime path is:
  - keybinding dispatch
  - read clipboard image
  - write image bytes to a temp file with mime-derived extension
  - insert that file path into the editor at the cursor
  - silently ignore clipboard errors
- TS clipboard image detection currently covers Linux command-based paths first (`wl-paste`, `xclip`), with special WSL PowerShell fallback, and converts unsupported clipboard image formats like BMP to PNG

New Rust behavior now covered in this slice:
- `pi-coding-agent-tui` now has a first clipboard-image runtime helper path aligned with the current TS interactive paste flow
- the Rust shell can now insert arbitrary text directly at the current prompt cursor, rather than only replacing the whole prompt value
- Rust clipboard runtime helper now supports:
  - reading command-based Linux clipboard image data through a pluggable source abstraction
  - Wayland `wl-paste` type discovery + image fetch
  - `xclip` fallback
  - WSL-style PowerShell PNG file fallback
  - unsupported image conversion to PNG for the migrated BMP case
  - writing the clipboard image to a temp file with mime-derived extension
  - inserting the resulting file path into the shell prompt at the current cursor position
  - returning `None` when the clipboard has no supported image, without mutating prompt state

Intentional limitation of this slice:
- this is the shell/runtime helper plus command-based clipboard-reader slice, not the full top-level interactive wiring yet
- there is still no Rust native clipboard provider parity for non-command paths outside the current migrated Linux/WSL flow
- the TS silent-error behavior is represented by a fallible helper API that callers can choose to ignore; the top-level interactive runtime still needs to wire that policy in later

## 3. Rust design summary

New `pi-coding-agent-tui::clipboard_image` module:
- `ClipboardImage`
- `ClipboardImageSource`
- `ClipboardPlatform`
- `CommandOutput`
- `ClipboardCommandRunner`
- `StdClipboardCommandRunner`
- `SystemClipboardImageSource`
- `is_wayland_session(...)`
- `extension_for_image_mime_type(...)`
- `paste_clipboard_image_into_shell(...)`

Supporting shell/input changes:
- `pi-tui::Input`
  - `insert_text_at_cursor(...)` is now public
- `StartupShellComponent`
  - `insert_input_text_at_cursor(...)`
  - `set_input_cursor(...)`

Design choices:
- keep clipboard image reading and temp-file insertion in `pi-coding-agent-tui`, where the interactive shell behavior already lives
- keep OS command execution injectable through `ClipboardCommandRunner` so tests can stay deterministic and no real clipboard access is needed in CI
- keep shell insertion narrow and cursor-based instead of broadening into a larger editor refactor in the same milestone

## 4. Files created/modified

Modified:
- `rust/crates/pi-tui/src/input.rs`
- `rust/crates/pi-tui/tests/input.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`

Created:
- `rust/crates/pi-coding-agent-tui/src/clipboard_image.rs`
- `rust/crates/pi-coding-agent-tui/tests/clipboard_image.rs`
- `migration/packages/coding-agent-milestone-53.md`

## 5. Tests added

Added Rust coverage for:
- public prompt insertion at the current cursor in `pi-tui::Input`
- clipboard mime-extension mapping
- Wayland detection shape
- `wl-paste` selection + fetch behavior
- `xclip` fallback behavior
- unsupported BMP clipboard image conversion to PNG
- Termux no-op behavior
- shell runtime helper writing a temp file and inserting its path at the current prompt cursor
- no-op behavior when clipboard image source returns no image

## 6. Validation results

Commands run:
- `cd rust && cargo test -p pi-coding-agent-tui --test clipboard_image`
- `cd rust && cargo test -p pi-tui --test input`
- `cd rust && cargo fmt --all`
- `cd rust && cargo test -p pi-coding-agent-tui`
- `cd rust && cargo test -q --workspace`
- `npm run check`

Results:
- all listed Rust tests passed
- workspace Rust test suite passed
- `npm run check` passed in the current environment

## 7. Open questions

- whether the next clipboard-related slice should add a real native clipboard backend for non-command paths, or wait until the broader interactive runtime wiring lands
- whether the eventual top-level interactive runtime should swallow clipboard errors exactly like TS or surface them through the new shell status slot in some cases
- whether prompt mutation should stay as direct shell/input methods or move to a dedicated prompt handle once broader interactive runtime ownership is ported

## 8. Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- wire the new clipboard-image runtime helper into the first real Rust interactive controller/top-level command path when that path lands
- after that, revisit native clipboard parity and broader interactive runtime ownership
- keep multiline editor parity and session-manager/resource-loader interactive wiring deferred until the top-level Rust interactive path exists
