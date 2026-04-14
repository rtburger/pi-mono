# packages/agent inventory

Date: 2026-04-14
Status: Step 1 complete

## Files analyzed

### Package metadata and docs
- `packages/agent/package.json`
- `packages/agent/README.md`
- `packages/agent/CHANGELOG.md`
- `packages/agent/vitest.config.ts`

### Source files
- `packages/agent/src/index.ts`
- `packages/agent/src/agent.ts`
- `packages/agent/src/agent-loop.ts`
- `packages/agent/src/proxy.ts`
- `packages/agent/src/types.ts`

### Tests and helpers
- `packages/agent/test/agent.test.ts`
- `packages/agent/test/agent-loop.test.ts`
- `packages/agent/test/e2e.test.ts`
- `packages/agent/test/utils/calculate.ts`
- `packages/agent/test/utils/get-current-time.ts`

### Rust grounding already read
- `rust/Cargo.toml`
- `rust/apps/pi/Cargo.toml`
- `rust/apps/pi/src/main.rs`
- `rust/crates/pi-agent/Cargo.toml`
- `rust/crates/pi-agent/src/lib.rs`
- `rust/crates/pi-agent/tests/agent.rs`
- `rust/crates/pi-agent/tests/agent_loop.rs`
- `rust/crates/pi-agent/tests/proxy.rs`
- `migration/packages/workspace-grounding.md`

## Exported API inventory

### Root entry (`src/index.ts`)
- re-exports `Agent`
- re-exports `agentLoop` and `agentLoopContinue`
- re-exports `streamProxy`
- re-exports all public types from `src/types.ts`

### `src/agent.ts`
- `Agent` class
- `AgentOptions`
- runtime queue mode type alias `QueueMode = "all" | "one-at-a-time"`
- public instance API:
  - `subscribe(listener)`
  - `state` getter
  - `steeringMode` getter/setter
  - `followUpMode` getter/setter
  - `steer(message)`
  - `followUp(message)`
  - `clearSteeringQueue()`
  - `clearFollowUpQueue()`
  - `clearAllQueues()`
  - `hasQueuedMessages()`
  - `signal` getter
  - `abort()`
  - `waitForIdle()`
  - `reset()`
  - `prompt(...)`
  - `continue()`
- internal runtime behavior hooks:
  - `convertToLlm`
  - `transformContext`
  - `streamFn`
  - `getApiKey`
  - `onPayload`
  - `beforeToolCall`
  - `afterToolCall`
  - `sessionId`
  - `thinkingBudgets`
  - `transport`
  - `maxRetryDelayMs`
  - `toolExecution`

### `src/agent-loop.ts`
- `agentLoop(prompts, context, config, signal?, streamFn?)`
- `agentLoopContinue(context, config, signal?, streamFn?)`
- internal helpers used by the low-level loop:
  - `runAgentLoop`
  - `runAgentLoopContinue`

### `src/proxy.ts`
- `streamProxy(model, context, options)`
- `ProxyStreamOptions`
- `ProxyAssistantMessageEvent`

### `src/types.ts`
- `StreamFn`
- `ToolExecutionMode`
- `AgentToolCall`
- `BeforeToolCallResult`
- `AfterToolCallResult`
- `BeforeToolCallContext`
- `AfterToolCallContext`
- `AgentLoopConfig`
- `ThinkingLevel`
- `CustomAgentMessages`
- `AgentMessage`
- `AgentState`
- `AgentToolResult`
- `AgentToolUpdateCallback`
- `AgentTool`
- `AgentContext`
- `AgentEvent`

## Internal architecture summary

### 1. `Agent` is a stateful wrapper around the low-level loop
`Agent` owns:
- transcript state
- pending steering/follow-up queues
- stream lifecycle state (`isStreaming`, `streamingMessage`, `pendingToolCalls`, `errorMessage`)
- listener registration
- abort controller state for the current run

The class is intentionally thin. It mostly:
- normalizes prompt input
- builds a loop config from current state
- forwards control to `agentLoop` / `agentLoopContinue`
- reduces emitted loop events back into public agent state

