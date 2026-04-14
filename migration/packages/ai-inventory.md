# packages/ai inventory

Date: 2026-04-14
Status: Step 1 complete

## Files analyzed

### Package metadata and docs
- `packages/ai/README.md`
- `packages/ai/package.json`
- `packages/ai/vitest.config.ts`
- `packages/ai/CHANGELOG.md`
- `packages/ai/scripts/generate-models.ts`

### Source files
- `packages/ai/src/api-registry.ts`
- `packages/ai/src/bedrock-provider.ts`
- `packages/ai/src/cli.ts`
- `packages/ai/src/env-api-keys.ts`
- `packages/ai/src/index.ts`
- `packages/ai/src/models.generated.ts`
- `packages/ai/src/models.ts`
- `packages/ai/src/oauth.ts`
- `packages/ai/src/providers/amazon-bedrock.ts`
- `packages/ai/src/providers/anthropic.ts`
- `packages/ai/src/providers/azure-openai-responses.ts`
- `packages/ai/src/providers/faux.ts`
- `packages/ai/src/providers/github-copilot-headers.ts`
- `packages/ai/src/providers/google-gemini-cli.ts`
- `packages/ai/src/providers/google-shared.ts`
- `packages/ai/src/providers/google.ts`
- `packages/ai/src/providers/google-vertex.ts`
- `packages/ai/src/providers/mistral.ts`
- `packages/ai/src/providers/openai-codex-responses.ts`
- `packages/ai/src/providers/openai-completions.ts`
- `packages/ai/src/providers/openai-responses-shared.ts`
- `packages/ai/src/providers/openai-responses.ts`
- `packages/ai/src/providers/register-builtins.ts`
- `packages/ai/src/providers/simple-options.ts`
- `packages/ai/src/providers/transform-messages.ts`
- `packages/ai/src/stream.ts`
- `packages/ai/src/types.ts`
- `packages/ai/src/utils/event-stream.ts`
- `packages/ai/src/utils/hash.ts`
- `packages/ai/src/utils/json-parse.ts`
- `packages/ai/src/utils/oauth/anthropic.ts`
- `packages/ai/src/utils/oauth/github-copilot.ts`
- `packages/ai/src/utils/oauth/google-antigravity.ts`
- `packages/ai/src/utils/oauth/google-gemini-cli.ts`
- `packages/ai/src/utils/oauth/index.ts`
- `packages/ai/src/utils/oauth/oauth-page.ts`
- `packages/ai/src/utils/oauth/openai-codex.ts`
- `packages/ai/src/utils/oauth/pkce.ts`
- `packages/ai/src/utils/oauth/types.ts`
- `packages/ai/src/utils/overflow.ts`
- `packages/ai/src/utils/sanitize-unicode.ts`
- `packages/ai/src/utils/typebox-helpers.ts`
- `packages/ai/src/utils/validation.ts`

