# packages/ai migration inventory

Status: milestone 21 adds Rust `stream_simple()` / `complete_simple()` parity for the narrowed in-scope providers and freezes the remaining high-value `SimpleStreamOptions` request-mapping behavior against the TypeScript implementation
Target crate: `rust/crates/pi-ai`

## 1. Files analyzed

Source inventory reviewed from `packages/ai/src`:
- `index.ts`, `types.ts`, `stream.ts`, `api-registry.ts`, `models.ts`, `env-api-keys.ts`, `cli.ts`, `oauth.ts`, `bedrock-provider.ts`
- `providers/register-builtins.ts`, `providers/simple-options.ts`, `providers/transform-messages.ts`, `providers/faux.ts`, `providers/github-copilot-headers.ts`
- utility modules reviewed directly: `utils/event-stream.ts`, `utils/json-parse.ts`, `utils/overflow.ts`, `utils/validation.ts`, `utils/hash.ts`, `utils/sanitize-unicode.ts`, `utils/typebox-helpers.ts`, `utils/oauth/index.ts`, `utils/oauth/types.ts`, `utils/oauth/pkce.ts`
- package metadata/docs reviewed: `README.md`, `package.json`
- large provider implementations inventoried by file and line count for migration planning: `anthropic.ts`, `openai-completions.ts`, `openai-responses.ts`, `openai-responses-shared.ts`, `openai-codex-responses.ts`, `google.ts`, `google-shared.ts`, `google-gemini-cli.ts`, `google-vertex.ts`, `mistral.ts`, `amazon-bedrock.ts`, `azure-openai-responses.ts`
- generated model catalog inventoried: `models.generated.ts` (14k+ LOC; model metadata source, not first-slice behavior target)

Test inventory reviewed directly or via targeted extraction from `packages/ai/test`:
- fully read for first-slice behavior: `faux-provider.test.ts`, `validation.test.ts`, `overflow.test.ts`, `abort.test.ts`, `tool-call-without-result.test.ts`, `transform-messages-copilot-openai-to-anthropic.test.ts`
- broad suite inventory via test titles and filenames: `stream.test.ts`, `tokens.test.ts`, `empty.test.ts`, `context-overflow.test.ts`, `cross-provider-handoff.test.ts`, `image-tool-result.test.ts`, `unicode-surrogate.test.ts`, `total-tokens.test.ts`, `responseid.test.ts`, `openai-codex-stream.test.ts`, `lazy-module-load.test.ts`, `cache-retention.test.ts`, OAuth/provider-specific tests, and provider normalization tests.

## 2. Exported API inventory

Current TS package exports these public surfaces:
- root barrel exports model registry, API registry, streaming helpers, env auth helpers, validation helpers, event stream helpers, overflow helpers, faux provider, TypeBox re-exports, OAuth types
- core entry points:
  - `stream(model, context, options)`
  - `complete(model, context, options)`
  - `streamSimple(model, context, options)`
  - `completeSimple(model, context, options)`
- model registry API:
  - `getModel`, `getModels`, `getProviders`, `modelsAreEqual`, `supportsXhigh`, `calculateCost`
- registry API:
  - `registerApiProvider`, `getApiProvider`, `getApiProviders`, `unregisterApiProviders`, `clearApiProviders`
- faux provider test API:
  - `registerFauxProvider`, `fauxAssistantMessage`, `fauxText`, `fauxThinking`, `fauxToolCall`
- validation + compat helpers:
  - `validateToolCall`, `validateToolArguments`, `transformMessages`, `isContextOverflow`, `parseStreamingJson`
- provider-specific lazy wrappers and provider option types
- OAuth provider registry and login/refresh helpers under `oauth`

## 3. Internal architecture summary

The TS package is layered roughly as:
1. shared discriminated-union message model in `types.ts`
2. event stream abstraction in `utils/event-stream.ts`
3. global API registry in `api-registry.ts`
4. model catalog/lookup in `models.ts` backed by generated metadata in `models.generated.ts`
5. generic `stream` / `complete` dispatch in `stream.ts`
6. provider implementations under `src/providers/`
7. auth/env/OAuth helpers around provider dispatch
8. extensive provider compatibility transforms for handoff, tool calls, thinking content, token accounting, and request shaping

Notable behavioral seams:
- provider modules are lazy-loaded from `register-builtins.ts`
- provider failures are encoded into terminal assistant messages rather than thrown from stream functions
- cross-provider replay uses message normalization rather than preserving provider-native transcript data blindly
- faux provider exists as deterministic scripted provider for tests and demos

## 4. Dependency summary

Key TS runtime dependencies:
- transport/client SDKs: `openai`, `@anthropic-ai/sdk`, `@google/genai`, `@mistralai/mistralai`, `@aws-sdk/client-bedrock-runtime`
- validation/schema: `@sinclair/typebox`, `ajv`, `ajv-formats`, `zod-to-json-schema`
- streaming/utilities: `partial-json`, `undici`, `proxy-agent`

First Rust target dependencies chosen:
- `tokio`, `futures`, `serde`, `serde_json`, `thiserror`, `async-stream`, `reqwest`
- `reqwest` is now used for the first minimal live OpenAI Responses transport path

## 5. Config / env var summary

Observed env/config semantics in TS:
- API key resolution by provider in `env-api-keys.ts`
- special auth detection for Vertex ADC and Bedrock credential chains
- cache retention option (`none | short | long`)
- session-aware prompt caching semantics for some providers
- max retry delay, payload inspection hooks, metadata headers, transport preference
- OAuth-backed providers: anthropic, github-copilot, google-gemini-cli, google-antigravity, openai-codex

Current Rust slice preserves:
- built-in model catalog loading directly from TypeScript `packages/ai/src/models.generated.ts`
- provider/model lookup helpers (`get_model`, `get_models`, `get_providers`)
- `supports_xhigh()` and `models_are_equal()` behavior from `packages/ai/src/models.ts`
- broader env API-key lookup coverage across the static provider env vars currently implemented in TS `env-api-keys.ts`
- `session_id`
- `cache_retention`
- explicit `api_key`
- `temperature`
- `max_tokens`
- `reasoning_effort`
- `reasoning_summary`
- abort signaling

## 6. Runtime behavior summary

Behavior that appears central across providers:
- normalized message/content model supports text, thinking, images, tool calls, tool results
- normalized stream emits ordered events: `start`, content start/delta/end events, then terminal `done` or `error`
- terminal errors are represented as assistant messages with `stopReason = error | aborted`
- tool call arguments may stream as JSON deltas before final structured tool call
- prompt caching affects usage accounting, not just transport payloads
- aborted turns may be retained in history but should not poison later replays
- cross-provider replay rewrites/drops provider-specific thinking/tool metadata when necessary

## 7. Test inventory summary

High-value behavioral clusters from the TS suite:
- basic streaming and provider interoperability: `stream.test.ts`
- abort semantics and continuation after abort: `abort.test.ts`
- token and usage accounting: `tokens.test.ts`, `total-tokens.test.ts`
- empty stream / empty assistant handling: `empty.test.ts`
- orphaned tool call recovery: `tool-call-without-result.test.ts`
- cross-provider handoff and reasoning replay: `cross-provider-handoff.test.ts`, `openai-responses-reasoning-replay-e2e.test.ts`
- tool/image routing and Unicode sanitization: `image-tool-result.test.ts`, `unicode-surrogate.test.ts`
- provider-specific request normalization and lazy loading tests
- deterministic faux-provider tests are the best first Rust port target

## 8. Edge cases and implicit behaviors

Confirmed or strongly implied by source/tests:
- stream implementations should not throw for normal upstream failures; they should yield terminal error events
- `toolcall_delta` partial args may be incomplete or empty
- tool call IDs need provider-specific normalization during handoff
- redacted thinking is same-model-only replay data
- validation is skipped in strict CSP / no-eval environments
- overflow detection is regex-based and provider-specific, with exclusions for rate limiting
- prompt cache accounting uses serialized context prefix matching in faux provider tests
- immediate abort before first chunk should still produce a terminal aborted assistant message

## 9. Compatibility notes for Rust rewrite

Phase 1 compatibility target is intentionally narrow:
- preserve the normalized event protocol
- preserve faux-provider queue semantics
- preserve usage estimation and prompt-cache simulation from the TS faux provider
- preserve abort behavior and terminal error-message encoding
- preserve the built-in model catalog and the small model-helper surface needed by coding-agent startup (`get_model`, `get_models`, `get_providers`, `supports_xhigh`, `models_are_equal`)