### 2. The low-level loop handles the actual agent runtime
`agent-loop.ts` is the core orchestration engine.

Important behavior:
- emits lifecycle events in order
- converts `AgentMessage[]` to LLM `Message[]` only at the model boundary
- supports optional `transformContext` before `convertToLlm`
- resolves API keys dynamically for each request
- processes streamed assistant responses
- executes tool calls
- supports sequential and parallel tool execution
- injects steering and follow-up messages
- stops early on aborted/error assistant turns

### 3. Tool execution is explicit and hookable
Tool calls are prepared in this order:
1. resolve tool by exact name
2. apply `prepareArguments` compatibility shim
3. validate against the schema
4. run `beforeToolCall`
5. execute the tool
6. run `afterToolCall`
7. emit tool result events

Parallel mode is intentionally not “fire everything and forget”; it still preflights sequentially and preserves assistant source order for final tool result emission.

### 4. Proxy mode reconstructs assistant messages from compact server events
`streamProxy` receives line-delimited JSON events from a backend proxy and reconstructs a full partial assistant message locally.

It mirrors the same assistant event protocol used by direct provider streams, including:
- text blocks
- thinking blocks
- tool calls
- terminal done/error states

## Dependency summary

### Runtime dependencies
From `package.json` and source imports:
- `@mariozechner/pi-ai`
- `@sinclair/typebox` (tool schemas in tests and tool definitions)

### Rust migration dependencies already present
The Rust target for this package is already split across:
- `pi-agent`
- `pi-ai`
- `pi-events`
- `futures`
- `async-stream`
- `tokio`
- `serde_json`
- `thiserror`

### External runtime assumptions
- Node.js >= 20 in the TS implementation
- AbortSignal-based cancellation
- async listener ordering is observable behavior

## Config / option summary

### Agent construction options
- `initialState`
- `convertToLlm`
- `transformContext`
- `streamFn`
- `getApiKey`
- `onPayload`
- `beforeToolCall`
- `afterToolCall`
- `steeringMode`
- `followUpMode`
- `sessionId`
- `thinkingBudgets`
- `transport`
- `maxRetryDelayMs`
- `toolExecution`

### Runtime state semantics
- assigning `state.tools` copies the top-level array
- assigning `state.messages` copies the top-level array
- `state.model` is replaceable
- `state.systemPrompt` and `state.thinkingLevel` are mutable
- `state.pendingToolCalls`, `state.isStreaming`, `state.streamingMessage`, and `state.errorMessage` are runtime-owned fields

### No direct env vars in this package
`packages/agent` does not read environment variables directly. It relies on `getApiKey` and the underlying `pi-ai` layer for auth/env resolution.

## Runtime behavior summary

### Prompting and continuation
- `prompt(string)` converts text into a user message with optional image attachments
- `prompt(AgentMessage)` accepts an already-formed message
- `prompt(AgentMessage[])` injects a batch of messages
- `continue()` resumes from the current transcript, but only when the last message is not an assistant message
- both `prompt()` and `continue()` throw if called while a run is already active

### Event barrier behavior
- `Agent.subscribe()` listeners are awaited in registration order
- listeners receive the active `AbortSignal`
- `agent_end` is the final emitted loop event, but `waitForIdle()` and `prompt()` settle only after awaited `agent_end` listeners finish
- `state.isStreaming` stays true until that settlement completes

### Queue behavior
- `steer()` queues a message to be injected after the current assistant turn finishes executing its tool calls
- `followUp()` queues a message only after the agent would otherwise stop
- `steeringMode` / `followUpMode` control whether one or all queued messages are drained
- `clearAllQueues()` clears both queues
- `continue()` can drain queued steering or follow-up messages when resuming from an assistant tail

### Tool execution behavior
- tool calls are executed against `AgentContext.tools`
- missing tool names become error tool results
- `beforeToolCall` can block execution
- `afterToolCall` can override content/details/isError after tool execution
- tool execution update callbacks are forwarded as `tool_execution_update` events
- parallel mode preserves source order for final tool result messages

