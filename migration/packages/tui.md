# packages/tui migration inventory

Status: milestone 2 adds the first Rust `pi-tui` behavior beyond fuzzy matching: a configurable keybinding registry slice aligned with TypeScript `keybindings.ts` and coding-agent keybinding extension needs.
Target crate: `rust/crates/pi-tui`

## 1. Files analyzed

TypeScript package metadata/docs read in full:
- `packages/tui/README.md`
- `packages/tui/package.json`
- `packages/tui/src/index.ts`

TypeScript source files read in full:
- `packages/tui/src/autocomplete.ts`
- `packages/tui/src/editor-component.ts`
- `packages/tui/src/fuzzy.ts`
- `packages/tui/src/keybindings.ts`
- `packages/tui/src/keys.ts`
- `packages/tui/src/kill-ring.ts`
- `packages/tui/src/stdin-buffer.ts`
- `packages/tui/src/terminal-image.ts`
- `packages/tui/src/terminal.ts`
- `packages/tui/src/tui.ts`
- `packages/tui/src/undo-stack.ts`
- `packages/tui/src/utils.ts`
- `packages/tui/src/components/box.ts`
- `packages/tui/src/components/cancellable-loader.ts`
- `packages/tui/src/components/editor.ts`
- `packages/tui/src/components/image.ts`
- `packages/tui/src/components/input.ts`
- `packages/tui/src/components/loader.ts`
- `packages/tui/src/components/markdown.ts`
- `packages/tui/src/components/select-list.ts`
- `packages/tui/src/components/settings-list.ts`
- `packages/tui/src/components/spacer.ts`
- `packages/tui/src/components/text.ts`
- `packages/tui/src/components/truncated-text.ts`

TypeScript tests/utilities read in full:
- `packages/tui/test/autocomplete.test.ts`
- `packages/tui/test/bug-regression-isimageline-startswith-bug.test.ts`
- `packages/tui/test/chat-simple.ts`
- `packages/tui/test/editor.test.ts`
- `packages/tui/test/fuzzy.test.ts`
- `packages/tui/test/image-test.ts`
- `packages/tui/test/input.test.ts`
- `packages/tui/test/keybindings.test.ts`
- `packages/tui/test/keys.test.ts`
- `packages/tui/test/key-tester.ts`
- `packages/tui/test/markdown.test.ts`
- `packages/tui/test/overlay-non-capturing.test.ts`
- `packages/tui/test/overlay-options.test.ts`
- `packages/tui/test/overlay-short-content.test.ts`
- `packages/tui/test/regression-regional-indicator-width.test.ts`
- `packages/tui/test/select-list.test.ts`
- `packages/tui/test/stdin-buffer.test.ts`
- `packages/tui/test/terminal-image.test.ts`
- `packages/tui/test/test-themes.ts`
- `packages/tui/test/truncated-text.test.ts`
- `packages/tui/test/truncate-to-width.test.ts`
- `packages/tui/test/tui-cell-size-input.test.ts`
- `packages/tui/test/tui-overlay-style-leak.test.ts`
- `packages/tui/test/tui-render.test.ts`
- `packages/tui/test/viewport-overwrite-repro.ts`
- `packages/tui/test/virtual-terminal.ts`
- `packages/tui/test/wrap-ansi.test.ts`

Related coding-agent files read for actual TUI consumption and keybinding/config coupling:
- `packages/coding-agent/src/core/keybindings.ts`
- `packages/coding-agent/src/cli/session-picker.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (startup/import surface section)
- `packages/coding-agent/src/modes/interactive/theme/theme.ts` (theme type and color-system surface)
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`

