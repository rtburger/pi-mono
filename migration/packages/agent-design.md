# packages/agent Rust target design

Date: 2026-04-14
Status: Step 2 complete

## Crate name

- Target Rust crate: `pi-agent`

## Scope for this migration phase

In scope:
- stateful agent wrapper
- low-level agent loop
- tool registry and execution flow
- steering and follow-up queues
- abort / retry / continue behavior
- proxy stream helper
- transcript/state serialization where needed

Out of scope for this phase:
- rewriting `pi-ai` provider internals
- TUI rendering
- coding-agent CLI/session orchestration

## Module layout

Existing Rust modules already line up with the intended design:
- `agent`
- `loop`
- `message`
- `state`
- `tool`
- `proxy`
- `validation`
- `partial_json`
- `error`

## Public API target

Keep the Rust surface centered on the TS compatibility contract:
- `Agent`
- `QueueMode`
- `AgentError`
- `agent_loop(...)`
- `agent_loop_continue(...)`
- `stream_proxy(...)`
- `AgentState`
- `AgentMessage`
- `AgentContext`
- `AgentEvent`
- `AgentTool`
- `AgentToolResult`
- `ToolExecutionMode`
- `BeforeToolCallContext` / `BeforeToolCallResult`
- `AfterToolCallContext` / `AfterToolCallResult`
- `StreamFn`

## Key types / enums / traits

- `Agent`
  - owns transcript state and active run lifecycle
- `AgentState`
  - public state snapshot with runtime-owned fields
- `AgentEvent`
  - lifecycle, message, and tool execution events
- `ToolExecutionMode`
  - sequential or parallel tool execution
- `StreamFn`
  - pluggable model-stream function compatible with `pi-ai`
- `AgentTool`
  - typed tool interface with schema validation and execution
- `BeforeToolCallResult`
  - preflight block signal
- `AfterToolCallResult`
  - post-execution override signal
- `AgentError`
  - structured error enum for invalid state and runtime failures

## Dependencies

Runtime:
- `pi-ai`
- `pi-events`
- `async-stream`
- `futures`
- `tokio`
- `serde_json`
- `thiserror`

Internal support:
- partial JSON parsing for streamed tool arguments
- tool argument validation helpers
- proxy stream helper

## Compatibility goals

1. Preserve observable event ordering.
2. Preserve transcript/state semantics, especially copy-on-assign for top-level arrays.
3. Preserve active-run barriers:
   - listeners receive the active abort signal
   - `agent_end` listeners are awaited before idle settlement
4. Preserve queue semantics:
   - steering messages inject after the current assistant tool batch
   - follow-up messages inject only when the agent would otherwise stop
5. Preserve tool semantics:
   - prepare arguments before validation when available
   - validate before execution
   - allow preflight blocking and post-execution mutation
   - keep parallel execution order stable in emitted results
6. Preserve `continue()` edge cases from the TS implementation.

## Known risks

- TS declaration merging does not map 1:1 to Rust.
- The Rust type system should replace `any`/`unknown` with explicit enums or generic payloads, but some compatibility helpers may still need boxed values.
- Proxy reconstruction logic must stay in lockstep with `pi-ai` event shapes.
- Parallel tool execution can introduce subtle ordering bugs if the Rust state machine is simplified too aggressively.

## Validation plan against the TS version

1. Use the TS `agent.test.ts` and `agent-loop.test.ts` as behavior specifications.
2. Keep the existing Rust `pi-agent` tests green while adding missing parity cases.
3. Add fixture-driven tests for:
   - queued steering after tool batches
   - follow-up injection after stop
   - async listener barriers
   - tool preflight blocking and postflight mutation
   - continue-from-assistant-tail behavior
4. Compare Rust event sequences against TS-derived expectations before expanding to higher-level coding-agent integration.

## Current implementation note

The Rust workspace already has a non-empty `pi-agent` crate with the intended module split. This target design should be read as a parity and refinement plan, not a proposal for a new crate layout.

## Validation findings

- Confirmed two TS-style falsy fallback behaviors and ported them into Rust:
  - an empty-string API key from `getApiKey` now falls back to `stream_options.api_key`
  - an empty blocked-tool reason now falls back to `"Tool execution was blocked"`
- Regression tests now cover both cases in `pi-agent`.
- High-level `Agent::prompt()` / `Agent::continue()` error strings now match the TS wrapper behavior:
  - prompt while active uses the queueing guidance message
  - empty `continue()` uses `No messages to continue from`
  - low-level `agent_loop_continue()` keeps the TS low-level `Cannot continue: no messages in context` message
- `pendingToolCalls` iteration now preserves insertion order via `IndexSet`, matching TS `Set` iteration order when listeners snapshot pending tool IDs.
- `Agent::prompt_text_with_images()` now mirrors the TS string-plus-images prompt shape by preserving the text block first and then appending image content blocks in order.

## Recommended next step

Validate the existing Rust `pi-agent` crate against the TS edge cases that are easiest to freeze into fixtures first:
- queued steering after tool batches
- `continue()` from assistant tails with queued messages
- async listener barrier timing
- parallel tool ordering
