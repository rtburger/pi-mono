# Rust rewrite TODO

Date: 2026-04-15
Status: active

Completed in this slice:
- Prompt stack foundation for the Rust `pi` app
- Default system prompt builder wired into `rust/apps/pi`
- `AGENTS.md` / `CLAUDE.md` context discovery
- `.pi/SYSTEM.md` and `.pi/APPEND_SYSTEM.md` loading with project-over-global precedence
- File-backed `--system-prompt` / `--append-system-prompt` resolution
- Validation completed with `cargo test --workspace` and `npm run check`

Scope reminder:
- Rust code lives under `rust/`
- Planning notes live under `migration/`
- TypeScript remains the behavior reference
- Active migration target: `packages/coding-agent`
- `pi-ai` is validation/regression-only for the current OpenAI Codex + Anthropic Claude Code scope unless a real mismatch is found

## Grounding

- [x] Confirm the existing Rust workspace under `rust/` before adding work
- [x] Confirm current migration status against the TypeScript source and migration notes
- [ ] Keep re-checking `rust/` first before each new slice so completed work is not duplicated

## P0

### packages/coding-agent

#### Session/history
- [ ] Port `session-manager.ts`
- [ ] Port `agent-session.ts`
- [ ] Port `agent-session-runtime.ts`
- [ ] Add JSONL persistence
- [ ] Add resume/new/fork/session tree flows
- [ ] Persist labels, session info, session naming, model/thinking changes, and custom entries

#### Prompt stack
- [x] Build a Rust default pi system prompt instead of starting with an empty prompt
- [x] Load `AGENTS.md` / `CLAUDE.md` from `~/.pi/agent` and ancestor directories
- [x] Load `.pi/SYSTEM.md` and `.pi/APPEND_SYSTEM.md` with project-over-global precedence
- [x] Resolve `--system-prompt` / `--append-system-prompt` file inputs when they point at files
- [ ] Surface loaded prompt/context resources in the interactive startup UI

#### Auth UX
- [ ] Implement Anthropic subscription login flow
- [ ] Implement OpenAI Codex subscription login flow
- [ ] Add logout/auth management commands and UI

#### Interactive command layer
- [ ] `/login`
- [ ] `/logout`
- [ ] `/scoped-models`
- [ ] `/settings`
- [ ] `/resume`
- [ ] `/new`
- [ ] `/name`
- [ ] `/session`
- [ ] `/tree`
- [ ] `/fork`
- [ ] `/compact`
- [ ] `/copy`
- [ ] `/export`
- [ ] `/share`
- [ ] `/reload`
- [ ] `/hotkeys`
- [ ] `/changelog`

#### CLI/modes
- [ ] Session CLI flags: `-c`, `-r`, `--no-session`, `--session`, `--fork`, `--session-dir`
- [ ] `--export`
- [ ] RPC mode parity
- [ ] JSON mode parity with TS event names and session metadata
- [ ] Decide and implement `--offline` behavior

#### Built-in tools
- [ ] Port `grep`
- [ ] Port `find`
- [ ] Port `ls`
- [ ] Implement tool selection flags and read-only mode

#### UI parity
- [ ] Markdown assistant rendering
- [ ] Inline image rendering
- [ ] Tool/thinking expansion toggles
- [ ] Footer live state wiring
- [ ] Startup header context/resource summary

### packages/tui
- [ ] Differential renderer parity
- [ ] Synchronized output support
- [ ] `Markdown` component
- [ ] `SelectList` component
- [ ] `SettingsList` component
- [ ] `Loader` component
- [ ] `CancellableLoader` component
- [ ] `Box` component
- [ ] `Image` component
- [ ] Theme interfaces and runtime theme support

## P1

### packages/coding-agent
- [ ] Full settings model parity and bootstrap wiring
- [ ] Compaction engine and branch summarization
- [ ] Keybinding action wiring parity
- [ ] Clipboard text copy
- [ ] HTML export
- [ ] Share/gist workflow
- [ ] Resource loader stack: prompt templates, skills, extensions, themes, packages
- [ ] Package manager commands (`install`, `remove`, `update`, `list`, `config`)

### packages/ai
- [ ] Public convenience APIs matching the TS provider entry points
- [ ] Anthropic public options parity (`toolChoice`, `interleavedThinking`)
- [ ] OAuth/login API surface parity
- [ ] Custom model `cost` metadata support
- [ ] Custom model `compat` support and merge semantics
- [ ] Real OpenAI-completions compat detection/override handling

### packages/agent
- [ ] Keep parity validation running while the product layer catches up

## Validation

- [x] `cargo test --workspace`
- [x] `npm run check`
