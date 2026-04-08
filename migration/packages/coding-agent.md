# packages/coding-agent migration inventory

Status: milestone 7 model/bootstrap/runtime/message-conversion slices plus default read/bash/edit/write tool wiring in `rust/crates/pi-coding-agent-core` and `rust/crates/pi-coding-agent-tools`
Target crates: `rust/crates/pi-coding-agent-core`, `rust/crates/pi-coding-agent-tools`, later `rust/crates/pi-coding-agent-cli`, and `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript files read in full for the current coding-agent-core slices:
- `packages/coding-agent/README.md`
- `packages/coding-agent/src/core/model-resolver.ts`
- `packages/coding-agent/src/core/model-registry.ts`
- `packages/coding-agent/src/core/auth-storage.ts`
- `packages/coding-agent/src/core/defaults.ts`
- `packages/coding-agent/src/core/index.ts`
- `packages/coding-agent/src/core/resolve-config-value.ts`
- `packages/coding-agent/src/core/sdk.ts`
- `packages/coding-agent/src/core/agent-session-services.ts`
- `packages/coding-agent/src/core/messages.ts`
- `packages/coding-agent/src/core/tools/index.ts`
- `packages/coding-agent/src/core/tools/read.ts`
- `packages/coding-agent/src/core/tools/write.ts`
- `packages/coding-agent/src/core/tools/bash.ts`
- `packages/coding-agent/src/core/tools/edit.ts`
- `packages/coding-agent/src/core/tools/edit-diff.ts`
- `packages/coding-agent/src/core/tools/truncate.ts`
- `packages/coding-agent/src/core/tools/path-utils.ts`
- `packages/coding-agent/src/core/bash-executor.ts`
- `packages/coding-agent/src/core/exec.ts`
- `packages/coding-agent/src/core/output-guard.ts`
- `packages/coding-agent/src/main.ts`
- `packages/coding-agent/src/cli/args.ts`
- `packages/coding-agent/test/model-resolver.test.ts`
- `packages/coding-agent/test/model-registry.test.ts`
- `packages/coding-agent/test/auth-storage.test.ts`
- `packages/coding-agent/test/args.test.ts`

Rust files reviewed before and during implementation:
- `rust/Cargo.toml`
- `rust/crates/pi-coding-agent-core/Cargo.toml`
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-core/src/runtime.rs`
- `rust/crates/pi-coding-agent-core/tests/runtime.rs`
- `rust/crates/pi-coding-agent-tools/Cargo.toml`
- `rust/crates/pi-coding-agent-tools/src/lib.rs`
- `rust/crates/pi-coding-agent-tools/src/path_utils.rs`
- `rust/crates/pi-coding-agent-tools/src/truncate.rs`
- `rust/crates/pi-coding-agent-tools/src/read.rs`
- `rust/crates/pi-coding-agent-tools/src/bash.rs`
- `rust/crates/pi-coding-agent-tools/src/edit.rs`
- `rust/crates/pi-coding-agent-tools/src/write.rs`
- `rust/crates/pi-coding-agent-tools/tests/read_write.rs`
- `rust/crates/pi-coding-agent-tools/tests/bash_edit.rs`
- `rust/crates/pi-agent/src/agent.rs`
- `rust/crates/pi-agent/src/error.rs`
- `rust/crates/pi-agent/src/loop.rs`
- `rust/crates/pi-agent/src/message.rs`
- `rust/crates/pi-agent/src/state.rs`
- `rust/crates/pi-agent/src/tool.rs`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-events/src/lib.rs`
- `migration/packages/agent.md`

Note: this inventory is still intentionally partial. It covers coding-agent core startup/model/bootstrap/runtime/message-conversion behavior, not the full package, session manager, tools, CLI, or TUI.

## 2. Behavior inventory summary

Observed TypeScript behavior now covered by Rust slices:
- provider-ordered default model IDs
- exact `provider/model` matching
- fuzzy matching by id/name
- `:<thinking>` suffix parsing
- strict CLI handling vs fallback warning handling for invalid thinking suffixes
- provider inference from the first `/`
- OpenRouter-style ids containing `/` and `:`
- explicit-provider fallback to custom model IDs
- `models.json` loading for coding-agent registry state
- provider-level baseUrl overrides for built-in models
- custom-model merge/replace semantics by `provider + id`
- per-model overrides for built-in models (current Rust slice: name/reasoning/input/contextWindow/maxTokens)
- request-time provider/model header resolution
- request-time provider API-key resolution from literal values, env vars, and shell commands
- `getAvailable()` using configured-auth presence without executing command-backed keys
- core startup/bootstrap selection behavior combining registry + resolver + session/default inputs:
  - CLI model selection and CLI thinking shorthand
  - saved-default-in-scope selection when not continuing
  - existing-session model restore when auth is configured
  - fallback to default available model when restore fails
  - thinking-level restoration/defaulting rules
  - thinking clamp to `off` for non-reasoning models
  - diagnostics returned for CLI model-resolution warnings/errors
- minimal non-interactive runtime behavior:
  - construct a `pi-agent::Agent` from bootstrap-selected model/thinking state
  - carry system prompt into `AgentState`
  - resolve request auth/headers through `ModelRegistry` on every model stream call
  - stream through `pi-ai` providers using existing Rust `pi-agent` + `pi-ai` infrastructure
  - surface request-auth resolution failures as assistant error messages via `pi-agent` failure materialization
- coding-agent custom message conversion parity from `packages/coding-agent/src/core/messages.ts` for:
  - `bashExecution`
  - `custom`
  - `branchSummary`
  - `compactionSummary`
- `bashExecution.excludeFromContext` filtering during conversion to provider context
- end-to-end runtime installation of the coding-agent-specific `convert_to_llm` hook, so custom `pi-agent::AgentMessage::Custom` entries now reach the provider as user-context messages instead of being dropped
- initial Rust tool implementations for:
  - `read`
  - `bash`
  - `edit`
  - `write`
- default non-interactive runtime tool registration now provides `read`, `bash`, `edit`, and `write` when no explicit tools are supplied
- end-to-end tool-call execution through `pi-agent` + `pi-ai` faux provider for:
  - the `write` tool
  - the `edit` tool (including legacy `oldText` / `newText` argument preparation)

Still deferred:
- dynamic provider registration/unregistration
- OAuth refresh/login behavior
- auth.json persistence/locking
- compat/cost metadata parity
- `resolveModelScope()` glob expansion
- built-in model catalog sourcing from Rust `pi-ai`
- `blockImages` filtering wrapper from `packages/coding-agent/src/core/sdk.ts`
- session-manager/settings-manager/resource-loader integration beyond bootstrap selection
- CLI and TUI layers

## 3. Compatibility notes and edge cases

Confirmed from TypeScript code/tests and preserved in Rust where implemented:
- invalid `:<suffix>` is a warning only in scope-style parsing; strict CLI parsing treats it as part of the raw model id
- provider inference from the first `/` is preferred for inputs like `zai/glm-5`, even when another provider exposes a literal `zai/glm-5` id
- if provider inference fails for an OpenRouter-style id like `openai/gpt-4o:extended`, resolution retries the full raw id across all models
- partial matching prefers alias ids over dated ids, sorting descending within each class
- saved defaults and restored sessions differ intentionally: saved defaults ignore current auth availability, restored sessions require configured auth
- command-backed API keys are intentionally not executed by `getAvailable()`; presence of config is enough for availability filtering
- request-time API key/header resolution is intentionally uncached in registry paths, matching TS `getApiKeyForProvider()` / `getApiKeyAndHeaders()` behavior
- `bashExecution` is rendered as `Ran \`<command>\`` plus fenced output or `(no output)`, then optional cancellation / non-zero exit-code / truncation annotations
- `write` success text intentionally mirrors TS wording, including reporting JS-style string length as "bytes"
- `read` preserves TS offset/limit continuation notices, including the trailing-empty-line behavior caused by splitting text files on `\n`
- `bash` success/error text mirrors the TS final result wording for non-zero exits and timeout messaging
- `edit` supports both canonical `edits[]` input and the legacy top-level `oldText` / `newText` form through Rust-side argument preparation
- `custom` messages with string content become a single user text block; `custom` messages with block arrays preserve text/image blocks unchanged
- `branchSummary` and `compactionSummary` use the exact TS summary wrapper strings now exported from Rust
- unknown custom roles are filtered out of provider context, matching the TS behavior of dropping unsupported message types from conversion
- runtime streaming now mirrors the TS `streamFn` seam conceptually: model/auth selection lives in coding-agent core, while actual provider streaming stays in `pi-ai`