Deferred from phase 1:
- real HTTP providers
- lazy provider module loading behavior
- OAuth flows
- full generated model catalog
- message transform parity for all provider combinations
- JSON partial parsing and schema validation parity

## 10. Rust target design (`pi-ai`)

Planned crate boundary:
- `pi-events`: shared assistant/user/tool message types and stream event enums
- `pi-ai`: provider registry, stream/complete entry points, provider trait, provider implementations

Current module shape implemented in first slice:
- `pi-events`: normalized message/event/model types
- `pi-ai`: registry, `AiProvider` trait, faux provider registration, `stream_response`, `complete`

Public API goals for `pi-ai`:
- `register_provider`, `unregister_provider`
- `register_faux_provider`
- `stream_response`
- `complete`
- explicit `AiError`
- `AssistantEventStream = Stream<Item = Result<AssistantEvent, AiError>>`

Known risks:
- current Rust slice uses a minimal subset of the TS type system
- built-in catalog currently parses the TypeScript-generated source at runtime via embedded source text rather than using a Rust-native generated catalog step
- no real provider normalization yet beyond the current OpenAI Responses slice
- prompt cache and serialization logic covers only first-slice faux behavior
- TS uses some behaviors based on JavaScript dynamic flexibility that will need more explicit Rust enums/traits later

Validation plan:
- port deterministic faux-provider cases first
- derive fixtures from TS event order expectations
- then add shared transform/usage tests before any HTTP provider work
- first real provider candidate should likely be OpenAI Responses or Anthropic, but only after faux parity is stable

## 11. OpenAI Responses provider-specific inventory

Files read fully for the first real-provider slice:
- `packages/ai/src/providers/openai-responses.ts`
- `packages/ai/src/providers/openai-responses-shared.ts`
- `packages/ai/test/openai-responses-copilot-provider.test.ts`
- `packages/ai/test/openai-responses-foreign-toolcall-id.test.ts`
- `packages/ai/test/openai-responses-reasoning-replay-e2e.test.ts`
- `packages/ai/test/openai-responses-tool-result-images.test.ts`

Observed OpenAI Responses behaviors relevant to the current real-provider slices:
- payload-building is split from stream processing
- system prompt becomes `developer` role for reasoning-capable models, else `system`
- request payloads may include function tool definitions derived from context tools
- assistant text history is replayed as completed assistant `message` items with output-text content
- streamed assistant text captures a reusable text signature encoding message id and optional phase
- streamed reasoning summaries capture a reusable serialized reasoning item signature
- same-model replay can feed serialized reasoning items back into OpenAI request input
- same-model replay can feed signed assistant text back with preserved message id/phase
- assistant tool calls are replayed as `function_call` items
- orphaned tool calls are backfilled with synthetic error tool results before replay continues
- errored/aborted assistant turns are skipped during replay
- foreign tool call item IDs are normalized into bounded `fc_<hash>` form for OpenAI-safe replay
- same-provider different-model handoff omits `fc_*` item IDs to avoid OpenAI pairing validation failures
- cross-model thinking is converted to plain assistant text when signatures are not reusable
- tool results with images stay nested inside `function_call_output.output` rather than being emitted as separate user messages
- same-provider / same-model replay preserves more metadata; cross-model/cross-provider replay drops or rewrites some data
- Copilot OpenAI Responses should omit `reasoning` payload when not requested

Current Rust provider slice implements deterministic request-building coverage for:
- foreign tool-call ID normalization
- assistant tool-call conversion to `function_call`
- tool-result image packing into `function_call_output`
- function tool-definition conversion from context tools with `strict: false`
- Copilot default omission of `reasoning`
- `developer` vs `system` role selection for system prompt replay
- OpenAI prompt-cache parameter shaping for `sessionId` + long retention

Deferred OpenAI Responses work:
- redacted reasoning parity and explicit encrypted-content replay rules
- broader Copilot auth parity beyond current bearer/env/header slice
- model-catalog integration for the runtime provider
- broader parity for provider-specific runtime options beyond the current minimal passthrough
- live aborted-stream usage/accounting parity where upstream reports usage before termination

Current Rust runtime provider path also includes:
- lazy built-in registration of the minimal `openai-responses` provider on first dispatch
- API key resolution from explicit stream options or provider env vars (`OPENAI_API_KEY`, Copilot token env fallbacks)
- passthrough of `max_tokens`, `temperature`, `reasoning_effort`, `reasoning_summary`, `session_id`, `cache_retention`, and context tools
- request-header merging via runtime options
- GitHub Copilot dynamic request headers (`X-Initiator`, `Openai-Intent`, `Copilot-Vision-Request`)
- immediate abort handling before HTTP send and while awaiting streamed body chunks

Current Rust transport-adjacent coverage now includes:
- SSE `data:` frame parsing from raw text
- incremental SSE frame assembly across arbitrary HTTP chunk boundaries
- `[DONE]` handling
- invalid-JSON SSE failure detection
- direct text-to-event-stream bridging for deterministic tests
- live HTTP POST -> `reqwest` body chunk stream -> normalized event stream flow
- HTTP failure mapping to terminal assistant error events
- provider-registry dispatch for a minimal `openai-responses` runtime path
- abort handling while waiting for the next HTTP body chunk

Current Rust streaming coverage now includes deterministic parsing for:
- `response.created`
- `response.output_item.added`
- `response.reasoning_summary_part.added`
- `response.reasoning_summary_text.delta`
- `response.reasoning_summary_part.done`
- `response.output_text.delta`
- `response.refusal.delta`
- `response.function_call_arguments.delta`
- `response.output_item.done`
- `response.completed`
- `response.failed`
- `error`

Covered stream/request semantics in Rust tests:
- text start/delta/end event order
- tool call start/delta/end event order
- reasoning start/delta/end event order
- streamed text signature capture from OpenAI message items
- streamed reasoning signature capture from OpenAI reasoning items
- same-model replay of serialized reasoning items and signed assistant text
- function tool-definition conversion in OpenAI request params
- `response.completed` stop-reason mapping to `stop` / `toolUse`
- `response.failed` to terminal assistant error event
- response-id capture on created/failed responses
- partial usage extraction on terminal error events
- incremental HTTP chunk parsing across split SSE frames
- abort while waiting for the next streamed body chunk
- replay filtering of aborted assistant turns
- synthetic tool-result insertion for orphaned tool calls
- same-provider different-model `fc_*` item-id elision
- cross-model thinking-to-text conversion for replay
- GitHub Copilot dynamic headers and runtime header override precedence

## 12. Unknowns requiring validation

- exact provider selection/order after the OpenAI Responses payload slice
- whether the temporary runtime parsing of TS `models.generated.ts` should later become a checked-in Rust-generated artifact or a build-time generation step
- how much of TS `SimpleStreamOptions` reasoning normalization should live in `pi-ai` vs provider-specific modules
- whether faux provider should remain in `pi-ai` or move to `pi-test-harness` after the first provider lands
- whether to continue AI with validation/tool execution plumbing for agent support or switch to Anthropic for a second end-to-end provider

## Milestone 8 update: Anthropic Messages provider slice

### Files analyzed

