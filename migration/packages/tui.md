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