Rust files read before modification:
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/fuzzy.rs`
- `rust/crates/pi-tui/tests/fuzzy.rs`

## 2. Behavior inventory summary

Observed TypeScript package layers:
1. terminal abstraction + input buffering (`terminal.ts`, `stdin-buffer.ts`)
2. raw key decoding and keybinding resolution (`keys.ts`, `keybindings.ts`)
3. differential renderer, focus management, overlays, IME cursor placement (`tui.ts`)
4. width/ANSI/grapheme utilities (`utils.ts`)
5. editor/input/autocomplete widgets (`components/editor.ts`, `components/input.ts`, `autocomplete.ts`)
6. markdown, selection, settings, image and supporting components

Current public API clusters exported from `packages/tui/src/index.ts`:
- terminal + core: `TUI`, `Container`, `Terminal`, `ProcessTerminal`, overlays, `CURSOR_MARKER`
- widgets: `Text`, `TruncatedText`, `Input`, `Editor`, `Markdown`, `Loader`, `CancellableLoader`, `SelectList`, `SettingsList`, `Image`, `Box`, `Spacer`
- key system: `Key`, `KeyId`, `matchesKey`, `parseKey`, `decodeKittyPrintable`, `isKeyRelease`, `isKeyRepeat`, Kitty protocol state setters
- keybinding system: `KeybindingsManager`, `TUI_KEYBINDINGS`, `getKeybindings`, `setKeybindings`
- autocomplete + fuzzy helpers
- image capability/protocol helpers
- width/wrapping helpers: `visibleWidth`, `truncateToWidth`, `wrapTextWithAnsi`
- editor extension interface: `EditorComponent`

High-value runtime behaviors confirmed by source/tests:
- differential rendering switches between first render, full redraw, and changed-line updates
- overlays have independent focus/visibility/stack ordering and can be non-capturing
- cursor positioning for IME uses a zero-width marker emitted by focused components
- visible-width logic is grapheme-aware, ANSI-aware, emoji-aware, and explicitly treats isolated regional indicators as width 2
- input buffering splits batched stdin into complete escape/key/mouse/paste events
- key parsing supports legacy terminal sequences, Kitty CSI-u, xterm `modifyOtherKeys`, and layout-aware alternate-key reporting
- editor behavior is extensive: multiline wrapping, history, kill ring, yank/yank-pop, undo, jump-to-char, sticky visual column, paste markers, slash/file autocomplete, large-paste marker substitution, and atomic undo for pastes/programmatic insertion
- markdown rendering includes headings, inline formatting, lists, nested lists, blockquotes, tables, code blocks, spacing normalization, and default-style layering without leaking styles across padded lines
- image rendering supports Kitty/iTerm2 inline protocols, fallback text, and cell-size queries

## 3. Dependency and integration summary

Key TS runtime dependencies:
- `chalk` for styling
- `marked` for markdown parsing
- `get-east-asian-width` for width calculation
- `mime-types` for image helpers
- `@xterm/headless` in tests for terminal emulation

Observed coding-agent dependency surface on `@mariozechner/pi-tui`:
- interactive mode currently imports and relies on: `TUI`, `ProcessTerminal`, `Container`, `Text`, `Spacer`, `Markdown`, `Loader`, `TruncatedText`, `CombinedAutocompleteProvider`, `matchesKey`, `visibleWidth`, `fuzzyFilter`, keybinding registry APIs, and editor/component types
- coding-agent extends TUI keybindings with app-specific bindings in `packages/coding-agent/src/core/keybindings.ts`
- coding-agent custom editors depend on the editor component API and keybinding manager semantics, not just raw widgets

## 4. Test inventory summary

Behavior clusters covered by the TS suite:
- fuzzy matching/filtering (`fuzzy.test.ts`) — already ported
- raw key parsing and matching (`keys.test.ts`)
- keybinding registry conflict/override semantics (`keybindings.test.ts`)
- input buffering and bracketed paste splitting (`stdin-buffer.test.ts`)
- autocomplete for slash commands, direct paths, quoted paths, and `@` fuzzy search (`autocomplete.test.ts`)
- single-line input editing, kill ring, undo, wide-character rendering (`input.test.ts`)
- multiline editor behavior, paste markers, autocomplete, sticky column, kill ring, undo, grapheme-aware wrapping (`editor.test.ts`)
- markdown rendering, tables, spacing, blockquote styling, heading/code style restoration (`markdown.test.ts`)
- overlay layout, focus, z-order, style isolation, and short-content behavior (`overlay-*.test.ts`, `tui-overlay-style-leak.test.ts`)
- differential rendering and viewport correctness (`tui-render.test.ts`, `viewport-overwrite-repro.ts`)
- width/regression helpers (`truncate-to-width.test.ts`, `wrap-ansi.test.ts`, `regression-regional-indicator-width.test.ts`)
- image-line detection and inline image helpers (`terminal-image.test.ts`, `bug-regression-isimageline-startswith-bug.test.ts`, `tui-cell-size-input.test.ts`)
- widget-specific rendering (`select-list.test.ts`, `truncated-text.test.ts`)

## 5. Edge cases and implicit behaviors

Confirmed or strongly implied from source/tests:
- keybinding conflicts are reported only for direct user-config collisions; defaults are not evicted when another action reuses the same key
- `KeyId` strings are treated as config semantics; TS conflict detection is string-based, not semantic key-equivalence-based
- differential rendering must hard-cap rendered line width and crash loudly when a component overflows
- overlay compositing must not leak ANSI styles into unaffected content or padding
- prompt/quote/heading/default text styles are intentionally layered and re-applied after inline resets
- editor paste markers are atomic for cursor/delete/wrap behavior only when the marker ID exists in the current paste map
- large pasted content is submitted via expanded content, not the marker text
- image-line detection must use containment, not just prefix checks, to avoid crashes when image escape sequences appear after text

## 6. Rust target design

Planned crate shape for `pi-tui`:
- `fuzzy` — already implemented
- `keybindings` — current milestone
- future modules after this slice:
  - `keys`
  - `terminal`
  - `stdin_buffer`
  - `text_width` / `ansi`
  - `render`
  - `widgets::{text,input,editor,markdown,select_list,settings_list,image}`

Current design choice for the keybinding slice:
- store key IDs as a typed `KeyId` newtype over the raw config string
- preserve TS conflict semantics by comparing raw key IDs as configured, not by introducing early key-event normalization
- keep the keybinding manager explicit and reusable so coding-agent can inject additional app bindings later, replacing TS declaration merging with explicit Rust composition

Public API added in this milestone:
- `KeyId`
- `KeybindingDefinition`
- `KeybindingConflict`
- `KeybindingsConfig`
- `KeybindingsManager`
- `TUI_KEYBINDINGS`

Compatibility goals for this slice:
- preserve TS default binding tables and descriptions
- preserve user override behavior
- preserve direct user-binding conflict reporting without removing defaults
- preserve downstream ability to layer coding-agent-specific bindings on top of TUI defaults

## 7. Behavior freeze for milestone 2

This milestone freezes the keybinding-registry behavior through direct Rust tests ported from:
- `packages/tui/test/keybindings.test.ts`

And through integration/design grounding from:
- `packages/coding-agent/src/core/keybindings.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`

Frozen scenarios for the first Rust keybinding slice:
- rebinding `tui.input.submit` does not evict `tui.select.confirm`
- rebinding `tui.select.up` does not evict `tui.editor.cursorUp`
- user-only collisions are reported as conflicts while unrelated defaults remain intact

## 8. Known gaps after this milestone

Still deferred for `pi-tui`:
- raw terminal key parsing parity (`keys.ts`)
- terminal abstraction and stdin batching parity
- differential rendering / overlay engine
- text width and ANSI helper parity
- editor/input/markdown/select/settings/image widget parity
- coding-agent interactive integration

## 9. Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest slice needed by coding-agent interactive mode:
- port raw key parsing + matching (`packages/tui/src/keys.ts`, `packages/tui/test/keys.test.ts`)
- then port the minimal container/input/select foundation that coding-agent selectors and editor composition require

## Milestone 3 update: raw key parsing + matching slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/keys.ts`
- `packages/tui/test/keys.test.ts`
- `packages/tui/test/key-tester.ts`
- `packages/tui/src/index.ts`
- `packages/tui/src/keybindings.ts`
- `packages/coding-agent/src/core/keybindings.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/keybindings.rs`
- `rust/crates/pi-tui/tests/keybindings.rs`
- `rust/crates/pi-tui/tests/fuzzy.rs`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- Kitty keyboard protocol global state tracking (`setKittyProtocolActive()` / `isKittyProtocolActive()` equivalent)
- raw key matching against configured key IDs for:
  - legacy control characters
  - legacy escape-prefixed alt sequences
  - legacy SS3 / CSI arrows, function keys, and rxvt-style modifier sequences
  - Kitty CSI-u sequences
  - xterm `modifyOtherKeys` sequences
- Kitty alternate-key/base-layout handling for non-Latin keyboard layouts, while preserving direct codepoint authority for Latin letters and symbol keys on remapped layouts
- Kitty keypad functional key normalization to logical digits, symbols, and navigation keys
- mode-aware ambiguity handling preserved from TS:
  - `\n` becomes `enter` in legacy mode and `shift+enter` when Kitty mode is active
  - `\x1b\r` is `alt+enter` only outside Kitty mode
  - legacy alt-prefixed printable sequences are ignored when Kitty mode is active
