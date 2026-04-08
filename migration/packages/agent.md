# packages/agent migration inventory

Status: milestone 4 scaffold + minimal loop/state slice + minimal Agent wrapper slice + sequential tool execution + tool prepare/before/after hook slice
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
- `AgentState` and `AgentContext` data model
- minimal `AgentEvent` stream for prompt + continue flows
- passthrough to `pi-ai` streaming with partial assistant updates
- default streamer via `pi_ai::stream_response`
- injectable streamer for deterministic tests
- minimal `Agent` wrapper with:
  - `prompt_text`, `prompt_messages`, `continue`
  - `abort`
  - `wait_for_idle`
  - ordered awaited listeners
  - run-state reduction into `AgentState`
  - synthetic assistant failure messages when the low-level loop errors unexpectedly
- minimal executable tool runtime with:
  - `AgentTool`
  - optional `prepare_arguments`
  - sequential tool execution
  - `tool_execution_start` / `tool_execution_end`
  - `toolResult` message emission
  - next-turn continuation after tool results
  - pending-tool-call state tracking
- minimal hook support with:
  - low-level `before_tool_call` and `after_tool_call` hooks
  - wrapper forwarding via `Agent::set_before_tool_call(...)` and `Agent::set_after_tool_call(...)`
  - before-hook blocking and in-place prepared-arg mutation via shared JSON value
  - after-hook content/details/error overrides

Deferred explicitly:
- custom agent messages and `convertToLlm`
- `transformContext`
- JSON-schema validation parity with TS `validateToolArguments`
- streaming tool progress updates
- steering/follow-up queues
- proxy reconstruction
- direct mutable state API parity with TS property-style mutation
- parallel tool execution

## 9. Rust target design (`pi-agent`)

Current modules:
- `error.rs`: crate error type and `pi-ai` error bridging
- `state.rs`: `ThinkingLevel`, `AgentContext`, `AgentState`, event-state reduction helpers
- `tool.rs`: `AgentTool`, `AgentToolResult`, `AgentToolError`, optional argument preparation
- `loop.rs`: `AgentLoopConfig`, `AssistantStreamer`, `agent_loop`, `agent_loop_continue`, `AgentEvent`, tool hook types
- `agent.rs`: minimal stateful `Agent` wrapper
- `lib.rs`: re-exports

Public API available after milestone 4:
- `AgentState::new(model)`
- `AgentState::context_snapshot()`
- `AgentState::begin_run()` / `apply_event()` / `finish_run()`
- `AgentLoopConfig::new(model)` with injectable streamer override
- `AgentLoopConfig::with_before_tool_call(...)`
- `AgentLoopConfig::with_after_tool_call(...)`
- `agent_loop(prompts, context, config)`
- `agent_loop_continue(context, config)`
- `AgentTool::new(definition, executor)`
- `AgentTool::with_prepare_arguments(...)`
- `AgentTool::execute(...)`
- `Agent::new(initial_state)`
- `Agent::with_parts(initial_state, streamer, stream_options)`
- `Agent::state()` / `Agent::update_state(...)`
- `Agent::subscribe(...)` / `Agent::unsubscribe(...)`
- `Agent::set_before_tool_call(...)` / `Agent::clear_before_tool_call()`
- `Agent::set_after_tool_call(...)` / `Agent::clear_after_tool_call()`
- `Agent::prompt_text(...)` / `Agent::prompt_messages(...)` / `Agent::continue()`
- `Agent::abort()` / `Agent::wait_for_idle()`

## 10. Validation plan

Milestone 4 validation target:
- deterministic scripted-stream tests for prompt/continue event order
- state reduction test driven by emitted events
- one faux-provider integration test through `pi-ai` default streaming path
- wrapper tests for listener barriers, `wait_for_idle`, abort, active-run rejection, and synthesized failure messages
- low-level sequential tool execution test with continuation turn
- wrapper-level pending-tool-call tracking test during tool execution
- low-level tests for prepare-arguments, before-hook mutation/blocking, and after-hook overrides
- wrapper-level test proving before/after hook forwarding

## 11. Known risks

- the current Rust slice only supports standard `pi-events::Message` values, not TS custom agent messages
- tool execution is still sequential-only
- JSON-schema validation parity is still missing; current Rust supports `prepare_arguments` but does not yet replicate TS `validateToolArguments` behavior or AJV coercion
- streaming tool progress updates are not implemented yet
- the wrapper has awaited listeners and idle barriers, but does not yet expose TS-equivalent steering/follow-up queue behavior
- the wrapper currently uses explicit methods plus `update_state(...)`, not full TS property-style mutable state parity

## 12. Recommended follow-up after this milestone

Next increment should add steering/follow-up queues on top of the current sequential tool loop, or add argument validation/tool-progress events if staying focused on tool parity first.
