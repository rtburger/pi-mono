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

## Milestone 16 update: `TruncatedText` widget slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/components/truncated-text.ts`
- `packages/tui/test/truncated-text.test.ts`
- `packages/tui/src/index.ts`
- grounding call sites reviewed via existing interactive inventory in `packages/coding-agent/src/modes/interactive/interactive-mode.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/text.rs`
- `rust/crates/pi-tui/src/spacer.rs`
- `rust/crates/pi-tui/tests/text.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- `pi-tui` now exports a first minimal `TruncatedText` widget matching `packages/tui/src/components/truncated-text.ts`
- the Rust widget preserves the current TypeScript behavior needed by coding-agent selectors, pending-message strips, and compact status rows:
  - render only the first logical line of text
  - stop at the first newline even when later lines exist
  - ANSI-aware truncation using the already-ported width helpers
  - exact-width output padding after truncation
  - vertical padding lines rendered as full-width blanks
  - empty text still renders a padded content line, unlike the existing Rust `Text` widget
- styled/truncated output keeps ANSI sequences and reset-before-ellipsis behavior through the shared truncation helper

### Rust design summary

New Rust module:
- `pi-tui::truncated_text`
  - `TruncatedText`

Public API added in this milestone:
- `TruncatedText::new(...)`
- `TruncatedText::set_text(...)`
- crate export via `pi_tui::TruncatedText`

Implementation choices for this slice:
- reuse the already-ported `truncate_to_width(...)` and `visible_width(...)` helpers rather than adding a second truncation path
- keep the widget intentionally narrow and single-line, matching the TS component instead of broadening into a generic text-layout abstraction
- preserve the behavioral distinction from `Text`: `TruncatedText` always emits a content line, even for empty input

### Validation summary

New Rust coverage added for:
- exact-width rendering with and without vertical padding
- long-line truncation with ellipsis
- ANSI-preserving rendering and reset-before-ellipsis behavior
- empty-text rendering
- newline stop behavior and truncation of only the first line

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test truncated_text` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- thin coding-agent transcript/chat composition on top of the current startup shell and text widgets
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`
- richer widgets are still missing (`Box`, multiline editor, autocomplete, markdown/select/settings/image widgets)
- no Rust transcript/chat view has been composed yet from the currently available primitives

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- use the new `TruncatedText` together with the existing startup-shell pieces to begin the first transcript/pending-message composition slice
- keep multiline editor and broader selector/widget parity deferred until that transcript shell exists

## Milestone 17 update: component viewport-size propagation slice

### Files analyzed

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/tests/tui.rs`
- downstream consumer grounding reviewed before implementation:
  - `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
  - `rust/crates/pi-coding-agent-tui/src/transcript.rs`
  - `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

Relevant TypeScript grounding already in scope for the surrounding renderer behavior:
- `packages/tui/src/tui.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts`

### Behavior summary

New TS-adjacent behavior now covered in Rust:
- Rust `pi-tui` components can now receive the current terminal viewport size before rendering through a new optional component hook
- `Tui::render_for_size(...)` and the normal render path now propagate `(width, height)` to root children before rendering, instead of leaving components width-only
- viewport-size propagation also now reaches overlay components before their render pass
- this new hook gives downstream Rust coding-agent components enough viewport context to implement transcript clipping/scroll behavior without widening the render signature or redesigning the full component tree API

Current intentional limitation for this slice:
- the new viewport-size hook is advisory only; it does not change the existing width-only `render(width)` contract
- no differential-render or immediate async callback behavior changed in this milestone; the hook only provides additional size context to interested components

### Rust design summary

Expanded `pi-tui::Component` with:
- `set_viewport_size(&self, width: usize, height: usize)` default no-op hook

Expanded `pi-tui::Container` with:
- propagation of the viewport-size hook to child components

Expanded `pi-tui::Tui` with:
- root viewport-size propagation before main-frame rendering
- overlay viewport-size propagation before overlay rendering

Design choices for this slice:
- keep the existing component render API stable instead of broadening it to `render(width, height)` mid-migration
- use a default no-op trait hook so already-ported widgets remain source-compatible unless they need viewport awareness
- keep the hook local to `tui.rs` and trait-based, which lets downstream crates opt in incrementally

