# packages/ai migration inventory

Status: milestone 5 scaffold + faux provider slice + OpenAI Responses streaming/replay + Copilot header slice
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
- no real provider normalization yet
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
- passthrough of `max_tokens`, `temperature`, `reasoning_effort`, `reasoning_summary`, `session_id`, and `cache_retention`
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
- whether Rust-side model metadata should be generated from TS `models.generated.ts` or from shared external source during migration
- how much of TS `SimpleStreamOptions` reasoning normalization should live in `pi-ai` vs provider-specific modules
- whether faux provider should remain in `pi-ai` or move to `pi-test-harness` after the first provider lands
- whether to continue OpenAI Responses next with redacted reasoning + abort-usage parity or switch to Anthropic for a second end-to-end provider
