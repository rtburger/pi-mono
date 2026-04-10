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
- CLI export mode
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
- the Rust runner explicitly rejects unsupported session/resource/export flags instead of partially emulating the full TS CLI surface
- Rust `@file` image preprocessing currently attaches supported images without TS auto-resize and without dimension-note text
- Rust `@file` preprocessing currently uses magic-byte image detection but does not yet port the full TS image-resize pipeline
- `rust/apps/pi` now uses the Rust `pi-ai` built-in catalog, but that catalog is still a migration-time parse of the TS generated source rather than a Rust-native generated artifact
- app-side auth coverage is broader now, but it still does not reach full TS parity for every provider/auth mode (for example OAuth-backed flows and some cloud-specific credential chains remain incomplete)
- CLI `--api-key` override now covers explicit `--model` flows and the current first-scoped-model `--models` path, but settings/session-backed scoped-model flows remain deferred
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
- settings/session-backed scoped-model wiring
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

## Milestone 11 update: `--list-models`

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/main.ts` (list-models dispatch path)
- `packages/coding-agent/docs/models.md`
- `packages/tui/README.md`
- `packages/tui/src/index.ts`
- `packages/tui/src/fuzzy.ts`
- `packages/tui/test/fuzzy.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/Cargo.toml`
- `rust/crates/pi-coding-agent-cli/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- top-level `--list-models` now exits successfully without requiring `--print` or entering interactive mode
- list-model rendering uses the current Rust `ModelRegistry::get_available()` path, so output is filtered by configured auth and `models.json` provider API-key configuration
- optional fuzzy search now follows the TS `packages/tui/src/fuzzy.ts` rules, including whitespace-token matching and the swapped alphanumeric fallback (`codex52` -> `5.2-codex` style matches)
- list output matches the TS column set and ordering semantics:
  - provider
  - model id
  - context window
  - max output
  - thinking support
  - image support
- token counts now use TS-style compact formatting (`K` / `M`, one decimal when needed)
- empty availability and no-match cases now use the TS list-model messages instead of the generic startup failure path

### Rust design summary

New Rust slices added:
- `rust/crates/pi-tui/src/fuzzy.rs`
  - `FuzzyMatch`
  - `fuzzy_match()`
  - `fuzzy_filter()`
- `rust/crates/pi-coding-agent-cli/src/list_models.rs`
  - list rendering over `ModelRegistry`
  - TS-style compact token-count formatting
  - unit tests covering table output, fuzzy search, no-match, and no-available-model cases
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
  - early `--list-models` handling before app-mode rejection and print-mode setup
  - minimal help text updated to advertise `--list-models [search]` as supported

### Validation summary

New Rust coverage added for:
- `pi-tui` fuzzy matching parity cases ported from `packages/tui/test/fuzzy.test.ts`
- coding-agent CLI list-model rendering and search behavior
- runner-level proof that `--list-models` bypasses the current interactive-mode rejection path

### Remaining gaps after this milestone

Still deferred for the CLI/model surface:
- `--models` scoped-model parsing/execution in the Rust runner
- session-manager/settings/resource-loader-backed list/help parity
- TS full help text
- JSON/session wrapper parity in print mode
- broader auth/OAuth parity that can still affect which providers appear as available

## Milestone 12 update: `--models` scoped-model slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/core/model-resolver.ts`
- `packages/coding-agent/src/main.ts` (scoped-model/session-option path)

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/Cargo.toml`
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/src/bootstrap.rs`
- `rust/crates/pi-coding-agent-core/tests/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/tests/bootstrap.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`

### Behavior summary

New TS-compatible behaviors now covered in Rust:
- `--models` is no longer rejected by the Rust runner
- scoped-model resolution now follows the TS `resolveModelScope()` split:
  - non-glob patterns use the existing model-pattern parser
  - glob patterns support matching against both `provider/modelId` and bare `modelId`
  - valid `:<thinking>` suffixes on glob patterns are preserved
  - invalid thinking suffixes on non-glob scope patterns become warnings, not hard errors
  - duplicate matches keep the first resolved scoped entry, matching TS dedupe behavior
- non-interactive startup now passes scoped models into bootstrap selection, so a scope can choose the initial model even without `--model`
- CLI `--api-key` overrides now work with a selected scoped model in the current non-interactive Rust path when the scope resolves to an available model
- saved-default-in-scope comparison in Rust bootstrap now matches TS `modelsAreEqual()` semantics (`provider + id`, not `api`)

### Rust design summary

New/expanded Rust slices:
- `pi-coding-agent-core::resolve_model_scope()`
  - returns scoped models plus warning messages
  - uses `globset` for case-insensitive glob matching
- `pi-coding-agent-cli::runner`
  - resolves scopes from the current registry `get_available()` set before core creation
  - emits scope warnings through stderr in the same warning style as other CLI diagnostics
  - passes scoped models into `SessionBootstrapOptions`
  - applies `--api-key` to the first scoped model when no explicit `--model` was supplied

### Validation summary

New Rust coverage added for:
- scoped-model resolution with glob patterns and glob thinking suffixes
- duplicate-scope handling and scope warning behavior
- runner-level initial-model selection from `--models`
- runner-level `--api-key` override when `--models` selects the initial model

### Remaining gaps after this milestone

Still deferred for the CLI/model surface:
- settings-manager-backed enabled-model scopes (`settingsManager.getEnabledModels()`)
- interactive scoped-model cycling and session-manager integration
- full TS help output and startup messaging around scoped models
- broader auth parity for providers where availability still depends on unported OAuth/cloud auth flows

## Milestone 13 update: CLI xhigh startup clamp parity

### Files analyzed

Additional TypeScript behavior reviewed for this slice:
- `packages/coding-agent/src/main.ts` (post-startup CLI thinking clamp path)
- `packages/coding-agent/src/core/agent-session.ts` (thinking-level clamp semantics)
- `packages/ai/src/models.ts` (`supportsXhigh()` behavior)
- `packages/coding-agent/test/model-resolver.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/bootstrap.rs`
- `rust/crates/pi-coding-agent-core/tests/bootstrap.rs`
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/tests/models.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- non-interactive startup now clamps explicit CLI-requested `xhigh` thinking to `high` when the selected startup model is reasoning-capable but not `supportsXhigh()`-capable
- the clamp applies to both CLI forms that trigger the TypeScript post-startup clamp path:
  - `--thinking xhigh`
  - `--model <pattern>:xhigh`
- xhigh is preserved for startup models that are xhigh-capable (for example Claude Opus 4.6)
- the existing startup clamp to `off` for non-reasoning models is unchanged
- no new warning/diagnostic text is emitted for the clamp, matching the current TypeScript startup behavior

### Rust design summary

Implementation stays in `pi-coding-agent-core::bootstrap_session()`:
- track whether startup thinking came from an explicit CLI source (`--thinking` or CLI model shorthand)
- after final model selection and default-thinking resolution, apply a narrow xhigh capability clamp using `pi_ai::supports_xhigh()`
- keep the clamp intentionally scoped to the current CLI startup path rather than broadening it to session/default/scoped-model sources, which would change current TS behavior

### Validation summary

New Rust coverage added for:
- CLI model shorthand `sonnet:xhigh` clamping to `high`
- explicit `--thinking xhigh` clamping when startup falls back to a non-xhigh reasoning model
- preservation of `xhigh` for xhigh-capable startup models

### Remaining gaps after this milestone

Still deferred for the CLI/model surface:
- settings-manager/session-manager-backed startup parity beyond the current bootstrap subset
- broader auth/OAuth/cloud-auth availability parity for providers whose startup availability does not come solely from env keys or current `models.json` support
- JSON/session wrapper parity and interactive coding-agent/TUI integration

## Milestone 14 update: auth.json startup auth source

### Files analyzed

Additional TypeScript behavior reviewed for this slice:
- `packages/coding-agent/src/core/auth-storage.ts`
- `packages/coding-agent/src/core/model-registry.ts`
- `packages/ai/src/oauth.ts`
- `packages/ai/src/utils/oauth/index.ts`
- `packages/ai/src/utils/oauth/anthropic.ts`
- `packages/ai/src/utils/oauth/github-copilot.ts`
- `packages/ai/src/utils/oauth/google-gemini-cli.ts`
- `packages/ai/src/utils/oauth/google-antigravity.ts`
- `packages/ai/src/utils/oauth/openai-codex.ts`
- `packages/ai/src/utils/oauth/types.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/auth.rs`
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/apps/pi/src/main.rs`
- `rust/apps/pi/Cargo.toml`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- the Rust app now reads `auth.json` alongside env vars for startup model availability and request auth
- stored `api_key` credentials in `auth.json` now participate in:
  - `get_available()` / initial model selection
  - request-time API key resolution
- stored OAuth credentials in `auth.json` now count as configured auth for startup availability, matching the TS `hasAuth()` shape check
- request-time API key derivation is now supported for the built-in OAuth providers that can be mapped directly from stored credentials without a refresh flow:
  - `anthropic`
  - `github-copilot`
  - `openai-codex`
  - `google-gemini-cli`
  - `google-antigravity`
- the Rust app now layers auth sources in TS order for the current non-interactive path:
  - runtime override (`OverlayAuthSource`, already present)
  - `auth.json`
  - environment variables

### Rust design summary

New auth-source slice in `pi-coding-agent-core`:
- `AuthFileSource`
  - reads `auth.json` on demand
  - supports `api_key` and `oauth` entries
  - resolves `api_key.key` with the existing config-value resolver
- `ChainedAuthSource`
  - composes multiple `AuthSource` implementations with first-match `get_api_key()` semantics and any-match `has_auth()` semantics

App integration:
- `rust/apps/pi/src/main.rs` now constructs a chained auth source using:
  - `AuthFileSource(<agentDir>/auth.json)`
  - `EnvAuthSource`

### Validation summary

New Rust coverage added for:
- `AuthFileSource` loading `api_key` credentials
- OAuth credential translation for `google-gemini-cli`
- chained auth-source fallback behavior
- full non-interactive runner startup using `auth.json` API keys to select the initial model and authenticate the request

### Remaining gaps after this milestone

Still deferred for the CLI/model/auth surface:
- OAuth refresh parity from `auth.json` (Rust currently only uses stored unexpired credentials; it does not refresh)
- OAuth-driven model mutation parity such as GitHub Copilot enterprise base-url rewriting
- settings-manager/session-manager-backed startup defaults and resource-loader integration
- JSON/session wrapper parity and interactive coding-agent/TUI integration

## Milestone 15 update: startup OAuth refresh + Copilot model mutation parity

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/ai/src/utils/oauth/anthropic.ts`
- `packages/ai/src/utils/oauth/google-gemini-cli.ts`
- `packages/ai/src/utils/oauth/google-antigravity.ts`
- `packages/ai/src/utils/oauth/openai-codex.ts`
- `packages/ai/test/anthropic-oauth.test.ts`
- `packages/ai/test/openai-codex-stream.test.ts`
- `packages/ai/test/github-copilot-oauth.test.ts`
- `packages/ai/src/providers/openai-responses.ts`
- `packages/ai/src/providers/github-copilot-headers.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/auth.rs`
- `rust/crates/pi-coding-agent-core/src/model_registry.rs`
- `rust/crates/pi-coding-agent-core/tests/auth.rs`
- `rust/crates/pi-coding-agent-core/tests/model_registry.rs`
- `rust/apps/pi/src/main.rs`

### Behavior summary