### Validation summary

New Rust coverage added for:
- root-child viewport-size propagation before `render_for_size(...)`
- existing overlay/focus/input renderer tests continue to pass with the new hook in place

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test tui` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`
- richer widgets are still missing (`Box`, multiline editor, autocomplete, markdown/select/settings/image widgets)
- no Rust transcript/chat view scroll interaction on top of the new viewport hook yet
- no full coding-agent interactive runtime integration yet

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- consume the new viewport-size hook in the coding-agent startup shell/transcript path to add transcript viewport clipping and scrolling
- keep multiline editor and broader widget parity deferred until that transcript viewport behavior exists

## Milestone 18 update: real `ProcessTerminal` stdin/raw-mode slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/terminal.ts`
- `packages/tui/src/tui.ts`
- `packages/coding-agent/src/main.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/terminal.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/apps/pi/src/main.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- `pi-tui::ProcessTerminal` now has a real process-backed stdin path instead of only test-time/manual sequence injection
- Rust `ProcessTerminal::start()` now enables raw mode for the real process-backed terminal path, preserving the existing startup control writes for:
  - bracketed paste enable
  - Kitty keyboard-protocol query
- real stdin bytes now flow through the existing migrated `StdinBuffer` logic before reaching `Tui`, so the live terminal path preserves the already-migrated sequence splitting behavior for:
  - plain text input
  - escape/control sequences
  - bracketed paste
  - Kitty protocol response detection
- Rust now enables the timed modifyOtherKeys fallback for the real process-backed path, matching the current TypeScript terminal startup intent more closely when Kitty does not answer quickly
- `Terminal` is now implemented for `Box<T: Terminal + ?Sized>`, which lets downstream interactive app code inject terminal implementations while still using the generic `Tui<T>` surface

Compatibility note for this slice:
- the Rust real-process path now covers raw stdin/input callback integration honestly enough for the first end-user interactive app slice, but it still does not port the full TypeScript OS-integration surface yet
- live resize/signal callback parity remains deferred; the current Rust process terminal now relies on later input/render activity rather than a dedicated real resize callback path
- Windows-specific VT-input setup parity from the TypeScript terminal remains deferred
- the background stdin reader thread is intentionally simple and best-effort for the current migration slice; broader lifecycle/thread shutdown refinement can follow later if needed

### Rust design summary

Expanded `rust/crates/pi-tui/src/terminal.rs` with:
- process/raw-mode integration via `crossterm`
- shared backend/handler state for the real-process path
- a real stdin reader thread that feeds `StdinBuffer::process_bytes(...)`
- timed modifyOtherKeys fallback wiring for the real-process path
- a blanket `Terminal for Box<T>` impl for injected terminal usage in downstream interactive app tests/runtime code

Dependency change:
- `rust/crates/pi-tui/Cargo.toml`
  - added `crossterm`

### Validation summary

New Rust coverage added for:
- the existing `pi-tui` terminal unit tests still validating protocol-state behavior after the real stdin/raw-mode implementation changes
- downstream end-to-end interactive app validation through the new coding-agent interactive runner test that mounts `Tui<Box<dyn Terminal>>` and drives live shell input through the same callback path

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test -p pi-coding-agent-cli --test runner` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- live resize callback parity in the real process-backed terminal path
- Windows VT-input setup parity
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`
- richer widgets are still missing (`Box`, multiline editor, autocomplete, markdown/select/settings/image widgets)
- broader image/widget/theme parity remains deferred

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- either add the next honest real-terminal parity gap (`resize` / remaining OS integration) or continue upward into the multiline/custom-editor slice now that the interactive app can consume live stdin through the Rust terminal path
- keep broader markdown/image/theme parity deferred until the higher-value interaction/runtime gaps are closed first

## Milestone 19 update: live resize callback parity in the real process-backed terminal path

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/terminal.ts`
- `packages/tui/test/tui-render.test.ts` (resize section)
- `packages/tui/test/virtual-terminal.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/terminal.rs`
- `rust/crates/pi-tui/tests/tui.rs`
- previously migrated interactive-path grounding kept in scope:
  - `rust/crates/pi-coding-agent-cli/src/runner.rs`
  - `rust/apps/pi/src/main.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-tui::ProcessTerminal` no longer relies only on injected/mock resize callbacks; the real process-backed terminal path now has a live resize notification loop too
