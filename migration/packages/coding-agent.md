# packages/coding-agent migration inventory

Status: milestone 10 adds Rust `pi-ai` built-in model catalog sourcing from TypeScript `models.generated.ts`, broadens env API-key lookup coverage, and wires `rust/apps/pi` to use `pi_ai::built_in_models()` instead of an empty catalog.
Target crates: `rust/crates/pi-coding-agent-core`, `rust/crates/pi-coding-agent-tools`, `rust/crates/pi-coding-agent-cli`, and later `rust/crates/pi-coding-agent-tui`

## 1. Files analyzed

TypeScript files read in full for the current CLI runner slice:
- `packages/coding-agent/README.md`
- `packages/coding-agent/src/main.ts`
- `packages/coding-agent/src/cli.ts`
- `packages/coding-agent/src/config.ts`
- `packages/coding-agent/src/cli/args.ts`
- `packages/coding-agent/src/cli/initial-message.ts`
- `packages/coding-agent/src/cli/file-processor.ts`
- `packages/coding-agent/src/cli/list-models.ts`
- `packages/coding-agent/src/modes/print-mode.ts`
- `packages/coding-agent/src/core/output-guard.ts`
- `packages/coding-agent/src/core/tools/path-utils.ts`
- `packages/coding-agent/src/core/tools/read.ts`
- `packages/coding-agent/test/args.test.ts`
- `packages/coding-agent/test/initial-message.test.ts`
- `packages/coding-agent/test/print-mode.test.ts`
- `packages/coding-agent/test/path-utils.test.ts`
- `packages/coding-agent/test/tools.test.ts`
- `packages/coding-agent/test/stdout-cleanliness.test.ts`
- `packages/coding-agent/test/image-processing.test.ts`

Previously analyzed TypeScript files still relevant to this slice:
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
- `packages/coding-agent/src/core/tools/write.ts`
- `packages/coding-agent/src/core/tools/bash.ts`
- `packages/coding-agent/src/core/tools/edit.ts`
- `packages/coding-agent/src/core/tools/edit-diff.ts`
- `packages/coding-agent/src/core/tools/truncate.ts`
- `packages/coding-agent/src/core/bash-executor.ts`
- `packages/coding-agent/src/core/exec.ts`
- `packages/coding-agent/test/model-resolver.test.ts`
- `packages/coding-agent/test/model-registry.test.ts`
- `packages/coding-agent/test/auth-storage.test.ts`