### Proxy behavior
- proxy events are compact and reconstructed locally
- errors and aborts become terminal assistant error messages
- the stream can be aborted by cancelling the active reader

## Test inventory

### `agent.test.ts`
Covers:
- default agent state
- custom initial state
- event subscription and unsubscribe behavior
- async subscriber settlement before prompt resolves
- `waitForIdle()` barrier behavior
- abort signal forwarding to subscribers
- mutating public state fields
- queueing steering and follow-up messages
- abort controller behavior
- prompt/continue concurrent-call errors
- `continue()` handling of queued follow-up and steering messages
- `sessionId` forwarding to the underlying stream function

### `agent-loop.test.ts`
Covers:
- low-level event flow
- custom message conversion through `convertToLlm`
- `transformContext` before `convertToLlm`
- tool call execution
- `beforeToolCall` argument mutation before execution
- `prepareArguments` compatibility shim
- parallel tool execution
- tool result ordering
- steering message injection after tool batches
- `agentLoopContinue()` validation and behavior
- custom message continuation via `convertToLlm`

### `e2e.test.ts`
Covers:
- integration with the faux provider
- basic text prompting
- tool execution and pending tool call tracking
- abort during streaming
- lifecycle event sequencing
- multi-turn context retention
- thinking block preservation
- `continue()` validation from empty/assistant/user/toolResult states

### Test helpers
- `calculate.ts` provides a simple tool used by multiple tests
- `get-current-time.ts` provides a timezone-aware tool used by time-related tests

## Edge cases and implicit behaviors

- `Agent` constructor defaults are intentionally minimal and permissive
- `state.tools` and `state.messages` copy only the top-level array; nested objects remain shared
- `Agent.subscribe()` does not emit an initial event
- `prompt()` with a string and image array preserves the image order after the text block
- `continue()` from an assistant tail can consume queued steering/follow-up messages instead of throwing, if any are available
- tool preparation errors are converted into tool-result errors rather than throwing out of the loop
- `beforeToolCall` sees already-validated args, but those args may have been mutated by `prepareArguments`
- `afterToolCall` receives the pre-overridden tool result and can replace content/details/isError wholesale
- `tool_execution_end` is emitted before final tool result message events
- `agent_end` listeners can keep the run active after the last loop event is emitted

## Compatibility notes

- TS declaration merging for `CustomAgentMessages` has no direct Rust equivalent; the Rust design will need an explicit replacement, likely via enums or trait-based extensions.
- `AgentTool` is generic in TS and can carry arbitrary details; the Rust design should keep that flexibility without falling back to `any`-like untyped blobs unless necessary.
- The TS proxy stream is a convenience layer, not core agent behavior; it can stay separate in Rust if not needed by the first vertical slice.
- The TS API allows custom `convertToLlm` and `transformContext` hooks to be failure-tolerant; Rust should preserve that contract.
- Public state copy semantics are observable and need to stay intact.

## Unknowns requiring validation

- How much of the proxy stream belongs in the initial Rust migration slice
- Whether the Rust public API should keep the exact TS method names or prefer idiomatic Rust wrapper methods in addition to a compatibility surface
- How to model TS custom message extension points in Rust without weakening type safety
- Whether queued steering/follow-up behavior in the Rust crate already matches the assistant-tail resume edge cases exactly
- Whether `sessionId`, `maxRetryDelayMs`, and transport forwarding are already wired through the Rust `pi-ai` crate exactly as the TS package does

## Rust grounding summary

The Rust workspace already has a dedicated `pi-agent` crate with the expected split:
- `agent`
- `loop`
- `message`
- `proxy`
- `state`
- `tool`
- `validation`
- `partial_json`
- `error`

That crate already exports the same high-level concepts (`Agent`, low-level loop helpers, proxy streaming, agent state, and tool abstractions), so the migration work here is refinement and parity validation rather than a greenfield rewrite.