New TS-compatible startup auth/model behaviors now covered in Rust:
- `rust/apps/pi` now refreshes expired built-in OAuth credentials from `auth.json` before constructing the runtime auth chain
- startup refresh now covers the built-in providers that do not require session-manager wiring:
  - `anthropic`
  - `github-copilot`
  - `google-gemini-cli`
  - `google-antigravity`
  - `openai-codex`
- refreshed `auth.json` entries now preserve the provider-specific fields that TS refresh returns:
  - `enterpriseUrl` for Copilot
  - `projectId` for Google providers
  - `accountId` for OpenAI Codex
- startup refresh remains non-fatal; refresh failures do not crash the Rust app, matching the TS startup goal of letting the user continue and re-authenticate later
- GitHub Copilot model mutation parity is now in place at registry load time:
  - if the stored Copilot access token contains `proxy-ep=...`, Rust rewrites all `github-copilot` model `base_url`s to the derived API host
  - otherwise Rust falls back to `enterpriseUrl` -> `https://copilot-api.<domain>`
  - the mutation applies after `models.json` merge/override handling, so it affects both built-in and custom Copilot models

### Rust design summary

Expanded auth slice in `pi-coding-agent-core::auth`:
- `refresh_auth_file_oauth()` remains the public startup entry point
- internal refresh dispatch now routes per provider to small explicit refresh helpers rather than a single Copilot-only path
- provider-specific refresh helpers now mirror the TS request shapes:
  - Anthropic refresh uses JSON POST with no `scope`
  - Google refresh uses form POST with provider-specific client id/secret and carries forward `projectId`
  - OpenAI Codex refresh uses form POST and extracts `accountId` from the refreshed access token JWT payload
  - GitHub Copilot refresh keeps the existing token endpoint + enterprise-domain handling
- `AuthSource::model_base_url()` continues to carry provider-specific model mutation without pulling session-manager or dynamic OAuth provider registration into Rust yet

### Validation summary

New Rust coverage added for:
- Anthropic startup refresh request shape and `auth.json` rewrite
- GitHub Copilot startup refresh request shape and `auth.json` rewrite
- Google Gemini CLI startup refresh request shape and `auth.json` rewrite
- Google Antigravity startup refresh request shape and `auth.json` rewrite
- OpenAI Codex startup refresh request shape, JWT `accountId` extraction, and `auth.json` rewrite
- existing Copilot model base-url mutation coverage remains in `tests/auth.rs` and `tests/model_registry.rs`

Validation run results:
- `cd rust && cargo test -p pi-coding-agent-core` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for the CLI/model/auth surface:
- auth-file locking/merge semantics for startup refresh (current Rust startup refresh is single-process best-effort only)
- surfacing startup refresh errors to the user instead of silently swallowing them in `rust/apps/pi`
- full TS runtime OAuth refresh-on-demand parity in the Rust auth source path (today Rust refreshes at app startup, not per request)
- settings-manager/session-manager-backed startup defaults and interactive/TUI integration

## Milestone 16 update: request-time OAuth refresh parity for non-interactive runtime

### Files analyzed

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/auth.rs`
- `rust/crates/pi-coding-agent-core/src/model_registry.rs`
- `rust/crates/pi-coding-agent-core/src/runtime.rs`
- `rust/crates/pi-coding-agent-core/tests/runtime.rs`
- `rust/crates/pi-coding-agent-core/tests/auth.rs`
- `rust/crates/pi-coding-agent-cli/src/auth.rs`

Relevant TypeScript behavior already grounding this slice:
- `packages/coding-agent/src/core/auth-storage.ts`
- `packages/coding-agent/src/core/model-registry.ts`

### Behavior summary

New TS-compatible request-time auth behavior now covered in Rust:
- runtime request auth resolution is no longer limited to the sync stored-value path
- expired OAuth credentials can now refresh on demand at request time, not just during app startup
- the request-time refresh path preserves auth-source precedence:
  - runtime override
  - `auth.json`
  - environment variables
- concurrent request-time refreshes now serialize through a simple `auth.json.lock` file, so a second caller waits and re-reads updated credentials instead of blindly refreshing again
- if another process refreshed the credential first, Rust now re-reads the locked file and uses the fresh token
- `RegistryBackedStreamer` now uses async request auth resolution before dispatching to `pi-ai`, so non-interactive runs can recover from expired OAuth state without restarting the app

### Rust design summary

Expanded auth/runtime slices:
- `pi-coding-agent-core::AuthSource`
  - new async-style `get_api_key_for_request()` hook with a sync default fallback
- `AuthFileSource`
  - new request-time path that refreshes expired OAuth credentials on demand
  - targeted refresh updates only the requested provider entry using the existing provider-specific refresh helpers
  - simple lock-file coordination via `auth.json.lock`
- `ChainedAuthSource`
  - now awaits each source's request-time API-key resolution in order
- `OverlayAuthSource`
  - now preserves CLI runtime override precedence for async request-time resolution too
- `ModelRegistry`
  - new `get_api_key_and_headers_async()` for runtime request auth resolution
  - sync `get_api_key_and_headers()` remains for the existing startup/config-only callers
- `RegistryBackedStreamer`
  - now resolves request auth asynchronously inside the returned event stream before calling `pi_ai::stream_response()`

### Validation summary

New Rust coverage added for:
- runtime proof that async request auth resolution is actually used by the coding-agent streamer path
- existing `auth.rs` unit coverage continues to validate provider-specific refresh request shapes and `auth.json` rewrites for all built-in OAuth providers

Validation run results:
- `cd rust && cargo test -p pi-coding-agent-core` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for the CLI/model/auth surface:
- lock-file semantics are intentionally minimal and still trail TS `proper-lockfile` behavior (no stale-lock recovery or compromised-lock reporting)
- request-time auth refresh errors are still returned as missing-auth/runtime failures rather than being accumulated in an `AuthStorage`-style error buffer
- startup refresh in `rust/apps/pi` is still best-effort and silent
- settings-manager/session-manager-backed startup defaults and interactive/TUI integration

## Milestone 17 update: `blockImages` runtime wrapper slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/core/sdk.ts`
- `packages/coding-agent/test/block-images.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-core/src/messages.rs`
- `rust/crates/pi-coding-agent-core/src/runtime.rs`
- `rust/crates/pi-coding-agent-core/tests/messages.rs`
- `rust/crates/pi-coding-agent-core/tests/runtime.rs`
- `rust/crates/pi-events/src/lib.rs`

### Behavior summary

New TS-compatible image-blocking behavior now covered in Rust:
- image blocking remains a convert-to-LLM defense-in-depth layer, not a read/file-processing restriction
- when blocking is enabled, only LLM-bound `user` and `toolResult` message content is rewritten
- image blocks are replaced with the exact TS placeholder text `Image reading is disabled.`
- consecutive image blocks collapse to a single placeholder text block, matching the TS dedupe behavior
- assistant messages are left unchanged
- the Rust core now supports dynamic toggling after construction, matching the TS intent that mid-session settings changes affect future requests

### Rust design summary

Expanded coding-agent-core/message conversion slices:
- `pi-coding-agent-core::messages`
  - new `BLOCKED_IMAGE_PLACEHOLDER` constant
  - new `filter_blocked_images()` helper over normalized `pi_events::Message` values
- `pi-coding-agent-core::runtime`
  - `CodingAgentCore` now carries a shared `AtomicBool` image-blocking flag
  - new `CodingAgentCore::block_images()` getter
  - new `CodingAgentCore::set_block_images(bool)` setter
  - installed convert-to-LLM hook now applies `convert_to_llm()` first, then conditionally runs `filter_blocked_images()` on each request

This stays intentionally below settings-manager/session-manager wiring: the runtime behavior is now present, while config persistence and CLI/UI control remain deferred.

### Validation summary

New Rust coverage added for:
- direct message-level filtering of user/tool-result images with placeholder dedupe
- runtime proof that `set_block_images(true/false)` changes the actual provider request context across successive prompts

Validation run results:
- `cd rust && cargo fmt` passed
- `cd rust && cargo test -p pi-coding-agent-core --test messages` passed
- `cd rust && cargo test -p pi-coding-agent-core --test runtime` passed
- `cd rust && cargo test -p pi-coding-agent-core` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for the coding-agent image/settings surface:
- no Rust settings-manager integration yet for persisting or loading `images.blockImages`
- no CLI/TUI settings control wired to `CodingAgentCore::set_block_images()` yet
- no TS image auto-resize parity yet in the Rust file-processing/runtime path
- session-manager-backed and interactive/TUI image-setting behavior remains deferred

## Milestone 18 update: settings-backed `blockImages` in the Rust non-interactive path

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/core/settings-manager.ts`
- `packages/coding-agent/src/core/sdk.ts`
- `packages/coding-agent/src/cli/file-processor.ts`
- `packages/coding-agent/test/block-images.test.ts`
- `packages/coding-agent/test/image-processing.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-config/Cargo.toml`
- `rust/crates/pi-config/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/Cargo.toml`
- `rust/crates/pi-coding-agent-cli/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/apps/pi/Cargo.toml`
- `rust/apps/pi/src/main.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- the Rust non-interactive app now reads `settings.json` for image blocking using the same path split as TS:
  - global: `<agentDir>/settings.json`
  - project: `<cwd>/.pi/settings.json`
- project settings override global settings for `images.blockImages`
- the loaded setting now reaches the actual request path by calling `CodingAgentCore::set_block_images(...)` before the run
- end-to-end non-interactive requests now honor stored `images.blockImages: true` by replacing LLM-bound image blocks with `Image reading is disabled.`
- invalid settings JSON is non-fatal and reported as a warning, matching the TS startup philosophy for settings load issues

Still deferred in this slice:
- `images.autoResize` loading is not wired yet because the Rust image-resize pipeline has not been ported
- interactive/TUI settings control is still deferred

### Rust design summary

New minimal config slice in `pi-config`:
- `SettingsScope`
- `SettingsWarning`
- `ImageSettings`
- `LoadedImageSettings`
- `load_image_settings(cwd, agent_dir)`

Integration changes:
- `pi-coding-agent-cli::RunCommandOptions` now accepts `agent_dir`
- `pi-coding-agent-cli::runner` loads image settings and renders warnings in the current CLI stderr style
- `rust/apps/pi` now passes the resolved agent dir into the runner so the default app path honors stored `settings.json`

This is intentionally narrow: it ports only the config-loading needed for the already-implemented runtime `blockImages` behavior, without introducing a full Rust `SettingsManager` yet.

### Validation summary

New Rust coverage added for:
- `pi-config` defaults when settings files are absent
- `pi-config` project-overrides-global behavior for `images.blockImages`
- `pi-config` warning behavior for invalid JSON
- runner-level end-to-end proof that global `settings.json` with `images.blockImages: true` changes the actual provider request context

Validation run results:
- `cd rust && cargo fmt` passed
- `cd rust && cargo test -p pi-config` passed
- `cd rust && cargo test -p pi-coding-agent-cli --test runner` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for the coding-agent settings/image surface:
- no full Rust `SettingsManager` API yet
- no settings-backed `images.autoResize` parity yet
- no session-manager/resource-loader-backed settings diagnostics aggregation yet
- no interactive/TUI settings UI or persistence editing flow yet