### Tests and fixtures
- `packages/ai/test/abort.test.ts`
- `packages/ai/test/anthropic-oauth.test.ts`
- `packages/ai/test/anthropic-thinking-disable.test.ts`
- `packages/ai/test/anthropic-tool-name-normalization.test.ts`
- `packages/ai/test/azure-utils.ts`
- `packages/ai/test/bedrock-models.test.ts`
- `packages/ai/test/bedrock-utils.ts`
- `packages/ai/test/cache-retention.test.ts`
- `packages/ai/test/context-overflow.test.ts`
- `packages/ai/test/cross-provider-handoff.test.ts`
- `packages/ai/test/data/red-circle.png`
- `packages/ai/test/empty.test.ts`
- `packages/ai/test/faux-provider.test.ts`
- `packages/ai/test/github-copilot-anthropic.test.ts`
- `packages/ai/test/github-copilot-oauth.test.ts`
- `packages/ai/test/google-gemini-cli-claude-thinking-header.test.ts`
- `packages/ai/test/google-gemini-cli-empty-stream.test.ts`
- `packages/ai/test/google-gemini-cli-retry-delay.test.ts`
- `packages/ai/test/google-shared-gemini3-unsigned-tool-call.test.ts`
- `packages/ai/test/google-shared-image-tool-result-routing.test.ts`
- `packages/ai/test/google-thinking-disable.test.ts`
- `packages/ai/test/google-thinking-signature.test.ts`
- `packages/ai/test/google-tool-call-missing-args.test.ts`
- `packages/ai/test/google-vertex-api-key-resolution.test.ts`
- `packages/ai/test/image-tool-result.test.ts`
- `packages/ai/test/interleaved-thinking.test.ts`
- `packages/ai/test/lazy-module-load.test.ts`
- `packages/ai/test/oauth.ts`
- `packages/ai/test/openai-codex-stream.test.ts`
- `packages/ai/test/openai-completions-tool-choice.test.ts`
- `packages/ai/test/openai-completions-tool-result-images.test.ts`
- `packages/ai/test/openai-responses-copilot-provider.test.ts`
- `packages/ai/test/openai-responses-foreign-toolcall-id.test.ts`
- `packages/ai/test/openai-responses-reasoning-replay-e2e.test.ts`
- `packages/ai/test/openai-responses-tool-result-images.test.ts`
- `packages/ai/test/openrouter-cache-write-repro.test.ts`
- `packages/ai/test/overflow.test.ts`
- `packages/ai/test/responseid.test.ts`
- `packages/ai/test/stream.test.ts`
- `packages/ai/test/supports-xhigh.test.ts`
- `packages/ai/test/tokens.test.ts`
- `packages/ai/test/tool-call-id-normalization.test.ts`
- `packages/ai/test/tool-call-without-result.test.ts`
- `packages/ai/test/total-tokens.test.ts`
- `packages/ai/test/transform-messages-copilot-openai-to-anthropic.test.ts`
- `packages/ai/test/unicode-surrogate.test.ts`
- `packages/ai/test/validation.test.ts`
- `packages/ai/test/xhigh.test.ts`
- `packages/ai/test/zen.test.ts`