Current compatibility deviations:
- Rust custom-message transport uses typed payload structs serialized into `pi-agent::CustomAgentMessage` payloads rather than a TS-style declaration-merging type system
- malformed payloads for recognized custom roles are currently skipped during conversion rather than surfaced as explicit diagnostics
- the TS `blockImages` wrapper is not yet ported into the runtime path because settings-manager wiring is still deferred
- image auto-resize parity from TS `read.ts` is not yet ported; Rust currently returns supported images as-is
- macOS filename fallback parity is partial; Rust currently handles Unicode-space normalization, `@` stripping, `~` expansion, and one curly-quote / AM-PM variant, but not full NFD retry behavior
- write/edit file-mutation queue semantics are not yet ported; current Rust write/edit execution is direct
- bash output updates are not streamed incrementally through `AgentToolUpdateCallback` yet; Rust currently returns finalized command output only
- edit tool details do not yet include TS-style rendered unified diff metadata
- Rust registry still operates over injected built-in `Vec<Model>` snapshots rather than a Rust-side `pi-ai` built-in model catalog
- Rust does not yet carry TS `compat` and `cost` metadata through registry state
- Rust does not yet port TS dynamic provider registration, OAuth provider integration, or auth.json persistence/locking
- shell command execution currently uses platform shell invocation without TS-style timeout handling
- xhigh-capability clamping from the TS CLI path is not yet ported because Rust does not yet expose the corresponding model-capability helper