## Milestone 19 update: non-interactive `@file` image auto-resize parity slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/cli/file-processor.ts`
- `packages/coding-agent/src/utils/image-resize.ts`
- `packages/coding-agent/test/image-processing.test.ts`
- `packages/coding-agent/src/core/settings-manager.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tools/Cargo.toml`
- `rust/crates/pi-coding-agent-tools/src/lib.rs`
- `rust/crates/pi-coding-agent-tools/src/read.rs`
- `rust/crates/pi-coding-agent-tools/tests/read_write.rs`
- `rust/crates/pi-coding-agent-cli/Cargo.toml`
- `rust/crates/pi-coding-agent-cli/src/lib.rs`
- `rust/crates/pi-coding-agent-cli/src/file_processor.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`
- `rust/crates/pi-coding-agent-cli/tests/runner.rs`
- `rust/crates/pi-config/src/lib.rs`
- `rust/crates/pi-config/tests/settings.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- non-interactive `@file` image preprocessing now defaults to auto-resizing inline image attachments
- resize behavior follows the current TS algorithm shape closely:
  - preserve original image unchanged when already within dimension and encoded-size limits
  - clamp dimensions to `2000x2000`
  - try PNG first, then JPEG quality fallbacks
  - if still too large, progressively shrink dimensions by 75%
  - omit the image with the TS placeholder note when it cannot be made small enough
- resized images now add the same dimension note text used by TS so the model can map displayed coordinates back to the original image
- Rust settings loading now includes `images.autoResize` with TS-compatible defaults and project-overrides-global behavior
- the Rust non-interactive runner now honors `images.autoResize: false` from `settings.json`, preserving the original image attachment without the resize note

Still deferred in this slice:
- read-tool image auto-resize parity is not wired yet
- EXIF orientation handling and the exact Photon-based TS implementation details remain unported
- interactive/TUI image preprocessing remains deferred

### Rust design summary

New shared image helper slice in `pi-coding-agent-tools`:
- `src/image.rs`
  - `ImageResizeOptions`
  - `ResizedImage`
  - `resize_image_bytes()`
  - `format_dimension_note()`
- exported through `pi-coding-agent-tools` so the same helper can be reused later by the read-tool path

CLI integration changes:
- `pi-coding-agent-cli::file_processor`
  - new `ProcessFileOptions`
  - image preprocessing now runs through the shared resize helper
- `pi-coding-agent-cli::runner`
  - passes settings-backed `auto_resize_images` into file preprocessing
- `pi-config::ImageSettings`
  - now includes both `auto_resize_images` and `block_images`

This remains intentionally scoped to the non-interactive `@file` path; the read tool will reuse the same helper in a later milestone.

### Validation summary

New Rust coverage added for:
- image resize helper parity cases:
  - unchanged small image
  - dimension-triggered resize
  - byte-limit-triggered resize
  - impossible byte limit returning `None`
- CLI file-processor behavior:
  - default auto-resize of oversized images
  - explicit disable path preserving original image data
- runner-level proof that `settings.json` with `images.autoResize: false` changes the actual non-interactive request payload
- updated config tests for `images.autoResize` defaults and override behavior

Validation run results:
- `cd rust && cargo fmt` passed
- `cd rust && cargo test -p pi-coding-agent-tools --test image_resize` passed
- `cd rust && cargo test -p pi-coding-agent-cli --test file_processor` passed
- `cd rust && cargo test -p pi-coding-agent-cli --test runner` passed
- `cd rust && cargo test -p pi-config` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for the coding-agent image/settings surface:
- read-tool image auto-resize parity is still missing
- TS Photon/EXIF parity is not complete in Rust
- no full Rust `SettingsManager` API yet
- no interactive/TUI image settings workflow yet

## Milestone 20 update: read-tool image auto-resize parity slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/core/tools/index.ts`
- `packages/coding-agent/src/core/agent-session.ts`
- `packages/coding-agent/src/core/tools/read.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/runtime.rs`
- `rust/crates/pi-coding-agent-core/tests/runtime.rs`
- `rust/crates/pi-coding-agent-tools/src/read.rs`
- `rust/crates/pi-coding-agent-tools/src/lib.rs`
- `rust/crates/pi-coding-agent-tools/tests/read_write.rs`
- `rust/crates/pi-coding-agent-cli/src/runner.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- the Rust `read` tool now auto-resizes image files by default instead of always returning the original image bytes
- read-tool image behavior now matches the TS shape already used in `packages/coding-agent/src/core/tools/read.ts`:
  - unchanged images stay unchanged when already within limits
  - oversized images are resized before being returned to the model
  - resized images include the same dimension note text used by the TS implementation
  - impossible-to-fit images return the TS omission note instead of an attachment
- the read tool now supports dynamic runtime toggling of auto-resize through a shared flag, mirroring the TS intent that settings changes can affect future tool behavior
- the Rust non-interactive runner now applies stored `images.autoResize` to both major image-entry points now implemented in Rust:
  - `@file` CLI preprocessing
  - default `read` tool runtime behavior

Still deferred in this slice:
- full Photon/EXIF parity remains unported
- interactive/TUI settings editing remains deferred

### Rust design summary

Expanded `pi-coding-agent-tools` image/read slices:
- `read.rs`
  - new `create_read_tool_with_auto_resize_flag(...)`
  - read execution now delegates image resizing to the shared `resize_image_bytes()` helper
- `lib.rs`
  - new `create_coding_tools_with_read_auto_resize_flag(...)`

Expanded `pi-coding-agent-core::runtime`:
- `CodingAgentCore` now tracks a shared `auto_resize_images` flag alongside `block_images`
- new methods:
  - `CodingAgentCore::auto_resize_images()`
  - `CodingAgentCore::set_auto_resize_images(bool)`
- default tool creation now wires the read tool to that shared flag so later setting changes can affect tool execution without recreating the core

Runner integration:
- `pi-coding-agent-cli::runner` now applies settings-backed `auto_resize_images` to the core after creation, alongside `block_images`

### Validation summary

New Rust coverage added for:
- read-tool behavior on valid small images
- read-tool default auto-resize on oversized images
- read-tool dynamic shared-flag toggle between resized and unresized output
- existing targeted runner/runtime coverage continues to validate surrounding startup integration

Validation run results:
- `cd rust && cargo fmt` passed
- `cd rust && cargo test -p pi-coding-agent-tools --test read_write` passed
- `cd rust && cargo test -p pi-coding-agent-core --test runtime` passed
- `cd rust && cargo test -p pi-coding-agent-cli --test runner` passed
- `cd rust && cargo test` passed
- `npm run check` still fails in this environment before repo checks run because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for the coding-agent image/settings surface:
- full Photon/EXIF parity is still missing
- no full Rust `SettingsManager` API yet
- no interactive/TUI image settings workflow yet
- broader session-manager-backed settings parity remains deferred

## Milestone 21 update: coding-agent keybindings manager + legacy keybinding migration slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/core/keybindings.ts`
- `packages/coding-agent/src/migrations.ts` (keybindings migration path)
- `packages/coding-agent/test/keybindings-migration.test.ts`
- `packages/coding-agent/docs/keybindings.md`
- `packages/coding-agent/src/config.ts`
- `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/Cargo.toml`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-tui/Cargo.toml`
- `rust/crates/pi-tui/src/keybindings.rs`
- `rust/crates/pi-tui/tests/keybindings.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now has the coding-agent-specific keybinding registry layered on top of the existing Rust `pi-tui` defaults
- app keybinding defaults now match the current TypeScript `KEYBINDINGS` table, including the platform split for `app.clipboard.pasteImage` (`alt+v` on Windows, `ctrl+v` elsewhere)
- legacy pre-namespaced keybinding ids are now recognized and migrated in Rust using the same mapping as TypeScript
- in-memory loading now accepts old keybinding names before any file rewrite, matching the TS manager behavior used during startup
- on-disk migration now preserves the TypeScript conflict rule where an already-present namespaced key wins over its legacy alias
- migrated file output now keeps known keybindings in the default table order before any unknown extra keys, matching the TS ordering intent from `orderKeybindingsConfig(...)`

Current intentional limitation for this slice:
- the new Rust keybindings migration/helper path lives in `pi-coding-agent-tui` and is not yet wired into the top-level Rust startup path; runtime usage will land with the first interactive-mode integration slice

### Rust design summary

New `pi-coding-agent-tui::keybindings` module:
- `DEFAULT_APP_KEYBINDINGS`
- `KeybindingsManager` (coding-agent wrapper over `pi_tui::KeybindingsManager`)
- `MigrateKeybindingsConfigResult`
- `migrate_keybindings_config(...)`
- `migrate_keybindings_file(...)`

Design choices for this slice:
- reuse the already-ported `pi-tui` keybinding manager and extend it with coding-agent defaults instead of creating a second keybinding implementation
- keep legacy-id migration data local to the coding-agent crate because those aliases are app-specific, not TUI-generic
- preserve raw JSON values during file migration so non-keybinding values are renamed without being normalized away, while still normalizing valid string/array entries into Rust `KeybindingsConfig` for runtime use
- keep file loading tolerant of malformed `keybindings.json`, matching the TS startup/migration behavior of ignoring malformed files instead of failing the app

### Validation summary

New Rust coverage added for:
- rewriting legacy ids on disk to namespaced ids
- keeping the namespaced value when both legacy and namespaced ids exist
- loading legacy ids in memory before file migration runs
- keeping migrated known keybindings ordered ahead of unknown extras on disk

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive/keybindings surface:
- top-level Rust startup still does not invoke the new keybindings migration helper
- no Rust port of `keybinding-hints.ts` / theme-formatted key-hint rendering yet
- no interactive-mode `pi-coding-agent-tui` integration yet
- session-manager/resource-loader-backed interactive startup remains deferred

## Milestone 22 update: top-level Rust startup keybindings migration wiring

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/migrations.ts`
- `packages/coding-agent/src/core/keybindings.ts`
- `packages/coding-agent/test/keybindings-migration.test.ts`

Additional Rust files read for this slice:
- `rust/apps/pi/Cargo.toml`
- `rust/apps/pi/src/main.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- the top-level Rust app startup path now runs the keybindings migration before non-interactive command handling
- startup migration now silently rewrites legacy `keybindings.json` ids to namespaced ids, matching the TS `runMigrations()` behavior for the keybindings file
- malformed `keybindings.json` remains non-fatal and is left untouched, matching the TS migration path that ignores malformed files during startup
- missing `keybindings.json` remains a no-op

Current intentional limitation for this slice:
- startup wiring is currently in `rust/apps/pi`; alternative Rust entrypoints or future test harness launch paths would need to call the same helper explicitly until a shared startup bootstrap layer exists

### Rust design summary

Startup integration changes:
- `rust/apps/pi` now depends on `pi-coding-agent-tui`
- new local helper in `rust/apps/pi/src/main.rs`:
  - `run_startup_migrations(agent_dir: &Path)`
- current implementation intentionally stays narrow and only calls:
  - `migrate_keybindings_file(agent_dir.join("keybindings.json"))`
- errors from the migration helper are intentionally ignored at startup to preserve TS non-fatal migration behavior

### Validation summary

New Rust coverage added for:
- startup helper rewrites a legacy `keybindings.json` file before command handling
- startup helper ignores malformed `keybindings.json` without modifying it

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive/keybindings startup surface:
- keybinding migration is only wired in the `rust/apps/pi` entrypoint, not yet behind a shared reusable bootstrap helper
- no Rust port of `keybinding-hints.ts` / theme-formatted key-hint rendering yet
- no interactive-mode `pi-coding-agent-tui` integration yet
- session-manager/resource-loader-backed interactive startup remains deferred

