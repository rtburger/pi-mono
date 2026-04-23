# REVIEW_pi-ai summary

## 1. Delta from prior review

### Tooling snapshot
- Current `cargo clippy -p pi-ai --all-targets -- -W clippy::all -W clippy::pedantic -W clippy::nursery` run reports **308 unique `pi-ai` warnings** across crate targets. That is **7 lower** than the prior known baseline of **315**.
- If you include local dependency output from `pi-events`, the combined unique warning count in `clippy.txt` is **317**.
- `cargo machete` is unavailable in this environment (`deps.txt`).
- `cargo doc -p pi-ai --no-deps` produced **no warnings** (`doc_warnings.txt` is empty).

### Prior findings now resolved (waves 1 and 2a)
- The prior dead-code finding is gone: `AdjustedThinkingTokens::thinking_budget` was removed from `src/lib.rs`.
- `AnthropicStreamState::process_event` no longer takes owned envelopes and no longer contains the old giant match body; the reducer is split into per-event handlers and borrows `&AnthropicStreamEnvelope`.
- `OpenAiResponsesStreamState::process_event` likewise now borrows `&OpenAiResponsesStreamEnvelope` and delegates to per-event handlers instead of the earlier monolith.
- Codex WebSocket cleanup is no longer open-coded in each terminal branch; `CodexSocketGuard` now centralizes socket release/keep behavior.
- Callback-server teardown is now RAII-backed: `CallbackServer` implements `Drop`, so cancellation/join happens even on early return paths.
- Hand-written base64 decoding was removed from the Codex path; `openai_codex_responses.rs` now uses the `base64` crate directly.
- The Anthropic SSE object-clone hot-path issue is resolved: `flush_sse_event` now matches `Value::Object` directly.
- The OpenAI Completions response-id hot-path clone is resolved: `process_chunk` now uses `or_else(|| self.output.response_id.take())`.

### Prior findings that still remain
- Abort-aware SSE transport code is still duplicated across Anthropic, OpenAI Responses, OpenAI Completions, and Codex HTTP.
- Message normalization plus orphaned-tool-result repair is still duplicated across Anthropic, OpenAI Responses, and OpenAI Completions.
- OAuth authorization-code acquisition and token-exchange scaffolding are still repeated in `src/oauth.rs`.
- The large request-shaping functions (`convert_anthropic_messages`, `convert_openai_responses_messages`, `convert_openai_completions_messages`) still carry too many concerns.
- The faux provider/test harness still lives in `src/lib.rs`.
- Cross-crate partial JSON duplication with `rust/crates/pi-agent/src/partial_json.rs` still exists.
- Public wire-format DTOs are still exported from provider modules, so API surface remains broad.

### New findings not called out in the prior review
- The public API risk around `StreamOptions` / `SimpleStreamOptions` is clearer now: they are growing provider-agnostic bags for provider-specific options.
- `OAuthLoginCallbacks` exposes callback fields publicly, which leaks orchestration details into the API surface.
- `OAuthProvider` still bakes boxed-future signatures into the public trait.
- `partial_json.rs::normalize_number` still round-trips numbers through `f64`, which is a concrete fidelity risk in a parser.
- `process_codex_events` still forces `vec![event]` allocation on the WebSocket hot path because it takes `Vec<OpenAiResponsesStreamEnvelope>`.

## 2. Cross-cutting patterns still present
- One abort-aware SSE transport scaffold is still copied four times with only endpoint/decoder/state types changing.
- One message-normalization/orphaned-tool-result repair pass is still copied three times with small provider-specific ID rules layered on top.
- The request converters for Anthropic / OpenAI Responses / OpenAI Completions still interleave multiple responsibilities: prompt role rules, tool-call normalization, tool-result shaping, image handling, and provider quirks.
- Public provider wire DTOs still expose raw request/event schema directly from implementation modules.
- OAuth still repeats the same callback/manual/prompt acquisition pattern and token request skeleton inside a single large file.
- Small utility islands remain isolated instead of converging on one pattern: partial JSON recovery, prompt-cache/test harness helpers, response ID hashing, and transport helpers.