## 4. Rust target design for current slices

Implemented in `pi-coding-agent-core`:

### Model resolution
- `ModelCatalog`
- `DEFAULT_MODELS`
- `DEFAULT_THINKING_LEVEL`
- `ScopedModel`
- `ParsedModelResult`
- `ResolveCliModelResult`
- `InitialModelOptions`
- `InitialModelResult`
- `RestoreModelResult`
- functions:
  - `default_model_id_for_provider()`
  - `parse_thinking_level()`
  - `find_exact_model_reference_match()`
  - `parse_model_pattern()`
  - `resolve_cli_model()`
  - `find_initial_model()`
  - `restore_model_from_session()`

### Model registry subset
- `AuthSource` trait
- `MemoryAuthStorage` test/runtime stub
- uncached config-resolution helpers in `config_value.rs`
- `ModelRegistry` with:
  - built-in model injection
  - optional `models.json` path
  - `refresh()`
  - `get_error()`
  - `get_all()`
  - `get_available()`
  - `catalog()`
  - `find()`
  - `has_configured_auth()`
  - `get_api_key_for_provider()`
  - `get_api_key_and_headers()`
- `RequestAuth` result type for resolved request auth

### Startup bootstrap slice
- `bootstrap.rs`
- exported types:
  - `BootstrapDiagnosticLevel`
  - `BootstrapDiagnostic`
  - `ExistingSessionSelection`
  - `SessionBootstrapOptions`
  - `SessionBootstrapResult`
- exported function:
  - `bootstrap_session()`

### Coding-agent message conversion slice
- `messages.rs`
- exported constants:
  - `BRANCH_SUMMARY_PREFIX`
  - `BRANCH_SUMMARY_SUFFIX`
  - `COMPACTION_SUMMARY_PREFIX`
  - `COMPACTION_SUMMARY_SUFFIX`
- exported types:
  - `BashExecutionMessage`
  - `CustomMessage`
  - `CustomMessageContent`
  - `BranchSummaryMessage`
  - `CompactionSummaryMessage`
- exported helpers:
  - `bash_execution_to_text()`
  - `convert_to_llm()`
  - `create_bash_execution_message()`
  - `create_custom_message()`
  - `create_branch_summary_message()`
  - `create_compaction_summary_message()`
- design choice:
  - use strongly typed Rust payload structs serialized into `pi-agent::AgentMessage::Custom` payload JSON, then decode them back during LLM conversion

### Initial tool slice (`pi-coding-agent-tools`)
- `truncate.rs`
  - `DEFAULT_MAX_LINES`
  - `DEFAULT_MAX_BYTES`
  - `TruncationOptions`
  - `TruncationResult`
  - `format_size()`
  - `truncate_head()`
  - `truncate_tail()`
- `path_utils.rs`
  - `resolve_to_cwd()`
  - `resolve_read_path()`
- `read.rs`
  - `read_tool_definition()`
  - `create_read_tool()`
- `bash.rs`
  - `bash_tool_definition()`
  - `create_bash_tool()`
- `edit.rs`
  - `edit_tool_definition()`
  - `create_edit_tool()`
- `write.rs`
  - `write_tool_definition()`
  - `create_write_tool()`
- `lib.rs`
  - `create_read_write_tools()`
  - `create_coding_tools()`

### Minimal runtime slice
- `runtime.rs`
- exported types:
  - `CodingAgentCoreOptions`
  - `CreateCodingAgentCoreResult`
  - `CodingAgentCore`
- exported function:
  - `create_coding_agent_core()`
- runtime methods:
  - `agent()`
  - `model_registry()`
  - `state()`
  - `prompt_text()`
  - `prompt_message()`
  - `continue_turn()`
  - `abort()`
  - `wait_for_idle()`
- runtime integration:
  - installs `convert_to_llm()` on the embedded `pi-agent::Agent`, so custom coding-agent messages now participate in provider context generation automatically
  - accepts optional `cwd` and explicit `tools`
  - defaults to `pi-coding-agent-tools::create_coding_tools(cwd)` when tools are not provided