## Milestone 23 update: keybinding-hint formatting slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`
- `packages/coding-agent/src/modes/interactive/theme/theme.ts`
- `packages/coding-agent/src/core/keybindings.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now exposes the keybinding-hint formatting helpers corresponding to the current TypeScript `keybinding-hints.ts` slice
- key text formatting now matches TS behavior:
  - no keys -> empty string
  - one key -> that key as-is
  - multiple keys -> `/`-joined
- hint rendering now preserves the TS output shape of dimmed key text followed by muted description text with a leading space
- raw key hints now format literal keys without keybinding lookup
- the Rust slice is theme-agnostic by design: callers provide a `KeyHintStyler` instead of relying on a global interactive theme singleton
- `PlainKeyHintStyler` now exists for unstyled or test usage

Current intentional limitation for this slice:
- there is still no Rust port of the interactive theme system, so this slice ports the formatting contract and styling interface, not the concrete theme implementation from TypeScript

### Rust design summary

New `pi-coding-agent-tui::keybinding_hints` module:
- `KeyHintStyler`
- `PlainKeyHintStyler`
- `key_text(...)`
- `key_hint(...)`
- `raw_key_hint(...)`

Design choices for this slice:
- depend on the already-ported coding-agent `KeybindingsManager` instead of a global mutable keybinding registry
- keep styling abstract via a small trait so a future Rust interactive theme can plug in directly without coupling `pi-coding-agent-tui` to a theme implementation yet
- keep the string-shape parity with TS exact, including the description leading-space behavior

### Validation summary

New Rust coverage added for:
- slash-joining multiple keys in `key_text(...)`
- empty-string behavior for unbound actions
- styled key-hint formatting with separate dim/muted channels
- raw key-hint formatting without lookup
- passthrough behavior from `PlainKeyHintStyler`

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive/keybinding-hint surface:
- no Rust interactive theme implementation yet to back a real styled TUI usage path
- no wiring from a Rust interactive header/startup component into these hint helpers yet
- no interactive-mode `pi-coding-agent-tui` integration yet
- session-manager/resource-loader-backed interactive startup remains deferred

## Milestone 24 update: minimal interactive startup-header component slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (built-in header construction section)
- `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`
- `packages/coding-agent/src/modes/interactive/theme/theme.ts`
- `packages/coding-agent/src/core/keybindings.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/src/terminal.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/keybinding_hints.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first minimal built-in startup-header slice derived from the current TypeScript interactive header content
- the Rust header text reproduces the current TS startup instruction list order and wording for the built-in header body:
  - interrupt / clear / exit / suspend
  - delete-to-end
  - thinking/model controls
  - tool/thinking/editor shortcuts
  - command/bash/follow-up/paste-image/file-drop hints
  - onboarding sentence
- the header now resolves actual key text from the Rust coding-agent keybindings manager, so user overrides affect rendered startup instructions just like the TS path
- quiet startup is now represented in the Rust slice by returning an empty header body
- the new header component renders through the existing Rust `pi-tui` `Component`/`Tui` render path and wraps long lines using the already-ported ANSI-aware wrapping helpers

Current intentional limitation for this slice:
- changelog rendering, theme-driven styled output, and spacer/border composition are still deferred; this slice ports only the built-in startup-header text/content plus minimal renderable component behavior

### Rust design summary

New `pi-coding-agent-tui::startup_header` module:
- `StartupHeaderStyler`
- `build_startup_header_text(...)`
- `StartupHeaderComponent`

Design choices for this slice:
- keep the header component focused on static built text instead of introducing a broader widget framework in `pi-coding-agent-tui`
- reuse the already-ported keybinding-hint helpers and coding-agent keybindings manager directly
- keep styling abstract with `StartupHeaderStyler` so a future Rust interactive theme can supply accent/bold behavior without forcing a theme subsystem into this milestone
- use `pi_tui::wrap_text_with_ansi(...)` in the component render path to stay compatible with future styled output and the existing renderer semantics

### Validation summary

New Rust coverage added for:
- exact startup-header text shape with default keybindings
- startup-header response to keybinding overrides
- quiet startup returning an empty header body
- rendering the startup-header component through `pi_tui::Tui` with wrapped long lines

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive header/startup surface:
- no Rust interactive theme implementation yet to produce TS-style colored/bold startup output
- changelog/header-border/spacer composition from TS `InteractiveMode` is not yet ported
- no Rust interactive session/editor/footer integration yet
- session-manager/resource-loader-backed interactive startup remains deferred

## Milestone 25 update: built-in header composition with condensed changelog notice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (built-in header layout / changelog section)
- `packages/coding-agent/src/modes/interactive/components/dynamic-border.ts`
- `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`
- `packages/coding-agent/src/modes/interactive/theme/theme.ts`
- `packages/coding-agent/src/core/keybindings.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/startup_header.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_header.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-tui/src/terminal.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first built-in header composition slice around the previously ported startup-header text
- the Rust built-in header now reproduces the current TS structural behavior for the migrated subset:
  - leading spacer before the built-in startup header
  - trailing spacer after the built-in startup header
  - condensed changelog notice rendering when requested
  - quiet-startup path showing only the condensed changelog notice (with a spacer) when changelog content is present
- condensed changelog notice text now matches the TS wording shape:
  - `Updated to v<version>. Use /changelog to view full changelog.`
- latest-version extraction now works from markdown headings like the TS regex path (`## [0.9.0]`)
- non-quiet condensed changelog rendering now includes dynamic-width border lines using the same `─` repeat strategy as the TS `DynamicBorder` component
- the new component renders directly through the existing Rust `Component` surface and stays compatible with the already-ported `pi-tui` renderer

Current intentional limitation for this slice:
- expanded changelog markdown rendering (`What's New` + markdown body + spacers) is still deferred; only the condensed changelog path is implemented in Rust so far

### Rust design summary

Expanded `pi-coding-agent-tui::startup_header` with:
- `build_condensed_changelog_notice(...)`
- `BuiltInHeaderComponent`
- internal semver extraction from changelog markdown headings

Design choices for this slice:
- keep the composition logic in the existing startup-header module instead of introducing a separate header/layout subsystem yet
- keep changelog support narrow to the condensed path that is easiest to validate without a Rust markdown widget
- reuse the existing startup-header component for the main body and layer structural spacing / borders / condensed notice on top
- keep styling abstract through `StartupHeaderStyler`; concrete colored border/bold rendering remains deferred until the Rust interactive theme exists

### Validation summary

New Rust coverage added for:
- extracting the latest version from changelog markdown into the condensed notice
- rendering built-in-header spacers, borders, and condensed changelog notice in the non-quiet path
- rendering quiet built-in-header output with only the condensed changelog notice and no borders
- previously added startup-header body tests continue to validate the underlying instruction text and wrapped rendering

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive header/startup surface:
- no Rust interactive theme implementation yet to produce TS-style colored/bold/border output
- full changelog markdown rendering (`What's New` header + markdown body) is not yet ported
- no Rust interactive session/editor/footer integration yet
- session-manager/resource-loader-backed interactive startup remains deferred

## Milestone 26 update: expanded changelog markdown in the built-in startup header

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (built-in header / changelog path)
- `packages/coding-agent/src/utils/changelog.ts`
- `packages/tui/src/components/markdown.ts`
- `packages/tui/test/markdown.test.ts`
- `packages/coding-agent/CHANGELOG.md`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/startup_header.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_header.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `BuiltInHeaderComponent` now supports the non-condensed changelog path used by TypeScript interactive startup when `collapseChangelog` is disabled
- non-quiet built-in header rendering now follows the current TS structure for the migrated slice:
  - startup header body
  - spacer
  - dynamic-width top border
  - `What's New` heading
  - spacer
  - rendered changelog body
  - spacer
  - dynamic-width bottom border
- the Rust changelog renderer now covers the markdown constructs exercised by current coding-agent changelog entries and startup display needs:
  - `##` headings rendered without the literal heading marker, matching the TS markdown component behavior for level-2 headings
  - `###` and deeper headings rendered with their hash prefix, matching the TS markdown component behavior for deeper headings
  - unordered and ordered lists
  - fenced code blocks with TS-style ``` fence lines and two-space code indentation
  - inline bold, inline code, strikethrough, and markdown links (`[text](url)` -> `text (url)` when text differs)
  - single-blank-line spacing normalization between rendered blocks
- quiet startup behavior is unchanged: Rust still shows only the condensed changelog notice in the silent path, matching the current TS startup flow

Current intentional limitation for this slice:
- this is still a startup-header-specific markdown renderer, not a full Rust port of the generic `@mariozechner/pi-tui` `Markdown` widget
- richer markdown behaviors already present in TS `packages/tui/src/components/markdown.ts` (tables, blockquotes, nested list layout parity, theme-aware inline style restoration, code highlighting, background styling) remain deferred until the broader interactive/widget migration needs them

### Rust design summary

Expanded `pi-coding-agent-tui::startup_header` with:
- internal `ChangelogContent::{Condensed, Expanded}` to preserve the TS condensed vs expanded startup decision in one component
- internal startup-header markdown rendering helpers for:
  - block parsing (`heading`, `list`, `paragraph`, fenced code block, horizontal rule)
  - inline formatting (`bold`, `code`, `strikethrough`, markdown links)
  - ANSI-aware wrapping through the existing `pi_tui::wrap_text_with_ansi(...)`
- widened `StartupHeaderStyler` with default no-op styling hooks for headings, links, code, list bullets, and code blocks so future theme wiring can layer styling without changing the current plain-text tests

Design choices for this slice:
- keep the work in `pi-coding-agent-tui::startup_header` instead of introducing a generic Rust markdown widget before coding-agent actually needs it elsewhere
- preserve the TS built-in-header composition and condensed/quiet rules first, while explicitly deferring generic markdown/widget parity work to later interactive milestones
- reuse the existing `pi-tui` wrapping helpers instead of introducing a second wrapping/rendering path for startup content

### Validation summary

New Rust coverage added for:
- expanded built-in header rendering with:
  - `What's New` heading
  - changelog version heading rendering
  - deep heading rendering
  - list items
  - markdown link rendering
  - fenced code block rendering
