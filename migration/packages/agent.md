# packages/agent migration inventory

Status: milestone 10 adds narrowed tool-argument validation/coercion parity plus proxy-stream reconstruction and HTTP proxy-client support on top of the existing loop/state/wrapper/tool/queue slices
Target crate: `rust/crates/pi-agent`

## 1. Files analyzed

TypeScript package files read in full:
- `packages/agent/README.md`
- `packages/agent/src/agent-loop.ts`
- `packages/agent/src/agent.ts`
- `packages/agent/src/index.ts`
- `packages/agent/src/proxy.ts`
- `packages/agent/src/types.ts`
- `packages/agent/test/agent-loop.test.ts`
- `packages/agent/test/agent.test.ts`
- `packages/agent/test/e2e.test.ts`
- `packages/agent/test/utils/calculate.ts`
- `packages/agent/test/utils/get-current-time.ts`

Rust files reviewed before implementation:
- `rust/Cargo.toml`
- `rust/crates/pi-agent/Cargo.toml`
- `rust/crates/pi-agent/src/lib.rs`
- `rust/crates/pi-ai/Cargo.toml`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-events/src/lib.rs`
- `rust/crates/pi-test-harness/Cargo.toml`
- `rust/crates/pi-test-harness/src/lib.rs`

## 2. Exported API inventory

Current TS public surface clusters:
- `Agent` stateful wrapper with prompt/continue/abort/waitForIdle/subscribe/queue helpers
- low-level loop functions: `agentLoop`, `agentLoopContinue`, `runAgentLoop`, `runAgentLoopContinue`
- proxy stream helper: `streamProxy`
- extensible agent types in `types.ts`

The TS package currently exposes both:
- stateful orchestration behavior
- low-level observational event streams

## 3. Internal architecture summary

Observed TS layering:
1. `types.ts` defines agent state, message unions, loop config, tool/runtime hooks, and events
2. `agent-loop.ts` owns low-level orchestration:
   - prompt injection
   - continue validation
   - assistant streaming
   - tool execution
   - steering/follow-up loop control
3. `agent.ts` wraps the low-level loop with mutable state, subscription barriers, abort lifecycle, and queues
4. `proxy.ts` reconstructs assistant partials from backend-sent proxy events

## 4. Dependency summary

TS runtime depends primarily on:
- `@mariozechner/pi-ai` for model types, assistant event stream, validation, and provider calls
- `@sinclair/typebox` for tool schemas

Minimal Rust slice depends on:
- `pi-ai` for normalized AI streaming and faux provider integration
- `pi-events` for message/model/event payload types
- `async-stream`, `futures` for async event streaming

## 5. Config / env / runtime behavior summary

High-value TS behavior observed:
- prompt path emits `agent_start -> turn_start -> user message events -> assistant stream events -> turn_end -> agent_end`
- `continue()` rejects empty context and rejects assistant-tail context
- `continue()` does not re-emit existing user message events
- assistant stream events update partial assistant state before final message_end
- tool execution, steering, follow-up queues, convertToLlm, transformContext, dynamic API keys, and before/after tool hooks are important, but can be deferred from the first Rust slice

## 6. Test inventory summary

TS tests cover these main behavior groups:
- low-level loop event ordering and convert/transform behavior
- tool execution, argument preparation, parallel tool execution ordering
- queued steering/follow-up semantics
- `Agent` state updates, idle barriers, abort, and session forwarding
- end-to-end faux-provider behavior, including tools and continue from tool results

For the first Rust milestone, the narrowest stable subset is:
- standard message-only context
- single assistant turn without tool execution
- continue validation and continuation event sequence
- state reduction of emitted loop events

## 7. Edge cases and implicit behaviors

Confirmed from TS source/tests:
- a stream may finish with only a terminal event; the loop must still emit assistant `message_start` before `message_end`
- aborted/error assistant turns still produce `turn_end` and `agent_end`
- `Agent.processEvents()` mutates state before awaiting listeners
- `isStreaming` in the TS `Agent` wrapper remains true until post-`agent_end` cleanup; that wrapper-level nuance is deferred in Rust until the stateful wrapper exists

## 8. Compatibility notes for the current Rust slice

Implemented now:
- `AgentMessage` abstraction with:
  - `AgentMessage::Standard(Message)`
  - `AgentMessage::Custom(CustomAgentMessage)` backed by JSON payloads
- `AgentState` and `AgentContext` data model over `AgentMessage`
- minimal `AgentEvent` stream for prompt + continue flows
- passthrough to `pi-ai` streaming with partial assistant updates
- default streamer via `pi_ai::stream_response`
- injectable streamer for deterministic tests
- low-level context shaping support with:
  - `transform_context` hook over `Vec<AgentMessage>`
  - `convert_to_llm` hook from `Vec<AgentMessage>` to `Vec<Message>`
  - default conversion that passes through standard LLM-visible messages and filters custom messages
- minimal `Agent` wrapper with:
  - generic `prompt(...)`, `prompt_messages(...)`, `continue`
  - `steer`, `follow_up`, queue clear helpers, and queue presence check
  - `set_transform_context(...)` / `clear_transform_context()`
  - `set_convert_to_llm(...)` / `clear_convert_to_llm()`
  - `abort`
  - `wait_for_idle`
  - ordered awaited listeners
  - run-state reduction into `AgentState`
  - synthetic assistant failure messages when the low-level loop errors unexpectedly
- minimal executable tool runtime with:
  - `AgentTool`
  - optional `prepare_arguments`
  - optional streamed partial tool updates via `AgentTool::new_with_updates(...)`
  - sequential tool execution
  - `tool_execution_start` / `tool_execution_update` / `tool_execution_end`
  - `toolResult` message emission
  - next-turn continuation after tool results
  - pending-tool-call state tracking during updates
- minimal queue-aware loop control with:
  - low-level steering polling before assistant turns
  - low-level follow-up polling after the agent would otherwise stop
  - assistant-tail `continue()` recovery via queued steering/follow-up messages
  - wrapper queue mode configuration for `All` vs `OneAtATime`
- minimal hook support with:
  - low-level `before_tool_call` and `after_tool_call` hooks
  - wrapper forwarding via `Agent::set_before_tool_call(...)` and `Agent::set_after_tool_call(...)`
  - before-hook blocking and in-place prepared-arg mutation via shared JSON value
  - after-hook content/details/error overrides

Deferred explicitly:
- JSON-schema validation parity with TS `validateToolArguments`
- proxy reconstruction
- direct mutable state API parity with TS property-style mutation
- parallel tool execution

## 9. Rust target design (`pi-agent`)

Current modules:
- `message.rs`: `AgentMessage`, `CustomAgentMessage`, message helpers
- `error.rs`: crate error type and `pi-ai` error bridging
- `state.rs`: `ThinkingLevel`, `AgentContext`, `AgentState`, event-state reduction helpers
- `tool.rs`: `AgentTool`, `AgentToolResult`, `AgentToolError`, optional argument preparation, optional tool update callbacks
- `loop.rs`: `AgentLoopConfig`, `AssistantStreamer`, `agent_loop`, `agent_loop_continue`, `AgentEvent`, tool hook types, tool progress event streaming
- `agent.rs`: minimal stateful `Agent` wrapper
- `lib.rs`: re-exports

Public API available after milestone 7:
- `CustomAgentMessage::new(...)`
- `AgentMessage::{Standard, Custom}` and helpers (`custom`, `role`, `timestamp`, `is_assistant`, standard-message accessors)
- `AgentState::new(model)`
- `AgentState::context_snapshot()`
- `AgentState::begin_run()` / `apply_event()` / `finish_run()`
- `AgentLoopConfig::new(model)` with injectable streamer override
- `AgentLoopConfig::with_convert_to_llm(...)`
- `AgentLoopConfig::with_transform_context(...)`
- `AgentLoopConfig::with_before_tool_call(...)`
- `AgentLoopConfig::with_after_tool_call(...)`
- `agent_loop(prompts, context, config)`
- `agent_loop_continue(context, config)`
- `AgentTool::new(definition, executor)`
- `AgentTool::new_with_updates(definition, executor)`
- `AgentTool::with_prepare_arguments(...)`
- `AgentTool::execute(...)` / `AgentTool::execute_with_updates(...)`
- `AgentToolUpdateCallback`
- `QueueMode::{All, OneAtATime}`
- `Agent::new(initial_state)`
- `Agent::with_parts(initial_state, streamer, stream_options)`
- `Agent::state()` / `Agent::update_state(...)`
- `Agent::subscribe(...)` / `Agent::unsubscribe(...)`
- `Agent::set_convert_to_llm(...)` / `Agent::clear_convert_to_llm()`
- `Agent::set_transform_context(...)` / `Agent::clear_transform_context()`
- `Agent::set_before_tool_call(...)` / `Agent::clear_before_tool_call()`
- `Agent::set_after_tool_call(...)` / `Agent::clear_after_tool_call()`
- `Agent::set_steering_mode(...)` / `Agent::steering_mode()`
- `Agent::set_follow_up_mode(...)` / `Agent::follow_up_mode()`
- `Agent::steer(...)` / `Agent::follow_up(...)`
- `Agent::clear_steering_queue()` / `Agent::clear_follow_up_queue()` / `Agent::clear_all_queues()`
- `Agent::has_queued_messages()`
- `Agent::prompt_text(...)` / `Agent::prompt(...)` / `Agent::prompt_messages(...)` / `Agent::continue()`
- `Agent::abort()` / `Agent::wait_for_idle()`

## 10. Validation plan

Milestone 8 validation target:
- deterministic scripted-stream tests for prompt/continue event order
- state reduction test driven by emitted events
- one faux-provider integration test through `pi-ai` default streaming path
- wrapper tests for listener barriers, `wait_for_idle`, abort, active-run rejection, and synthesized failure messages
- low-level sequential tool execution test with continuation turn
- wrapper-level pending-tool-call tracking test during tool execution
- low-level tests for prepare-arguments, before-hook mutation/blocking, and after-hook overrides
- wrapper-level test proving before/after hook forwarding

## 11. Known risks

- custom agent messages are now supported via `CustomAgentMessage`, but the current representation is intentionally JSON-backed and not declaration-merge style like TS
- tool execution is still sequential-only
- JSON-schema validation parity is still missing; current Rust supports `prepare_arguments` but does not yet replicate TS `validateToolArguments` behavior or AJV coercion
- tool progress updates are now implemented for sequential execution only; parallel execution parity is still missing with the broader parallel tool slice
- the wrapper has awaited listeners, idle barriers, queue-mode configuration, transform/convert hooks, and custom messages, but still uses Rust method-based APIs rather than TS property-style setters/getters
- the wrapper currently uses explicit methods plus `update_state(...)`, not full TS property-style mutable state parity

## 12. Recommended follow-up after this milestone

Next increment should stay on `packages/agent` and add JSON-schema validation parity, or begin the first `packages/coding-agent` core integration layer on top of the now-more-complete `pi-agent` API.

## Milestone 9 update: narrowed tool-argument validation + coercion slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/agent/src/agent-loop.ts`
- `packages/ai/src/utils/validation.ts`
- `packages/ai/test/validation.test.ts`
- previously grounded `packages/agent/test/agent-loop.test.ts`
- previously grounded `packages/agent/test/e2e.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-agent/src/lib.rs`
- `rust/crates/pi-agent/src/loop.rs`
- `rust/crates/pi-agent/src/tool.rs`
- `rust/crates/pi-agent/tests/agent_loop.rs`
- `rust/crates/pi-agent/Cargo.toml`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- tool-call argument validation now runs after `prepare_arguments(...)` and before `before_tool_call(...)`, matching the TypeScript `agent-loop.ts` ordering through `validateToolArguments(...)`
- narrowed JSON-schema validation now exists for the Rust agent tool path across the schema features exercised by the current migrated tools/tests:
  - object schemas with `properties` + `required`
  - array item validation
  - primitive type checks for `string`, `number`, `integer`, `boolean`, and `null`
