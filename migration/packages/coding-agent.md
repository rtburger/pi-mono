# packages/coding-agent migration inventory

Status: milestone 4 model-resolution + model-registry + startup bootstrap + minimal non-interactive runtime slices in `rust/crates/pi-coding-agent-core`
Target crates: `rust/crates/pi-coding-agent-core`, later `rust/crates/pi-coding-agent-cli`

## 1. Files analyzed

TypeScript files read in full for these slices:
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
- `packages/coding-agent/src/main.ts`
- `packages/coding-agent/src/cli/args.ts`
- `packages/coding-agent/test/model-resolver.test.ts`
- `packages/coding-agent/test/model-registry.test.ts`
- `packages/coding-agent/test/auth-storage.test.ts`
- `packages/coding-agent/test/args.test.ts`

Rust files reviewed before implementation:
- `rust/Cargo.toml`
- `rust/crates/pi-coding-agent-core/Cargo.toml`
- `rust/crates/pi-coding-agent-core/src/lib.rs`
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/src/model_registry.rs`
- `rust/crates/pi-agent/src/agent.rs`
- `rust/crates/pi-agent/src/error.rs`
- `rust/crates/pi-agent/src/loop.rs`
- `rust/crates/pi-agent/src/message.rs`
- `rust/crates/pi-agent/src/state.rs`
- `rust/crates/pi-agent/src/tool.rs`
- `rust/crates/pi-coding-agent-cli/Cargo.toml`
- `rust/crates/pi-coding-agent-cli/src/lib.rs`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-events/src/lib.rs`
- `migration/packages/agent.md`

Note: this is still a partial inventory scoped to coding-agent core startup/model/bootstrap/runtime behavior, not the whole package.

## 2. Behavior inventory summary

Observed TS behavior now covered by Rust slices:
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

Still deferred:
- dynamic provider registration/unregistration
- OAuth refresh/login behavior
- compat/cost metadata parity
- `resolveModelScope()` glob expansion
- built-in model catalog sourcing from Rust `pi-ai`
- coding-agent custom-message conversion parity (`convertToLlm(messages.ts)`) in the runtime object
- tools/session manager/settings manager integration beyond bootstrap selection

## 3. Compatibility notes and edge cases

Confirmed from TS code/tests and preserved in current Rust slice where implemented:
- invalid `:<suffix>` is a warning only in scope-style parsing; strict CLI parsing treats it as part of the raw model id
- provider inference from the first `/` is preferred for inputs like `zai/glm-5`, even when another provider exposes a literal `zai/glm-5` id
- if provider inference fails for an OpenRouter-style id like `openai/gpt-4o:extended`, resolution retries the full raw id across all models
- partial matching prefers alias ids over dated ids, sorting descending within each class
- saved defaults and restored sessions differ intentionally: saved defaults ignore current auth availability, restored sessions require configured auth
- command-backed API keys are intentionally not executed by `getAvailable()`; presence of config is enough for availability filtering
- request-time API key/header resolution is intentionally uncached in registry paths, matching TS `getApiKeyForProvider()` / `getApiKeyAndHeaders()` behavior
- startup bootstrap mirrors `buildSessionOptions()` plus `createAgentSession()` selection flow closely enough to exercise one real vertical model-selection path
- runtime streaming now mirrors the TS `streamFn` seam conceptually: model/auth selection lives in coding-agent core, while actual provider streaming stays in `pi-ai`

Compatibility deviations currently documented:
- Rust registry currently operates over injected built-in `Vec<Model>` snapshots rather than a Rust-side `pi-ai` built-in model catalog
- Rust does not yet carry TS `compat` and `cost` metadata through registry state
- Rust does not yet port TS dynamic provider registration, OAuth provider integration, or auth.json persistence/locking
- shell command execution currently uses platform shell invocation without TS-style timeout handling
- xhigh-capability clamping from the TS CLI path is not yet ported because Rust does not yet expose the corresponding model-capability helper
- the current runtime object only supports standard prompt/message flow; custom coding-agent message conversion and tool wiring are still pending

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

Design choices for these milestones:
- reuse `pi_agent::ThinkingLevel` for agent parity
- reuse `pi_events::Model` rather than inventing a second model type for the current slice
- keep auth persistence/OAuth out of scope for now behind `AuthSource`
- keep registry side-effect-free apart from explicit disk reads and shell/env resolution
- keep CLI/process exits out of core; warnings/errors return as diagnostics
- make bootstrap pure and non-interactive so later CLI/runtime layers can call it directly
- make runtime reuse `pi-agent::Agent` directly instead of building a parallel state machine in coding-agent core

## 5. Validation plan / test coverage

Rust regression coverage now mirrors TS behavior for:
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

Deferred to later milestones:
- full auth storage port (`auth.json`, runtime overrides, persistence, locking, OAuth refresh)
- dynamic provider lifecycle APIs
- model compat/cost metadata behavior
- scope globbing and settings/session wiring
- tool registration and coding-agent custom-message conversion on top of the runtime object

## 6. Known risks / open questions

- Rust still lacks a built-in model catalog sourced from `packages/ai`; callers currently inject built-in models
- `pi_events::Model` is narrower than TS `Model<Api>` and will likely need widening or side metadata for compat/cost-sensitive behavior
- the current `AuthSource` seam is intentionally minimal and may need reshaping when auth.json/OAuth support is ported
- `resolveModelScope()` glob semantics remain unported, so model cycling/config scope is still incomplete
- request command execution does not yet implement the TS timeout behavior
- runtime currently exposes `pi-agent::Agent` directly; later milestones need to decide whether to keep that as the primary core API or wrap it in a coding-agent-specific session/runtime facade

## 7. Recommended next step

Port the next coding-agent-core integration layer on top of this runtime object:
- add a minimal Rust equivalent of the coding-agent message conversion layer from `messages.ts`
- wire in at least the default coding tools (`read`, `bash`, `edit`, `write`) through `pi-agent::AgentTool`
- keep it non-interactive and session-manager-free for one more milestone

That would produce the first genuinely useful non-interactive coding-agent-core path, not just startup/model selection.