- the Rust process terminal now preserves the current observable resize behavior needed by the migrated TUI stack:
  - terminal size changes are detected after `start()`
  - the registered resize callback is invoked when the visible terminal dimensions actually change
  - unchanged repeated size observations do not emit redundant resize callbacks
  - resize notifications stop after `stop()`
- this closes the previously documented real-terminal gap where the Rust interactive path could read live stdin but still had no honest resize source outside tests

Compatibility note for this slice:
- TypeScript uses `process.stdout.on("resize", ...)`; Rust now preserves the same observable callback behavior through a small polling loop over terminal size instead of a platform-specific event hook
- the current Rust implementation is therefore callback-compatible but not a byte-for-byte port of the Node runtime mechanism

### Rust design summary

Expanded `rust/crates/pi-tui/src/terminal.rs` with:
- `RESIZE_POLL_INTERVAL`
- shared `last_known_size` state on `ProcessTerminal`
- `start_resize_poll_loop()` for backends that opt into resize polling
- backend helpers for:
  - reading size once under a single backend lock
  - checking whether a backend supports resize polling
- `TerminalBackend::supports_resize_polling()` with the default behavior tied to real process-backed terminals

Test-only design additions:
- a `ResizableMockBackend` that can mutate its size under test while reusing the real `ProcessTerminal` resize loop
- polling tests that freeze callback-on-change and stop-after-stop behavior

Design choices for this slice:
- keep resize detection inside `pi-tui::ProcessTerminal` instead of widening the higher-level `Tui` event model again
- use a small polling loop rather than introducing a platform-specific signal watcher dependency in the same milestone
- keep stdin/raw-mode behavior unchanged; this slice only closes the live resize callback gap

### Validation summary

New Rust coverage added for:
- resize callbacks firing when a resize-polling backend changes dimensions
- unchanged dimensions not firing duplicate resize callbacks
- resize polling stopping after `ProcessTerminal::stop()`
- existing `pi-tui` TUI tests continuing to validate queued resize rerenders once callbacks reach `Tui`

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --lib terminal` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- Windows VT-input setup parity beyond the current migrated slice
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`
- richer widgets are still missing (`Box`, multiline editor, autocomplete, markdown/select/settings/image widgets)
- broader image/widget/theme parity remains deferred

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- move up to the next honest interactive blocker above the now-live resize path, most likely the multiline/custom-editor slice
- keep broader markdown/image/theme parity deferred until the higher-value interaction/runtime gaps are closed first