- existing startup-header tests continue to validate:
  - exact startup instruction text
  - keybinding override behavior
  - condensed changelog notice rendering
  - quiet-startup condensed notice behavior
  - TUI wrapping of the startup header component

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_header` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive header/startup surface:
- no Rust interactive theme implementation yet to produce TS-style colored/bold/border output beyond the current styler hooks
- no full generic Rust markdown widget yet; broader `packages/tui` markdown parity still remains deferred
- no Rust interactive session/editor/footer integration yet
- session-manager/resource-loader-backed interactive startup remains deferred

## Milestone 27 update: minimal interactive startup shell slice

### Files analyzed

Additional TypeScript files reviewed for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (constructor/startup layout section around header + editor wiring)
- previously ported startup/header and keybinding sources remained the behavior baseline:
  - `packages/coding-agent/src/core/keybindings.ts`
  - `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_header.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-coding-agent-tui/src/keybinding_hints.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/input.rs`
- `rust/crates/pi-tui/src/tui.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_header.rs`

### Behavior summary

New TS-compatible interactive-startup behavior now covered in Rust:
- `pi-coding-agent-tui` now exposes a first minimal `StartupShellComponent` that composes the already-ported built-in startup header with the already-ported single-line `pi-tui::Input`
- the shell now follows the current TypeScript startup layout shape for the migrated subset:
  - built-in header content above the prompt when startup is not quiet
  - prompt-only layout when quiet startup has no changelog
  - changelog/header content still rendered before the prompt through the existing `BuiltInHeaderComponent`
- focus and input now flow through the composed shell when mounted in a `pi_tui::Tui`, so the prompt inherits the existing hardware-cursor / marker behavior from `pi-tui`
- shell input submission and escape handling now reuse the same callback semantics already present on the Rust `Input`
- the same coding-agent keybinding manager instance now drives both halves of the startup shell:
  - header hints render resolved app keybindings
  - input honors overridden TUI keybindings like a custom submit binding
- quiet built-in header rendering without changelog now returns no lines, matching the TypeScript silent-startup path more closely than the previous placeholder empty-line behavior

### Rust design summary

New `pi-coding-agent-tui::startup_shell` module:
- `StartupShellComponent`
  - owns `BuiltInHeaderComponent`
  - owns `pi_tui::Input`
  - forwards `render`, `handle_input`, and `set_focused` through the composed shell surface
  - exposes narrow prompt control hooks needed by the first integration slice:
    - `set_on_submit(...)`
    - `clear_on_submit()`
    - `set_on_escape(...)`
    - `clear_on_escape()`
    - `input_value()`
    - `set_input_value(...)`
    - `clear_input()`
    - `is_focused()`

Compatibility choices for this slice:
- keep the shell as a single `Component` instead of introducing a larger Rust interactive-mode/runtime wrapper yet
- reuse the existing coding-agent `KeybindingsManager` and clone its resolved `pi_tui::KeybindingsManager` into the embedded input widget, preserving current keybinding semantics without adding new global state
- keep chat transcript, footer, multiline editor, overlays, and session/runtime wiring deferred until this startup shell exists and is testable through the current Rust `Tui`

### Validation summary

New Rust coverage added for:
- rendering startup header content above the prompt through `pi_tui::Tui`
- quiet startup without changelog rendering the prompt on the first line
- routing typed input and submit events through focused `Tui` input delivery into the startup shell
- shared keybinding behavior across startup-header hints and prompt input bindings
- quiet built-in header with no changelog rendering no lines

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_header` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive shell surface:
- no Rust multiline editor / custom-editor parity yet; the shell still uses the narrow single-line `Input` slice
- no transcript/chat container composition yet above the prompt
- no Rust footer integration yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- use the startup shell plus the newly ported `TruncatedText` widget to begin the first pending-message or transcript composition slice
- keep multiline editor, footer, and runtime/session wiring deferred until that shell can show real interactive context above the prompt

## Milestone 28 update: pending-message strip slice in the Rust startup shell

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (pending-messages container update path)
- `packages/tui/src/components/truncated-text.ts`
- `packages/tui/test/truncated-text.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `migration/packages/coding-agent.md`

### Behavior summary

New TS-compatible interactive-shell behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first pending-message strip slice derived from the current TypeScript `InteractiveMode.updatePendingMessagesDisplay()` behavior
- the Rust shell now renders queued message summaries between header content and the prompt when pending messages exist
- the rendered queued-message slice preserves the current TS output shape for the migrated subset:
  - leading blank spacer before the pending-message list
  - `Steering: ...` lines for steering messages
  - `Follow-up: ...` lines for follow-up messages
  - a final dequeue hint line using the configured `app.message.dequeue` binding text
- pending-message lines now truncate through the already-ported Rust `pi_tui::TruncatedText` widget, matching the TS single-line queued-message strip behavior
- clearing pending messages removes the strip entirely, restoring prompt-only output in the quiet-shell path

Current intentional limitation for this slice:
- this is still a startup-shell/prompt-shell subset, not full interactive transcript composition
- pending messages are managed explicitly through startup-shell methods; they are not yet wired to a Rust session runtime or agent queue implementation
- styling currently remains as whatever the supplied key-hint styler emits; full interactive theme parity is still deferred

### Rust design summary

New `pi-coding-agent-tui::pending_messages` module:
- `PendingMessagesComponent`
  - stores queued-message display lines
  - reuses `key_text(...)` to resolve the dequeue binding once
  - renders via `pi_tui::TruncatedText`

Expanded `pi-coding-agent-tui::startup_shell` with:
- owned `PendingMessagesComponent`
- new methods:
  - `set_pending_messages(...)`
  - `clear_pending_messages()`
  - `has_pending_messages()`
- render order now follows the current migrated shell layout:
  - built-in header
  - pending-message strip
  - prompt input

Design choices for this slice:
- keep pending-message rendering as a small focused component instead of jumping directly to a broader transcript container
- reuse the existing startup shell rather than introducing a second shell type
- keep queue display text preformatted in `pi-coding-agent-tui`, while leaving real queue ownership and runtime wiring deferred to later interactive milestones

### Validation summary

New Rust coverage added for:
- rendering steering/follow-up lines plus the dequeue hint above the prompt
- queued-message truncation within terminal width bounds
- clearing pending messages back to prompt-only output
- previously added startup-shell tests continue to validate header rendering and input routing

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive shell surface:
- no Rust transcript/chat container composition yet above the prompt beyond the queued-message strip
- no Rust multiline editor / custom-editor parity yet; the shell still uses the narrow single-line `Input` slice
- no Rust footer integration yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- use the startup shell + pending-message strip as the base for the first transcript/chat container slice
- keep footer, multiline editor, and runtime/session wiring deferred until that transcript shell exists

## Milestone 29 update: minimal transcript container slice in the Rust startup shell

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (layout order for `chatContainer`, `pendingMessagesContainer`, and editor)
- `packages/tui/src/components/truncated-text.ts`
- `packages/tui/test/truncated-text.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-tui/src/tui.rs`
- `migration/packages/coding-agent.md`

### Behavior summary

New TS-compatible interactive-shell behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first minimal transcript container slice corresponding to the current TypeScript `chatContainer` placement in interactive mode
- the Rust startup shell now renders transcript content between the built-in header and the pending-message strip, matching the current TS layout order:
  - header
  - transcript/chat content
  - queued pending messages
  - prompt input
- transcript child order is preserved, matching the current `Container`-driven append semantics in TypeScript interactive mode
- transcript items can now be removed individually or cleared wholesale from the shell, which provides the minimum mutation surface needed before wiring real runtime/session message rendering
- existing pending-message truncation behavior continues to hold when transcript content is present above it

Current intentional limitation for this slice:
- this is still only a generic transcript container; it does not yet port concrete coding-agent message widgets like user/assistant/tool/custom message components
- transcript entries are managed manually through shell methods; they are not yet connected to a Rust `AgentSession` or interactive runtime
- no scroll behavior or footer/status integration is wired yet

### Rust design summary

New `pi-coding-agent-tui::transcript` module:
- `TranscriptComponent`
  - owns a `pi_tui::Container`
  - exposes:
    - `add_item(...)`
    - `remove_item(...)`
    - `clear_items()`
    - `item_count()`
  - delegates rendering/invalidation to the underlying `Container`

Expanded `pi-coding-agent-tui::startup_shell` with:
- owned `TranscriptComponent`
- new methods:
  - `add_transcript_item(...)`
  - `remove_transcript_item(...)`
  - `clear_transcript()`
  - `transcript_item_count()`
- render order updated to:
  - built-in header
  - transcript
  - pending messages
  - prompt input

Design choices for this slice:
- keep the transcript surface generic and component-based so later user/assistant/tool widgets can plug in directly without redesigning the shell again
- reuse `pi_tui::Container` rather than inventing a second list/render abstraction in `pi-coding-agent-tui`
- keep message-widget parity deferred until the generic placement and mutation semantics are covered by tests

### Validation summary

New Rust coverage added for:
- transcript rendering before pending messages and the prompt
- transcript child order preservation
- transcript item removal and full transcript clearing through the startup shell API
- pending-message truncation continuing to respect terminal width when transcript content is also present

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive shell surface:
- no concrete Rust transcript message widgets yet (`user-message`, `assistant-message`, `tool-execution`, `custom-message`, etc.)
- no Rust multiline editor / custom-editor parity yet; the shell still uses the narrow single-line `Input` slice
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- port the first concrete transcript widget on top of the new transcript container, preferably the smallest message component that does not require a full markdown or box framework
- keep footer, multiline editor, scrolling, and runtime/session wiring deferred until at least one real transcript message type is renderable in Rust

## Milestone 30 update: first concrete transcript widget (`BranchSummaryMessageComponent`) slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/branch-summary-message.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (branch-summary insertion path and transcript layout grounding)
- `packages/coding-agent/src/core/messages.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/messages.rs`
- `rust/crates/pi-coding-agent-tui/Cargo.toml`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/transcript.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

### Behavior summary

New TS-compatible interactive-transcript behavior now covered in Rust:
- `pi-coding-agent-tui` now has its first concrete transcript message widget mirroring the current TypeScript `BranchSummaryMessageComponent`
- the Rust widget preserves the current branch-summary interaction shape for the migrated slice:
  - collapsed by default
  - `[branch]` label
  - collapsed summary line with the configured `app.tools.expand` keybinding hint
  - expandable state via `set_expanded(...)`
  - expanded rendering showing a `Branch Summary` header plus the stored summary text
- the widget now plugs into the previously ported transcript container and startup shell, so a real coding-agent message component can render above pending messages and the prompt

Current intentional compatibility limitation for this slice:
- the Rust widget does not yet use the TS `Box`/theme background treatment
- expanded rendering currently shows plain wrapped text through `pi_tui::Text`, not full markdown rendering through the TS `Markdown` widget
- the label/content styling remains plain in Rust until the broader theme/widget surface is ported

### Rust design summary

New `pi-coding-agent-tui::branch_summary` module:
- `BranchSummaryMessageComponent`
  - backed by `pi_coding_agent_core::BranchSummaryMessage`
  - stores collapsed/expanded state
  - rebuilds an internal `pi_tui::Container` from:
    - spacer
    - label text
    - spacer
    - collapsed or expanded text body
    - trailing spacer
  - exposes `set_expanded(bool)`

Crate-boundary change:
- `pi-coding-agent-tui` now depends on `pi-coding-agent-core` so interactive widgets can consume the existing coding-agent message types directly instead of inventing duplicate Rust-side payload structs

Design choices for this slice:
- use the existing Rust `BranchSummaryMessage` type from core as the compatibility payload source of truth
- keep the widget narrow and text-based for now instead of blocking on full `Box` and `Markdown` parity in `pi-tui`
- validate integration through the existing startup-shell transcript path rather than building a separate transcript harness first

### Validation summary

New Rust coverage added for:
- collapsed branch-summary rendering with expand hint text
- expanded branch-summary rendering with header and summary text
- startup-shell transcript integration with a real branch-summary widget above pending messages and the prompt

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test branch_summary` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust theme/background parity yet for summary/custom transcript widgets
- no full markdown rendering yet in transcript widgets that need it
- no additional concrete message widgets yet (`compaction-summary`, `skill-invocation`, `user-message`, `assistant-message`, `tool-execution`, `custom-message`)
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- port the next smallest transcript widget that can reuse the same text-first pattern, likely `CompactionSummaryMessageComponent` or `SkillInvocationMessageComponent`
- keep full markdown, themed backgrounds, multiline editor parity, and runtime/session wiring deferred until there are a few concrete transcript widgets in place