## 3. Top 10 remaining refactors, ordered by value
1. **Extract one shared abort-aware SSE transport scaffold**  
   Effort: **L**  
   Files: `src/anthropic_messages.rs`, `src/openai_completions.rs`, `src/openai_responses.rs`, `src/openai_codex_responses.rs`  
   Scope: **pi-ai-internal**
2. **Extract one shared message-normalization + orphaned-tool-result repair pass**  
   Effort: **L**  
   Files: `src/anthropic_messages.rs`, `src/openai_completions.rs`, `src/openai_responses.rs`  
   Scope: **pi-ai-internal**
3. **Deduplicate OAuth authorization-code acquisition and token exchange scaffolding**  
   Effort: **M/L**  
   Files: `src/oauth.rs`  
   Scope: **pi-ai-internal**
4. **Split the three large request converters into per-role helpers**  
   Effort: **M**  
   Files: `src/anthropic_messages.rs`, `src/openai_completions.rs`, `src/openai_responses.rs`  
   Scope: **pi-ai-internal**
5. **Modularize the provider files by concern (`request`, `stream`, `provider`, etc.)**  
   Effort: **M/L**  
   Files: `src/anthropic_messages.rs`, `src/openai_completions.rs`, `src/openai_responses.rs`, `src/openai_codex_responses.rs`, `src/oauth.rs`  
   Scope: **pi-ai-internal**
6. **Move the faux provider harness out of `lib.rs` into an internal module**  
   Effort: **M**  
   Files: `src/lib.rs`  
   Scope: **pi-ai-internal**
7. **Fix lossy numeric normalization in `partial_json.rs`**  
   Effort: **S/M**  
   Files: `src/partial_json.rs`  
   Scope: **pi-ai-internal**
8. **Make Codex event pumping iterator-based and share more HTTP/WebSocket emission logic**  
   Effort: **M**  
   Files: `src/openai_codex_responses.rs`  
   Scope: **pi-ai-internal**
9. **Review and intentionally trim/reshape the public API surface**  
   Effort: **M/L**  
   Files: `src/lib.rs`, `src/oauth.rs`, `src/anthropic_messages.rs`, `src/openai_completions.rs`, `src/openai_responses.rs`, `src/openai_codex_responses.rs`  
   Scope: **pi-ai-internal**, but API-breaking review required
10. **Resolve partial-JSON duplication with `pi-agent`**  
    Effort: **M/L**  
    Files: `rust/crates/pi-ai/src/partial_json.rs`, `rust/crates/pi-agent/src/partial_json.rs`  
    Scope: **cross-crate (defer)**

## 4. Dead code tally
- **0 high-confidence dead items found** in the current `pi-ai` source review.
- The lone prior dead-code finding was resolved by the refactor waves.

## 5. Public API concerns
- **`StreamOptions` / `SimpleStreamOptions` (`src/lib.rs`)**: these are public, provider-agnostic option bags carrying provider-specific fields. That reduces freedom to make provider-specific options typed or scoped later.
- **Public provider wire DTOs (`src/anthropic_messages.rs`, `src/openai_completions.rs`, `src/openai_responses.rs`, `src/openai_codex_responses.rs`)**: request/event structs and enums expose implementation-level wire schema directly. That makes internal request-shape changes breaking.
- **`OpenAiCompletionsCompat` (`src/openai_completions.rs`)**: the public boolean-heavy compatibility bag is already brittle. Regrouping those booleans into stronger types later would be an API change.
- **`OAuthProvider` and `OAuthLoginCallbacks` (`src/oauth.rs`)**: boxed-future trait signatures and public callback fields leak implementation mechanics into the public surface, reducing room to simplify the async callback model.
- **Public faux-provider harness (`src/lib.rs`)**: test-harness types are exported from the main crate surface, which preserves internal scaffolding as API.

## 6. Recommendation
`pi-ai` is **done enough to move on** unless you specifically want another cleanup pass for long-term maintainability. The refactor waves removed the most valuable reducer, RAII, and hot-path issues. What remains is mostly structural consolidation, API-surface cleanup, and duplication reduction rather than correctness blockers.

If more provider work is planned in `pi-ai`, there is still meaningful value in doing items 1-4 above first. If the goal is to shift attention to other crates, the current state is serviceable and materially better than the pre-wave review baseline.