## Milestone 20 update: minimal multiline editor slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/editor-component.ts`
- `packages/tui/src/components/editor.ts`
- `packages/tui/test/editor.test.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/input.rs`
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/tests/input.rs`
- `rust/crates/pi-tui/tests/tui.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-tui` now exports a first minimal multiline `Editor` widget plus `word_wrap_line(...)`
- the Rust editor preserves the current high-value TypeScript behavior needed for the first custom-editor / extension-editor migration slice:
  - multi-line text storage with `get_text()`, `set_text()`, `get_lines()`, and `get_cursor()`
  - insertion at the current cursor position via `insert_text_at_cursor(...)`
  - grapheme-aware left/right movement and backspace/delete behavior across line boundaries
  - line-start / line-end movement
  - word-left / word-right movement and matching word deletion helpers
  - `Enter` submit with reset-to-empty behavior and trimmed submitted text
  - the TypeScript backslash-before-enter newline workaround (`\\` immediately before Enter inserts a newline instead of submitting)
  - bracketed-paste buffering that preserves newlines and expands tabs to spaces
  - history storage plus up/down browsing through multi-line entries
  - word-wrapped rendering with top/bottom borders, viewport-aware vertical clipping, and `CURSOR_MARKER` emission when focused
- `word_wrap_line(...)` now freezes the first useful subset of the TypeScript wrapping behavior directly in Rust, including whitespace-boundary cases exercised by the TS editor tests

Compatibility note for this slice:
- this is intentionally the first honest multiline-editor foundation, not full `packages/tui/src/components/editor.ts` parity
- still deferred from the TypeScript editor:
  - autocomplete
  - kill ring / yank / yank-pop
  - undo stack parity
  - jump-to-char
  - sticky visual-column edge cases beyond the basic current implementation
  - large-paste marker substitution and marker-aware atomic editing
  - theme-driven border/select-list integration and the TS constructor shape that depends on `TUI`
- the current Rust `Editor` is therefore suitable as the first shared multiline text component, but not yet a drop-in replacement for the full TypeScript main interactive editor

### Rust design summary

New Rust module:
- `pi-tui::editor`
  - `Editor`
  - `EditorCursor`
  - `EditorOptions`
  - `TextChunk`
  - `word_wrap_line(...)`

Public API added in this milestone:
- `Editor::new()`
- `Editor::with_options(...)`
- `Editor::with_keybindings(...)`
- `Editor::with_keybindings_and_options(...)`
- `Editor::get_text()`
- `Editor::get_expanded_text()`
- `Editor::set_text(...)`
- `Editor::get_lines()`
- `Editor::get_cursor()`
- `Editor::insert_text_at_cursor(...)`
- `Editor::set_on_submit(...)` / `clear_on_submit()`
- `Editor::set_on_change(...)` / `clear_on_change()`
- `Editor::add_to_history(...)`
- `Editor::padding_x()` / `set_padding_x(...)`
- `Editor::is_showing_autocomplete()` (currently fixed `false` for the narrowed slice)
- crate exports via `pi_tui::{Editor, EditorCursor, EditorOptions, TextChunk, word_wrap_line}`

Implementation choices for this slice:
- keep the first Rust editor self-contained and keybinding-driven instead of pulling autocomplete, kill ring, undo, and select-list dependencies forward in the same milestone
- reuse the already-ported key parsing, width helpers, grapheme handling, `CURSOR_MARKER`, and component viewport hook instead of introducing a second rendering/input stack
- preserve the current TS editor behavior that is most valuable for downstream migration first: multi-line text, cursor movement, submit/newline semantics, paste handling, and wrapped rendering
- leave the richer TS editor internals deferred until a downstream coding-agent component actually needs them

### Validation summary

New Rust coverage added for:
- backslash-enter newline behavior without accidental submit
- submit/reset behavior with trimmed output
- backspace merging lines at the start of a line
- multi-line history navigation
- bracketed-paste newline preservation and tab expansion
- core `word_wrap_line(...)` whitespace-boundary behavior
- wide-text wrapped rendering with `CURSOR_MARKER` and width safety

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test editor` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- full TypeScript multiline-editor parity (autocomplete, undo, kill ring, paste markers, jump mode, richer sticky-column behavior)
- broader widget surface still missing (`Box`, markdown, select/settings/image widgets)
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`
- the Rust coding-agent interactive path still needs a component that actually consumes this new editor slice

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- consume the new `Editor` in the smallest downstream coding-agent component that genuinely needs multiline editing, most likely the Rust equivalent of the extension-editor/custom-editor path
- keep full main-editor parity deferred until that downstream consumer proves which of the remaining TS editor behaviors are actually needed next

## Milestone 21 update: first downstream multiline-editor consumer in coding-agent extension-editor

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/editor-component.ts`
- `packages/tui/src/components/editor.ts`
- `packages/tui/test/editor.test.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/extension_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/extension_editor.rs`
- `migration/packages/tui.md`
- `migration/packages/coding-agent.md`

### Behavior summary

New TS-grounded downstream behavior now covered:
- the migrated Rust `pi-tui::Editor` now has a real coding-agent consumer through `pi-coding-agent-tui::ExtensionEditorComponent`
- the downstream consumer now freezes the first high-value TypeScript extension-editor workflow on top of the Rust multiline editor:
  - prefilled multiline text editing
  - submit/cancel routing through coding-agent keybindings
  - default external-editor round-trip through a temp file when `VISUAL` / `EDITOR` is configured
  - host stop/start/rerender sequencing around the external-editor process