Additional TypeScript files read for this slice:
- `packages/ai/src/providers/anthropic.ts`
- `packages/ai/src/providers/register-builtins.ts` (Anthropic lazy-registration path)
- `packages/ai/test/anthropic-thinking-disable.test.ts`
- `packages/ai/test/anthropic-tool-name-normalization.test.ts`
- `packages/ai/test/anthropic-oauth.test.ts`
- `packages/ai/test/github-copilot-anthropic.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/tests/openai_responses_payload.rs`
- `rust/crates/pi-ai/tests/openai_responses_stream.rs`
- `rust/crates/pi-ai/tests/openai_responses_provider.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- built-in runtime registration of a second real provider path: `anthropic-messages`
- Anthropic request conversion for:
  - system prompts
  - user text/image messages
  - assistant text/thinking/tool-call replay
  - grouped tool-result replay as a single user message
  - tool-definition conversion
- Anthropic cross-provider replay shaping for the current slice:
  - foreign tool-call ID normalization to Anthropic-safe IDs
  - skipping aborted/errored assistant turns during replay
  - synthetic tool-result insertion for orphaned tool calls
  - cross-model thinking-to-text fallback while preserving same-model signed/redacted thinking blocks
- Anthropic cache-control shaping for the current request slice:
  - short/long retention mapping to `cache_control: { type: "ephemeral" }`
  - `ttl: "1h"` only for direct `api.anthropic.com` long-retention requests
- Anthropic OAuth / Copilot request behavior for the current slice:
  - Claude Code identity system prompt for Anthropic OAuth tokens
  - Claude Code tool-name canonicalization on outbound requests and reverse normalization on inbound streamed tool calls
  - GitHub Copilot bearer auth plus dynamic headers (`X-Initiator`, `Openai-Intent`, `Copilot-Vision-Request`)
  - Copilot static header slice needed by the migrated tests (`User-Agent`, `Editor-Version`, `Editor-Plugin-Version`, `Copilot-Integration-Id`)
- Anthropic SSE parsing and normalized stream behavior for:
  - `message_start`
  - `content_block_start`
  - `content_block_delta` for text, thinking, signature, and tool JSON deltas
  - `content_block_stop`
  - `message_delta`
  - `error`
- runtime terminal-message behavior parity for:
  - missing API key -> terminal assistant error message
  - HTTP failure -> terminal assistant error message
  - explicit streamed error event -> terminal assistant error message

Current intentional limitations of this slice:
- Rust still does not expose TS `streamSimple()` / `completeSimple()` or full provider-specific option parity; the runtime bridge currently maps the existing generic `StreamOptions.reasoning_effort` into a narrow Anthropic thinking configuration
- partial tool JSON parsing is still best-effort and currently falls back to `{}` until JSON becomes valid
- model-catalog headers are still not loaded generically from `models.generated.ts`; the current Copilot Anthropic path carries the required static headers in provider code for this slice
- OAuth login/refresh remains in coding-agent auth sources, not in `pi-ai` provider modules yet

### Rust design summary

New Rust module added:
- `rust/crates/pi-ai/src/anthropic_messages.rs`

Public/provider-facing Rust surface added in that module:
- `AnthropicOptions`
- `AnthropicRequestParams`
- `AnthropicStreamEnvelope`
- `build_anthropic_request_params()`
- `convert_anthropic_messages()`
- `normalize_anthropic_tool_call_id()`
- `stream_anthropic_http()`
- `stream_anthropic_sse_events()`
- `register_anthropic_provider()`

Integration changes:
- `rust/crates/pi-ai/src/lib.rs`
  - now exposes `pub mod anthropic_messages`
  - `register_builtin_providers()` now registers both Anthropic Messages and OpenAI Responses

Behavior-freeze artifacts added:
- `rust/crates/pi-ai/tests/fixtures/anthropic_messages_stream_mixed.json`

### Validation summary

New Rust coverage added for:
- Anthropic thinking-disable payload shaping
- Anthropic long-cache TTL shaping
- Anthropic OAuth tool-name normalization
- grouped tool-result conversion
- foreign tool-call ID normalization for Anthropic-safe replay
- Anthropic mixed text/thinking/tool-call stream event ordering
- Anthropic explicit streamed error events
- end-to-end HTTP dispatch through the provider registry
- GitHub Copilot Anthropic header behavior
- missing API-key terminal error behavior

Validation run results:
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- full TS `streamSimple` / `completeSimple` API parity
- broader Anthropic provider option parity (`toolChoice`, richer metadata passthrough, direct provider-specific public API surface from the crate root)
- Anthropic abort regression tests and additional parity around empty streams / Unicode / token-accounting edge cases from the broader TS suite
- next providers after Anthropic (`openai-completions`, Google, Mistral, Bedrock, Azure Responses)

## Milestone 9 update: static model-header extraction slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/providers/openai-responses.ts` (header merge order)
- `packages/ai/src/models.generated.ts` (built-in `headers` fields for Copilot models)

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/models.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/tests/models.rs`
- `rust/crates/pi-ai/tests/openai_responses_provider.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_http.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- built-in static request headers are now parsed from `packages/ai/src/models.generated.ts`
- Rust `pi-ai` now exposes catalog-backed header lookup helpers for:
  - exact `provider + model id`
  - provider-level fallback when a runtime model id is not directly catalogued but the built-in provider uses shared static headers (current Copilot tests use this path)
- `anthropic-messages` runtime header construction now uses TS-generated static model headers before dynamic Copilot headers and user overrides
- `openai-responses` runtime header construction now uses TS-generated static model headers before dynamic Copilot headers and user overrides
- Copilot static header behavior is therefore no longer hardcoded only in the Anthropic provider slice

Compatibility note for this slice:
- provider-level fallback is intentionally narrow migration glue. It preserves the prior Rust behavior for Copilot-flavored runtime models whose ids are not direct catalog hits, without widening `pi_events::Model` yet.

### Rust design summary

Expanded `rust/crates/pi-ai/src/models.rs` with:
- catalog storage of parsed static model headers
- `get_model_headers(provider, model_id)`
- `get_provider_headers(provider)`

Provider integrations updated:
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`

The merge order now matches the TS provider code more closely:
- built-in model headers
- provider-generated dynamic headers
- per-request override headers

### Validation summary

New Rust coverage added for:
- loading static Copilot headers from the TS-generated catalog
- provider-level static-header fallback lookup
- OpenAI Responses Copilot dispatch now asserting static header presence in HTTP tests
- Anthropic Copilot dispatch still passing with static headers sourced from the catalog/fallback path

Validation run results:
- `cd rust && cargo test -p pi-ai --test models --test openai_responses_provider --test anthropic_messages_http` passed
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- carrying static built-in headers directly on the Rust `Model` value instead of side-channel catalog lookup
- full TS `streamSimple` / `completeSimple` API parity
- next provider slice after Anthropic / header extraction, likely `openai-completions`

## Milestone 10 update: OpenAI Completions request-shaping slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/providers/openai-completions.ts`
- `packages/ai/src/providers/transform-messages.ts`
- `packages/ai/test/openai-completions-tool-choice.test.ts`
- `packages/ai/test/openai-completions-tool-result-images.test.ts`
- targeted references from broader suites where `openai-completions` behavior is exercised:
  - `packages/ai/test/stream.test.ts`
  - `packages/ai/test/abort.test.ts`
  - `packages/ai/test/tool-call-without-result.test.ts`
  - `packages/ai/test/image-tool-result.test.ts`
  - `packages/ai/test/tokens.test.ts`
  - `packages/ai/test/total-tokens.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/tests/openai_responses_payload.rs`
- `rust/crates/pi-ai/tests/openai_responses_params.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- first `openai-completions` migration slice exists in `pi-ai` as a request-shaping/module-level compatibility layer
- Rust now ports the message replay shaping needed before a runtime transport lands:
  - system prompt role selection (`developer` vs `system`)
  - user text/image conversion
  - assistant text/tool-call replay
  - synthetic orphaned tool results (`No result provided`)
  - consecutive tool-result image batching into a follow-up user image message
  - optional assistant bridge insertion after tool results for compat-constrained providers
- Rust now ports the first `openai-completions` compat/detection slice from TS:
  - Groq `qwen/qwen3-32b` reasoning-effort remap to `default`
  - OpenRouter reasoning-object shaping
  - z.ai `tool_stream` detection for the current TS-backed model ids (`glm-5`, `glm-4.7`, `glm-4.7-flash`, `glm-4.6v`)
  - Chutes `max_tokens` field detection
  - strict-tool-definition omission when compat disables `strict`
- Rust request param shaping now covers:
  - `tool_choice`
  - `stream_options.include_usage`
  - `store: false` when supported
  - `max_completion_tokens` vs `max_tokens`
  - OpenAI/OpenRouter/z.ai thinking parameter differences for the migrated slice

Intentional limitation of this milestone:
- this is not yet a runtime provider registration slice; no Rust `openai-completions` HTTP/SSE transport has been added yet
- token-cost calculation, abort/runtime streaming parity, and `streamSimple()` / `completeSimple()` parity remain deferred until the transport path lands

### Rust design summary

New Rust module added:
- `rust/crates/pi-ai/src/openai_completions.rs`

Public surface added in that module:
- `ReasoningEffort`
- `OpenAiCompletionsCompat`
- `OpenAiCompletionsRequestOptions`
- `OpenAiCompletionsToolChoice`
- `OpenAiCompletionsRequestParams`
- `OpenAiCompletionsMessageParam`
- `detect_openai_completions_compat()`
- `build_openai_completions_request_params()`
- `convert_openai_completions_messages()`
- `normalize_openai_completions_tool_call_id()`

Integration change:
- `rust/crates/pi-ai/src/lib.rs` now exposes `pub mod openai_completions`

### Validation summary

New Rust coverage added for:
- tool-result image batching for `openai-completions` message conversion
- synthetic orphaned tool-result insertion
- reasoning-model system-prompt role selection
- assistant-bridge insertion after tool results when compat requires it
- tool-choice passthrough and default `strict: false`
- strict omission when compat disables strict mode
- OpenRouter reasoning-object shaping
- Groq Qwen reasoning-effort remapping
- z.ai `tool_stream` + `enable_thinking` shaping
- Chutes `max_tokens` field detection

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- `openai-completions` runtime transport/stream parsing and provider registration
- token/cost accounting parity for `openai-completions` streamed chunks
- broader compat override plumbing from TS model metadata (`model.compat`) instead of only the current Rust detection/request-option surface
- next provider/runtime slice should stay on `openai-completions` and add the actual HTTP/SSE stream path before moving on to another provider

## Milestone 11 update: OpenAI Completions runtime + scope-pruning slice

### Files analyzed

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/models.rs`
- `rust/crates/pi-ai/src/openai_completions.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/tests/models.rs`
- `rust/crates/pi-ai/tests/openai_completions_params.rs`
- `rust/crates/pi-ai/tests/openai_completions_stream.rs`
- `rust/crates/pi-ai/tests/openai_completions_http.rs`
- `rust/crates/pi-ai/tests/openai_responses_http.rs`
- `rust/crates/pi-ai/tests/openai_responses_payload.rs`
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/src/auth.rs`
- `rust/crates/pi-coding-agent-core/tests/model_resolver.rs`
- `rust/crates/pi-coding-agent-core/tests/model_registry.rs`
- `rust/crates/pi-coding-agent-core/tests/auth.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- `openai-completions` now has a runtime provider path in `pi-ai`
- Rust now supports live `openai-completions` HTTP POST + SSE streaming for the current migration scope:
  - `/chat/completions` transport via `reqwest`
  - incremental SSE decoding across arbitrary chunk boundaries
  - `[DONE]` sentinel handling
  - abort before send and while waiting for the next response chunk
  - terminal assistant error messages for missing API key, HTTP failures, and aborted requests