- Windows Terminal raw `0x08` backspace heuristic parity for `backspace` vs `ctrl+backspace`
- `parseKey()`-equivalent parsing into canonical key-id strings with TS-style modifier ordering (`shift+ctrl+...`)
- `decodeKittyPrintable()` handling for printable CSI-u keypad input
- `isKeyRelease()` / `isKeyRepeat()` fast detection with bracketed-paste false-positive suppression

### Rust design summary

New `pi-tui::keys` module added with:
- `KeyEventType`
- `set_kitty_protocol_active()` / `is_kitty_protocol_active()`
- `matches_key()`
- `parse_key()`
- `decode_kitty_printable()`
- `is_key_release()` / `is_key_repeat()`

Implementation choices for this slice:
- reuse the existing `KeyId` newtype from the keybinding slice rather than introducing a second key-id representation
- keep protocol parsing explicit in one focused module instead of pulling in a larger terminal abstraction early
- use a narrow regex-backed parser only for the structured Kitty / `modifyOtherKeys` escape formats; keep legacy sequence handling as direct string matching
- preserve the current Rust keybinding manager API while making the lower-level raw key parser available for future input/terminal slices

### Validation summary

New Rust coverage added for:
- non-Latin Kitty alternate-key matching/parsing
- Kitty keypad normalization and shifted/event variants
- `modifyOtherKeys` matching/parsing across enter/tab/backspace/space/symbol/digit cases
- legacy ctrl/alt/symbol/backspace/function/arrow/rxvt behaviors
- Kitty printable decoding
- release/repeat detection false-positive regression cases

Validation run results:
- `cd rust && cargo fmt` passed
- `cd rust && cargo test -p pi-tui --test keys` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- terminal abstraction and stdin batching parity (`terminal.ts`, `stdin-buffer.ts`)
- differential rendering / overlay engine
- text width and ANSI helper parity
- input/editor/autocomplete widget parity
- markdown/select/settings/image widget parity
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and port the next smallest interactive foundation slice now that raw key parsing exists:
- `packages/tui/src/stdin-buffer.ts` + `packages/tui/test/stdin-buffer.test.ts`
- then the minimal container/input/select path needed by coding-agent selectors and editor composition

## Milestone 4 update: stdin buffering slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/stdin-buffer.ts`
- `packages/tui/test/stdin-buffer.test.ts`
- `packages/tui/src/index.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/Cargo.toml`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- buffering of incomplete escape sequences across chunk boundaries
- splitting mixed stdin batches into complete logical units for:
  - plain characters
  - CSI/SS3/meta sequences
  - Kitty CSI-u sequences
  - mouse SGR sequences
  - old-style mouse `ESC[M` packets
  - OSC / DCS / APC responses terminated by `BEL` or `ESC \\`
- timeout-based flushing of incomplete buffered escape sequences
- explicit `flush()`, `clear()`, `get_buffer()`, and `destroy()` behavior matching the TS shape for the migrated slice
- bracketed paste handling with dedicated paste events and suppression of normal data events during pasted content
- high-byte single-byte conversion parity from the TS `Buffer` path (`byte > 127` -> `ESC + (byte - 128)`)

### Rust design summary

New `pi-tui::stdin_buffer` module added with:
- `StdinBuffer`
- `StdinBufferOptions`
- `StdinBufferEvent::{Data, Paste}`

Implementation choices for this slice:
- use an explicit event enum plus `subscribe()` channel registration instead of a JS-style EventEmitter API
- keep batching/splitting logic synchronous and local to the buffer, with a small detached timeout thread for delayed flush behavior
- preserve TS sequence-detection rules directly rather than introducing a broader terminal parser early

### Validation summary

New Rust coverage added for:
- immediate plain-character pass-through
- complete and partial escape sequence handling
- mixed content and Kitty event batching
- mouse SGR and old-style mouse packets
- bracketed paste events and surrounding input preservation
- empty input, lone escape timeout, flush/clear/destroy, and byte-input conversion
- long CSI and OSC/DCS/APC terminal response handling

Validation run results:
- `cd rust && cargo fmt` passed
- `cd rust && cargo test -p pi-tui --test stdin_buffer` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- terminal abstraction parity (`terminal.ts`)
- render tree / container / overlay engine
- width/ANSI helpers
- input/editor/autocomplete widgets
- markdown/select/settings/image widgets
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and port the next concrete interactive foundation layer:
- the minimal terminal/input path (`packages/tui/src/terminal.ts`)
- or, if staying narrower, the first container/input widget slice needed by coding-agent interactive mode

## Milestone 5 update: terminal control + protocol-state slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/terminal.ts`
- `packages/tui/test/virtual-terminal.ts`
- `packages/tui/test/tui-render.test.ts`
- `packages/tui/test/overlay-non-capturing.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/Cargo.toml`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- exported terminal surface now exists in Rust with:
  - `Terminal` trait
  - `ProcessTerminal`
- terminal output/control ANSI helpers now mirror the TS command shapes for:
  - relative cursor movement
  - cursor hide/show
  - clear line / clear from cursor / full clear-screen
  - terminal title setting
- `ProcessTerminal::start()` now performs the same startup control writes for the migrated slice:
  - enable bracketed paste
  - query Kitty keyboard protocol support
- `ProcessTerminal` now integrates the previously ported stdin buffer for protocol-aware input splitting in the migrated slice
- Kitty protocol response handling now matches the TS shape:
  - detect `CSI ? <flags> u`
  - mark Kitty protocol active
  - update global key-parser Kitty state
  - emit the TS enable sequence (`CSI > 7 u`)
  - do not forward the raw protocol response to the input handler
- bracketed paste events from the stdin buffer are rewrapped and forwarded as the original TS input-handler shape (`\x1b[200~...\x1b[201~`)
- modifyOtherKeys fallback state handling now exists for the migrated slice, including disable-on-drain/stop behavior
- `drain_input()` and `stop()` now disable active input protocols in TS order for the implemented subset
- terminal dimension accessors now exist with Rust-side fallback semantics

Current intentional compatibility limitation for this slice:
- Rust does not yet attach `ProcessTerminal` to real stdin/stdout event loops, raw-mode management, resize signals, Windows VT-input setup, or the timed modifyOtherKeys fallback timer. This milestone ports the terminal control/protocol state machine first, not the full OS integration path.