- this proves the current Rust `Editor` slice is viable for at least one real multiline coding-agent surface, while still leaving the broader TypeScript main-editor feature set deferred

Compatibility note for this slice:
- no new `pi-tui` widget behavior was required for this milestone; the value of this slice is proving downstream consumption and identifying the remaining editor gaps from a real coding-agent use site instead of a standalone widget test only

### Rust design summary

No `pi-tui` implementation changes were required in this milestone.

The downstream design now relies on:
- `pi-tui::Editor` as the multiline text engine
- `pi-coding-agent-tui::CustomEditor` as the coding-agent keybinding wrapper
- `pi-coding-agent-tui::ExtensionEditorComponent` as the first real multiline-editor consumer

### Validation summary

New downstream Rust coverage added for:
- extension-editor default external-editor round-trip on top of the Rust multiline editor
- extension-editor callback precedence and keybinding routing on top of the Rust multiline editor
- existing `pi-tui` multiline-editor tests continue to freeze the lower-level widget behavior directly

Validation run results:
- `cd rust && cargo test -p pi-coding-agent-tui --test extension_editor` passed
- `cd rust && cargo test -p pi-tui --test editor` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- full TypeScript main-editor parity remains incomplete (`autocomplete`, undo/kill-ring, paste markers, jump mode, richer sticky-column behavior)
- no broader widget parity yet (`Box`, markdown, select/settings/image widgets)
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- use the newly grounded extension-editor consumer to choose the next highest-value multiline-editor parity gap instead of broadening the editor spec in the abstract
- keep full main-editor parity deferred until another real downstream consumer proves it is necessary