- normalized streamed event coverage now exists for `openai-completions`:
  - text start/delta/end
  - thinking start/delta/end from `reasoning_content` / `reasoning` / `reasoning_text`
  - tool-call start/delta/end from streamed `tool_calls`
  - terminal `done` / `error` mapping from `finish_reason`
  - streamed usage normalization including cached-token and reasoning-token handling
- built-in provider registration now includes `openai-completions`
- Rust migration scope pruning now removes the remaining non-target provider branches from the current Rust AI/core surface:
  - `pi-ai` built-in model catalog now exposes only `anthropic`, `openai`, and `openai-codex`
  - `pi-ai` env-key lookup now resolves only the current in-scope provider env vars
  - `openai-responses` replay/provider allowlists no longer mention out-of-scope runtime providers
  - `pi-coding-agent-core` default-model table now only contains Anthropic/OpenAI/OpenAI Codex entries
  - `pi-coding-agent-core` OAuth/auth-file handling no longer carries Google refresh/translation logic in Rust

Compatibility note for this slice:
- the prior request-shaping-only `openai-completions` compat branches for OpenRouter/Groq/z.ai were intentionally removed from the current Rust migration scope rather than preserved behind dead compatibility toggles

### Rust design summary

`rust/crates/pi-ai/src/openai_completions.rs` now includes runtime/provider-facing additions:
- `OpenAiCompletionsChunk` SSE payload model
- SSE decoder + text parser helpers
- `stream_openai_completions_sse_text()`
- `stream_openai_completions_chunks()`
- `stream_openai_completions_http()`
- `stream_openai_completions_http_with_headers()`
- `OpenAiCompletionsProvider`
- `register_openai_completions_provider()`

Integration updates:
- `rust/crates/pi-ai/src/lib.rs`
  - built-in env-key resolution narrowed to the currently migrated providers
  - built-in provider registration now includes `openai-completions`
- `rust/crates/pi-ai/src/models.rs`
  - catalog filtering narrowed to the currently migrated providers
- `rust/crates/pi-coding-agent-core/src/model_resolver.rs`
  - default model table narrowed to the current migration scope
- `rust/crates/pi-coding-agent-core/src/auth.rs`
  - removed Google OAuth refresh/credential-translation branches, leaving Anthropic + OpenAI Codex handling for the current scope

### Validation summary

New Rust coverage added for:
- `openai-completions` SSE parsing with `[DONE]`
- text streaming event order
- tool-call streaming event order
- reasoning streaming event order
- live HTTP transport for `openai-completions`
- registry dispatch through `stream_response()` for `openai-completions`
- abort while waiting for next streamed body chunk
- missing API-key terminal error behavior
- filtered provider catalog / env-key behavior for the narrowed migration scope
- narrowed coding-agent-core default-model/auth/model-registry behavior

Validation run results:
- `cd rust && cargo test -p pi-ai --test models --test openai_completions_params --test openai_responses_http --test openai_responses_payload --test openai_completions_stream --test openai_completions_http` passed
- `cd rust && cargo test -p pi-coding-agent-core --test auth --test model_registry --test model_resolver` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- `openai-codex-responses` runtime/provider registration as a distinct Rust API path
- fuller `openai-completions` parity for provider-specific compat overrides sourced from TS model metadata
- broader token/cost parity against the TS suite beyond the current streamed usage normalization slice
- additional Rust ports for TS regressions around empty streams, Unicode, cross-provider handoff, and context-overflow behavior within the narrowed provider scope

## Milestone 12 update: OpenAI Codex Responses runtime/provider slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/providers/openai-codex-responses.ts`
- `packages/ai/src/providers/openai-responses-shared.ts`
- `packages/ai/src/env-api-keys.ts`
- `packages/ai/test/openai-codex-stream.test.ts`
- targeted narrowed-regression references reviewed before the next slice:
  - `packages/ai/test/abort.test.ts`
  - `packages/ai/test/empty.test.ts`
  - `packages/ai/test/responseid.test.ts`
  - `packages/ai/test/tool-call-without-result.test.ts`
  - `packages/ai/test/image-tool-result.test.ts`
  - `packages/ai/test/unicode-surrogate.test.ts`
  - `packages/ai/test/context-overflow.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/models.rs`
- `rust/crates/pi-ai/tests/openai_responses_http.rs`
- `rust/crates/pi-ai/tests/openai_responses_stream.rs`
- `rust/crates/pi-ai/tests/openai_responses_payload.rs`
- `rust/crates/pi-ai/tests/openai_completions_http.rs`
- `rust/crates/pi-ai/tests/openai_completions_stream.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_http.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_stream.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- `openai-codex-responses` now has its own built-in Rust provider registration and runtime API path
- Codex request shaping now reuses the migrated OpenAI Responses message-replay conversion while preserving Codex-specific request-body differences:
  - `instructions` carries the system prompt instead of replaying it as an input message
  - `text.verbosity` defaults to `medium`
  - `tool_choice: "auto"` and `parallel_tool_calls: true`
  - `prompt_cache_key` follows `session_id`
  - Rust now also sends `prompt_cache_retention: "in-memory"` when `session_id` is present, following the current TS test expectation for the Codex path
  - Codex reasoning-effort clamping now matches the current TS source for the migrated model families (`gpt-5.2+ minimal -> low`, `gpt-5.1 xhigh -> high`, `gpt-5.1-codex-mini` remap)
- Codex SSE transport now reuses/adapts the migrated OpenAI Responses decoder/state machine rather than reimplementing a separate event model
- Codex terminal-stream compatibility now covers the event-name differences exercised by the TS tests:
  - `response.done` -> normalized completed response
  - `response.completed` -> completed response
  - `response.incomplete` -> completed response with `status: incomplete` so Rust emits `StopReason::Length`
- Codex auth/header behavior now covers the current migration target:
  - bearer token auth
  - JWT account-id extraction into `chatgpt-account-id`
  - `originator: pi`
  - Codex beta header (`OpenAI-Beta: responses=experimental`)
  - `session_id` plus `conversation_id` when a session id is provided
- registry/HTTP coverage now exists for the Codex endpoint path `/codex/responses`
- immediate terminal error behavior now exists for missing Codex credentials and invalid/non-decodable OAuth-style tokens

Compatibility note for this slice:
- current Rust Codex transport intentionally implements only the SSE runtime path; the TS WebSocket transport/fallback and retry loop remain deferred
- there is a source-vs-test ambiguity in current TypeScript around Codex session handling: the TS provider source currently shows only `session_id` + `prompt_cache_key`, while the TS test explicitly expects `conversation_id` and `prompt_cache_retention: "in-memory"`; Rust follows the explicit test-observed behavior for this migration slice

### Rust design summary

New Rust module added:
- `rust/crates/pi-ai/src/openai_codex_responses.rs`

Internal reuse/refactor in existing Rust code:
- `rust/crates/pi-ai/src/openai_responses.rs`
  - OpenAI Responses SSE decoder/state helpers are now crate-visible so the new Codex provider can reuse the existing transport/event-normalization machinery without introducing another parallel implementation

Provider-facing Rust surface added in the Codex module:
- `OpenAiCodexResponsesRequestOptions`
- `OpenAiCodexResponsesRequestParams`
- `OpenAiCodexResponsesTextConfig`
- `OpenAiCodexResponsesToolDefinition`
- `build_openai_codex_responses_request_params()`
- `parse_openai_codex_sse_text()`
- `stream_openai_codex_sse_text()`
- `stream_openai_codex_http()`
- `register_openai_codex_responses_provider()`

Integration update:
- `rust/crates/pi-ai/src/lib.rs`
  - built-in provider registration now includes `openai-codex-responses`

### Validation summary

New Rust coverage added for:
- Codex raw SSE stream normalization of `response.done`
- Codex raw SSE stream normalization of `response.incomplete` -> `StopReason::Length`
- registry dispatch through `stream_response()` for `openai-codex-responses`
- Codex bearer/account-id/originator/beta-header behavior on `/codex/responses`
- early completion after terminal SSE event even when the HTTP body remains open
- session header + prompt-cache field shaping for the Codex path
- reasoning-effort clamping for newer Codex models
- missing API-key terminal error behavior

Validation run results:
- `cd rust && cargo test -p pi-ai --test openai_codex_responses_stream --test openai_codex_responses_http -- --nocapture` passed
- `cd rust && cargo fmt --all && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- porting the narrowed AI regression set for `anthropic-messages`, `openai-responses`, `openai-completions`, and `openai-codex-responses`
- Unicode/surrogate sanitization parity for the OpenAI-backed Rust providers (current migrated OpenAI Rust code still carries placeholder sanitization)
- context-overflow detection helper parity with TS `isContextOverflow()`
- Codex retry/WebSocket transport parity beyond the current SSE slice