### Rust design summary

New `pi-tui::terminal` module added with:
- `Terminal` trait
- `ProcessTerminal`
- internal stdout backend abstraction for testable write capture

Implementation choices for this slice:
- keep the migrated terminal logic centered on protocol/control behavior that can be validated deterministically without running an interactive app
- reuse the existing `StdinBuffer` and Kitty global-state helpers instead of duplicating sequence handling
- keep resize notification and modifyOtherKeys fallback as explicit internal hooks for now, so the future OS-integration layer can call them without redesigning the terminal surface again
- widen `TuiError` to carry I/O failures from terminal write operations

### Validation summary

New Rust coverage added for:
- startup writes for bracketed paste + Kitty query
- Kitty protocol response handling and non-forwarding of the raw response
- normal input forwarding and bracketed paste rewrapping
- modifyOtherKeys fallback activation plus drain-time disable ordering
- stop-time protocol disable ordering
- ANSI helper output sequences
- Kitty protocol response recognition helper behavior

Validation run results:
- `cd rust && cargo fmt` passed
- `cd rust && cargo test -p pi-tui terminal` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- real process stdin event-loop wiring and raw-mode handling
- real resize/signal integration
- Windows VT-input setup parity
- timed modifyOtherKeys fallback parity
- render tree / container / overlay engine
- width/ANSI helpers
- input/editor/autocomplete widgets
- markdown/select/settings/image widgets
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest layer that reduces interactive-mode risk:
- either complete the remaining OS-facing `ProcessTerminal` integration
- or port the first render/container slice needed to make `Terminal` consumable by a Rust `TUI`

## Milestone 6 update: width / ANSI helper slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/utils.ts`
- `packages/tui/test/truncate-to-width.test.ts`
- `packages/tui/test/wrap-ansi.test.ts`
- `packages/tui/test/regression-regional-indicator-width.test.ts`
- `packages/tui/src/index.ts`
- `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/fuzzy.rs`
- `rust/crates/pi-tui/src/keybindings.rs`
- `rust/crates/pi-tui/src/keys.rs`
- `rust/crates/pi-tui/src/stdin_buffer.rs`
- `rust/crates/pi-tui/src/terminal.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- visible-width calculation for:
  - printable ASCII fast path
  - tabs as width 3
  - CSI styling codes and OSC/APC sequences ignored for width
  - wide CJK graphemes
  - conservative width-2 handling for regional-indicator singletons and common emoji graphemes to avoid streaming drift
- truncation with ANSI preservation:
  - reset inserted before and after ellipsis
  - wide-ellipsis clipping
  - optional fixed-width padding
  - malformed escape-prefix tolerance
  - contiguous-prefix preservation (do not skip a wide grapheme and resume later)
- ANSI-aware word wrapping:
  - wraps plain text and long tokens by grapheme width
  - carries active SGR styles across wrapped lines
  - uses underline-only reset at wrapped line ends to avoid style bleed while preserving background color
  - preserves OSC-width semantics during wrapping decisions
- small helper surface for later render/widget work:
  - ANSI extraction
  - punctuation / whitespace classification

### Rust design summary

New `pi-tui::utils` module added with:
- `AnsiCode`
- `extract_ansi_code()`
- `visible_width()`
- `wrap_text_with_ansi()`
- `truncate_to_width()`
- `is_whitespace_char()`
- `is_punctuation_char()`

Implementation choices for this slice:
- use `unicode-segmentation` for grapheme-safe iteration
- use `unicode-width` for base terminal width with explicit emoji/regional-indicator overrides grounded in the TS tests
- keep ANSI parsing explicit and narrow to the escape classes used by the current TS helper/tests (CSI style/control, OSC, APC)
- port the TS SGR state tracker shape directly so wrapped lines preserve foreground/background state while only resetting underline at intermediate line boundaries

### Validation summary

New Rust coverage added for:
- large-unicode truncation bounds
- ANSI-preserving truncation reset behavior
- malformed ANSI truncation tolerance
- wide-ellipsis clipping
- contiguous-prefix truncation regression
- OSC width stripping
- regional-indicator width regression cases
- emoji intermediate width stability
- underline/background wrapping regressions
- color-preserving wrap continuation behavior

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- slice-by-column / segment extraction helpers from `utils.ts`
- render tree / container / overlay engine
- input/editor/autocomplete widgets
- markdown/select/settings/image widgets
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and keep the scope narrow:
- either finish the remaining `utils.ts` extraction helpers needed by rendering (`sliceByColumn`, `extractSegments`)
- or start the first minimal render/container slice now that width/truncation/wrapping primitives exist

## Milestone 7 update: slicing / segment-extraction helper slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/tui.ts` (overlay compositing call sites around `compositeLineAt()`)
- `packages/tui/src/components/input.ts` (horizontal scrolling call sites)
- `packages/tui/test/tui-render.test.ts`
- `packages/tui/test/overlay-non-capturing.test.ts`
- `packages/tui/test/tui-overlay-style-leak.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/utils.rs`
- `rust/crates/pi-tui/tests/utils.rs`
- `rust/crates/pi-tui/src/lib.rs`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- column-based ANSI-aware slicing now exists for the remaining `utils.ts` render primitives:
  - `sliceByColumn()`
  - `sliceWithWidth()`
  - `extractSegments()`
- strict wide-character boundary behavior is now covered, matching the TS helper semantics used by overlay compositing and input horizontal scrolling
- pending ANSI codes that occur before the first visible sliced grapheme are now preserved in the sliced result, matching TS behavior
- `extractSegments()` now carries active style state from the pre-overlay region into the extracted `after` region, matching the TS compositing strategy that prevents style loss when overlays replace the middle of a styled line

### Rust design summary

Expanded `pi-tui::utils` with:
- `SliceWithWidthResult`
- `ExtractSegmentsResult`
- `slice_by_column()`
- `slice_with_width()`
- `extract_segments()`

Implementation choices for this slice:
- preserve the TS helper split rather than folding the behavior into a future renderer prematurely
- keep these helpers independent from any `TUI`/container implementation so they can be reused by the future render engine and input widgets
- maintain TS-style ANSI carry-forward behavior in `extract_segments()` via the existing Rust SGR tracker instead of introducing a separate render-state abstraction early

### Validation summary