- narrowed AJV-style coercion now exists for the high-value migrated cases:
  - string -> integer
  - string -> number
  - string -> boolean
  - number/bool -> string
- validation failures now stay inside the normal agent loop protocol instead of surfacing as uncaught Rust errors:
  - `tool_execution_end` emits `is_error: true`
  - a tool-result message is still emitted with the formatted validation error text
  - the turn then continues normally
- before-hook mutation behavior remains intentionally unchanged after validation:
  - validated arguments are shared into `before_tool_call(...)`
  - hook mutations are not revalidated, matching the current TypeScript behavior

Compatibility note for this slice:
- this is a narrowed Rust validation slice, not full AJV parity
- unsupported JSON-schema keywords still pass through without enforcement today
- the migrated coercion/error formatting now covers the concrete schema shapes used by the current Rust agent/coding-agent tool surface and the TypeScript validation helper shape, without trying to port every AJV feature at once

### Rust design summary

New internal Rust module added:
- `rust/crates/pi-agent/src/validation.rs`

New internal validation surface:
- `validate_tool_arguments(tool, arguments)`
- recursive schema validation/coercion helpers over `serde_json::Value`
- TS-shaped formatted error messages for tool validation failures

Integration change:
- `rust/crates/pi-agent/src/loop.rs`
  - tool-call preparation now runs:
    1. `prepare_arguments(...)`
    2. `validate_tool_arguments(...)`
    3. `before_tool_call(...)`
  - validation failures now materialize as immediate error tool results inside the normal event stream