## Milestone 13 update: overflow helper + abort regression coverage slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/utils/overflow.ts`
- `packages/ai/test/overflow.test.ts`
- previously reviewed narrowed-regression references kept in scope for alignment:
  - `packages/ai/test/abort.test.ts`
  - `packages/ai/test/context-overflow.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_http.rs`
- `rust/crates/pi-ai/tests/openai_codex_responses_http.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- `pi-ai` now exposes a shared `is_context_overflow()` helper with TS-aligned regex detection for:
  - Anthropic-style prompt-too-long errors
  - request-too-large / byte-size overflow errors
  - OpenAI context-window errors
  - Google token-count overflow errors
  - xAI / Groq / OpenRouter / Copilot / llama.cpp / LM Studio / MiniMax / Kimi / Mistral patterns
  - Cerebras-style `400/413 (no body)` overflow signatures
  - silent-overflow detection via `usage.input + cache_read > context_window`
- Rust overflow detection also ports the TS non-overflow exclusions for rate-limit/throttling-style messages so `too many tokens` throttling is not misclassified as overflow
- abort regression coverage has been widened for the narrowed provider set:
  - `anthropic-messages` now has immediate-abort coverage before request send
  - `anthropic-messages` now has mid-stream abort coverage while waiting for the next HTTP body chunk
  - `openai-codex-responses` now has immediate-abort coverage before request send
  - `openai-codex-responses` now has mid-stream abort coverage while waiting for the next HTTP body chunk

Compatibility note for this slice:
- Unicode/surrogate sanitization parity is still intentionally deferred; Rust now has the overflow helper parity that TS uses in the `context-overflow` regression suite, but the OpenAI/Anthropic text sanitizers remain a separate remaining slice

### Rust design summary

New Rust module added:
- `rust/crates/pi-ai/src/overflow.rs`

Public surface added:
- `is_context_overflow()`
- `overflow_patterns()`

Integration update:
- `rust/crates/pi-ai/src/lib.rs`
  - now exports the overflow helper surface from the crate root

Test coverage updates:
- `rust/crates/pi-ai/tests/overflow.rs`
- expanded HTTP abort coverage in:
  - `rust/crates/pi-ai/tests/anthropic_messages_http.rs`
  - `rust/crates/pi-ai/tests/openai_codex_responses_http.rs`

### Validation summary

New Rust coverage added for:
- explicit overflow-pattern detection
- non-overflow exclusion handling
- silent overflow detection via usage > context window
- Anthropic abort-before-send and abort-mid-stream
- Codex abort-before-send and abort-mid-stream

Validation run results:
- `cd rust && cargo test -p pi-ai --test anthropic_messages_http --test openai_codex_responses_http --test overflow -- --nocapture` passed
- `cd rust && cargo fmt --all && cargo test -p pi-ai && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- empty-message / empty-assistant regression coverage for the narrowed provider set
- explicit narrowed regression files for response-id, image-tool-result, and tool-call-without-result across all four in-scope providers
- Unicode/surrogate sanitization parity for request shaping
- Codex retry/WebSocket transport parity beyond the current SSE slice

## Milestone 14 update: narrowed empty-message / empty-assistant regression slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/test/empty.test.ts`
- `packages/ai/src/providers/transform-messages.ts`
- `packages/ai/src/providers/anthropic.ts`
- `packages/ai/src/providers/openai-responses.ts`
- `packages/ai/src/providers/openai-responses-shared.ts`
- `packages/ai/src/providers/openai-completions.ts`
- `packages/ai/src/providers/openai-codex-responses.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/openai_completions.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_params.rs`
- `rust/crates/pi-ai/tests/openai_responses_payload.rs`
- `rust/crates/pi-ai/tests/openai_completions_messages.rs`
- `rust/crates/pi-ai/tests/openai_codex_responses_http.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- narrowed empty-content-array regressions now exist for all four in-scope provider paths:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- narrowed empty-assistant replay regressions now exist for the same four provider paths
- Rust now matches the current TypeScript Anthropic request-shaping behavior more closely for empty user messages:
  - empty user content arrays are dropped during message conversion
  - `build_anthropic_request_params()` no longer injects a synthetic empty user message when all user content collapses away
- OpenAI-backed Rust request conversion is now explicitly frozen for this slice so empty assistant turns are skipped during replay across:
  - OpenAI Responses
  - OpenAI Completions
  - OpenAI Codex Responses

Compatibility note for this slice:
- this milestone intentionally narrows the ported empty-message coverage to the Rust message-model cases that map directly to the TypeScript source/tests (`content: []` empty user messages and empty assistant turns). TypeScript’s separate string-content empty/whitespace cases remain a later compatibility question because Rust currently uses normalized `Vec<UserContent>` user messages.

### Rust design summary

New Rust regression file added:
- `rust/crates/pi-ai/tests/empty_messages.rs`

Implementation adjustment:
- `rust/crates/pi-ai/src/anthropic_messages.rs`
  - removed the synthetic empty-user fallback in `build_anthropic_request_params()` to preserve the current TypeScript request-shaping behavior for empty user-message arrays

### Validation summary

New Rust coverage added for:
- Anthropic empty user-message array conversion
- Anthropic empty assistant replay skipping
- OpenAI Responses empty user-message array conversion
- OpenAI Responses empty assistant replay skipping
- OpenAI Completions empty user-message array conversion
- OpenAI Completions empty assistant replay skipping
- OpenAI Codex empty user-message array conversion
- OpenAI Codex empty assistant replay skipping

Validation run results:
- `cd rust && cargo test -p pi-ai --test empty_messages` passed
- `cd rust && cargo fmt --all && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- explicit narrowed regression files for response-id, image-tool-result, and tool-call-without-result across all four in-scope providers
- Unicode/surrogate sanitization parity for request shaping
- Codex retry/WebSocket transport parity beyond the current SSE slice