New Rust coverage added for:
- strict slicing at wide-character boundaries
- ANSI carry-forward in sliced ranges
- style inheritance from `before` to `after` segments
- strict `after` extraction around wide-character boundaries

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- render tree / container / overlay engine
- input/editor/autocomplete widgets using the new helper surface
- markdown/select/settings/image widgets
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and move to the first real renderer slice:
- port the minimal container/component/render path from `packages/tui/src/tui.ts`
- use the now-ported width/truncation/slicing helpers directly instead of adding new abstraction layers first

## Milestone 8 update: minimal container / overlay render slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/tui.ts` (full file)
- `packages/tui/test/virtual-terminal.ts`
- `packages/tui/test/overlay-options.test.ts`
- `packages/tui/test/overlay-short-content.test.ts`
- previously-read render/overlay call sites remained relevant:
  - `packages/tui/test/tui-render.test.ts`
  - `packages/tui/test/overlay-non-capturing.test.ts`
  - `packages/tui/test/tui-overlay-style-leak.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/terminal.rs`
- `rust/crates/pi-tui/src/utils.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-tui/tests/utils.rs`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- first minimal render/container surface now exists in `pi-tui`:
  - `Component`
  - `Container`
  - `Tui`
  - `CURSOR_MARKER`
- overlay rendering now supports the first useful layout/compositing subset from `packages/tui/src/tui.ts`:
  - anchor positioning
  - absolute row/col positioning
  - percentage row/col positioning
  - min width and percentage width resolution
  - max-height truncation
  - margin and offset handling
  - stacked overlay ordering (later overlays render on top)
  - hide-top-overlay behavior for the current minimal slice
- overlay compositing now uses the previously ported width/slicing helpers, including:
  - defensive truncation of overlay lines to declared overlay width
  - composite padding by visible width
  - final terminal-width clamping via strict column slicing
- short-content overlay placement is now preserved by padding the render working area to terminal height before compositing, matching the TS behavior that keeps overlays screen-relative even when base content is short
- rendered lines now receive the same segment reset suffix strategy used by TS overlay compositing to reduce style leakage between segments

Current intentional compatibility limitation for this slice:
- Rust `Tui` currently does full-frame redraws on `start()` / `request_render()`; it does not yet port TS differential rendering, cursor extraction/IME positioning, focus management, input routing, or overlay handle semantics

### Rust design summary

New `pi-tui::tui` module added with:
- `Component`
- `ComponentId`
- `OverlayId`
- `Container`
- `OverlayAnchor`
- `OverlayMargin`
- `SizeValue`
- `OverlayOptions`
- `Tui<T: Terminal>`
- `CURSOR_MARKER`

Implementation choices for this slice:
- keep the first renderer generic over the existing Rust `Terminal` trait instead of pulling in a separate virtual-terminal dependency
- use stable ids for children/overlays now, while keeping focus/input APIs deferred until their behavior is ported and validated
- keep redraw behavior intentionally simple (full-frame) so the first slice validates layout/composition semantics before differential rendering complexity is introduced
- preserve the TS overlay-layout math and compositing order closely, while deferring visibility/focus interaction semantics beyond the basic hidden/visible slice

### Validation summary

New Rust coverage added for:
- child-container render ordering
- overlay rendering with short base content
- percentage width and `minWidth` behavior
- anchor/margin/offset/absolute positioning
- bottom-right and percentage positioning
- `maxHeight` truncation
- stacked overlay ordering and hide-top-overlay behavior
- overlay width-overflow protection on styled/wide content
- basic `start()` / `request_render()` terminal-write path

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- differential rendering from `packages/tui/src/tui.ts`
- cursor-marker extraction and hardware-cursor positioning
- focusable component model and focus management
- input routing and overlay handle parity (`focus()`, `unfocus()`, `setHidden()`, non-capturing overlays)
- input/editor/autocomplete widgets
- markdown/select/settings/image widgets
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest renderer behavior slice:
- port cursor-marker extraction plus hardware-cursor positioning from `packages/tui/src/tui.ts`
- then add focus/input routing and only afterward attempt TS differential rendering parity

## Milestone 9 update: cursor-marker extraction + hardware-cursor positioning slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/tui.ts`
- `packages/tui/test/tui-render.test.ts`
- `packages/tui/test/virtual-terminal.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/src/terminal.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/tests/tui.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- visible-viewport cursor-marker extraction now runs before line-reset suffixes are applied
- cursor column calculation now uses ANSI-aware `visible_width(...)`, matching the TypeScript `extractCursorPosition()` behavior
- full-frame rendering now strips the first bottom-most visible `CURSOR_MARKER` occurrence from rendered output and repositions the terminal cursor to that row/column after writing the frame
- hardware-cursor visibility now follows the TS shape for the migrated slice:
  - no marker => cursor hidden
  - marker present + hardware cursor disabled => cursor positioned then hidden
  - marker present + hardware cursor enabled => cursor positioned then shown
- Rust `render_for_size()` now returns marker-stripped screen lines for the visible-marker slice, which keeps renderer tests aligned with actual on-screen output

### Rust design summary

Expanded `pi-tui::tui` with:
- internal `CursorPosition`
- internal `RenderedFrame`
- `Tui::show_hardware_cursor()`
- `Tui::set_show_hardware_cursor(bool)`
- internal cursor extraction + post-frame cursor-positioning helpers

Implementation choices for this slice:
- keep the renderer on the existing simple full-frame redraw path; do not pull differential-render cursor tracking forward yet
- keep cursor positioning local to `tui.rs` using ANSI writes after the frame instead of widening the terminal trait again for a one-slice need
- default the Rust hardware-cursor flag from `PI_HARDWARE_CURSOR`, matching the TypeScript constructor/env shape closely enough for the current slice
- keep focusable-component and input-routing parity deferred; components can already emit `CURSOR_MARKER`, but Rust still does not manage focus state yet

### Validation summary

New Rust coverage added for:
- marker stripping from rendered output
- post-frame cursor row/column positioning to a visible marker
- hardware-cursor show/hide behavior with and without a marker

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test tui` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails outside this migration slice in `packages/web-ui` with existing TypeScript module-resolution / implicit-any errors; not in the allowed Rust migration scope

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- differential rendering from `packages/tui/src/tui.ts`
- focusable component model and focus management
- input routing and overlay handle parity (`focus()`, `unfocus()`, `setHidden()`, non-capturing overlays)
- editor/input/autocomplete widgets
- markdown/select/settings/image widgets
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest interactive renderer slice:
- port focusable-component state plus overlay focus/input routing from `packages/tui/src/tui.ts`
- keep differential rendering deferred until focus, overlay-handle, and input-routing semantics are in place