## Milestone 31 update: second concrete transcript widget (`CompactionSummaryMessageComponent`) slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/compaction-summary-message.ts`
- `packages/coding-agent/src/core/messages.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/branch_summary.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/transcript.rs`
- `rust/crates/pi-coding-agent-tui/tests/branch_summary.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-core/src/messages.rs`
- `rust/crates/pi-tui/src/lib.rs`
- `rust/crates/pi-tui/src/text.rs`

### Behavior summary

New TS-compatible interactive-transcript behavior now covered in Rust:
- `pi-coding-agent-tui` now has a second concrete transcript widget mirroring the current TypeScript `CompactionSummaryMessageComponent`
- the Rust widget preserves the current compaction-summary interaction shape for the migrated slice:
  - collapsed by default
  - `[compaction]` label
  - collapsed summary line with grouped token counts and the configured `app.tools.expand` hint
  - expandable state via `set_expanded(...)`
  - expanded rendering showing the grouped token-count header plus the stored summary text
- the widget plugs into the already-ported transcript container and startup shell, so a second real coding-agent transcript message type can render above pending messages and the prompt

Current intentional compatibility limitation for this slice:
- the Rust widget currently renders the expanded summary through `pi_tui::Text`, not the full TS `Markdown` widget/theme path
- background/theme parity for custom-message-style summary widgets remains deferred

### Rust design summary

New `pi-coding-agent-tui::compaction_summary` module:
- `CompactionSummaryMessageComponent`
  - backed by `pi_coding_agent_core::CompactionSummaryMessage`
  - stores collapsed/expanded state
  - reuses the existing coding-agent keybinding manager for the expand hint
  - rebuilds an internal `pi_tui::Container` from spacer + label + collapsed/expanded body + trailing spacer
- small internal grouped-number formatter for TS-style token-count display (`12,345`)

Crate-surface change:
- `pi-coding-agent-tui` now exports `CompactionSummaryMessageComponent`

Design choices for this slice:
- keep the widget narrow and text-first like the prior branch-summary slice instead of blocking on broader markdown/theme parity
- validate integration through the existing startup-shell transcript path rather than introducing a separate transcript harness first

### Validation summary

New Rust coverage added for:
- collapsed compaction-summary rendering with grouped token counts and expand hint text
- expanded compaction-summary rendering with grouped token counts and summary content
- startup-shell transcript integration with a real compaction-summary widget above pending messages and the prompt
- unit coverage for grouped-number formatting

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test compaction_summary` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` fails in this environment because `biome` is not installed (`sh: biome: command not found`)

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no full markdown/theme parity yet for summary widgets
- no additional concrete message widgets yet for `skill-invocation`, `user-message`, `assistant-message`, `tool-execution`, or `custom-message`
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- port the next text-first transcript widget, with `SkillInvocationMessageComponent` now the most natural follow-up if the required Rust skill payload can be isolated cleanly
- otherwise continue with another low-dependency interactive widget before attempting full markdown/theme/runtime integration

## Milestone 32 update: skill-block parser + `SkillInvocationMessageComponent` slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/core/agent-session.ts` (skill-block parser section)
- `packages/coding-agent/src/modes/interactive/components/skill-invocation-message.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (user-message skill-block insertion path)
- `packages/coding-agent/src/index.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-core/tests/messages.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/branch_summary.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-core` now has a first Rust skill-block parser corresponding to the TypeScript `parseSkillBlock(...)` helper from `agent-session.ts`
- the Rust parser preserves the current TS shape for the migrated slice:
  - exact `<skill name="..." location="..."> ... </skill>` envelope parsing
  - multiline body extraction
  - optional trailing user message after a double newline
  - user-message trimming with empty result -> `None`
  - rejection of non-matching text and malformed trailing suffixes
- `pi-coding-agent-tui` now has `SkillInvocationMessageComponent`, the next concrete transcript widget used by the TypeScript interactive-mode path for parsed skill blocks
- the Rust widget preserves the current TS interaction shape for the migrated slice:
  - collapsed by default
  - collapsed single-line `[skill] <name> (<expand-key> to expand)` summary
  - expanded label + skill name + full skill content rendering
  - the embedded trailing user message is intentionally not rendered by the widget itself, matching the TS split where the skill block and user message render separately
- the widget now plugs into the already-ported transcript container and startup shell, so a third real coding-agent transcript message type can render above pending messages and the prompt

Current intentional compatibility limitation for this slice:
- the expanded Rust widget still uses plain `pi_tui::Text` output rather than TS `Markdown` + theme/background styling
- the separate Rust `UserMessageComponent` path is still unported, so this milestone stops at the parser + skill-block widget split only

### Rust design summary

New `pi-coding-agent-core::skill_block` module:
- `ParsedSkillBlock`
- `parse_skill_block(...)`

Core surface change:
- `pi-coding-agent-core` now exports `ParsedSkillBlock` and `parse_skill_block`

New `pi-coding-agent-tui::skill_invocation` module:
- `SkillInvocationMessageComponent`
  - backed by `pi_coding_agent_core::ParsedSkillBlock`
  - stores collapsed/expanded state
  - resolves the expand hint through the existing coding-agent keybinding manager
  - rebuilds an internal `pi_tui::Container` for collapsed vs expanded rendering

Design choices for this slice:
- keep the parser in `pi-coding-agent-core`, where the TypeScript source of truth already defines it, instead of inventing a TUI-local skill payload type
- keep the widget text-first and low-dependency like the previous branch/compaction summary slices
- stop before porting the separate user-message rendering branch so the parser split remains explicit and testable

### Validation summary

New Rust coverage added for:
- parsing valid skill blocks with and without trailing user messages
- rejecting malformed/non-matching skill-block text
- collapsed skill-invocation rendering with expand hint text
- expanded skill-invocation rendering with skill content while excluding the trailing user message
- startup-shell transcript integration with a real skill-invocation widget above pending messages and the prompt

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-core --test skill_block` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test skill_invocation` passed
- `cd rust && cargo test -p pi-coding-agent-core` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust `UserMessageComponent` yet to complete the TS skill-block + user-message rendering pair
- no full markdown/theme/background parity yet for skill/summary widgets
- no additional concrete message widgets yet for `assistant-message`, `tool-execution`, or generic `custom-message`
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-core`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- port `UserMessageComponent` next so the newly added skill-block parser can drive the full TS split rendering path for parsed user messages
- keep markdown/theme parity and runtime/session wiring deferred until the user/assistant transcript basics are in place

## Milestone 33 update: `UserMessageComponent` slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/user-message.ts`
- `packages/coding-agent/src/modes/interactive/components/assistant-message.ts`
- `packages/coding-agent/src/modes/interactive/components/index.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (user-message and parsed-skill-block insertion path)

Additional Rust files read for this slice:
- `rust/crates/pi-tui/src/tui.rs` (container surface)
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `migration/packages/coding-agent.md`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first Rust `UserMessageComponent` corresponding to the current TypeScript `user-message.ts` widget
- the Rust widget preserves the current TS shape for the migrated slice:
  - leading spacer before the user message body
  - padded user-message body rendered as transcript content
  - OSC 133 prompt-zone wrapping around the rendered user-message block (`A` on first line, `B` + `C` on last line)
- the widget now plugs into the existing transcript container and startup shell, so regular user transcript content can render above pending messages and the prompt
- the newly added Rust skill-block parser plus existing `SkillInvocationMessageComponent` can now be exercised in the same split shape as TypeScript interactive mode:
  - skill block rendered as a collapsible transcript widget
  - trailing user message rendered separately as a user-message widget

Current intentional compatibility limitation for this slice:
- the Rust widget still uses plain `pi_tui::Text` instead of the TS `Markdown` widget with themed background/text coloring
- first-user-message special handling and the broader assistant/user transcript visual system remain deferred

### Rust design summary

New `pi-coding-agent-tui::user_message` module:
- `UserMessageComponent`
  - backed by an internal `pi_tui::Container`
  - composed from a leading `Spacer` and padded `Text`
  - overrides `render(...)` to add the OSC 133 zone markers matching the TS widget contract for the migrated slice

Crate-surface change:
- `pi-coding-agent-tui` now exports `UserMessageComponent`

Design choices for this slice:
- keep the widget narrow and text-first, consistent with the current Rust transcript widgets, instead of blocking on generic markdown/theme parity
- explicitly preserve the OSC 133 prompt-zone behavior now because it is observable output behavior from the TS component even before full styling parity lands
- validate both standalone user-message rendering and parsed skill-block split rendering through the existing startup-shell transcript path

### Validation summary

New Rust coverage added for:
- OSC 133 wrapping of rendered user-message output
- startup-shell transcript integration with a real user-message widget
- parsed skill-block + trailing user-message split rendering order in the transcript

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test user_message` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust markdown/theme/background parity yet for user/skill/summary widgets
- no Rust `AssistantMessageComponent` yet
- no Rust `ToolExecutionComponent` or generic `CustomMessageComponent` yet
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- port `AssistantMessageComponent` next, since it is the core transcript counterpart to the newly added user-message widget
- keep markdown/theme parity and runtime/session wiring deferred until the basic user/assistant transcript pair exists in Rust

## Milestone 34 update: `AssistantMessageComponent` slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/assistant-message.ts`
- `packages/coding-agent/src/modes/interactive/components/user-message.ts`
- `packages/coding-agent/src/modes/interactive/components/index.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (assistant/user transcript insertion grounding)

Additional Rust files read for this slice:
- `rust/crates/pi-events/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/user_message.rs`
- `rust/crates/pi-coding-agent-tui/tests/user_message.rs`
- `rust/crates/pi-tui/src/text.rs`
- `rust/crates/pi-tui/src/spacer.rs`
- `migration/packages/coding-agent.md`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first Rust `AssistantMessageComponent` corresponding to the current TypeScript `assistant-message.ts` widget
- the Rust widget preserves the current TS transcript behavior for the migrated slice:
  - renders assistant text blocks in order
  - renders thinking blocks in order when visible
  - supports `hideThinkingBlock` behavior with a configurable replacement label
  - inserts spacing between visible thinking/text blocks but avoids spacing decisions based on tool-call-only tail content
  - renders terminal aborted/error text only when there are no tool calls in the message
  - maps aborted default text to `Operation aborted`
  - maps error text to `Error: <message>` with `Unknown error` fallback
- the widget now plugs into the existing transcript container and startup shell, so both user and assistant transcript basics now exist in Rust

Current intentional compatibility limitation for this slice:
- the Rust widget still uses plain `pi_tui::Text` rather than the TS `Markdown` widget and theme-driven styling
- tool-call execution blocks are still not rendered by a Rust `ToolExecutionComponent`; this widget only preserves the TS rule that terminal error text is suppressed when tool calls are present

### Rust design summary

New `pi-coding-agent-tui::assistant_message` module:
- `DEFAULT_HIDDEN_THINKING_LABEL`
- `AssistantMessageComponent`
  - stores the last `pi_events::AssistantMessage`
  - supports `set_hide_thinking_block(...)`
  - supports `set_hidden_thinking_label(...)`
  - supports `update_content(...)`
  - rebuilds an internal `pi_tui::Container` from assistant text/thinking/error content each time the message or visibility settings change

Crate-surface change:
- `pi-coding-agent-tui` now exports `AssistantMessageComponent` and `DEFAULT_HIDDEN_THINKING_LABEL`

Design choices for this slice:
- keep the widget text-first and low-dependency so transcript behavior can continue landing without waiting on full markdown/theme parity
- preserve the TS-visible thinking-hide/show and terminal error-suppression rules now, because they affect transcript content ordering and observability even without theme support
- validate through the existing startup-shell transcript path instead of adding a separate renderer harness

### Validation summary

New Rust coverage added for:
- rendering assistant text plus visible thinking blocks
- hiding thinking blocks with the default/custom hidden label
- rendering aborted and error terminal text when no tool calls are present
- suppressing terminal error text when tool calls are present
- startup-shell transcript integration with a real assistant-message widget

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test assistant_message` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust markdown/theme/background parity yet for user/assistant/skill/summary widgets
- no Rust `ToolExecutionComponent` or generic `CustomMessageComponent` yet
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- port `ToolExecutionComponent` next, since the new assistant-message widget now has the TS-aligned rule that tool-call-bearing assistant messages suppress their terminal error text
- keep markdown/theme parity and runtime/session wiring deferred until the core transcript widgets are all present in Rust