Behavior-freeze coverage added in Rust:
- unit tests in `validation.rs` for:
  - root required-property formatting
  - nested array-path formatting
  - integer coercion from string input
- integration tests in `rust/crates/pi-agent/tests/agent_loop.rs` for:
  - validation failures becoming error tool results
  - string-number coercion before tool execution

### Validation summary

New Rust coverage added for:
- formatted validation failure text matching the current TS helper shape
- nested array/object validation path formatting
- integer coercion before tool execution
- validation errors flowing through tool-result messages instead of aborting the loop
- existing prepare/before-hook regression still proving there is no revalidation after hook mutation

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-agent` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-agent`:
- full AJV/json-schema keyword parity beyond the currently exercised schema subset
- proxy stream reconstruction parity for `packages/agent/src/proxy.ts`
- broader wrapper/property-surface parity with the TypeScript `Agent` class
- explicit re-grounding of agent-level parallel tool-execution semantics in the migration note now that the Rust implementation already supports the current narrowed parallel path

## Milestone 10 update: proxy-stream reconstruction + HTTP proxy client slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/agent/src/proxy.ts`
- `packages/agent/README.md` (proxy usage section)
- previously grounded `packages/ai/src/types.ts` for camelCase wire-shape alignment

Additional Rust files read for this slice:
- `rust/Cargo.toml`
- `rust/crates/pi-ai/src/lib.rs`
- `rust/crates/pi-agent/src/lib.rs`
- `rust/crates/pi-agent/Cargo.toml`
- `rust/crates/pi-events/src/lib.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- `pi-agent` now has a proxy-stream helper corresponding to `packages/agent/src/proxy.ts`
- Rust now supports browser-style backend proxy streaming through HTTP POST to `<proxyUrl>/api/stream` with:
  - bearer auth via `Authorization: Bearer <token>`
  - JSON request body containing model/context/options
  - SSE response parsing from `data: ...` lines
- Rust now reconstructs normalized assistant partials from proxy events that omit the `partial` payload, matching the TS bandwidth-saving design:
  - `start`
  - `text_start` / `text_delta` / `text_end`
  - `thinking_start` / `thinking_delta` / `thinking_end`
  - `toolcall_start` / `toolcall_delta` / `toolcall_end`
  - terminal `done` / `error`
- proxy request serialization now uses the TS camelCase wire shape instead of raw Rust struct serialization for the migrated slice, including:
  - `baseUrl`
  - `contextWindow`
  - `maxTokens`
  - `systemPrompt`
  - `responseId`
  - `stopReason`
  - `toolCallId`
  - `toolName`
  - `isError`
  - `mimeType`
- proxy response usage payloads now deserialize from the TS camelCase wire shape into Rust `pi_events::Usage`
- proxy error handling now follows the current TS behavior closely:
  - non-2xx HTTP responses become terminal assistant `error` events
  - JSON error bodies with `{ error: ... }` become `Proxy error: <message>`
  - request/read/JSON failures become terminal assistant `error` events
  - pre-aborted requests become terminal assistant `aborted` events with `Request aborted by user`

Compatibility note for this slice:
- the Rust proxy request body can only serialize the model/context fields present in the current Rust `pi_events` types, so TS-only metadata not yet represented there cannot be forwarded yet
- partial tool-argument parsing is intentionally narrowed: Rust updates streamed tool-call arguments when the accumulated JSON becomes valid, and guarantees final tool-call arguments on `toolcall_end`; it does not yet port TS `partial-json` permissiveness for every incomplete intermediate shape

### Rust design summary

New Rust module added:
- `rust/crates/pi-agent/src/proxy.rs`

New public Rust surface:
- `ProxyStreamConfig`
- `ProxyStreamer`
- `stream_proxy(...)`

Design choices for this slice:
- `ProxyStreamer` implements the existing `AssistantStreamer` trait so it can plug directly into the current Rust `Agent` / `AgentLoopConfig` integration points
- `stream_proxy(...)` remains available as the lower-level helper corresponding to the TS public API
- request/response wire-shape conversion is kept local to `proxy.rs` rather than widening `pi-events` serialization rules for every crate
- proxy failures are encoded as terminal assistant `error` / `aborted` events inside the returned stream, matching the TS contract that stream functions should not throw normal request/runtime failures

Behavior-freeze coverage added in Rust:
- unit tests in `proxy.rs` for:
  - camelCase request body shaping
  - proxy event reconstruction for streamed tool calls and usage
  - assistant/tool-result camelCase request field names
- integration tests in `rust/crates/pi-agent/tests/proxy.rs` for:
  - end-to-end HTTP proxy request/response behavior
  - non-2xx JSON error mapping
  - pre-aborted request behavior

### Validation summary

New Rust coverage added for:
- end-to-end proxy POST body camelCase compatibility
- reconstruction of text/tool-call/done event sequences from stripped proxy events
- terminal usage propagation from camelCase proxy payloads into Rust `Usage`
- JSON proxy error body mapping to terminal assistant error events
- abort-before-request behavior without a network dependency

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-agent` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-agent`:
- fuller intermediate partial-JSON tool-argument reconstruction parity with TS `partial-json`
- broader wrapper/property-surface parity with the TypeScript `Agent` class
- explicit re-grounding of agent-level parallel tool-execution semantics in the migration note now that the Rust implementation already supports the current narrowed parallel path

## Milestone 11 update: default-streamer simple-reasoning path slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/agent/src/agent.ts`
- `packages/agent/src/types.ts`
- `packages/agent/test/agent.test.ts`
- `packages/agent/test/agent-loop.test.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-agent/src/agent.rs`
- `rust/crates/pi-agent/src/loop.rs`
- `rust/crates/pi-agent/tests/agent.rs`
- `rust/crates/pi-agent/tests/agent_loop.rs`
- `rust/crates/pi-ai/src/lib.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- the Rust `Agent` wrapper now forwards `AgentState.thinking_level` into per-request stream options for each run
- the Rust default assistant streamer no longer dispatches through the raw `pi_ai::stream_response()` path for its default behavior
- instead, the default streamer now uses the narrowed `pi_ai::stream_simple()` path, matching the TypeScript default `streamSimple` wiring more closely
- this means the Rust agent now inherits the already-migrated `pi-ai` simple-option behavior for default runs, including:
  - reasoning level mapping from agent state
  - xhigh clamping handled in `pi-ai`
  - Anthropic non-adaptive max-token adjustment handled in `pi-ai`
- low-level Rust `AgentLoopConfig.stream_options.reasoning_effort` now also benefits from the simple-path mapping when the default streamer is used

Compatibility note for this slice:
- this milestone intentionally targets the default-streamer path only
- Rust still does not expose the full TypeScript `AgentOptions` property surface (`thinkingBudgets`, `transport`, `maxRetryDelayMs`, `onPayload`, etc.) as first-class wrapper setters/getters yet
- the coding-agent runtime has its own custom streamer and therefore remains a separate downstream parity step

### Rust design summary

Implementation changes:
- `rust/crates/pi-agent/src/agent.rs`
  - run preparation now maps `AgentState.thinking_level` to the outgoing request `reasoning_effort` string
- `rust/crates/pi-agent/src/loop.rs`
  - `DefaultAssistantStreamer` now converts `StreamOptions` into narrowed `pi_ai::SimpleStreamOptions`
  - default dispatch now calls `pi_ai::stream_simple(...)`
  - added local reasoning-string parsing helper for the simple-path bridge

Behavior-freeze coverage added in Rust:
- `rust/crates/pi-agent/tests/agent.rs`
  - wrapper-level proof that `AgentState.thinking_level` becomes outgoing `reasoning_effort`
- `rust/crates/pi-agent/tests/agent_loop.rs`
  - default-streamer proof that a reasoning request now goes through the simple-path mapping and picks up Anthropic max-token adjustment

### Validation summary

New Rust coverage added for:
- wrapper forwarding of `thinking_level = high` and `thinking_level = off`
- default-streamer simple-path reasoning mapping for an Anthropic-style model
- inherited Anthropic simple max-token adjustment via the default agent streamer path

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-agent` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-agent`:
- fuller wrapper/property-surface parity with the TypeScript `Agent` class
- explicit wrapper/runtime support for additional TS stream-simple knobs such as `thinkingBudgets` and other top-level agent options
- fuller intermediate partial-JSON reconstruction parity in the proxy/tool path
- downstream coding-agent parity still needs its own runtime streamer update so selected thinking levels affect the non-interactive app end-to-end

## Milestone 12 update: default-streamer `thinkingBudgets` forwarding slice

### Files analyzed

Additional TypeScript grounding used for this slice:
- `packages/agent/src/agent.ts`
- `packages/agent/src/types.ts`
- `packages/agent/test/agent.test.ts`
- `packages/agent/test/agent-loop.test.ts`
- `packages/ai/src/providers/simple-options.ts`

Additional Rust files read for this slice:
- `rust/crates/pi-agent/src/agent.rs`
- `rust/crates/pi-agent/src/loop.rs`
- `rust/crates/pi-agent/src/lib.rs`
- `rust/crates/pi-agent/tests/agent.rs`
- `rust/crates/pi-agent/tests/agent_loop.rs`
- `rust/crates/pi-ai/src/lib.rs`

### Behavior summary

New TS-grounded behaviors now covered in Rust:
- the Rust default assistant-streamer path now forwards custom `thinkingBudgets` into `pi_ai::stream_simple(...)`
- low-level Rust `AgentLoopConfig` now has explicit `with_thinking_budgets(...)` support for the default-streamer path
- the Rust `Agent` wrapper now exposes first-class default-streamer budget control with:
  - `Agent::set_thinking_budgets(...)`
  - `Agent::thinking_budgets()`
- `pi-agent` now re-exports `ThinkingBudgets`, matching the TypeScript package split more closely for callers that configure agent reasoning budgets alongside agent state
- Anthropic non-adaptive simple-path requests now honor caller-provided high-level budgets through `pi-agent`, not just the `pi-ai` defaults

Compatibility note for this slice:
- this milestone still intentionally targets the default-streamer path only
- custom Rust `AssistantStreamer` implementations still receive `StreamOptions`, not full simple-option structs, so `thinkingBudgets` currently affects only the built-in default streamer
- the remaining TS top-level knobs (`transport`, `maxRetryDelayMs`, broader wrapper property parity) are still deferred

### Rust design summary

Implementation changes:
- `rust/crates/pi-agent/src/loop.rs`
  - `DefaultAssistantStreamer` now carries `ThinkingBudgets`
  - `AgentLoopConfig` now tracks whether it is still using the default streamer and can apply `with_thinking_budgets(...)` without clobbering custom streamers
- `rust/crates/pi-agent/src/agent.rs`
  - `Agent` now stores default-streamer thinking budgets separately from `StreamOptions`
  - run preparation now threads those budgets into `AgentLoopConfig` when the wrapper is still using the built-in default streamer
- `rust/crates/pi-agent/src/lib.rs`
  - now re-exports `ThinkingBudgets`

Behavior-freeze coverage added in Rust:
- `rust/crates/pi-agent/tests/agent_loop.rs`
  - default-streamer proof that custom high thinking budgets change the Anthropic simple-path `max_tokens`
- `rust/crates/pi-agent/tests/agent.rs`
  - wrapper-level proof that `Agent::set_thinking_budgets(...)` reaches the default-streamer request path end-to-end

### Validation summary

New Rust coverage added for:
- low-level default-streamer custom thinking-budget forwarding
- wrapper-level default-streamer custom thinking-budget forwarding
- registry-mutation serialization in the affected default-streamer tests so provider-registration tests stay deterministic under parallel test execution

Validation run results:
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -p pi-agent --test agent --test agent_loop` passed
- `cd rust && cargo test -p pi-agent` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

### Remaining gaps after this milestone

Still deferred in `pi-agent`:
- fuller wrapper/property-surface parity with the TypeScript `Agent` class
- additional TS top-level simple-stream knobs beyond the current default-streamer `thinkingBudgets` slice
- fuller intermediate partial-JSON reconstruction parity in the proxy/tool path
- downstream coding-agent parity still needs its own runtime streamer update so selected thinking levels and budgets affect the non-interactive app end-to-end