## Milestone 10 update: focus state + overlay focus/input-routing slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/tui.ts`
- `packages/tui/test/overlay-non-capturing.test.ts`
- `packages/tui/test/tui-render.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/tests/tui.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- components can now participate in focus and input handling through explicit component hooks on the Rust `Component` trait
- the Rust `Tui` now tracks focused child/overlay state and updates component focus flags when focus changes
- overlay focus behavior now matches the TypeScript shape for the migrated slice:
  - capturing overlays auto-focus when shown if visible
  - non-capturing overlays preserve existing focus when shown
  - focusing an overlay bumps its visual order to the top
  - unfocusing or hiding a focused overlay restores the previous capturing overlay or saved pre-focus target
  - unhiding a non-capturing overlay does not auto-focus it
- manual input routing now exists in Rust via `Tui::handle_input(...)`:
  - input goes to the focused target
  - key-release events are filtered unless the focused component opts in
  - if the focused overlay becomes invisible, routing redirects to the next visible capturing overlay and skips non-capturing overlays

Current intentional compatibility limitation for this slice:
- Rust still does not wire `Tui::start()` to the routed input path. Input routing is now implemented and tested through the explicit `Tui::handle_input(...)` surface, while safe terminal-callback integration remains deferred.

### Rust design summary

Expanded `pi-tui::tui` with:
- component hooks on `Component`:
  - `handle_input(...)`
  - `wants_key_release()`
  - `set_focused(...)`
- internal `FocusTarget`
- `Tui::set_focus_child(...)`
- `Tui::clear_focus()`
- `Tui::is_child_focused(...)`
- `Tui::focus_overlay(...)`
- `Tui::unfocus_overlay(...)`
- `Tui::is_overlay_focused(...)`
- `Tui::has_overlay()`
- `Tui::handle_input(...)`

Implementation choices for this slice:
- keep focus ownership inside `tui.rs` with ids over owned children/overlays rather than trying to port the full JS object-reference model immediately
- keep input routing explicit and deterministic instead of forcing unsafe terminal callback capture into this milestone
- preserve TS overlay pre-focus restoration and focus-order bump semantics closely, while still deferring full overlay-handle parity

### Validation summary

New Rust coverage added for:
- capturing vs non-capturing overlay focus behavior
- overlay focus/unfocus/hide restoration to the previous child focus target
- redirecting input away from an invisible focused overlay while skipping non-capturing overlays
- visual-order bumping when a lower overlay is focused
- non-capturing unhide behavior not stealing focus

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test tui` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails outside this migration slice in `packages/web-ui` with existing TypeScript module-resolution / implicit-any errors; not in the allowed Rust migration scope

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- safe terminal-callback wiring from `Tui::start()` into the routed input path
- input-listener pipeline, cell-size response consumption, and debug-key forwarding from TS `handleInput()`
- full overlay-handle parity (`hide()`, `focus()`, `unfocus()`, `isFocused()`) as a first-class Rust type
- differential rendering from `packages/tui/src/tui.ts`
- editor/input/autocomplete widgets
- markdown/select/settings/image widgets
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest interactive-control slice:
- port the TS `handleInput()` pre-routing pipeline (input listeners, cell-size response consumption, debug-key hook)
- then decide whether to add safe terminal-callback wiring or move directly to a Rust overlay-handle API before differential rendering