## Milestone 35 update: minimal text-first `ToolExecutionComponent` slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/tool-execution.ts`
- `packages/coding-agent/src/modes/interactive/components/assistant-message.ts`
- `packages/coding-agent/src/modes/interactive/components/index.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (tool-call / tool-result insertion and expansion-toggle sections)
- `packages/coding-agent/src/core/messages.ts`
- `packages/coding-agent/src/core/tools/render-utils.ts`
- `packages/coding-agent/test/tool-execution-component.test.ts`
- `packages/coding-agent/test/edit-tool-no-full-redraw.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/Cargo.toml`
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/assistant_message.rs`
- `rust/crates/pi-coding-agent-tui/src/branch_summary.rs`
- `rust/crates/pi-coding-agent-tui/src/compaction_summary.rs`
- `rust/crates/pi-coding-agent-tui/src/skill_invocation.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/transcript.rs`
- `rust/crates/pi-coding-agent-tui/src/user_message.rs`
- `rust/crates/pi-coding-agent-tui/tests/assistant_message.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `rust/crates/pi-coding-agent-core/src/messages.rs`
- `rust/crates/pi-tui/src/text.rs`
- `rust/crates/pi-tui/src/spacer.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first Rust `ToolExecutionComponent` corresponding to the generic text fallback path in TypeScript `tool-execution.ts`
- the Rust widget preserves the current TS fallback shape for the migrated slice:
  - tool name first
  - pretty-printed JSON args below the tool name
  - text result content appended below the args once a result arrives
  - live arg mutation via `update_args(...)`
  - live result mutation via `update_result(...)`
- the Rust widget now renders image result blocks as textual fallback markers in the same `imageFallback(...)` output shape family used by the TS TUI path (`[Image: [mime/type]]` for the current minimal slice)
- the widget now plugs into the existing transcript container and startup shell, so tool-execution transcript content can render above pending messages and the prompt

Current intentional compatibility limitation for this slice:
- the Rust widget is intentionally the generic text-first fallback only; it does not yet port TS built-in/custom renderer slots, theme backgrounds, inline image widgets, diff previews, or renderer-state sharing
- `set_expanded(...)`, `mark_execution_started(...)`, and `set_args_complete(...)` are present for forward compatibility with the TS component lifecycle, but the current Rust fallback slice does not yet have renderer-specific visual changes attached to those states
- image rendering parity remains deferred; the Rust slice currently emits textual fallback markers instead of inline images

### Rust design summary

New `pi-coding-agent-tui::tool_execution` module:
- `ToolExecutionOptions`
- `ToolExecutionResult`
- `ToolExecutionComponent`
  - stores tool name, tool-call id, JSON args, current result, and the migrated state flags needed by the TS lifecycle
  - rebuilds an internal `pi_tui::Container` from a leading spacer plus a padded `Text` block for the current fallback rendering path
  - exposes the narrow mutation surface needed by the TypeScript interaction model:
    - `update_args(...)`
    - `mark_execution_started()`
    - `set_args_complete()`
    - `update_result(...)`
    - `set_expanded(...)`
    - `set_show_images(...)`

Crate-surface change:
- `pi-coding-agent-tui` now exports `ToolExecutionComponent`, `ToolExecutionOptions`, and `ToolExecutionResult`

Design choices for this slice:
- keep the first Rust tool widget deliberately aligned with the TS generic fallback path instead of jumping directly to the full built-in/custom renderer system
- avoid adding a new dependency on `pi-agent`; the widget uses `pi_events::UserContent` plus a small local result struct so it stays focused on transcript rendering only
- validate transcript placement through the existing startup-shell path before attempting runtime/session wiring or built-in renderer parity

### Validation summary

New Rust coverage added for:
- rendering tool name + pretty JSON args + text result output
- updating args after construction and rendering image fallback text for image result blocks
- startup-shell transcript integration with a real tool-execution widget above pending messages and the prompt

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test tool_execution` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust built-in/custom tool renderer parity yet (`renderCall`, `renderResult`, shared renderer state, edit diff previews, built-in tool-specific layouts)
- no Rust inline image rendering parity yet inside tool execution blocks
- no Rust markdown/theme/background parity yet for the broader transcript widget set
- no Rust `CustomMessageComponent` yet
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- port the next concrete transcript widget with similarly narrow scope, likely `CustomMessageComponent`, or deepen `ToolExecutionComponent` with the first built-in renderer slice if tool transcript fidelity is the more immediate need
- keep full markdown/theme parity, scrolling, and runtime/session wiring deferred until the remaining core transcript widgets are present in Rust

## Milestone 36 update: minimal text-first `CustomMessageComponent` slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/custom-message.ts`
- `packages/coding-agent/src/modes/interactive/components/index.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts` (custom-message transcript insertion path)
- `packages/coding-agent/src/core/messages.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/lib.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/src/transcript.rs`
- `rust/crates/pi-coding-agent-tui/src/tool_execution.rs`
- `rust/crates/pi-coding-agent-core/src/messages.rs`
- `migration/packages/coding-agent.md`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `pi-coding-agent-tui` now has a first Rust `CustomMessageComponent` corresponding to the default fallback path in TypeScript `custom-message.ts`
- the Rust widget preserves the current TS fallback shape for the migrated slice:
  - a visible `[customType]` label
  - custom-message text content rendered below the label
  - support for both string content and block-array content from `pi-coding-agent-core::CustomMessage`
- for block-array content, the Rust fallback mirrors the TS default renderer behavior by rendering only text blocks and ignoring image blocks
- the widget now plugs into the existing transcript container and startup shell, so extension/custom transcript content can render above pending messages and the prompt

Current intentional compatibility limitation for this slice:
- the Rust widget does not yet port TS custom renderer callbacks (`MessageRenderer`) or renderer failure fallback behavior
- the Rust widget is intentionally text-first and does not yet port TS boxed background styling or Markdown rendering
- `set_expanded(...)` is present for transcript compatibility but does not yet change visual output in the current fallback slice

### Rust design summary

New `pi-coding-agent-tui::custom_message` module:
- `CustomMessageComponent`
  - backed by `pi_coding_agent_core::CustomMessage`
  - stores expanded state for future renderer parity
  - rebuilds an internal `pi_tui::Container` from a leading spacer plus a padded `Text` block containing the label and fallback body text

Crate-surface change:
- `pi-coding-agent-tui` now exports `CustomMessageComponent`

Design choices for this slice:
- keep the first Rust custom-message widget aligned with the TS fallback path rather than introducing extension renderer plumbing before the surrounding runtime is ported
- reuse the existing core `CustomMessage` payload type directly instead of creating a TUI-local duplicate
- stop at text-block rendering and defer image/Markdown/theme/custom-renderer parity until the interactive runtime actually needs them

### Validation summary

New Rust coverage added for:
- rendering label + string custom-message content
- rendering only text blocks from block-array custom-message content while ignoring image blocks
- startup-shell transcript integration with a real custom-message widget above pending messages and the prompt

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test custom_message` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust custom message renderer callback parity yet
- no Rust boxed background / Markdown parity yet for custom messages
- no Rust built-in/custom tool renderer parity yet inside `ToolExecutionComponent`
- no Rust inline image rendering parity yet inside transcript widgets
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- either deepen `ToolExecutionComponent` with the first built-in renderer slice or port the next concrete transcript widget that still fits the text-first pattern
- keep Markdown/theme parity, scrolling, and runtime/session wiring deferred until the remaining core transcript widgets are present in Rust

## Milestone 37 update: first built-in `ToolExecutionComponent` renderer slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/tool-execution.ts`
- `packages/coding-agent/src/core/tools/read.ts`
- `packages/coding-agent/src/core/tools/write.ts`
- `packages/coding-agent/src/core/tools/edit.ts`
- `packages/coding-agent/src/core/tools/bash.ts`
- `packages/coding-agent/src/core/tools/render-utils.ts`
- `packages/coding-agent/test/tool-execution-component.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/tool_execution.rs`
- `rust/crates/pi-coding-agent-tui/tests/tool_execution.rs`
- `rust/crates/pi-coding-agent-tools/src/read.rs`
- `rust/crates/pi-coding-agent-tools/src/write.rs`
- `rust/crates/pi-coding-agent-tools/src/edit.rs`
- `migration/packages/coding-agent.md`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `ToolExecutionComponent` now has a first built-in renderer branch for the migrated subset of built-in tools instead of always falling back to pretty-printed JSON args
- the Rust built-in slice now covers:
  - `read`
    - path rendering from either `path` or legacy `file_path`
    - offset/limit suffix rendering as `:start-end`
    - trailing-empty-line trimming for rendered text results
  - `write`
    - path rendering from `path`
    - inline preview from the `content` argument instead of raw JSON
    - trailing-empty-line trimming for preview content
    - success-result suppression, matching the TS built-in write renderer shape more closely
  - `edit`
    - path rendering from `path`
    - text-result append path retained for the current Rust slice while diff-preview parity remains deferred
- unknown tools still use the existing generic fallback path
- the current Rust image fallback behavior inside tool results is unchanged and still works in both built-in and generic paths

Current intentional compatibility limitation for this slice:
- this is only the first built-in renderer branch; Rust still does not support TS `renderCall` / `renderResult` callback parity, renderer-state reuse, or built-in override inheritance rules
- `write` preview truncation with keybinding hint text is still deferred; the Rust built-in slice currently focuses on path/content rendering plus trailing-blank-line handling
- `edit` diff rendering parity is still deferred because the current Rust edit-tool details do not yet carry the TS unified diff payload
- `bash` still uses the generic fallback path; its dedicated streaming/result UI remains a separate component in TS and is not part of this slice

### Rust design summary

Expanded `pi-coding-agent-tui::tool_execution` with a narrow built-in formatting path:
- internal built-in dispatch by `tool_name`
- built-in formatters for:
  - `format_read_tool_execution(...)`
  - `format_write_tool_execution(...)`
  - `format_edit_tool_execution(...)`
- small shared helpers for:
  - path extraction from `path` / `file_path`
  - numeric offset/limit extraction
  - read range suffix formatting
  - trailing-empty-line trimming

Design choices for this slice:
- keep the implementation in `tool_execution.rs` instead of introducing a general renderer registry before the Rust interactive runtime exists
- target the highest-value built-in text renderers first (`read`, `write`, `edit`) because they have direct TypeScript tests and visible transcript impact
- preserve the generic fallback path for all other tools so the migrated widget remains broadly usable while built-in parity grows incrementally

### Validation summary