## Milestone 22 update: multiline-editor kill-ring / yank / yank-pop slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/kill-ring.ts`
- targeted kill-ring/editor behavior sections from `packages/tui/src/components/editor.ts`
- targeted kill-ring regression coverage from `packages/tui/test/editor.test.ts`
- downstream consumer grounding kept in scope:
  - `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
  - `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-tui::Editor` now ports the first Emacs-style kill-ring slice from the TypeScript multiline editor
- the Rust editor now preserves the current high-value TypeScript kill/yank behavior needed by the first downstream multiline consumer:
  - `ctrl+w` / `alt+backspace` push backward word deletions into a kill ring
  - `alt+d` pushes forward word deletions into the same kill ring
  - `ctrl+u` pushes delete-to-line-start text or merged newlines into the kill ring
  - `ctrl+k` pushes delete-to-line-end text or merged newlines into the kill ring
  - `ctrl+y` yanks the most recent kill-ring entry at the cursor
  - `alt+y` now performs yank-pop cycling immediately after a yank when multiple entries exist
- consecutive kill operations now accumulate into a single kill-ring entry in the same prepend/append direction as TypeScript:
  - backward kills prepend
  - forward kills append
  - newline merges are preserved inside the accumulated killed text
- non-kill editing/navigation actions now break the yank-pop / accumulation chain in the migrated Rust slice, matching the TypeScript ownership of `lastAction` closely enough for the newly added regressions
- this closes one of the main multiline-editor gaps still visible after the extension-editor consumer landed, without widening the Rust editor to full autocomplete/undo parity yet

Current intentional limitation for this slice:
- this milestone ports kill-ring behavior only; undo-stack parity is still deferred
- the Rust editor still does not have TypeScript autocomplete, paste-marker handling, jump mode, or the fuller sticky-column edge behavior

### Rust design summary

New internal Rust module:
- `rust/crates/pi-tui/src/kill_ring.rs`

Expanded `pi-tui::Editor` with:
- internal `KillRing`
- internal `EditorAction::{Kill, Yank}` tracking
- new internal helpers for:
  - yank insertion
  - yank-pop replacement of the previously yanked text
- kill-ring-aware handling in:
  - `delete_word_backward()`
  - `delete_word_forward()`
  - `delete_to_line_start()`
  - `delete_to_line_end()`
- new keybinding handling for:
  - `tui.editor.yank`
  - `tui.editor.yankPop`

Design choices for this slice:
- keep the kill ring local to `pi-tui::Editor` instead of widening `pi-coding-agent-tui`, because the TypeScript source of truth defines this as generic editor behavior
- stop before undo-stack work so the milestone stays focused on the first multiline parity gap the extension-editor consumer makes visible
- keep the Rust implementation byte-oriented and deterministic around the already-migrated normalized text model rather than introducing a broader generalized editor-state framework early

### Validation summary

New Rust coverage added for:
- backward word kill plus yank restoration (`ctrl+w` + `ctrl+y`)
- delete-to-line-end kill plus yank restoration (`ctrl+k` + `ctrl+y`)
- yank-pop cycling across multiple kill-ring entries (`alt+y`)
- multiline kill accumulation across merged newlines
- forward word-kill accumulation via `alt+d`
- downstream proof that the existing Rust `ExtensionEditorComponent` tests still pass on top of the newly expanded multiline editor

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test editor` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test extension_editor` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- undo-stack parity is still missing from the multiline editor
- full TypeScript main-editor parity remains incomplete (`autocomplete`, paste markers, jump mode, richer sticky-column behavior)
- no broader widget parity yet (`Box`, markdown, select/settings/image widgets)
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- continue with the next highest-value multiline-editor parity gap now that the extension-editor consumer also has kill/yank behavior, most likely undo-stack parity
- keep broader widget parity and the full theme/markdown/image stack deferred until the next real downstream consumer requires them

## Milestone 23 update: multiline-editor undo-stack parity slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/tui/src/undo-stack.ts`
- targeted undo/editor behavior sections from `packages/tui/src/components/editor.ts`
- targeted undo regression coverage from `packages/tui/test/editor.test.ts`
- downstream consumer grounding kept in scope:
  - `packages/tui/src/editor-component.ts`
  - `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
  - `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/extension_editor.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-tui::Editor` now ports the first real undo-stack slice from the TypeScript multiline editor
- the Rust editor now preserves the current high-value TypeScript undo behavior needed by the already-migrated extension-editor consumer:
  - `ctrl+-` no-op behavior on an empty undo stack
  - fish-style typing coalescing where consecutive non-whitespace typing shares one undo unit
  - whitespace/newline boundaries starting new undo units
  - undo for backspace, forward delete, word deletion, and line-start/line-end kill operations
  - atomic undo for bracketed paste and `insert_text_at_cursor(...)`
  - programmatic `set_text(...)` changes becoming undoable when content actually changes
  - undo for yank operations on top of the existing kill-ring slice
  - first-entry history browsing snapshots so undo exits history mode back to the pre-browse buffer
  - submit clearing the undo stack, matching the TypeScript reset behavior
- cursor/navigation actions now intentionally break typing coalescing in the Rust editor, matching the current TypeScript `lastAction` semantics closely enough for the new regressions

Current intentional limitation for this slice:
- this milestone ports undo-stack behavior only for the already-migrated Rust editor surface
- the Rust editor still does not have TypeScript autocomplete, paste-marker handling, jump mode, or the fuller sticky-column edge behavior

### Rust design summary

New internal Rust module:
- `rust/crates/pi-tui/src/undo_stack.rs`

Expanded `pi-tui::Editor` with:
- internal `EditorSnapshot`
- internal `UndoStack<EditorSnapshot>`
- `EditorAction::TypeWord` for TS-style typing coalescing
- undo snapshot capture before:
  - structural edits
  - kill/yank edits
  - paste/programmatic insertion
  - first entry into history browsing
- explicit undo-stack clearing on submit

Design choices for this slice:
- keep the undo stack local to `pi-tui::Editor`, matching the TypeScript ownership boundary instead of widening `pi-coding-agent-tui`
- keep snapshots focused on normalized text/cursor state rather than over-modeling the full widget runtime, which stays aligned with the current TypeScript `UndoStack<EditorState>` usage
- stop before paste-marker/autocomplete/jump-mode work so the milestone stays focused on the highest-value remaining multiline parity gap proven by the downstream extension-editor consumer

### Validation summary

New Rust coverage added for:
- empty-stack undo no-op behavior
- typing coalescing and whitespace/newline undo boundaries
- undo for backspace and forward delete
- undo for kill/yank flows
- atomic undo for bracketed paste and `insert_text_at_cursor(...)`
- undo for programmatic `set_text(...)`
- history-browsing undo restoration
- undo-stack clearing on submit
- downstream proof that the existing Rust `ExtensionEditorComponent` tests still pass on top of the expanded multiline editor

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test editor` passed
- `cd rust && cargo test -p pi-tui` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test extension_editor` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- full TypeScript main-editor parity remains incomplete (`autocomplete`, paste markers, jump mode, richer sticky-column behavior)
- no broader widget parity yet (`Box`, markdown, select/settings/image widgets)
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- continue with the next editor gap that matters in real use, most likely large-paste marker/atomic behavior or jump-mode parity
- keep broader widget parity and the full theme/markdown/image stack deferred until the next real downstream consumer requires them

## Milestone 24 update: multiline-editor character jump slice

### Files analyzed

Additional TypeScript files read for this slice:
- targeted character-jump sections from `packages/tui/src/components/editor.ts`
- targeted character-jump regressions from `packages/tui/test/editor.test.ts`
- previously grounded downstream consumer context kept in scope:
  - `packages/tui/src/editor-component.ts`
  - `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
  - `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `migration/packages/tui.md`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-tui::Editor` now ports the first character-jump slice from the TypeScript multiline editor