## Milestone 15 update: narrowed image-tool-result regression slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/test/image-tool-result.test.ts`
- `packages/ai/test/openai-responses-tool-result-images.test.ts`
- `packages/ai/test/openai-completions-tool-result-images.test.ts`
- `packages/ai/src/providers/anthropic.ts`
- `packages/ai/src/providers/openai-responses-shared.ts`
- `packages/ai/src/providers/openai-completions.ts`
- `packages/ai/src/providers/openai-codex-responses.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/openai_completions.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
- `rust/crates/pi-ai/tests/openai_responses_payload.rs`
- `rust/crates/pi-ai/tests/openai_completions_messages.rs`
- `rust/crates/pi-ai/tests/openai_codex_responses_http.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- explicit narrowed `image-tool-result` regression coverage now exists for all four in-scope provider paths:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- Anthropic tool-result image shaping is now explicitly frozen in Rust for the narrowed migration scope:
  - text+image tool results stay grouped inside a single Anthropic `tool_result` content block
  - image-only tool results retain the Anthropic placeholder text fallback `"(see attached image)"` plus the image block
- OpenAI Responses and OpenAI Codex image-tool-result shaping is now explicitly frozen in Rust:
  - tool-result text+image stays nested inside `function_call_output.output`
  - no follow-up synthetic user message is emitted after that `function_call_output`
- OpenAI Completions image-tool-result shaping is now explicitly frozen in Rust:
  - textual tool output stays in the `tool` message
  - image content is emitted in a follow-up user multipart message with the image-attachment helper text

Compatibility note for this slice:
- the new regression file focuses on request-shaping parity rather than full live end-to-end vision execution. This matches the highest-value deterministic TS behavior for migration validation and avoids introducing provider-network nondeterminism into the Rust regression set.

### Rust design summary

New Rust regression file added:
- `rust/crates/pi-ai/tests/image_tool_result.rs`

No runtime implementation changes were required for this slice; the current Rust provider code already matched the narrowed TypeScript behavior once explicit tests were added.

### Validation summary

New Rust coverage added for:
- Anthropic text+image tool-result grouping
- Anthropic image-only tool-result placeholder behavior
- OpenAI Responses `function_call_output` image routing
- OpenAI Completions follow-up user-image routing after tool results
- OpenAI Codex `function_call_output` image routing

Validation run results:
- `cd rust && cargo test -p pi-ai --test image_tool_result` passed
- `cd rust && cargo fmt --all && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- explicit narrowed regression files for response-id and tool-call-without-result across all four in-scope providers
- Unicode/surrogate sanitization parity for request shaping
- Codex retry/WebSocket transport parity beyond the current SSE slice

## Milestone 16 update: narrowed tool-call-without-result regression slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/test/tool-call-without-result.test.ts`
- `packages/ai/src/providers/transform-messages.ts`
- `packages/ai/src/providers/anthropic.ts`
- `packages/ai/src/providers/openai-responses-shared.ts`
- `packages/ai/src/providers/openai-completions.ts`
- `packages/ai/src/providers/openai-codex-responses.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/tests/anthropic_messages_params.rs`
- `rust/crates/pi-ai/tests/openai_responses_payload.rs`
- `rust/crates/pi-ai/tests/openai_completions_messages.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- explicit narrowed `tool-call-without-result` regression coverage now exists for all four in-scope provider paths:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- Rust now explicitly freezes the synthetic orphaned-tool-call recovery path that the TypeScript `transformMessages()` logic relies on:
  - when a user follow-up interrupts a pending tool call with no tool result
  - a synthetic `No result provided` tool result is inserted before the follow-up user message
- provider-specific replay shaping is now explicitly covered for that synthetic insertion:
  - Anthropic emits a synthetic `tool_result` user message before the follow-up user turn
  - OpenAI Responses emits a synthetic `function_call_output`
  - OpenAI Completions emits a synthetic `tool` message
  - OpenAI Codex follows the OpenAI Responses-style synthetic `function_call_output` path

Compatibility note for this slice:
- this milestone ports the narrowed deterministic request-shaping behavior rather than the full live end-to-end completion flow from the TypeScript test. The important compatibility guarantee here is that orphaned tool calls do not poison replay and that the follow-up user turn remains reachable after provider-specific synthetic repair.

### Rust design summary

New Rust regression file added:
- `rust/crates/pi-ai/tests/tool_call_without_result.rs`

No runtime implementation changes were required for this slice; the current Rust replay/conversion code already matched the narrowed TypeScript behavior once explicit cross-provider regression tests were added.

### Validation summary

New Rust coverage added for:
- Anthropic synthetic orphaned-tool-result insertion
- OpenAI Responses synthetic orphaned-tool-result insertion
- OpenAI Completions synthetic orphaned-tool-result insertion
- OpenAI Codex synthetic orphaned-tool-result insertion

Validation run results:
- `cd rust && cargo test -p pi-ai --test tool_call_without_result` passed
- pending broader package/workspace validation after this slice

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- explicit narrowed regression file for response-id across the four in-scope providers
- Unicode/surrogate sanitization parity for request shaping
- Codex retry/WebSocket transport parity beyond the current SSE slice

## Milestone 17 update: narrowed response-id regression slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/test/responseid.test.ts`
- related previously migrated provider stream/request tests kept in scope for parity checks:
  - `packages/ai/src/providers/anthropic.ts`
  - `packages/ai/src/providers/openai-responses.ts`
  - `packages/ai/src/providers/openai-responses-shared.ts`
  - `packages/ai/src/providers/openai-completions.ts`
  - `packages/ai/src/providers/openai-codex-responses.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/tests/anthropic_messages_stream.rs`
- `rust/crates/pi-ai/tests/openai_responses_stream.rs`
- `rust/crates/pi-ai/tests/openai_completions_stream.rs`
- `rust/crates/pi-ai/tests/openai_codex_responses_stream.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- explicit narrowed `responseId` regression coverage now exists for all four in-scope provider paths:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- Rust now explicitly freezes the provider-specific response-id capture points used by the TypeScript E2E suite:
  - Anthropic captures `message.id` from `message_start`
  - OpenAI Responses captures `response.id` from `response.created` / terminal response objects
  - OpenAI Completions captures the streaming chunk `id`
  - OpenAI Codex captures `response.id` through the Codex-to-Responses SSE normalization path

Compatibility note for this slice:
- this milestone narrows the TS `responseid.test.ts` E2E expectation into deterministic stream-level regression tests. The compatibility guarantee is that each migrated provider path carries a stable non-empty response id into the terminal assistant message.

### Rust design summary

New Rust regression file added:
- `rust/crates/pi-ai/tests/response_id.rs`

No runtime implementation changes were required for this slice; the current Rust stream state already preserved response ids correctly for the narrowed provider scope.

### Validation summary

New Rust coverage added for:
- Anthropic terminal response-id propagation
- OpenAI Responses terminal response-id propagation
- OpenAI Completions terminal response-id propagation
- OpenAI Codex terminal response-id propagation

Validation run results:
- `cd rust && cargo test -p pi-ai --test response_id` passed
- `cd rust && cargo fmt --all && cargo test -p pi-ai && cargo test -q --workspace` passed
- pending `npm run check` after this slice

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- Unicode/surrogate sanitization parity for request shaping
- Codex retry/WebSocket transport parity beyond the current SSE slice

## Milestone 18 update: Unicode request-text freeze slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/utils/sanitize-unicode.ts`
- `packages/ai/test/unicode-surrogate.test.ts`
- targeted provider request-shaping references:
  - `packages/ai/src/providers/anthropic.ts`
  - `packages/ai/src/providers/openai-responses-shared.ts`
  - `packages/ai/src/providers/openai-completions.ts`
  - `packages/ai/src/providers/openai-codex-responses.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/openai_completions.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
- `migration/packages/ai.md`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- request-shaping regression coverage now explicitly freezes Unicode-bearing text across all four in-scope provider paths:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- the new Rust regression slice freezes the practical behavior exercised by `packages/ai/test/unicode-surrogate.test.ts` at the Rust string boundary:
  - valid emoji and multilingual text survive request shaping unchanged
  - text that has already crossed into Rust via lossy UTF-16 decoding (`String::from_utf16_lossy`) remains JSON-serializable and stable through provider request builders
- OpenAI Completions now routes request text through the same explicit sanitization hook used elsewhere in the migrated provider surface for:
  - system prompts
  - user text content
  - assistant text/thinking replay
  - tool-result text replay
- OpenAI Codex Responses now routes `instructions` through the same explicit sanitization hook
- Anthropic Messages and OpenAI Responses now keep their existing request-shaping call sites but delegate to the shared helper instead of provider-local placeholder no-ops

Compatibility note for this slice:
- TypeScript removes unpaired UTF-16 surrogate code units before JSON serialization. Rust `String` values cannot represent standalone UTF-16 surrogates at all, so exact surrogate-removal behavior is structurally unrepresentable on the Rust side.
- This milestone therefore freezes the observable Rust-side compatibility boundary honestly: once text has crossed into a Rust `String`, provider request shaping preserves valid Unicode unchanged and remains safe for JSON serialization.

### Rust design summary

New Rust module added:
- `rust/crates/pi-ai/src/unicode.rs`
  - `sanitize_provider_text(...)`

Integration updates:
- `rust/crates/pi-ai/src/lib.rs`
  - now declares the shared internal Unicode helper module
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/src/openai_completions.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
  - request-shaping text paths now go through the shared helper, keeping the Rust/TypeScript compatibility boundary explicit in one place

Behavior-freeze artifact added:
- `rust/crates/pi-ai/tests/unicode_request_text.rs`

### Validation summary

New Rust coverage added for:
- Anthropic request shaping preserving emoji/multilingual text and lossy UTF-16 fallback text
- OpenAI Responses request shaping preserving emoji/multilingual text and lossy UTF-16 fallback text
- OpenAI Completions request shaping preserving emoji/multilingual text and lossy UTF-16 fallback text across system/user/assistant/tool-result paths
- OpenAI Codex request shaping preserving emoji/multilingual text and lossy UTF-16 fallback text across `instructions` and replayed input items
- internal helper tests documenting the Rust string-boundary behavior directly

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-ai --test unicode_request_text` passed
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- Codex retry/WebSocket transport parity beyond the current SSE slice

