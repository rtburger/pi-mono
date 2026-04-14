# packages/ai Rust target design

Date: 2026-04-14
Status: Step 2 complete

## Crate name

- Rust crate: `pi-ai`

## Scope for this migration phase

In scope for Rust parity now:
- Anthropic Messages
- OpenAI Responses
- OpenAI Chat Completions
- OpenAI Codex Responses
- faux provider for tests
- shared model lookup, env API key lookup, normalized assistant event streaming

Out of scope for this phase:
- Azure
- Google / Gemini / Vertex / Gemini CLI
- Bedrock
- Mistral
- OpenRouter / Groq / xAI / z.ai / other OpenAI-compatible providers beyond what current OpenAI paths require

## Module layout

### Public modules
- `anthropic_messages`
- `models`
- `openai_codex_responses`
- `openai_completions`
- `openai_responses`
- `overflow`

### Private/shared modules
- `unicode`
- provider registration and faux-provider support in `src/lib.rs`

## Public API target

Keep the crate surface centered on normalized streaming and completion helpers:
- model helpers
  - `built_in_models()`
  - `get_model()`
  - `get_models()`
  - `get_providers()`
  - `models_are_equal()`
  - `supports_xhigh()`
- request dispatch
  - `stream_response()`
  - `complete()`
  - `stream_simple()`
  - `complete_simple()`
- provider registration
  - `register_provider()`
  - `unregister_provider()`
  - `register_builtin_providers()`
- env/auth helpers
  - `get_env_api_key()`
- focused provider-specific helpers where tests need them
  - e.g. Codex/OpenAI request param builders and SSE parsers

## Key types / enums / traits

- `AiProvider`
  - trait boundary for provider implementations
- `AssistantEventStream`
  - `Stream<Item = Result<AssistantEvent, AiError>>`
- `AiError`
  - typed crate error enum
- `StreamOptions`
  - low-level provider options
- `SimpleStreamOptions`
  - simplified reasoning-focused options mapped to provider-native requests
- `Transport`
  - `Sse | WebSocket | Auto`
- `CacheRetention`
  - `None | Short | Long`
- provider request/response structs per API module
  - explicit serde-backed request payloads
  - explicit SSE/WebSocket event envelopes where practical

Shared event/message model stays in `pi-events`, not `pi-ai`.

## Dependencies

Runtime:
- `tokio`
- `reqwest`
- `serde`
- `serde_json`
- `futures`
- `tokio-tungstenite`
- `async-stream`
- `regex`
- `thiserror`

Dev/test:
- `httpmock`
- fixture files under `rust/crates/pi-ai/tests/fixtures/`

## Compatibility goals

1. Preserve the TypeScript observable contract, not its implementation structure.
2. Keep one normalized assistant event stream across all in-scope providers.
3. Match TypeScript request semantics for:
   - reasoning effort mapping
   - session/cache headers and payload fields
   - tool call normalization
   - image tool-result routing
   - Unicode sanitization
4. Match TypeScript terminal-stream behavior:
   - terminate on provider completion events
   - propagate usage and stop reasons
   - preserve partial content on abort/error where TS does
5. Keep model data sourced from the TypeScript-generated catalog, filtered to in-scope providers.

## Known risks

1. OAuth placement is still unresolved.
   - TypeScript exposes runtime OAuth helpers from `packages/ai/oauth`.
   - Rust currently lacks full equivalent surface.
2. Cross-provider replay behavior is broad in TypeScript.
   - Rust must validate only the in-scope Anthropic/OpenAI/Codex cases first.
3. OpenAI-compatible edge cases can leak into nominal OpenAI behavior.
   - especially tool-call IDs, reasoning replay, and tool-result image routing.
4. Codex transport has two code paths.
   - SSE and WebSocket parity must be kept aligned.
5. `pi-test-harness` is still underbuilt.
   - short term tests stay local to `pi-ai` until a shared harness exists.

## Validation plan

Near-term validation order:
1. Freeze TypeScript-derived fixtures for Codex SSE terminal behavior. Done.
2. Validate Codex SSE parser and HTTP transport against those fixtures. Done.
3. Freeze exact TypeScript `shortHash()` parity for foreign OpenAI Responses tool-call item IDs. Done.
4. Keep existing Rust provider tests green for:
   - OpenAI Responses
   - OpenAI Completions
   - OpenAI Codex Responses
   - Anthropic Messages
5. Expand fixture-driven tests next for:
   - Anthropic tool-name normalization / OAuth path behavior
   - orphaned tool-call synthetic tool results
   - tool-result image routing
   - Unicode surrogate sanitization

## Current milestone decision

The Anthropic Claude Code OAuth tool-name slice is now validated in Rust.
Coverage now exists for:
- outbound Claude Code canonical casing on matching tools (`todowrite` -> `TodoWrite`, `read` -> `Read`)
- passthrough of non-Claude tools (`find`, `my_custom_tool`)
- inbound streamed `tool_use` events round-tripping back to the original tool names on the OAuth path

Validation landed in:
- `rust/crates/pi-ai/tests/anthropic_messages_params.rs`
- `rust/crates/pi-ai/tests/anthropic_messages_stream.rs`
- `rust/crates/pi-ai/tests/cross_provider_handoff.rs`

This keeps the Anthropic-to-OpenAI Codex replay path frozen at the request-shape level: Anthropic thinking is downgraded to plain text, tool results remain paired, and no OpenAI reasoning replay items are emitted across the provider boundary. The same regression file also covers the reverse OpenAI Responses-to-Anthropic request shape.

Partial JSON tool-call streaming is now also validated in Rust.
Coverage now exists for:
- partial string fragments inside tool call arguments
- missing-value tails at the end of nested objects and arrays
- partial literals (`true`, `false`, `null`)
- numeric fragments, including exponent salvage (`1e`) and incomplete decimals (`1.`)
- a provider-level OpenAI Responses stream regression that exercises the shared partial JSON parser

Validation landed in:
- `rust/crates/pi-ai/src/partial_json.rs`
- `rust/crates/pi-ai/tests/openai_responses_stream.rs`

OpenAI Responses request replay is now pinned at the HTTP boundary as well.
Coverage now exists for:
- skipping aborted reasoning-only assistant turns from the outgoing request body
- same-provider different-model handoff with tool calls, including dropping `fc_` item IDs on replayed function calls
- successful mocked round-trips while those request-shape guarantees hold

Validation landed in:
- `rust/crates/pi-ai/tests/openai_responses_reasoning_replay.rs`

## Suggested fixture candidates for the next slice
- Begin `packages/agent` inventory and behavior validation against the now-pinned `pi-ai` request shapes.