New Rust coverage added for:
- built-in `read` support for legacy `file_path` plus `offset`/`limit` range rendering
- built-in `write` preview rendering with trailing-blank-line trimming and hidden success text
- built-in `read` result rendering with trailing-blank-line trimming
- existing generic tool-execution and startup-shell transcript integration tests continue to pass

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test tool_execution` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust custom renderer callback parity yet for tool execution (`renderCall`, `renderResult`, shared renderer state, built-in override inheritance)
- no Rust `edit` diff preview parity yet
- no Rust `write` preview truncation + expand-hint parity yet
- no Rust inline image rendering parity yet inside transcript widgets
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- continue `ToolExecutionComponent` with the next highest-value built-in renderer increment, likely `edit` diff preview parity or `write` preview truncation/expand behavior
- keep Markdown/theme parity, scrolling, and runtime/session wiring deferred until the remaining core transcript/widget behavior is present in Rust

## Milestone 38 update: built-in `write` preview truncation + expand-hint slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/tool-execution.ts`
- `packages/coding-agent/src/core/tools/write.ts`
- `packages/coding-agent/src/core/tools/render-utils.ts`
- `packages/coding-agent/src/core/keybindings.ts`
- `packages/coding-agent/test/tool-execution-component.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/tool_execution.rs`
- `rust/crates/pi-coding-agent-tui/tests/tool_execution.rs`
- `rust/crates/pi-coding-agent-tui/src/keybindings.rs`
- `rust/crates/pi-coding-agent-tui/src/keybinding_hints.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- `ToolExecutionComponent` now ports the first collapsed/expanded preview behavior from the TypeScript built-in `write` renderer
- the Rust built-in `write` slice now matches the current TS shape for the migrated subset:
  - trailing empty preview lines are still trimmed before rendering
  - collapsed preview shows at most the first 10 lines
  - collapsed preview appends the TS-style summary line `... (<remaining> more lines, <total> total, <key> to expand)`
  - expanded preview shows the full write content and removes the summary line
- expand-hint text is now configurable from the coding-agent keybindings manager instead of being hardcoded, so custom `app.tools.expand` bindings change the rendered hint text just like the TS path
- `ToolExecutionComponent` constructor now takes a `KeybindingsManager`, aligning it with the other Rust transcript widgets that render expand hints from resolved app keybindings

Current intentional limitation for this slice:
- this milestone only ports the `write` preview truncation/expand behavior; Rust still does not have TS `edit` diff preview parity or the broader custom/built-in renderer callback system inside `ToolExecutionComponent`

### Rust design summary

Expanded `pi-coding-agent-tui::tool_execution` with:
- stored `expand_key_text` resolved from `KeybindingsManager`
- built-in `write` preview line limiting via `WRITE_COLLAPSED_PREVIEW_MAX_LINES`
- TS-style collapsed summary-line formatting using the resolved `app.tools.expand` key text
- `set_expanded(...)` now affecting the built-in `write` preview path instead of being a no-op for that renderer slice

Design choices for this slice:
- keep the current Rust tool widget text-first and self-contained instead of introducing the larger TS renderer registry/state system early
- resolve the expand-hint text once at construction time, matching the pattern already used by the Rust branch/compaction/skill widgets
- stay on the deterministic built-in rendering path that can be validated without runtime/session wiring

### Validation summary

New Rust coverage added for:
- collapsed long `write` previews with a configurable expand-hint keybinding
- expanded long `write` previews after `set_expanded(true)`
- existing built-in `read` / `write` trimming and transcript integration tests continue to pass

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test tool_execution` passed
- `cd rust && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust custom renderer callback parity yet for tool execution (`renderCall`, `renderResult`, shared renderer state, built-in override inheritance)
- no Rust `edit` diff preview parity yet
- no Rust inline image rendering parity yet inside transcript widgets
- no Rust markdown/theme/background parity yet for the broader transcript widget set
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- continue `ToolExecutionComponent` with the next highest-value built-in renderer increment, now most likely `edit` diff preview parity
- keep Markdown/theme parity, scrolling, and runtime/session wiring deferred until the remaining core transcript/widget behavior is present in Rust

## Milestone 39 update: `edit` diff-details + transcript diff-rendering slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/tool-execution.ts`
- `packages/coding-agent/src/modes/interactive/components/diff.ts`
- `packages/coding-agent/src/core/tools/edit.ts`
- `packages/coding-agent/src/core/tools/edit-diff.ts`
- `packages/coding-agent/test/tool-execution-component.test.ts`
- `packages/coding-agent/test/edit-tool-no-full-redraw.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tools/src/edit.rs`
- `rust/crates/pi-coding-agent-tools/tests/bash_edit.rs`
- `rust/crates/pi-coding-agent-tui/src/tool_execution.rs`
- `rust/crates/pi-coding-agent-tui/tests/tool_execution.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- successful Rust `edit` tool executions now return diff details alongside `firstChangedLine`
- the new Rust diff-details slice is derived from the same exact-text replacement matching already used by the edit tool, so the rendered diff stays aligned with the actual applied edit ranges
- Rust tool-execution transcript rendering now matches the current TS built-in `edit` shape more closely:
  - call header remains `edit <path>`
  - successful edit results render diff content from `details.diff`
  - success text is suppressed when diff details are present
  - error results still render textual error output
- startup-shell transcript integration now renders the diff body for built-in `edit` executions instead of only the success string

Compatibility note for this slice:
- Rust currently ports the line-level diff/details behavior, not the full TS themed `renderDiff(...)` presentation. The Rust transcript widget remains text-first, so there is no intra-line inverse highlighting or themed color treatment yet.
- the generated Rust diff is compact and context-limited for the current tool-edit shape, but it is not a byte-for-byte clone of the TS `generateDiffString(...)` + `renderDiff(...)` pipeline yet.

### Rust design summary

Expanded `pi-coding-agent-tools::edit` with:
- `AppliedEditsResult.matched_edits`
- internal compact diff generation based on the matched edit ranges already produced by the exact-replacement engine
- tool result `details` now include:
  - `diff`
  - `firstChangedLine`

Expanded `pi-coding-agent-tui::tool_execution` with:
- built-in `edit` rendering that prefers `result.details.diff` over the success text payload
- continued text fallback for error cases

Design choices for this slice:
- keep the diff generation inside the Rust edit-tool implementation instead of adding a broader generic diff/render subsystem first
- reuse the already-validated matched-edit metadata from the exact replacement engine so this slice stays deterministic and local
- keep the TUI rendering text-first and diff-string-based until broader theme/markdown/widget parity is needed

### Validation summary

New Rust coverage added for:
- successful edit tool results carrying `diff` + `firstChangedLine`
- built-in `edit` transcript rendering preferring diff details over success text
- startup-shell transcript integration rendering edit diff output

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tools --test bash_edit` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test tool_execution` passed
- `cd rust && cargo test -p pi-coding-agent-tools && cargo test -p pi-coding-agent-tui` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust themed/intra-line diff rendering parity yet for edit results (`renderDiff(...)` styling)
- no Rust custom renderer callback parity yet for tool execution (`renderCall`, `renderResult`, shared renderer state, built-in override inheritance)
- no Rust inline image rendering parity yet inside transcript widgets
- no Rust markdown/theme/background parity yet for the broader transcript widget set
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- deepen `ToolExecutionComponent` with the next highest-value renderer gap after raw edit diff details, likely themed/intra-line edit diff presentation or built-in/custom renderer override parity
- keep scrolling and runtime/session wiring deferred until the remaining transcript/widget behavior is present in Rust

## Milestone 40 update: styled/intra-line edit diff rendering slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/coding-agent/src/modes/interactive/components/tool-execution.ts`
- `packages/coding-agent/src/modes/interactive/components/diff.ts`
- `packages/coding-agent/test/tool-execution-component.test.ts`
- `packages/coding-agent/test/edit-tool-no-full-redraw.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-coding-agent-tui/src/tool_execution.rs`
- `rust/crates/pi-coding-agent-tui/tests/tool_execution.rs`
- `rust/crates/pi-tui/src/text.rs`
- `rust/crates/pi-tui/src/utils.rs`

### Behavior summary

New TS-compatible behavior now covered in Rust:
- built-in `edit` transcript rendering now ports the next visible part of the TypeScript diff presentation rather than showing the raw diff string unchanged
- Rust now applies a first themed/styled diff rendering pass derived from TS `diff.ts`:
  - context lines rendered in dim/context color
  - removed lines rendered in removed-line color
  - added lines rendered in added-line color
  - tabs in diff content replaced with spaces for stable rendering
- Rust now ports the first intra-line edit highlighting rule from TypeScript:
  - when a diff hunk contains exactly one removed line followed by one added line
  - changed word-level segments are highlighted inside the colored lines using inverse video
  - leading indentation is kept outside inverse highlighting, matching the TS `renderIntraLineDiff(...)` behavior
- existing built-in `edit` transcript rendering still prefers `details.diff` over success text, so the new styling applies directly to the already-migrated diff-detail path
- startup-shell transcript integration continues to render edit diffs correctly with the new ANSI-styled output

Current intentional limitation for this slice:
- Rust now ports the visible styling/intra-line behavior, but it still does not have the full TS theme system behind those colors
- the word diff is a small local token-based implementation, not a byte-for-byte clone of the TypeScript `diffWords(...)` library behavior for every edge case
- the Rust TUI still lacks the broader no-full-redraw/differential-render parity from the TS TUI stack; this milestone is limited to the tool-execution component’s rendered content

### Rust design summary

Expanded `pi-coding-agent-tui::tool_execution` with:
- internal diff-line parser for the Rust `details.diff` text
- ANSI-styled diff rendering helpers for:
  - context
  - removed
  - added
- a small word-token diff path used only for one-removed/one-added line pairs to port the current TS intra-line highlighting rule
- inverse-video highlighting that keeps leading indentation outside the highlighted region

Design choices for this slice:
- keep the styling/rendering logic local to `tool_execution.rs` instead of introducing a generic Rust diff widget before other transcript widgets need it
- reuse the existing ANSI-aware `pi-tui` text rendering path rather than adding a separate rendering layer
- keep the token diff intentionally narrow and deterministic to match the current TS component behavior without overbuilding a general diff engine

### Validation summary

New Rust coverage added for:
- colored context/added/removed edit diff rendering
- inverse-video intra-line highlighting for a single removed/added line pair
- tab replacement in rendered edit diff output
- existing diff-detail and startup-shell transcript integration tests continue to pass using ANSI-stripped assertions where appropriate

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test tool_execution` passed
- `cd rust && cargo test -p pi-coding-agent-tools && cargo test -p pi-coding-agent-tui && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred for the coding-agent interactive transcript surface:
- no Rust custom renderer callback parity yet for tool execution (`renderCall`, `renderResult`, shared renderer state, built-in override inheritance)
- no Rust inline image rendering parity yet inside transcript widgets
- no Rust markdown/theme/background parity yet for the broader transcript widget set
- no Rust footer integration yet
- no scroll behavior or transcript viewport management yet
- no session-manager/resource-loader-backed interactive runtime wiring yet
- no top-level Rust interactive command path yet that instantiates this shell

### Recommended next step

Stay in `packages/coding-agent/src/modes/interactive/components`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-tui`:
- either deepen `ToolExecutionComponent` with built-in/custom renderer override parity or move to transcript viewport/scroll behavior if interaction fidelity is now the higher priority
- keep session/runtime wiring deferred until the remaining transcript/widget behavior is present in Rust