## Milestone 19 update: Codex WebSocket + HTTP retry transport slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/providers/openai-codex-responses.ts`
- `packages/ai/test/openai-codex-stream.test.ts`
- targeted transport grounding from the broader suite:
  - `packages/ai/test/stream.test.ts` (Codex WebSocket path)
  - `packages/ai/src/types.ts` (`transport` option semantics)

Additional Rust files read for this slice:
- `rust/Cargo.toml`
- `rust/crates/pi-ai/Cargo.toml`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/tests/openai_codex_responses_http.rs`
- `rust/crates/pi-ai/tests/openai_codex_responses_stream.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- `pi-ai` now exposes a transport selector on `StreamOptions` for providers that support multiple transports
- `openai-codex-responses` now supports all three TS transport modes for the migrated slice:
  - `sse`
  - `websocket`
  - `auto`
- explicit Codex WebSocket transport now covers the current high-value TS runtime behavior:
  - WebSocket URL derivation from the Codex base URL (`http -> ws`, `https -> wss`)
  - WebSocket upgrade request headers including:
    - bearer auth
    - `chatgpt-account-id`
    - `originator: pi`
    - `OpenAI-Beta: responses_websockets=2026-02-06`
    - `x-client-request-id`
    - `session_id`
  - `response.create` WebSocket request payload shaping
  - normalized event streaming through the existing OpenAI Responses stream-state machinery
- Codex `auto` transport now follows the current TS fallback rule for the migrated slice:
  - try WebSocket first
  - if the WebSocket connect/send path fails before streaming starts, fall back to SSE
  - explicit `websocket` transport does not fall back to SSE on the same failure path
- Codex SSE HTTP transport now has retry/backoff behavior for the current TS-compatible slice:
  - retries retryable HTTP statuses (`429`, `500`, `502`, `503`, `504`)
  - retries retryable transient-text responses like rate limits / overloaded / service unavailable
  - retries network send failures
  - exponential backoff starting at `1000ms`
  - abort-aware retry sleep and request dispatch

Compatibility note for this slice:
- Rust now ports the observable Codex multi-transport behavior and retry loop, but it still does not implement the TS session-scoped WebSocket cache / idle TTL reuse path
- the Rust transport option is currently used only by the Codex provider slice; other migrated providers still stay on their existing single-transport paths

### Rust design summary

Workspace/dependency changes:
- `rust/Cargo.toml`
  - added `tokio-tungstenite` as a workspace dependency
- `rust/crates/pi-ai/Cargo.toml`
  - `pi-ai` now depends on `tokio-tungstenite`

Core surface change:
- `rust/crates/pi-ai/src/lib.rs`
  - new `Transport` enum
  - `StreamOptions.transport: Option<Transport>`

Codex provider changes in `rust/crates/pi-ai/src/openai_codex_responses.rs`:
- added transport-aware provider dispatch:
  - `stream_openai_codex_http(...)`
  - `stream_openai_codex_websocket(...)`
  - `stream_openai_codex_auto(...)`
- added WebSocket helpers for:
  - request-header shaping
  - request-id generation
  - WebSocket request construction
  - `response.create` payload serialization
  - raw WebSocket JSON-event parsing into the normalized OpenAI Responses envelope shape
- added HTTP retry helpers for:
  - retry classification
  - abort-aware sleep
  - retrying POST dispatch with final error materialization
- reused the existing OpenAI Responses SSE/event normalization machinery so the WebSocket path does not introduce a second assistant-event protocol

Behavior-freeze artifact added:
- `rust/crates/pi-ai/tests/openai_codex_responses_transport.rs`

### Validation summary

New Rust coverage added for:
- explicit Codex WebSocket transport with handshake-header assertions and `response.create` payload assertions
- `auto` transport fallback from failed WebSocket connect/handshake into SSE
- explicit `websocket` transport not falling back to SSE on the same failed-handshake path
- retrying retryable HTTP failures before succeeding on a later SSE attempt

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-ai --test openai_codex_responses_transport` passed
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- Codex session-scoped WebSocket cache / idle-time reuse parity from the TypeScript provider
- broader `streamSimple()` / `completeSimple()` API parity remains deferred across the crate

## Milestone 20 update: Codex session-scoped WebSocket cache / idle-time reuse slice

### Files analyzed

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
- `rust/crates/pi-ai/tests/openai_codex_responses_transport.rs`
- `migration/packages/ai.md`

Relevant TypeScript grounding already in scope for this slice:
- `packages/ai/src/providers/openai-codex-responses.ts`
- current TS cache/reuse helpers around:
  - `CachedWebSocketConnection`
  - `acquireWebSocket(...)`
  - `scheduleSessionWebSocketExpiry(...)`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- Codex WebSocket transport now has session-scoped connection reuse when `session_id` is provided
- sequential Codex requests with the same session id can now reuse the same open WebSocket connection instead of reconnecting every turn
- Rust now preserves the current TS busy-path behavior for the migrated slice:
  - if a cached session connection is already checked out/busy
  - a concurrent/acquired-again request gets a temporary uncached connection instead of blocking on the cached one
- cached session WebSockets now expire after an idle TTL and are closed/removed from the cache automatically
- if a reused cached socket has gone stale and the first send fails before streaming starts, Rust now drops it and reconnects once for the same session

Compatibility note for this slice:
- Rust now matches the high-value TS reuse/idle semantics, but the internal implementation differs:
  - TS uses browser/Node-style WebSocket objects with ready-state checks and JS timers
  - Rust uses a global cache plus owned `tokio-tungstenite` streams returned to the cache after a successful turn
- the idle-time unit test uses the Rust test build’s shortened TTL constant so the behavior can be validated without a multi-minute wait; non-test runtime behavior still keeps the TS five-minute TTL target

### Rust design summary

Expanded `rust/crates/pi-ai/src/openai_codex_responses.rs` with:
- cached-session connection state:
  - `CachedCodexWebSocketEntry`
  - `AcquiredCodexWebSocket`
  - global `codex_websocket_cache()`
- cache lifecycle helpers:
  - `abort_idle_task(...)`
  - `remove_cached_websocket_entry_if_same(...)`
  - `schedule_session_websocket_expiry(...)`
  - `acquire_codex_websocket(...)`
- WebSocket startup now returns an acquired connection wrapper instead of a bare socket, so the stream path can:
  - return successful sockets to the cache on terminal `done`
  - drop cached sockets on abort/error/close-before-complete paths
- cached reused sockets now reconnect once if the initial `response.create` send fails before the stream starts

Validation additions:
- integration transport coverage keeps validating explicit WebSocket behavior and same-session reuse
- new unit coverage inside `openai_codex_responses.rs` validates idle-expiry reconnection with the test TTL

### Validation summary

New Rust coverage added for:
- same-session WebSocket reuse across sequential Codex turns
- idle-expiry reconnection after the cached session socket ages out
- existing transport coverage for explicit WebSocket, `auto` fallback, and retryable HTTP behavior still passes on top of the cache layer

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-ai --test openai_codex_responses_transport` passed
- `cd rust && cargo test -p pi-ai openai_codex_responses::tests::reconnects_after_cached_websocket_idle_expiry -- --nocapture` passed
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- broader `streamSimple()` / `completeSimple()` API parity remains deferred across the crate