## Milestone 11 update: handleInput pre-routing + terminal-image state slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/tui.ts`
- `packages/tui/src/terminal-image.ts`
- `packages/tui/test/tui-cell-size-input.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/tests/tui.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- the Rust `Tui::handle_input(...)` now includes the first TS pre-routing pipeline before focus-based dispatch:
  - ordered input-listener processing
  - consume/replace semantics
  - empty-input short-circuiting
  - cell-size response consumption
  - debug-key interception for `shift+ctrl+d`
- cell-size response handling now matches the TypeScript shape for the migrated slice:
  - exact `ESC [ 6 ; <height> ; <width> t` responses are consumed
  - valid responses update shared cell dimensions as `{ widthPx, heightPx }`
  - zero/invalid-size responses are consumed without forwarding
  - bare escape still forwards normally
  - consumed cell-size responses trigger invalidation and a rerender in the current full-frame Rust path
- Rust now has a minimal `terminal_image` state/config slice aligned with the TypeScript module for the behavior currently needed by `Tui`:
  - terminal capability detection/cache
  - shared cell-dimension storage
- `Tui::start()` now sends the TS-style cell-size query (`CSI 16 t`) when the detected terminal image capabilities indicate image support

Current intentional compatibility limitation for this slice:
- `Tui::start()` still does not wire terminal callbacks into `Tui::handle_input(...)`; the pre-routing pipeline is implemented and validated through the explicit `handle_input(...)` API only.

### Rust design summary

New Rust module:
- `pi-tui::terminal_image`
  - `ImageProtocol`
  - `TerminalCapabilities`
  - `CellDimensions`
  - `detect_capabilities()`
  - `get_capabilities()`
  - `reset_capabilities_cache()`
  - `get_cell_dimensions()`
  - `set_cell_dimensions()`

Expanded `pi-tui::tui` with:
- `InputListenerId`
- `InputListenerResult`
- `Tui::add_input_listener(...)`
- `Tui::remove_input_listener(...)`
- `Tui::clear_input_listeners()`
- `Tui::set_debug_handler(...)`
- `Tui::clear_debug_handler()`
- internal `query_cell_size()`
- internal `consume_cell_size_response(...)`

Implementation choices for this slice:
- keep listener/debug routing explicit on `Tui` instead of introducing a broader event-emitter abstraction
- keep terminal-image support intentionally narrow to the shared state and detection needed for the current `Tui` behavior, not the full TS image-rendering stack yet
- keep callback wiring deferred to avoid forcing a larger ownership redesign in the same milestone

### Validation summary

New Rust coverage added for:
- input-listener transform/consume/remove behavior
- debug-key interception before focused-component delivery
- cell-size response consumption plus later-input forwarding
- bare-escape forwarding regression for the cell-size-response path

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test tui` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails outside this migration slice in `packages/web-ui` with existing TypeScript module-resolution / implicit-any errors; not in the allowed Rust migration scope

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- safe terminal-callback wiring from `Tui::start()` into the routed input path
- full overlay-handle parity (`hide()`, `focus()`, `unfocus()`, `isFocused()`) as a first-class Rust type
- differential rendering from `packages/tui/src/tui.ts`
- editor/input/autocomplete widgets
- markdown/select/settings/image widgets
- broader terminal-image rendering helpers and image widget parity
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest control-path slice:
- wire `Tui::start()` safely into the existing Rust `handle_input(...)` pipeline
- then add a first-class Rust overlay-handle API before attempting differential rendering

## Milestone 12 update: queued terminal-callback bridge slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/tui.ts`
- `packages/tui/test/virtual-terminal.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/tests/tui.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-adjacent behaviors now covered in Rust:
- `Tui::start()` no longer drops terminal callbacks on the floor for the migrated slice
- start-time terminal input and resize callbacks are now captured and queued in Rust instead of being ignored
- the new queued callback path feeds terminal-originated input through the same Rust `handle_input(...)` pipeline already implemented in earlier milestones, so terminal input now honors:
  - input listeners
  - debug-key interception
  - cell-size response consumption
  - focus/input routing
  - rerender-after-input behavior
- queued resize callbacks now trigger rerenders when drained
- `Tui::stop()` now clears queued terminal events for the migrated slice

Current intentional compatibility limitation for this slice:
- callback delivery is now wired safely through a queue, but draining is still explicit via `Tui::drain_terminal_events()`. Rust does not yet process terminal callbacks immediately/asynchronously the way the full TypeScript event path effectively does.

### Rust design summary

Expanded `pi-tui::tui` with:
- internal `TerminalEvent::{Input, Resize}` queue
- shared pending-event storage captured by `start()` callbacks
- `Tui::drain_terminal_events()`

Implementation choices for this slice:
- choose a safe queued bridge over unsafe self-referential callback capture
- keep the bridge minimal and explicit instead of redesigning the whole `Tui` ownership model in one milestone
- reuse the existing `handle_input(...)` and render paths rather than introducing a second callback-only dispatch path

### Validation summary

New Rust coverage added for:
- terminal input callbacks draining through the existing input pipeline
- terminal resize callbacks triggering rerender on drain

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test tui` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails outside this migration slice in `packages/web-ui` with existing TypeScript module-resolution / implicit-any errors; not in the allowed Rust migration scope

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- immediate/asynchronous callback processing without an explicit drain step
- full overlay-handle parity (`hide()`, `focus()`, `unfocus()`, `isFocused()`) as a first-class Rust type
- differential rendering from `packages/tui/src/tui.ts`
- editor/input/autocomplete widgets
- markdown/select/settings/image widgets
- broader terminal-image rendering helpers and image widget parity
- coding-agent interactive integration

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest API/control slice:
- add a first-class Rust overlay-handle API on top of the existing id-based overlay controls
- then revisit whether queued callback draining is sufficient or whether a broader `Tui` ownership refactor is justified before differential rendering

## Milestone 13 update: minimal single-line input widget slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/components/input.ts`
- `packages/tui/test/input.test.ts`
- `packages/tui/src/keybindings.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/footer.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (submit / follow-up grounding)

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/src/keybindings.rs`
- `rust/crates/pi-tui/src/keys.rs`
- `rust/crates/pi-tui/src/utils.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- `pi-tui` now exports a first minimal `Input` widget for the single-line input slice from `packages/tui/src/components/input.ts`
- the Rust `Input` now supports the core behaviors needed for the first coding-agent editor path:
  - printable character insertion including literal backslashes
  - submit callback on the configured `tui.input.submit` binding and raw `\n`
  - cancel callback on `tui.select.cancel`
  - bracketed-paste buffering with newline stripping and tab expansion
  - grapheme-aware cursor-left / cursor-right movement
  - cursor-to-start / cursor-to-end movement
  - word-left / word-right movement using the existing Rust punctuation/whitespace helpers
  - backspace and forward delete
  - delete word backward / forward
  - delete to line start / end
  - horizontal scrolling with wide-character-safe rendering
  - `CURSOR_MARKER` emission when focused so the existing Rust `Tui` hardware-cursor path can position IME/cursor state correctly
- rendered input lines now stay within the requested width for the wide-text cases ported from the TypeScript tests

Current intentional compatibility limitation for this slice:
- this milestone ports only the smallest useful single-line input subset
- the broader TypeScript input/editor behaviors remain deferred:
  - kill ring and yank / yank-pop
  - undo stack parity
  - history browsing
  - multiline editor behavior
  - autocomplete
  - copy/selection behavior
  - large-paste marker substitution used by the multiline editor path

### Rust design summary

New Rust module:
- `pi-tui::input`
  - `Input`

Public API added in this milestone:
- `Input::new()`
- `Input::with_keybindings(...)`
- `Input::value()` / `get_value()`
- `Input::set_value(...)`
- `Input::clear()`
- `Input::cursor()` / `set_cursor(...)`
- `Input::is_focused()` / `set_focused(...)`
- `Input::set_on_submit(...)` / `clear_on_submit()`
- `Input::set_on_escape(...)` / `clear_on_escape()`
- crate export via `pi_tui::Input`

Implementation choices for this slice:
- keep the widget single-line and self-contained instead of pulling the much larger TS `Editor` surface forward prematurely
- inject a `KeybindingsManager` into the widget instead of porting the TS global `getKeybindings()` / `setKeybindings()` singleton yet
- reuse already-ported raw-key parsing, grapheme utilities, width helpers, and `CURSOR_MARKER` support instead of introducing parallel editor-specific primitives
- intentionally defer kill-ring and undo internals until a larger editor/widget milestone actually needs them

### Validation summary

New Rust coverage added for:
- submit callback preserving a trailing backslash on Enter
- literal backslash insertion
- escape/cancel callback behavior
- bracketed-paste newline stripping and tab expansion
- `Ctrl+W` word deletion
- wide-text render width stability
- horizontal-scroll cursor visibility

Validation run results:
- `cd rust && cargo test -p pi-tui --test input` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- first-class overlay-handle API
- differential rendering from `packages/tui/src/tui.ts`
- richer input/editor parity (kill ring, undo, multiline editor, history, autocomplete)
- markdown/select/settings/image widgets
- coding-agent interactive integration on top of the new input widget

### Recommended next step

Stay in `packages/tui` / `rust/crates/pi-tui` and continue with the next smallest interaction slice that unlocks coding-agent integration without overbuilding:
- either add the first-class overlay-handle API still deferred from the control path
- or port the next minimal supporting widgets (`Text` / `Spacer`) plus a thin coding-agent shell layer that can actually use the new `Input`

## Milestone 14 update: first-class overlay-handle API slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/tui.ts`
- `packages/tui/src/index.ts`
- `packages/tui/test/overlay-non-capturing.test.ts`
- `packages/tui/test/overlay-options.test.ts`
- `packages/tui/test/overlay-short-content.test.ts`
- `packages/tui/test/tui-render.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/tests/tui.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- `pi-tui` now exposes a first-class `OverlayHandle` surface on top of the existing id-based overlay controls
- the handle now covers the current TypeScript overlay-control behaviors needed by coding-agent selector/dialog work:
  - hide an overlay
  - toggle temporary hidden state
  - query hidden state
  - focus an overlay
  - unfocus an overlay
  - query focused state
- non-capturing overlay behavior remains aligned with the earlier Rust focus work while now being addressable through the new handle API:
  - focusing a non-capturing overlay explicitly transfers focus
  - unfocusing restores the previous target
  - rehiding/unhiding a non-capturing overlay does not auto-focus it
- handle no-op guard behavior is now covered in Rust for the migrated slice:
  - focusing a hidden overlay is a no-op
  - focusing or unfocusing a removed overlay handle is a no-op

Current intentional compatibility limitation for this slice:
- unlike the TypeScript object handle, the Rust `OverlayHandle` methods require an explicit `&mut Tui<_>` or `&Tui<_>` parameter instead of closing over the owning `Tui`
- stale Rust handles do not preserve their own independent hidden-state shadow after removal; once an overlay is removed, handle queries fall back to `false`

### Rust design summary

Expanded `pi-tui::tui` with:
- `OverlayHandle`
- `Tui::show_overlay_handle(...)`
- `Tui::is_overlay_hidden(...)`

Design choices for this slice:
- keep the existing id-based overlay API intact and layer the handle API on top, so the migration does not force a broader `Tui` ownership redesign in the same milestone
- use a lightweight handle keyed by `OverlayId` instead of introducing shared interior-mutability between `Tui` and overlay handles
- keep handle operations as thin delegations to the already-validated overlay focus/visibility machinery

### Validation summary

New Rust coverage added for:
- handle-driven focus/unfocus and hidden-state transitions on a non-capturing overlay
- handle-driven no-op behavior for hidden overlays and removed overlays
- existing overlay/focus/render tests continue to validate the shared underlying overlay machinery

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test tui` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` to be re-run after this milestone; current repo history indicates it may pass, but validation for this slice should report the actual result from the current worktree

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- differential rendering parity is still partial relative to TS `packages/tui/src/tui.ts`
- richer widget surface still missing (`Text`, `Spacer`, multiline editor, autocomplete, markdown/select/settings/image widgets)
- no thin coding-agent interactive shell has been wired on top of the current Rust `Input`, startup-header, and overlay foundations yet
- immediate terminal-callback processing still uses the current explicit queued-drain bridge instead of a more direct event path

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- port the next smallest layout/widget slice that can actually consume the new overlay handles in coding-agent
- the best next candidate is a minimal `Text` / `Spacer` + shell composition layer for a Rust interactive startup view, keeping multiline editor and transcript work deferred until that shell exists

## Milestone 15 update: minimal `Text` + `Spacer` widget slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/components/text.ts`
- `packages/tui/src/components/spacer.ts`
- `packages/tui/src/utils.ts` (background-application helper section)
- `packages/tui/src/index.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `packages/coding-agent/src/modes/interactive/components/visual-truncate.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/utils.rs`
- `rust/crates/pi-tui/tests/utils.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- `pi-tui` now exports first minimal `Text` and `Spacer` widgets
- the Rust `Text` slice now preserves the current TypeScript behavior needed by coding-agent startup and status/message composition:
  - blank/whitespace-only text renders no lines
  - horizontal padding is applied before final width padding
  - vertical padding inserts full-width blank lines above and below content
  - tabs are normalized to three spaces before wrapping
  - wrapped lines are padded to the requested width
  - optional background styling can be applied to both content and trailing padding for each rendered line
- the Rust `Spacer` now preserves the TS shape of rendering a configurable count of empty lines

Current intentional compatibility limitation for this slice:
- the Rust `Text` component does not yet port the TypeScript render cache; current behavior is recomputed on every render
- no generic `applyBackgroundToLine(...)` helper was added to `pi-tui::utils` yet; the background-padding behavior currently lives inside the Rust `Text` component only

### Rust design summary

New Rust modules:
- `pi-tui::text`
  - `Text`
- `pi-tui::spacer`
  - `Spacer`

Public API added in this milestone:
- `Text::new(...)`
- `Text::with_custom_bg_fn(...)`
- `Text::set_text(...)`
- `Text::set_custom_bg_fn(...)`
- `Text::clear_custom_bg_fn()`
- `Spacer::new(...)`
- `Spacer::set_lines(...)`
- crate exports via `pi_tui::Text` and `pi_tui::Spacer`

Implementation choices for this slice:
- keep the first Rust `Text` focused on the behavior coding-agent currently uses heavily (`Text(...)` with padding and optional background), rather than broadening into the more complex markdown/box/widget surfaces first
- reuse the already-ported `wrap_text_with_ansi(...)` and `visible_width(...)` helpers instead of introducing a second text-layout path
- keep the background function as an explicit closure stored on the component, matching the TS extensibility shape closely enough for later coding-agent message widgets

### Validation summary

New Rust coverage added for:
- blank text rendering no lines
- spacer rendering the requested number of empty lines
- wrapped text with horizontal padding and full-width output
- vertical padding behavior
- tab normalization to three spaces
- background application over both content and padding width

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test text` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- thin coding-agent shell composition on top of the now-ported `BuiltInHeaderComponent`, `Input`, `Text`, and `Spacer`
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`
- richer widgets are still missing (`TruncatedText`, multiline editor, autocomplete, markdown/select/settings/image widgets)
- no Rust transcript/chat view has been composed yet from the currently available primitives

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- build the first thin Rust interactive startup shell using the already-ported pieces (`BuiltInHeaderComponent`, `Input`, `Text`, `Spacer`, overlay handles, and `Tui`)
- keep multiline editor, transcript rendering, and broader widget parity deferred until that shell is rendering and focusable end-to-end