### Rust grounding used for comparison
- `rust/Cargo.toml`
- `rust/crates/pi-ai/Cargo.toml`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-ai/src/models.rs`
- `rust/crates/pi-ai/tests/*` file inventory from workspace grounding

## Scope note

The TypeScript package supports far more providers than the requested Rust migration scope.

User-requested Rust provider scope for `pi-ai`:
- in scope
  - Anthropic Messages
  - OpenAI Responses
  - OpenAI Chat Completions
  - OpenAI Codex Responses
  - Anthropic Claude Code OAuth/CLI behavior on the Anthropic path
- out of scope for the Rust rewrite now
  - Azure
  - Google / Gemini / Vertex / Gemini CLI / Antigravity
  - Bedrock
  - Mistral
  - Copilot as a distinct provider path
  - OpenRouter / Groq / xAI / z.ai / MiniMax / Hugging Face / OpenCode

The full TypeScript package was still read because it defines package-wide behavior, model metadata conventions, handoff semantics, and shared utilities.

## Exported API inventory

### Root entry exports (`src/index.ts`)
- TypeBox re-exports
  - `Type`
  - type-only `Static`, `TSchema`
- registry/env/model helpers
  - everything from `api-registry.ts`
  - everything from `env-api-keys.ts`
  - everything from `models.ts`
- provider option type exports
  - `BedrockOptions`
  - `AnthropicOptions`
  - `AzureOpenAIResponsesOptions`
  - `GoogleOptions`
  - `GoogleGeminiCliOptions`, `GoogleThinkingLevel`
  - `GoogleVertexOptions`
  - `MistralOptions`
  - `OpenAICodexResponsesOptions`
  - `OpenAICompletionsOptions`
  - `OpenAIResponsesOptions`
- faux provider exports
  - everything from `providers/faux.ts`
- built-in provider lazy wrappers
  - everything from `providers/register-builtins.ts`
- high-level generation API
  - everything from `stream.ts`
- core types
  - everything from `types.ts`
- utilities
  - event stream helpers
  - `parseStreamingJson`
  - overflow helpers
  - `StringEnum`
  - validation helpers
- OAuth subtypes re-exported from root
  - `OAuthAuthInfo`
  - `OAuthCredentials`
  - `OAuthLoginCallbacks`
  - `OAuthPrompt`
  - `OAuthProvider`
  - `OAuthProviderId`
  - `OAuthProviderInfo`
  - `OAuthProviderInterface`

### OAuth subpath export (`src/oauth.ts`)
- re-exports everything from `src/utils/oauth/index.ts`
- intended as the Node/CLI-facing OAuth entry after the package root stopped exporting runtime OAuth functions

### Main public runtime API
- model discovery
  - `getModel(provider, id)`
  - `getModels(provider)`
  - `getProviders()`
  - `modelsAreEqual()`
  - `supportsXhigh()`
  - `calculateCost()`
- request dispatch
  - `stream()`
  - `complete()`
  - `streamSimple()`
  - `completeSimple()`
- provider registration
  - `registerApiProvider()`
  - `getApiProvider()`
  - `getApiProviders()`
  - `unregisterApiProviders()`
  - `clearApiProviders()`
  - `resetApiProviders()`
- built-in lazy wrapper exports
  - `streamAnthropic`, `streamOpenAIResponses`, `streamOpenAICompletions`, `streamOpenAICodexResponses`, etc.
- test/demo provider
  - `registerFauxProvider()`
  - `fauxAssistantMessage()`, `fauxText()`, `fauxThinking()`, `fauxToolCall()`
- OAuth registry and helpers
  - `getOAuthProvider()`
  - `getOAuthProviders()`
  - `registerOAuthProvider()`
  - `unregisterOAuthProvider()`
  - `resetOAuthProviders()`
  - `refreshOAuthToken()`
  - `getOAuthApiKey()`
- env/auth helpers
  - `getEnvApiKey()`
- validation/helpers
  - `validateToolCall()`
  - `validateToolArguments()`
  - `parseStreamingJson()`
  - `isContextOverflow()`
  - `getOverflowPatterns()`
  - `createAssistantMessageEventStream()`

### CLI surface
- executable `pi-ai`
- commands
  - `login [provider]`
  - `list`
  - `help`
- login writes `auth.json` in current directory

## Internal architecture summary

### 1. Core type and event layer
`src/types.ts` defines the compatibility contract:
- provider/API identity types
- model metadata
- messages and content blocks
- tool schema interface
- stream options and simplified reasoning options
- assistant event protocol
- OpenAI compatibility configuration types

Important design point: the package normalizes all providers into one event stream protocol:
- `start`
- text block lifecycle
- thinking block lifecycle
- tool call lifecycle
- terminal `done` or `error`

### 2. Registry-driven dispatch
There are two registries:
- API provider registry (`api-registry.ts`) for stream functions by API name
- OAuth provider registry (`utils/oauth/index.ts`) for login/refresh/auth conversion by OAuth provider id

Built-in API providers are registered at module import time via `providers/register-builtins.ts`, but the actual SDK modules are lazy-loaded when invoked.

### 3. High-level generation layer
`stream.ts` is thin:
- import/register built-ins
- resolve API provider from model.api
- delegate to provider-specific `stream` / `streamSimple`
- `complete` and `completeSimple` consume the event stream and return the terminal assistant message

### 4. Shared request utilities
Important shared logic lives outside any one provider:
- `simple-options.ts`
  - maps `SimpleStreamOptions` to provider-specific low-level options
  - clamps xhigh to high where unsupported
  - adjusts max tokens when reasoning budgets need room
- `transform-messages.ts`
  - cross-provider/cross-model history transformation
  - thinking conversion to text on model changes
  - tool call id normalization hook
  - synthetic tool-result insertion for orphaned tool calls
  - filtering of errored/aborted assistant messages from replay
- `json-parse.ts`
  - best-effort partial JSON parsing for streaming tool args
- `sanitize-unicode.ts`
  - removes unpaired surrogates before provider serialization
- `validation.ts`
  - AJV-based tool argument validation with CSP/runtime-codegen fallback
- `overflow.ts`
  - central overflow detection patterns across providers

### 5. Provider implementations
The package has a mixed provider architecture:

#### In-scope Rust reference providers
- `anthropic.ts`
- `openai-responses.ts`
- `openai-responses-shared.ts`
- `openai-completions.ts`
- `openai-codex-responses.ts`
- `faux.ts`

#### Out-of-scope now but package-defining
- `azure-openai-responses.ts`
- `google.ts`
- `google-shared.ts`
- `google-vertex.ts`
- `google-gemini-cli.ts`
- `amazon-bedrock.ts`
- `mistral.ts`

### 6. OAuth subsystem
OAuth is implemented as provider-specific login/refresh modules plus a registry layer:
- Anthropic OAuth (authorization code + PKCE + localhost callback server)
- GitHub Copilot device flow
- Google Gemini CLI and Antigravity OAuth
- OpenAI Codex OAuth

### 7. Generated model catalog
`models.generated.ts` is a generated provider/model database.
Generation logic in `scripts/generate-models.ts` merges:
- models.dev
- OpenRouter
- Vercel AI Gateway
- many hard-coded overrides/additions/fallbacks

The generated catalog contains:
- provider id
- model id/name
- API mapping
- base URL
- reasoning capability
- input modalities
- pricing metadata
- context/max token limits
- optional headers / compat metadata

## Dependency summary

### Runtime dependencies
From `package.json`:
- provider SDKs
  - `openai`
  - `@anthropic-ai/sdk`
  - `@google/genai`
  - `@mistralai/mistralai`
  - `@aws-sdk/client-bedrock-runtime`
- schema/validation
  - `@sinclair/typebox`
  - `ajv`
  - `ajv-formats`
- parsing and transport support
  - `partial-json`
  - `undici`
  - `proxy-agent`
- misc
  - `chalk`
  - `zod-to-json-schema`

### Runtime platform assumptions
- ESM package
- Node >= 20
- browser-safe entrypoint is intentionally preserved with dynamic imports in Node-only areas

### Rust comparison
Current Rust `pi-ai` dependencies match the narrower requested migration scope better than TS:
- `reqwest`, `tokio`, `futures`, `serde`, `serde_json`
- no Google/Mistral/AWS SDK surface in Rust public scope for now

## Config and env var summary

### Generic options (`StreamOptions` / `SimpleStreamOptions`)
- `temperature`
- `maxTokens`
- `signal`
- `apiKey`
- `transport`
- `cacheRetention`
- `sessionId`
- `onPayload`
- `headers`
- `maxRetryDelayMs`
- `metadata`
- `reasoning` (simple options)
- `thinkingBudgets` (simple options)

### Core env vars read directly by `packages/ai`
- `OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`
- `ANTHROPIC_OAUTH_TOKEN`
- `AZURE_OPENAI_API_KEY`
- `AZURE_OPENAI_BASE_URL`
- `AZURE_OPENAI_RESOURCE_NAME`
- `AZURE_OPENAI_API_VERSION`
- `AZURE_OPENAI_DEPLOYMENT_NAME_MAP`
- `GEMINI_API_KEY`
- `GOOGLE_CLOUD_API_KEY`
- `GOOGLE_CLOUD_PROJECT`
- `GOOGLE_CLOUD_PROJECT_ID`
- `GCLOUD_PROJECT`
- `GOOGLE_CLOUD_LOCATION`
- `GOOGLE_APPLICATION_CREDENTIALS`
- `GROQ_API_KEY`
- `CEREBRAS_API_KEY`
- `XAI_API_KEY`
- `OPENROUTER_API_KEY`
- `AI_GATEWAY_API_KEY`
- `ZAI_API_KEY`
- `MISTRAL_API_KEY`
- `MINIMAX_API_KEY`
- `MINIMAX_CN_API_KEY`
- `HF_TOKEN`
- `OPENCODE_API_KEY`
- `KIMI_API_KEY`
- `COPILOT_GITHUB_TOKEN`
- `GH_TOKEN`
- `GITHUB_TOKEN`
- `PI_CACHE_RETENTION`
- `PI_AI_ANTIGRAVITY_VERSION`
- Bedrock/AWS-related auth envs
  - `AWS_PROFILE`
  - `AWS_ACCESS_KEY_ID`
  - `AWS_SECRET_ACCESS_KEY`
  - `AWS_BEARER_TOKEN_BEDROCK`
  - `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`
  - `AWS_CONTAINER_CREDENTIALS_FULL_URI`
  - `AWS_WEB_IDENTITY_TOKEN_FILE`
  - `AWS_REGION`
  - `AWS_DEFAULT_REGION`
  - `AWS_BEDROCK_SKIP_AUTH`
  - `AWS_BEDROCK_FORCE_HTTP1`
  - `AWS_BEDROCK_FORCE_CACHE`
- proxy envs used in some Node paths
  - `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`
  - lowercase variants

### Important config semantics
- `PI_CACHE_RETENTION=long`
  - Anthropic direct API -> cache TTL `1h`
  - OpenAI direct API -> prompt cache retention `24h`
  - proxies do not automatically receive those direct-provider cache settings
- `sessionId`
  - OpenAI Responses -> `prompt_cache_key`
  - OpenAI Codex -> cache key plus session/conversation headers and websocket reuse
  - Faux provider -> simulated cache accounting
- `metadata.user_id`
  - only Anthropic currently consumes it into provider metadata
- provider `headers`
  - merged with model headers and dynamic provider headers; caller overrides win last

## Runtime behavior summary

### Provider registration and loading
- built-ins are registered on import
- built-in wrappers lazy-load provider modules, avoiding eager SDK imports
- Bedrock loading is deliberately Node-only and browser-safe

### Event stream behavior
- every provider emits the normalized assistant event protocol
- `AssistantMessageEventStream.result()` resolves from either terminal `done` or terminal `error`
- partial tool args are exposed during `toolcall_delta`

### Message replay / cross-provider handoff
- same provider + same API + same exact model keeps reasoning/signatures where possible
- different model or provider converts thinking to plain text or drops opaque redacted reasoning when invalid to replay
- tool call ids may be normalized for target-provider constraints
- errored/aborted assistant messages are filtered out from replay
- orphaned tool calls receive synthetic error tool results when a later user/assistant message would otherwise break the chain

### Anthropic-specific behavior
- supports API-key, Claude OAuth, and Copilot Anthropic paths
- Claude Code OAuth path applies canonical tool-name casing outward and maps back to caller tool names inward
- adaptive thinking on Opus/Sonnet 4.6, token-budget thinking on older reasoning models
- prompt caching is modeled using `cache_control` on system and last user blocks
- grouped tool results are emitted as a single Anthropic user turn
- temperature is omitted when thinking is enabled

### OpenAI Responses-specific behavior
- uses Responses API `input` items via shared converter
- preserves response/message IDs and reasoning items
- can send tool result images inside `function_call_output`
- service tier pricing can adjust final cost
- Copilot path omits `reasoning` entirely when not requested
- prompt cache semantics differ between direct OpenAI and proxies

### OpenAI Completions-specific behavior
- extensive compat matrix for OpenAI-compatible servers
- different servers may require:
  - `system` instead of `developer`
  - different max-token field names
  - no `store`
  - no `strict`
  - different reasoning knobs
  - assistant bridge after tool results
  - tool result name field
  - thinking converted to plain text
- tool result images are rerouted into a later synthetic user image message instead of staying in the tool result turn
- usage may come from either `chunk.usage` or `choice.usage`

### OpenAI Codex-specific behavior
- custom transport, not SDK-based
- supports SSE and WebSocket
- WebSocket connections are cached per session for 5 minutes
- auth token is a JWT from which account id is extracted
- request body uses `instructions` for system prompt and shared responses conversion for history
- retries transient errors with backoff
- stream must finish as soon as `response.completed`/`response.incomplete` arrives even if the body/socket remains open

### Azure OpenAI Responses behavior
- deployment name may come from explicit option or env mapping
- base URL may be constructed from resource name
- otherwise follows the same shared Responses conversion model

### Google/Gemini shared semantics relevant to package-wide behavior
- thought signatures are opaque replay tokens, not proof that a part is thinking
- Gemini 3 unsigned tool calls use `skip_thought_signature_validator`
- image tool result routing differs by model family: Gemini 3 and some non-Gemini Cloud Code Assist paths can inline images in function responses; older Gemini paths need a synthetic user image turn

### Faux provider behavior
- deterministic scripted queue for tests/demos
- estimates usage roughly by char count
- simulates prompt caching by session id
- chunks thinking/text/tool args over time and supports aborts

### Validation behavior
- TypeBox + AJV coercion/validation when runtime code generation is available
- browser extension / CSP-restricted runtimes skip validation and return raw args instead of failing noisily

### Unicode behavior
- unpaired surrogates are stripped before provider serialization
- valid surrogate pairs/emoji must remain intact

### Overflow behavior
- explicit overflow errors are recognized via regex patterns across many providers
- some providers can silently overflow or truncate; helper can also use usage/context-window comparison where applicable

## Test inventory

### Core contract / utility tests
- `validation.test.ts`
- `overflow.test.ts`
- `supports-xhigh.test.ts`
- `xhigh.test.ts`
- `lazy-module-load.test.ts`
- `faux-provider.test.ts`
- `responseid.test.ts`

### In-scope provider behavior tests for Rust migration
- Anthropic
  - `anthropic-oauth.test.ts`
  - `anthropic-thinking-disable.test.ts`
  - `anthropic-tool-name-normalization.test.ts`
  - `interleaved-thinking.test.ts` (Anthropic portions)
- OpenAI Responses
  - `openai-responses-copilot-provider.test.ts`
  - `openai-responses-foreign-toolcall-id.test.ts`
  - `openai-responses-reasoning-replay-e2e.test.ts`
  - `openai-responses-tool-result-images.test.ts`
- OpenAI Completions
  - `openai-completions-tool-choice.test.ts`
  - `openai-completions-tool-result-images.test.ts`
- OpenAI Codex
  - `openai-codex-stream.test.ts`
- Shared cross-provider/core
  - `stream.test.ts`
  - `abort.test.ts`
  - `empty.test.ts`
  - `cache-retention.test.ts`
  - `image-tool-result.test.ts`
  - `tokens.test.ts`
  - `total-tokens.test.ts`
  - `tool-call-id-normalization.test.ts`
  - `tool-call-without-result.test.ts`
  - `transform-messages-copilot-openai-to-anthropic.test.ts`
  - `unicode-surrogate.test.ts`
  - `cross-provider-handoff.test.ts`

### Out-of-scope-for-now provider tests still useful for shared semantics
- Google / Vertex / Gemini CLI / Antigravity
  - multiple targeted tests for retry delay, empty streams, thought signatures, disable-thinking, image/tool result routing, unsigned Gemini 3 tool calls, Vertex auth fallback
- Bedrock
  - model smoke tests, credentials helper, interleaved thinking portions
- OpenRouter / Vercel / xAI / Groq / Cerebras / Hugging Face / MiniMax / Kimi / OpenCode / Mistral
  - coverage appears mainly through broad end-to-end test matrices in `stream.test.ts`, `abort.test.ts`, `empty.test.ts`, `context-overflow.test.ts`, `tokens.test.ts`, `total-tokens.test.ts`, `image-tool-result.test.ts`, and specific smoke tests

## Edge cases and implicit behaviors

1. `streamSimple()` is not a trivial wrapper
- it maps human reasoning levels into provider-native flags
- it may adjust max tokens for reasoning budgets
- some provider-specific simple wrappers synchronously throw if auth is missing

2. Error handling is mixed
- design comments say provider failures should be encoded in terminal stream events
- in practice, some `streamSimple...` wrappers throw before streaming when credentials are absent

3. Tool call IDs are a major compatibility surface
- OpenAI Responses IDs may contain `call_id|item_id` with huge opaque item IDs
- Anthropic and some OpenAI-compatible targets require length/pattern normalization
- some same-provider different-model replays must drop or rewrite IDs to avoid reasoning/function-call pairing validation

4. Thinking content is not portable as-is
- same exact model may preserve signatures and opaque blocks
- different model/provider usually requires downgrading to plain text or dropping redacted content

5. Images in tool results are provider-specific
- OpenAI Responses keeps them inline in function-call output
- OpenAI Completions reroutes them into a follow-up user image message
- Google family behavior depends on model family and path

6. Anthropic OAuth path is not generic Anthropic API-key behavior
- special beta headers
- special user agent/app identity
- Claude Code tool-name casing normalization

7. Browser-safe TS loading is intentional
- Node-only imports are deferred in several files
- this is an implementation detail, but it explains why some TS modules avoid top-level runtime imports

8. Model metadata is not purely fetched
- the generator applies many manual overrides, backfills, deletions, provider-specific compat hints, and static additions
- therefore `models.generated.ts` is part of the package behavior, not just incidental generated data

9. Unicode sanitation is required at serialization boundaries
- tests specifically target real-world emoji and intentionally malformed surrogate data in tool results

10. Context overflow detection is intentionally broad and heuristic
- regex-based
- includes explicit exclusions for throttling/rate-limit patterns to avoid false positives

## Compatibility notes for the Rust migration

### What the current Rust `pi-ai` already matches reasonably well
Current Rust work already covers a meaningful subset of the TypeScript package for the requested scope:
- model catalog loaded from TypeScript `models.generated.ts`
- provider registry
- `complete` / `stream` / `stream_simple`
- in-scope provider implementations for:
  - Anthropic Messages
  - OpenAI Responses
  - OpenAI Completions
  - OpenAI Codex Responses
- faux provider
- cache-retention behavior tests
- response-id tests
- image tool result tests
- empty-message handling tests
- overflow tests
- unicode request text tests
- tool-call-without-result tests

### What is clearly still missing or incomplete in Rust relative to TS behavior
1. OAuth surface parity is incomplete at the `pi-ai` crate boundary
- TypeScript package has full programmatic OAuth login/refresh/provider registry surface
- Rust `pi-ai` currently exposes no equivalent OAuth login module from the crate surface that was grounded
- Anthropic Claude Code OAuth path is user-requested in scope, so this is a real compatibility gap

2. TS model/provider surface is much broader than Rust by design
- this is acceptable for the user-requested scope
- but shared behaviors derived from out-of-scope providers still matter when they affect common types/utilities/tests

3. Codex terminal completion behavior is now covered by a TypeScript-derived fixture
- Rust now has fixture-driven coverage for the `response.completed`-before-`[DONE]` Codex case from `openai-codex-stream.test.ts`
- workspace validation is green after adding that coverage

4. Some TS tests still do not appear fully frozen in Rust for the in-scope providers
Remaining likely gaps:
- cross-provider handoff scenarios involving Anthropic/OpenAI/OpenAI Codex
- same-provider different-model replay edge cases
- full total-token accounting parity
- full stream/test matrix parity
- lazy-loading semantics are TS-specific, but equivalent “do not over-initialize provider clients” may still be desirable in Rust

5. TS validation semantics differ from current Rust layering
- TS validates tools inside `pi-ai` with AJV/TypeBox helpers
- Rust validation appears to live more in `pi-agent`
- acceptable if observable behavior remains compatible, but worth explicitly validating

### Implementation relevance by source area
For Rust migration, the most authoritative TS source-of-truth files are:
- `src/types.ts`
- `src/stream.ts`
- `src/models.ts`
- `src/env-api-keys.ts`
- `src/providers/simple-options.ts`
- `src/providers/transform-messages.ts`
- `src/utils/json-parse.ts`
- `src/utils/overflow.ts`
- `src/utils/sanitize-unicode.ts`
- `src/utils/validation.ts`
- `src/providers/anthropic.ts`
- `src/providers/openai-responses.ts`
- `src/providers/openai-responses-shared.ts`
- `src/providers/openai-completions.ts`
- `src/providers/openai-codex-responses.ts`
- `src/providers/faux.ts`
- `src/providers/register-builtins.ts`
- `scripts/generate-models.ts`

### Files that are informative but should not drive current Rust implementation scope
- Google/Vertex/Gemini CLI provider files
- Bedrock provider files
- Mistral provider files
- out-of-scope provider model sections in `models.generated.ts`

## Unknowns requiring validation

1. OAuth boundary placement in Rust
- should OAuth login/refresh live in `pi-ai`, `pi-config`, or `pi-coding-agent-core`?
- TS places login/refresh in the AI package via the `/oauth` entrypoint
- user scope explicitly includes Anthropic Claude Code OAuth behavior on the provider path

2. Extent of cross-provider handoff support required in Rust now
- user provider scope is narrow, but TS has broad handoff logic
- minimum required likely includes:
  - Anthropic -> OpenAI/OpenAI Codex
  - OpenAI Responses -> Anthropic
  - OpenAI model-to-model handoff

3. Which TS compat flags need true Rust equivalents
- TS `OpenAICompletionsCompat` contains many flags for out-of-scope providers
- for current Rust scope, only the subset exercised by OpenAI/Codex/Anthropic should be preserved
- must validate which flags are still needed even for nominal OpenAI/Codex behavior

4. Whether Azure behavior should be ignored entirely in Rust
- user scope says Azure is out of scope
- TS package includes Azure provider and tests
- current Rust should likely exclude it, but note that TS `models.generated.ts` and README still mention it

5. CLI login parity expectations
- TS package exposes a standalone `pi-ai` CLI for OAuth login/listing
- user request is about rewriting packages, but it is unclear whether this CLI must exist in Rust now or whether app-level auth flows are enough

## Recommended next step

Proceed with the next unresolved Step 3 compatibility slice for `packages/ai`:
- keep using TS-derived fixtures where possible instead of broad rewrites
- pick the smallest remaining in-scope fixture target before moving to `packages/agent`

## Suggested fixture candidates for Step 3
- OpenAI Codex SSE stream that must terminate on `response.completed` even if body stays open
- OpenAI Responses foreign tool-call ID normalization
- orphaned tool-call synthetic tool-result insertion
- OpenAI Responses tool result image routing
- OpenAI Completions tool-result image rerouting
- partial JSON tool-call streaming examples
- Unicode surrogate sanitization examples