Rust files reviewed before and during implementation:
- `rust/Cargo.toml`
- `rust/Cargo.lock`
- `rust/apps/pi/Cargo.toml`
- `rust/apps/pi/src/main.rs`
- `rust/crates/pi-coding-agent-cli/Cargo.toml`
- `rust/crates/pi-coding-agent-cli/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/src/args.rs`
- `rust/crates/pi-coding-agent-cli/src/initial_message.rs`
- `rust/crates/pi-coding-agent-cli/src/print_mode.rs`
- `rust/crates/pi-coding-agent-core/Cargo.toml`
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-core/src/auth.rs`
- `rust/crates/pi-coding-agent-core/src/bootstrap.rs`
- `rust/crates/pi-coding-agent-core/src/model_registry.rs`
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/src/runtime.rs`
- `rust/crates/pi-coding-agent-core/tests/bootstrap.rs`
- `rust/crates/pi-coding-agent-core/tests/runtime.rs`
- `rust/crates/pi-coding-agent-tools/Cargo.toml`
- `rust/crates/pi-coding-agent-tools/src/lib.rs`
- `rust/crates/pi-coding-agent-tools/src/path_utils.rs`
- `rust/crates/pi-coding-agent-tools/src/read.rs`
- `rust/crates/pi-agent/Cargo.toml`
- `rust/crates/pi-agent/src/lib.rs`
- `rust/crates/pi-agent/src/agent.rs`
- `rust/crates/pi-agent/src/loop.rs`
- `rust/crates/pi-agent/src/message.rs`
- `rust/crates/pi-agent/src/state.rs`
- `rust/crates/pi-agent/src/tool.rs`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-events/src/lib.rs`
- `migration/packages/coding-agent.md`
- `migration/notes/coding-agent-next-prompt.md`

Note: this inventory is still intentionally partial. It now covers coding-agent model/bootstrap/runtime/message-conversion/default-tool slices, the Rust print-mode CLI library slice, and the first top-level non-interactive runner, but not the full TS package, session manager, resource loader, extensions, or TUI.

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
- per-model overrides for built-in models (name/reasoning/input/contextWindow/maxTokens)
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
- end-to-end runtime installation of the coding-agent-specific `convert_to_llm` hook, so custom `pi-agent::AgentMessage::Custom` entries reach the provider as user-context messages instead of being dropped
- initial Rust tool implementations for:
  - `read`
  - `bash`
  - `edit`
  - `write`
- default non-interactive runtime tool registration now provides `read`, `bash`, `edit`, and `write` when no explicit tools are supplied
- end-to-end tool-call execution through `pi-agent` + `pi-ai` faux provider for:
  - the `write` tool
  - the `edit` tool (including legacy `oldText` / `newText` argument preparation)
- first Rust CLI-side non-interactive behaviors from `packages/coding-agent/src/cli/args.ts`, `cli/initial-message.ts`, and `modes/print-mode.ts`:
  - parse core print-mode flags and diagnostics (`--print`, `--mode`, `--provider`, `--model`, `--api-key`, `--system-prompt`, `--append-system-prompt`, `--thinking`, `@file`, message args, unknown long flags)
  - preserve TS warning/error semantics for invalid thinking levels, unknown tools, and unknown short flags
  - resolve app mode the same way as TS (`rpc` > `json` > print when `-p` or stdin is piped > interactive)
  - merge stdin text, file text, and the first CLI message into one initial prompt while shifting the remaining messages forward
  - preserve file-image passthrough in the initial-message builder result
  - run print mode directly against `pi-coding-agent-core::CodingAgentCore`
  - in text mode, emit only final assistant text blocks, each newline-terminated
  - in text mode, treat assistant `error` / `aborted` stop reasons as exit code `1` with stderr fallback text
  - in json mode, serialize buffered `pi-agent` event sequences as newline-delimited JSON without requiring session-manager wiring
- newly covered top-level non-interactive runner behaviors:
  - parse argv, normalize piped stdin the TS way (`trim()` + drop empty input), and feed the result into initial-message assembly
  - basic `@file` preprocessing for text files and supported images before print-mode execution
  - CLI `--api-key` runtime override applied to the resolved request provider before streaming
  - non-interactive error rendering for parse diagnostics, bootstrap diagnostics, unsupported flags, and no-model startup
  - minimal `--help` and `--version` handling in the Rust CLI path
  - minimal `rust/apps/pi` entrypoint that forwards argv/stdin/stdout/stderr into the Rust runner
- newly covered AI/catalog behaviors now consumed by coding-agent:
  - Rust `pi-ai` built-in model catalog parsed directly from TypeScript `packages/ai/src/models.generated.ts`
  - Rust `rust/apps/pi` now injects `pi_ai::built_in_models()` into the coding-agent runtime path instead of an empty catalog
  - broader env API-key lookup coverage now feeds coding-agent model availability checks through `EnvAuthSource`

Still deferred:
- session-manager/settings-manager/resource-loader integration
- extension lifecycle and session headers in JSON mode
- `blockImages` filtering wrapper from `packages/coding-agent/src/core/sdk.ts`
- CLI list-models output and export mode
- interactive mode and TUI layers

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
- CLI initial-message building preserves the TS mutation behavior of consuming only the first message into `initialMessage`
- text print mode writes only assistant text blocks and ignores non-text assistant content, matching TS `runPrintMode()`
- text print mode uses `assistant.errorMessage ?? "Request <stopReason>"` behavior for assistant `error` / `aborted` messages
- top-level stdin handling now mirrors TS `readPipedStdin()` by trimming trailing/leading whitespace and treating empty stdin as absent
- text `@file` arguments are embedded with the same `<file name="...">...</file>` envelope shape as TS

Current compatibility deviations:
- Rust json print mode still emits serialized `pi-agent` events only; it does not include TS session-manager JSON headers or extension/session wrapper events
- Rust json print mode buffers lines until the run completes instead of writing directly to stdout as events arrive
- Rust help text is currently a short migration-oriented help block, not TS full help output
- the Rust runner explicitly rejects unsupported flags (`--models`, session flags, resource flags, `--list-models`, `--export`) instead of partially emulating TS behavior
- Rust `@file` image preprocessing currently attaches supported images without TS auto-resize and without dimension-note text
- Rust `@file` preprocessing currently uses magic-byte image detection but does not yet port the full TS image-resize pipeline
- `rust/apps/pi` now uses the Rust `pi-ai` built-in catalog, but that catalog is still a migration-time parse of the TS generated source rather than a Rust-native generated artifact
- app-side auth coverage is broader now, but it still does not reach full TS parity for every provider/auth mode (for example OAuth-backed flows and some cloud-specific credential chains remain incomplete)
- CLI `--api-key` override is currently wired for explicit `--model` flows only; TS `--models` scoped-model interactions remain deferred with the rest of model-scope support
- malformed payloads for recognized custom roles are currently skipped during conversion rather than surfaced as explicit diagnostics
- the TS `blockImages` wrapper is not yet ported into the runtime path because settings-manager wiring is still deferred
- image auto-resize parity from TS `read.ts` is not yet ported; Rust currently returns supported images as-is
- macOS filename fallback parity is partial; Rust currently handles Unicode-space normalization, `@` stripping, `~` expansion, and a curly-quote / AM-PM variant, but not full TS NFD retry behavior in the Rust path-utils slice
- write/edit file-mutation queue semantics are not yet ported; current Rust write/edit execution is direct
- bash output updates are not streamed incrementally through `AgentToolUpdateCallback` yet; Rust currently returns finalized command output only
- edit tool details do not yet include full TS-style rendered unified diff metadata
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
  - `detect_supported_image_mime_type()`
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
  - re-exports for `resolve_read_path()` / `resolve_to_cwd()` and image detection helpers used by the CLI slice

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

### CLI library slices (`pi-coding-agent-cli`)
- `args.rs`
  - `Mode`
  - `PrintOutputMode`
  - `AppMode`
  - `DiagnosticKind`
  - `Diagnostic`
  - `ToolName`
  - `ListModels`
  - `UnknownFlagValue`
  - `Args`
  - `is_valid_thinking_level()`
  - `parse_thinking_level()`
  - `parse_args()`
  - `resolve_app_mode()`
  - `to_print_output_mode()`
- `initial_message.rs`
  - `InitialMessageResult`
  - `build_initial_message()`
- `print_mode.rs`
  - `PrintModeOptions`
  - `PrintModeRunResult`
  - `run_print_mode()`
- new modules in this milestone:
  - `auth.rs`
    - `OverlayAuthSource`
    - `EnvAuthSource`
  - `file_processor.rs`
    - `ProcessedFiles`
    - `process_file_arguments()`
  - `runner.rs`
    - `RunCommandOptions`
    - `RunCommandResult`
    - `run_command()`
- design choices:
  - keep the Rust CLI session-manager-free and TUI-free for now
  - make the runner reuse `create_coding_agent_core()` directly instead of introducing a second orchestration stack
  - keep unsupported flags explicit rather than silently ignoring them
  - reuse tool/path helper code from `pi-coding-agent-tools` instead of duplicating path resolution or image signature logic again
  - keep stdout/stderr buffered in the library so tests can assert exact behavior without running a subprocess

### Minimal binary scaffold (`rust/apps/pi`)
- `src/main.rs`
  - forwards argv/stdin into `run_command()`
  - writes buffered stdout/stderr and returns `ExitCode`
  - resolves `PI_CODING_AGENT_DIR` / default `~/.pi/agent/models.json`
- current design intentionally keeps the app thin and lets `pi-coding-agent-cli` own non-interactive behavior

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
- CLI slice coverage for:
  - print-mode flag parsing
  - tool/unknown-flag/thinking diagnostics
  - `--list-models` optional search parsing
  - app-mode resolution parity
  - initial-message merge/mutation behavior
  - text-mode final assistant rendering
  - json-mode agent-event serialization through tool execution
  - assistant-error exit code handling in text mode
- new runner coverage in this milestone for:
  - `--api-key` runtime override reaching the provider stream options
  - merged stdin + `@file` text + `@file` image + first-message prompt construction through the full runner path
  - interactive-mode rejection in the current Rust app slice
  - top-level `--api-key` usage error when no explicit model is supplied

Deferred to later milestones:
- full auth storage port (`auth.json`, runtime overrides beyond in-memory/env tests, persistence, locking, OAuth refresh)
- dynamic provider lifecycle APIs
- model compat/cost metadata behavior
- scope globbing and settings/session wiring
- `blockImages` filtering parity
- full help/list-models/export/session CLI behavior
- TUI rendering/state tests

## 6. Known risks / open questions

- built-in catalog sourcing now exists, but it is implemented by parsing the TS generated source at runtime, which is acceptable for migration but not the likely final form
- `EnvAuthSource` coverage is broader now, but it still trails TS for full provider/auth parity
- the current runner rejects many flags explicitly; that keeps behavior honest, but it means the Rust binary is still far from TS CLI surface parity
- JSON print mode will need reshaping once session-manager/runtime wrapper events are ported, otherwise downstream consumers may bind to the temporary agent-event schema
- the current `@file` path only ports basic text/image preprocessing; TS image resizing, dimension notes, and some path-resolution edge cases still need work
- top-level help/version are now wired, but help text is intentionally minimal and not yet a TS-compatible snapshot
- runtime currently exposes `pi-agent::Agent` directly; later milestones still need to decide whether to keep that as the primary core API or wrap it in a coding-agent-specific session/runtime facade
- bash execution currently favors simple finalized output parity over TS-style live partial updates and shell backend customization hooks
- edit replacement logic now covers uniqueness/overlap/legacy args, but diff-detail parity remains incomplete

## 7. Recommended next step

Stay in `packages/coding-agent`, `rust/crates/pi-coding-agent-cli`, `rust/crates/pi-coding-agent-core`, and `rust/crates/pi-ai`:
- port `--list-models` on top of the new Rust-backed catalog and current registry/auth path
- then continue filling in provider/auth parity gaps that still affect model availability and startup selection
- keep session-manager and TUI work deferred until the non-interactive CLI surface is broader