- the Rust editor now preserves the current high-value `Ctrl+]` / `Ctrl+Alt+]` behavior needed by the migrated editor surface:
  - enter forward jump mode via `tui.editor.jumpForward`
  - enter backward jump mode via `tui.editor.jumpBackward`
  - jump to the next matching character on the current line or later lines
  - jump backward to the previous matching character on the current line or earlier lines
  - keep the cursor unchanged when no match exists
  - cancel jump mode when the same shortcut is pressed again
  - cancel jump mode on non-printable input and then fall through to normal editor handling
- jumping now resets the Rust editor's typing coalescing state, matching the TypeScript `lastAction = null` behavior closely enough for undo parity

Current intentional limitation for this slice:
- this milestone ports only the character-jump behavior; large-paste marker semantics and the remaining editor parity gaps are still deferred
- the Rust jump implementation currently uses the existing normalized/grapheme-aware cursor model instead of trying to mimic JavaScript string-index quirks beyond the tested cases

### Rust design summary

Expanded `pi-tui::Editor` with:
- internal `JumpMode::{Forward, Backward}`
- pending jump-mode state in the editor runtime
- `jump_to_char(...)` multi-line search helper
- top-of-input handling that gives jump mode the same priority/cancel behavior as the TypeScript editor before normal editing resumes

Design choices for this slice:
- keep jump handling local to `pi-tui::Editor`, matching the TypeScript ownership boundary instead of widening `pi-coding-agent-tui`
- preserve the TS interaction contract first, while reusing the Rust editor's existing grapheme-safe cursor helpers instead of introducing another cursor index model
- stop before paste-marker work so the milestone stays small and directly verifiable

### Validation summary

New Rust coverage added for:
- forward jump on the same line
- backward jump across multiple lines
- cancel on repeated jump shortcut
- cancel on escape with later normal character insertion
- no-match cursor stability
- undo coalescing reset after a jump

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-tui --test editor` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for `pi-tui`:
- large-paste marker/atomic editing parity is still missing from the multiline editor
- full TypeScript main-editor parity remains incomplete (`autocomplete`, richer sticky-column behavior, paste markers)
- no broader widget parity yet (`Box`, markdown, select/settings/image widgets)
- differential rendering parity remains partial relative to TS `packages/tui/src/tui.ts`

### Recommended next step

Stay in `packages/tui`, `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-tui`, and `rust/crates/pi-coding-agent-tui`:
- continue with the next editor gap that now has the clearest downstream value: large-paste marker/atomic behavior
- keep broader widget parity and the full theme/markdown/image stack deferred until the next real downstream consumer requires them