## Milestone 21 update: narrowed `stream_simple()` / `complete_simple()` parity slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/types.ts`
- `packages/ai/src/stream.ts`
- `packages/ai/src/providers/simple-options.ts`
- `packages/ai/src/providers/anthropic.ts` (`streamSimpleAnthropic`)
- `packages/ai/src/providers/openai-responses.ts` (`streamSimpleOpenAIResponses`)
- `packages/ai/src/providers/openai-completions.ts` (`streamSimpleOpenAICompletions`)
- `packages/ai/src/providers/openai-codex-responses.ts` (`streamSimpleOpenAICodexResponses`)
- focused TS test grounding for the narrowed API slice:
  - `packages/ai/test/anthropic-thinking-disable.test.ts`
  - `packages/ai/test/openai-completions-tool-choice.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/openai_completions.rs`
- `rust/crates/pi-ai/tests/openai_responses_http.rs`
- `rust/crates/pi-ai/tests/openai_completions_http.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_http.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- `pi-ai` now exposes top-level Rust `stream_simple()` and `complete_simple()` entry points alongside the existing `stream_response()` / `complete()` API
- Rust now has explicit `SimpleStreamOptions`, `ThinkingLevel`, and `ThinkingBudgets` types for the narrowed migration scope
- `stream_simple()` preserves registry dispatch rather than bypassing registered providers, so faux/custom providers still work through the simple API path
- TS `buildBaseOptions()` parity now exists for the narrowed Rust simple API surface:
  - default `max_tokens` / `max_output_tokens` now clamp to `min(model.max_tokens, 32000)` when the caller does not provide one
  - existing shared options (`signal`, `api_key`, `transport`, `cache_retention`, `session_id`, `headers`, `temperature`) now flow through the simple path
- TS reasoning mapping now exists for the narrowed in-scope providers:
  - OpenAI Responses / OpenAI Completions / OpenAI Codex clamp `xhigh` to `high` when the target model does not `supportsXhigh()`
  - Anthropic preserves raw reasoning levels so adaptive models can still distinguish `xhigh`
  - non-adaptive Anthropic simple requests now apply the TS `adjustMaxTokensForThinking()` behavior before dispatching to the existing provider runtime path
- OpenAI Completions now accepts the remaining high-value TS `streamSimple()` passthrough behavior needed by current tests:
  - `tool_choice` survives the Rust simple path and reaches the request body

Compatibility note for this slice:
- this milestone intentionally narrows `SimpleStreamOptions` parity to the currently in-scope providers and the highest-value fields observed in the TypeScript implementation/tests
- broader provider-specific `streamSimple()` extras outside the narrowed provider scope remain deferred

### Rust design summary

Core API changes in `rust/crates/pi-ai/src/lib.rs`:
- added `ThinkingLevel`
- added `ThinkingBudgets`
- added `SimpleStreamOptions`
- added `stream_simple()`
- added `complete_simple()`
- added shared simple-option mapping helpers for:
  - TS-style default max-token handling
  - xhigh clamping
  - Anthropic non-adaptive thinking-budget max-token adjustment

Runtime option integration:
- `StreamOptions` now carries optional `tool_choice` so the existing registry/provider path can preserve OpenAI Completions simple-tool-choice behavior without bypassing provider dispatch

Provider update:
- `rust/crates/pi-ai/src/openai_completions.rs`
  - provider runtime now forwards `StreamOptions.tool_choice` into `OpenAiCompletionsRequestOptions`

Behavior-freeze artifact added:
- `rust/crates/pi-ai/tests/simple_stream.rs`

### Validation summary

New Rust coverage added for:
- OpenAI Responses simple-path `xhigh` clamping plus default `max_output_tokens`
- OpenAI Completions simple-path `tool_choice` passthrough plus default `max_completion_tokens`
- Anthropic simple-path non-adaptive thinking max-token adjustment
- faux-provider dispatch through `complete_simple()` proving registry/custom-provider compatibility

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-ai --test simple_stream` passed
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed in the current environment; the earlier migration-note `biome` blocker no longer reproduced in this session

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- broader `streamSimple()` / `completeSimple()` API parity beyond the narrowed in-scope providers and current high-value passthrough fields
- provider-specific simple-option parity that is outside the current migration scope

## Milestone 22 update: `metadata` + async `on_payload` simple-option parity slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/ai/src/types.ts`
- `packages/ai/src/providers/simple-options.ts`
- `packages/ai/src/providers/anthropic.ts`
- `packages/ai/src/providers/openai-responses.ts`
- `packages/ai/src/providers/openai-completions.ts`
- `packages/ai/src/providers/openai-codex-responses.ts`
- focused TS usage/tests grounding:
  - `packages/ai/test/anthropic-thinking-disable.test.ts`
  - `packages/ai/test/openai-completions-tool-choice.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/anthropic_messages.rs`
- `rust/crates/pi-ai/src/openai_responses.rs`
- `rust/crates/pi-ai/src/openai_completions.rs`
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
- `rust/crates/pi-ai/tests/simple_stream.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- `StreamOptions` and `SimpleStreamOptions` now carry the remaining high-value shared request fields from the narrowed TS `buildBaseOptions()` slice:
  - `metadata`
  - async `on_payload`
- `stream_simple()` / `complete_simple()` now preserve those fields when mapping into the existing runtime provider path instead of dropping them
- the four in-scope real-provider Rust paths now apply `on_payload` before request dispatch:
  - `anthropic-messages`
  - `openai-responses`
  - `openai-completions`
  - `openai-codex-responses`
- post-hook payloads now flow to transport as raw JSON values, so arbitrary added or replaced JSON fields are preserved instead of being collapsed back into the typed Rust request structs before dispatch
- Anthropic request shaping now honors the narrowed TS metadata behavior already present in the provider source:
  - `metadata.user_id` is extracted from the shared metadata map and serialized into the Anthropic request body
- payload replacement errors are now surfaced as normal terminal assistant error events instead of panicking or silently ignoring invalid replacements

Compatibility note for this slice:
- this milestone intentionally narrows metadata parity to the currently exercised Anthropic `user_id` extraction path from the TypeScript provider code
- the new Rust `PayloadHook` surface is an explicit strongly typed Rust wrapper around the TS `onPayload` concept; the observable behavior preserved here is request inspection/replacement before dispatch, not the TypeScript function signature verbatim
- Rust now matches the important TypeScript dynamic-object behavior more closely on this path: provider payload hooks can add fields that are not part of the typed request structs and those fields still reach the final HTTP/WebSocket request body

### Rust design summary

Core API changes in `rust/crates/pi-ai/src/lib.rs`:
- added `PayloadHookResult`
- added `PayloadHookFuture`
- added `PayloadHook`
- added internal `apply_payload_hook()` helper returning raw JSON after optional replacement
- expanded `StreamOptions` with:
  - `metadata`
  - `on_payload`
- expanded `SimpleStreamOptions` with:
  - `metadata`
  - `on_payload`
- `map_simple_stream_options()` now forwards those fields into the runtime provider path

Provider runtime changes:
- `rust/crates/pi-ai/src/anthropic_messages.rs`
  - extracts `metadata.user_id` into Anthropic request metadata
  - applies typed payload replacement before HTTP dispatch
- `rust/crates/pi-ai/src/openai_responses.rs`
  - applies typed payload replacement before HTTP dispatch
- `rust/crates/pi-ai/src/openai_completions.rs`
  - applies typed payload replacement before HTTP dispatch
- `rust/crates/pi-ai/src/openai_codex_responses.rs`
  - applies typed payload replacement before SSE/WebSocket dispatch

Behavior-freeze coverage extended in:
- `rust/crates/pi-ai/tests/simple_stream.rs`

### Validation summary

New Rust coverage added for:
- OpenAI Responses simple-path payload replacement before request send, including preservation of added unknown JSON fields
- OpenAI Completions simple-path payload replacement before request send, including preservation of added unknown JSON fields
- OpenAI Codex simple-path payload replacement before request send, including preservation of added unknown JSON fields
- Anthropic simple-path `metadata.user_id` request serialization plus preservation of added unknown JSON fields

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-ai --test simple_stream` passed
- `cd rust && cargo test -p pi-ai` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-ai`:
- broader `streamSimple()` / `completeSimple()` API parity beyond the narrowed in-scope providers and current shared-option slice
- provider-specific simple-option parity outside the current migration scope
- downstream `pi-agent` / `pi-coding-agent` exposure of the new Rust `on_payload` hook remains a later crate-integration step, not part of this `pi-ai` milestone