Design choices for these milestones:
- reuse `pi_agent::ThinkingLevel` for agent parity
- reuse `pi_events::Model` rather than inventing a second model type for the current slice
- keep auth persistence/OAuth out of scope for now behind `AuthSource`
- keep registry side-effect-free apart from explicit disk reads and shell/env resolution
- keep CLI/process exits out of core; warnings/errors return as diagnostics
- make bootstrap pure and non-interactive so later CLI/runtime layers can call it directly
- make runtime reuse `pi-agent::Agent` directly instead of building a parallel state machine in coding-agent core
- keep custom message support minimal and local to coding-agent-core instead of widening `pi-agent` with coding-agent-specific roles
- put initial filesystem/shell tools in `pi-coding-agent-tools` rather than bloating core with file IO logic
- keep the first tool slice intentionally focused on the default coding tools only, not the optional read-only set

## 5. Validation plan / test coverage

Rust regression coverage now mirrors TypeScript behavior for:
- exact model matching
- fuzzy matching
- OpenRouter-style ids with `/` and `:`
- invalid thinking-level fallback warnings
- provider inference vs gateway-style ids
- explicit-provider custom-id fallback
- initial model default ordering
- restore fallback behavior
- baseUrl-only provider overrides
- custom-model merge and replacement semantics
- built-in model override application
- refresh reloading from disk
- invalid `models.json` fallback to built-ins
- uncached command-backed provider key resolution
- `getAvailable()` not executing command-backed keys
- request auth/header merging including `authHeader`
- error reporting for failed request-time auth resolution
- startup bootstrap selection for:
  - CLI model + shorthand thinking
  - explicit CLI thinking override
  - saved default selection within scoped models
  - existing-session restore with configured auth
  - restore fallback to available default model
  - existing-session thinking fallback rules
  - surfaced CLI-resolution diagnostics
- runtime creation and prompt flow for:
  - successful faux-provider streaming through `pi-agent`
  - selected model carrying `models.json` overrides into runtime state
  - prompt-time auth resolution failure materializing as assistant error message
  - no-model startup failure path
  - custom coding-agent messages affecting provider prompt context through the installed conversion hook
- coding-agent message conversion for:
  - bash execution text formatting
  - `excludeFromContext` skipping
  - custom string payloads
  - custom text/image block payloads
  - branch summary wrappers
  - compaction summary wrappers
  - unknown custom-role filtering
- runtime creation and prompt flow for:
  - successful default read/bash/edit/write tool registration
  - end-to-end `write` tool execution through a faux-provider tool-call turn
  - end-to-end `edit` tool execution through a faux-provider tool-call turn
  - end-to-end legacy `oldText` / `newText` edit argument preparation through agent tool execution
- tool unit coverage for:
  - read offset/limit continuation behavior
  - read offset out-of-bounds errors
  - read supported-image detection and attachment conversion
  - write parent-directory creation and JS-length success text
  - bash success output
  - bash non-zero exit handling
  - bash timeout handling
  - edit multi-replacement application
  - edit duplicate-match detection
  - edit legacy argument preparation

Deferred to later milestones:
- full auth storage port (`auth.json`, runtime overrides, persistence, locking, OAuth refresh)
- dynamic provider lifecycle APIs
- model compat/cost metadata behavior
- scope globbing and settings/session wiring
- `blockImages` filtering parity
- CLI behavior and TUI rendering/state tests

## 6. Known risks / open questions

- Rust still lacks a built-in model catalog sourced from `packages/ai`; callers currently inject built-in models
- `pi_events::Model` is narrower than TS `Model<Api>` and will likely need widening or side metadata for compat/cost-sensitive behavior
- the current `AuthSource` seam is intentionally minimal and may need reshaping when auth.json/OAuth support is ported
- `resolveModelScope()` glob semantics remain unported, so model cycling/config scope is still incomplete
- `blockImages` remains separate from the current runtime path until settings-manager state exists in Rust
- runtime currently exposes `pi-agent::Agent` directly; later milestones need to decide whether to keep that as the primary core API or wrap it in a coding-agent-specific session/runtime facade
- bash execution currently favors simple finalized output parity over TS-style live partial updates and shell backend customization hooks
- edit replacement logic now covers uniqueness/overlap/legacy args, but diff-detail parity remains incomplete

## 7. Recommended next step

Move to the next non-interactive coding-agent layer:
- start the `pi-coding-agent-cli` print-mode slice on top of the current core + default tools
- keep it session-manager-free and TUI-free for now
- only revisit the `blockImages` wrapper from `sdk.ts` if the CLI path immediately needs attachment filtering
