---
## crates/pi-ai/src/anthropic_messages.rs
**Purpose:** Implements Anthropic request shaping, SSE decoding, and the Anthropic provider stream adapter.
**LOC:** 1882

### Dead code (high-confidence only)
- None.

### DRY
- `convert_user_message_content` and `convert_tool_result_content` duplicate the same text/image-to-`AnthropicContentBlock` shaping; only the image-eligibility policy differs.
- `stream_anthropic_http` repeats the same abort-aware HTTP send/body-read/SSE-decode/terminal-yield scaffold also present in `openai_completions.rs`, `openai_responses.rs`, and `openai_codex_responses.rs`.
- `transform_messages_for_anthropic` repeats the same normalization plus orphaned-tool-result repair pass that also exists in `openai_completions.rs` and `openai_responses.rs`.

### Idiomatic Rust (medium/high severity only)
- `convert_anthropic_messages` is still a 126-line dispatcher that mixes user shaping, assistant shaping, tool-result draining, and cache attachment. It naturally wants per-role helpers.
```rust
// Before
match &transformed_messages[index] {
    Message::User { content, .. } => { /* user shaping */ }
    Message::Assistant { content, .. } => { /* assistant shaping */ }
    Message::ToolResult { .. } => { /* drain consecutive tool results */ }
}

// After
match &transformed_messages[index] {
    Message::User { content, .. } => push_user_message(&mut params, content, model),
    Message::Assistant { content, .. } => push_assistant_message(&mut params, content, is_oauth_token),
    Message::ToolResult { .. } => drain_tool_results(&transformed_messages, &mut index, &mut params),
}
```
- `usize_field` still casts `u64` to `usize` with `as`, which can silently truncate malformed provider indices on 32-bit targets.
```rust
// Before
fn usize_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<usize> {
    object.get(key)?.as_u64().map(|value| value as usize)
}

// After
fn usize_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<usize> {
    object
        .get(key)?
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
}
```

### Module structure / visibility
- Request DTOs, request construction, SSE transport, stream-state reduction, and provider registration still live in one 1.8k-line module. Split into internal `request`, `sse`, and `provider` submodules.
- No high-confidence visibility reductions without auditing external consumers of the public Anthropic wire types.

### Public API concerns (top-level visibility)
- `AnthropicOptions`, `AnthropicRequestParams`, `AnthropicMessageParam`, `AnthropicContentBlock`, and `AnthropicStreamEnvelope` expose Anthropic wire/schema details directly from the crate root module. That makes future request-shape changes or Anthropic API-version pivots semver-sensitive instead of internal refactors.

### Low-severity summary
Approx. 25 lower-severity clippy items remain: mostly derives/`#[must_use]` and small control-flow cleanups (clippy.txt lines 128-142, 146-151, 153-156).

---
## crates/pi-ai/src/bin/pi-ai-catalog.rs
**Purpose:** Provides a small CLI for validating, formatting, and summarizing the Rust-owned model catalog JSON.
**LOC:** 335

### Dead code (high-confidence only)
- None.

### DRY
- `check_catalog`, `format_catalog`, and `print_summary` repeat the same read/parse/validate pipeline before doing their command-specific work.

### Idiomatic Rust (medium/high severity only)
- Reviewed, clean.

### Module structure / visibility
- Current structure is fine for a small binary. If the command set grows, move catalog loading/validation helpers behind one internal module rather than expanding the top-level function list.

### Public API concerns (top-level visibility)
- None.

### Low-severity summary
1 lower-severity clippy item remains: `parse_args` can take `&[String]` instead of owning `Vec<String>` (clippy.txt line 362).

---
## crates/pi-ai/src/lib.rs
**Purpose:** Defines the crate's public streaming surface, provider registry, option mapping, and faux provider harness.
**LOC:** 857

### Dead code (high-confidence only)
- None.

### DRY
- `complete` and `complete_simple` still duplicate the same terminal-event folding logic.
- The `FauxProvider::stream` text and thinking branches duplicate the same chunk/sleep/abort/yield loop with only the event constructors differing.

### Idiomatic Rust (medium/high severity only)
- `FauxProvider::stream` is still over 100 lines and mixes queue management, abort handling, per-block emission, and final message synthesis. Split per-block emitters out so the provider loop only sequences state transitions.
```rust
// Before
for (index, block) in response.content.iter().enumerate() {
    match block {
        FauxContentBlock::Text(text) => { /* emit text chunks */ }
        FauxContentBlock::Thinking(thinking) => { /* emit thinking chunks */ }
        FauxContentBlock::ToolCall { .. } => { /* emit tool call */ }
    }
}

// After
for (index, block) in response.content.iter().enumerate() {
    emit_faux_block(
        &mut partial,
        index,
        block,
        &context,
        &state,
        &options,
    ).await?;
}
```

### Module structure / visibility
- `lib.rs` is still doing too much: public API types, provider registration, option mapping, faux-provider support, token estimation, and utility helpers all live together. Move the faux harness into an internal module and keep `lib.rs` focused on the stable surface.
- No safe visibility-tightening recommendation without reviewing external users of the public faux harness.

### Public API concerns (top-level visibility)
- `StreamOptions` and `SimpleStreamOptions` are public catch-all bags carrying provider-specific knobs (`reasoning_summary`, `text_verbosity`, `tool_choice`, `service_tier`, etc.). That makes every provider-specific option expansion a crate-wide semver surface instead of a provider-local change.
- The public `FauxModelDefinition`, `FauxResponse`, `RegisterFauxProviderOptions`, and `FauxRegistration` types expose test-harness scaffolding from the root API, which makes later internal cleanup or relocation breaking.

### Low-severity summary
Approx. 33 lower-severity clippy items remain: mostly docs/`#[must_use]` and small signature/control-flow cleanups (clippy.txt lines 323-355).

---
## crates/pi-ai/src/main.rs
**Purpose:** Implements the standalone `pi-ai` terminal OAuth login helper.
**LOC:** 203

### Dead code (high-confidence only)
- None.

### DRY
- The interactive terminal OAuth flow here still mirrors the same browser/prompt/auth-file shape in `rust/crates/pi-coding-agent-cli/src/auth.rs`. This is cross-crate duplication; flag it, but any consolidation decision is out of scope for this crate-only review.

### Idiomatic Rust (medium/high severity only)
- Reviewed, clean.

### Module structure / visibility
- The binary is appropriately thin. Keep it as a wrapper around library OAuth primitives rather than growing more flow logic here.

### Public API concerns (top-level visibility)
- None.

### Low-severity summary
3 lower-severity clippy items remain: unnecessary `Result`, a `format!` allocation, and an `async` function with no await points (clippy.txt lines 397-399).

---
## crates/pi-ai/src/models.rs
**Purpose:** Loads the embedded model catalog and exposes lookup/cost helpers for built-in models.
**LOC:** 168

### Dead code (high-confidence only)
- None.

### DRY
- Reviewed, clean.

### Idiomatic Rust (medium/high severity only)
- Reviewed, clean.

### Module structure / visibility
- Current structure is appropriate for the file size. No high-confidence visibility changes recommended.

### Public API concerns (top-level visibility)
- None.

### Low-severity summary
12 lower-severity clippy items remain: mostly `#[must_use]` annotations plus precision-loss casts in cost calculation (clippy.txt lines 157-168).

---
## crates/pi-ai/src/oauth.rs
**Purpose:** Implements the OAuth provider registry plus Anthropic/OpenAI Codex login, refresh, callback-server, and token parsing flows.
**LOC:** 1565

### Dead code (high-confidence only)
- None.

### DRY
- `login_anthropic_with_urls` and `login_openai_codex_with_url` still duplicate the same browser-launch, callback/manual race, prompt fallback, and state/code validation flow.
- `refresh_anthropic_token_with_url`, `exchange_anthropic_authorization_code`, `refresh_openai_codex_token_with_url`, and `exchange_openai_codex_authorization_code` still repeat the same HTTP-token-request/status/body/deserialize scaffold.

### Idiomatic Rust (medium/high severity only)
- `login_anthropic_with_urls` is still 108 lines and mixes authorize-URL construction with three different code-acquisition paths. Extract the callback/manual/prompt orchestration into one helper.
```rust
// Before
let mut code = None;
let mut state = None;
let mut redirect_uri_for_exchange = ANTHROPIC_REDIRECT_URI.to_string();
let callback_receiver = server.take_receiver();
if callbacks.on_manual_code_input.is_some() { /* ... */ }

// After
let authorization = await_authorization_code(
    &callbacks,
    &mut server,
    server.take_receiver(),
    &verifier,
    ANTHROPIC_REDIRECT_URI,
).await?;
```
- `start_callback_server` still mixes the listener loop, HTTP parsing, URL construction, handler dispatch, and response writing in one function. Move per-connection handling into a helper so the loop just manages accept/shutdown behavior.
```rust
// Before
match listener.accept() {
    Ok((mut stream, _)) => {
        let target = read_http_request_target(&mut stream)?;
        let url = Url::parse(&format!("http://localhost{target}"))?;
        match handler(&url) { /* ... */ }
    }

// After
match listener.accept() {
    Ok((mut stream, _)) => handle_callback_connection(&mut stream, &handler, &mut result_sender),
    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(CALLBACK_POLL_INTERVAL),
    Err(_) => finish_callback_listener(&mut result_sender),
}
```

### Module structure / visibility
- Registry/types, Anthropic OAuth, OpenAI Codex OAuth, callback-server machinery, and HTML rendering still live in one file. Split provider-specific flows and callback-server helpers into internal submodules.
- No safe visibility reduction recommendation without a broader external API audit.

### Public API concerns (top-level visibility)
- `OAuthProvider` exposes boxed-future signatures (`OAuthCredentialsFuture<'a>`) directly. That bakes allocation-heavy async plumbing into every implementation and makes future trait evolution awkward.
- `OAuthLoginCallbacks` exposes its callback fields publicly, so callers can couple to orchestration internals instead of the builder-style API. That reduces freedom to change callback wiring later.

### Low-severity summary
Approx. 49 lower-severity clippy items remain: docs/`#[must_use]`/lifetime polish plus small string and control-flow cleanups (clippy.txt lines 169-207, 209-216, 356-358).

---
## crates/pi-ai/src/openai_codex_responses.rs
**Purpose:** Implements OpenAI Codex Responses request shaping plus HTTP/WebSocket/auto transports and session WebSocket caching.
**LOC:** 1446

### Dead code (high-confidence only)
- None.

### DRY
- The HTTP and WebSocket paths still duplicate the same decode/process/yield-until-terminal loop; only the frame source changes.
- Codex HTTP still carries essentially the same abort-aware SSE transport scaffold as Anthropic, OpenAI Completions, and OpenAI Responses.

### Idiomatic Rust (medium/high severity only)
- `process_codex_events` still requires a `Vec<OpenAiResponsesStreamEnvelope>`, which forces the websocket hot path to allocate `vec![event]` for every frame. Accept an iterator instead.
```rust
// Before
fn process_codex_events(
    state: &mut OpenAiResponsesStreamState,
    events: Vec<OpenAiResponsesStreamEnvelope>,
) -> Vec<AssistantEvent>

// After
fn process_codex_events(
    state: &mut OpenAiResponsesStreamState,
    events: impl IntoIterator<Item = OpenAiResponsesStreamEnvelope>,
) -> Vec<AssistantEvent>
```

### Module structure / visibility
- Request building, HTTP transport, WebSocket transport/cache, auth/header helpers, and provider registration are still interleaved. Split into internal `request`, `transport`, `cache`, and `provider` modules.
- No high-confidence visibility reductions without confirming external users of the public Codex DTOs.

### Public API concerns (top-level visibility)
- `OpenAiCodexResponsesRequestParams`, `OpenAiCodexResponsesTextConfig`, and `OpenAiCodexResponsesToolDefinition` expose Codex wire-format details publicly. That makes internal request-shape changes or transport-specific tweaks semver-sensitive.

### Low-severity summary
25 lower-severity clippy items remain: mostly docs, small clone/control-flow polish, and helper signature cleanup (clippy.txt lines 217-240).

---
## crates/pi-ai/src/openai_completions.rs
**Purpose:** Implements OpenAI-compatible chat-completions request shaping, compatibility detection, SSE decoding, and streaming state management.
**LOC:** 2066

### Dead code (high-confidence only)
- None.

### DRY
- `transform_messages_for_openai_completions` repeats the same normalization plus orphaned-tool-result repair pass that also exists in `anthropic_messages.rs` and `openai_responses.rs`.
- `stream_openai_completions_http_with_headers` still repeats the same abort-aware SSE transport scaffold used by Anthropic, OpenAI Responses, and Codex HTTP.

### Idiomatic Rust (medium/high severity only)
- `convert_openai_completions_messages` is still a 232-line request shaper that mixes system/developer insertion, assistant conversion, tool-result bridging, image fan-out, and provider quirks. Split it into per-role helpers.
```rust
// Before
match message {
    Message::User { content, .. } => { /* user shaping */ }
    Message::Assistant { content, .. } => { /* assistant shaping */ }
    Message::ToolResult { .. } => { /* tool result shaping */ }
}

// After
match message {
    Message::User { content, .. } => push_user_message(&mut params, content, model),
    Message::Assistant { content, .. } => push_assistant_message(&mut params, content, compat),
    Message::ToolResult { .. } => drain_tool_results(&transformed_messages, &mut index, &mut params, model, compat),
}
```
- `transform_messages_for_openai_completions` is still a 173-line pass that combines provider-specific normalization and synthetic tool-result insertion. Split those into distinct passes so each can be reasoned about and tested independently.
```rust
// Before
fn transform_messages_for_openai_completions(model: &Model, messages: &[Message]) -> Vec<Message> {
    let mut tool_call_id_map = BTreeMap::<String, String>::new();
    let mut transformed = Vec::new();
    /* normalize + inject missing tool results */
}

// After
fn transform_messages_for_openai_completions(model: &Model, messages: &[Message]) -> Vec<Message> {
    let normalized = normalize_openai_completions_messages(model, messages);
    inject_missing_tool_results(normalized)
}
```

### Module structure / visibility
- Compatibility detection, request DTOs, message conversion, normalization, SSE decoding, stream-state mutation, and provider registration still live in one file. Split into internal `compat`, `request`, and `stream` modules.
- No high-confidence visibility reductions without reviewing external users of the public completions DTOs.

### Public API concerns (top-level visibility)
- `OpenAiCompletionsCompat` is a broad public boolean bag. Every new provider quirk becomes another field, which makes regrouping that state into enums/sub-structs a future breaking change.
- `OpenAiCompletionsRequestParams` and the related message/tool DTOs expose raw chat-completions wire shape publicly, tying internal request construction to a semver surface.

### Low-severity summary
Approx. 29 lower-severity clippy items remain: derives/`#[must_use]` and smaller clone/style cleanups on the streaming path (clippy.txt lines 241-249, 251-255, 257-271).

---
## crates/pi-ai/src/openai_responses.rs
**Purpose:** Implements OpenAI Responses request shaping, SSE decoding, and event-to-`AssistantEvent` state reduction.
**LOC:** 1962

### Dead code (high-confidence only)
- None.

### DRY
- `transform_messages_for_openai_responses` still repeats the same normalization plus orphaned-tool-result repair shape found in `anthropic_messages.rs` and `openai_completions.rs`.
- `stream_openai_responses_http_with_runtime_options` still duplicates the same abort-aware SSE transport scaffold used by Anthropic, OpenAI Completions, and Codex HTTP.

### Idiomatic Rust (medium/high severity only)
- `convert_openai_responses_messages` is still a 157-line converter that mixes system-prompt insertion, assistant item mapping, tool-call normalization, and tool-result image serialization. Split it into per-role helpers so the per-provider quirks stay isolated.
```rust
// Before
match message {
    Message::User { content, .. } => { /* user item shaping */ }
    Message::Assistant { content, .. } => { /* assistant item shaping */ }
    Message::ToolResult { .. } => { /* tool result shaping */ }
}

// After
match message {
    Message::User { content, .. } => push_user_items(&mut items, content, model),
    Message::Assistant { content, .. } => push_assistant_items(&mut items, content, model, message_index),
    Message::ToolResult { .. } => push_tool_result_item(&mut items, message, model),
}
```

### Module structure / visibility
- Request DTOs, message conversion, ID normalization, SSE decoding, stream-state handlers, and provider registration are still interleaved. Split into internal `request`, `ids`, and `stream` modules.
- No safe visibility reductions without auditing external users of the public Responses DTOs.

### Public API concerns (top-level visibility)
- `OpenAiResponsesRequestParams`, `ResponsesInputItem`, `ResponsesContentPart`, `ResponsesFunctionCallOutput`, `ResponsesToolDefinition`, and `OpenAiResponsesStreamEnvelope` expose raw Responses wire schema publicly. That limits freedom to change internal request shaping or event normalization later.

### Low-severity summary
Approx. 42 lower-severity clippy items remain: derives/docs plus smaller clone/control-flow/literal cleanups (clippy.txt lines 118-126, 272-287, 289-305).

---
## crates/pi-ai/src/overflow.rs
**Purpose:** Detects context-window overflows from provider error text and token counts.
**LOC:** 80

### Dead code (high-confidence only)
- None.

### DRY
- Reviewed, clean.

### Idiomatic Rust (medium/high severity only)
- Reviewed, clean.

### Module structure / visibility
- Reviewed, clean.

### Public API concerns (top-level visibility)
- None.

### Low-severity summary
2 lower-severity clippy items remain: a `#[must_use]` annotation and panic docs for regex compilation (clippy.txt lines 306-307).

---
## crates/pi-ai/src/partial_json.rs
**Purpose:** Parses truncated JSON fragments into best-effort `serde_json::Value` maps for streaming tool-call arguments.
**LOC:** 314

### Dead code (high-confidence only)
- None.

### DRY
- `parse_object` and `parse_array` duplicate the same partial-container recovery loop with only delimiters and insertion targets changed.
- This parser still overlaps heavily with `rust/crates/pi-agent/src/partial_json.rs`; that cross-crate duplication is real, but any shared extraction decision is out of scope for this crate-only review.

### Idiomatic Rust (medium/high severity only)
- `normalize_number` still round-trips numbers through `f64`, then casts back to integers. That is a lossy path inside a parser whose job is to preserve partial tool-call arguments as faithfully as possible.
```rust
// Before
if let Ok(Value::Number(number)) = serde_json::from_str::<Value>(candidate) {
    return Some(Self::normalize_number(number));
}

// After
if let Ok(integer) = candidate.parse::<i64>() {
    return Some(Number::from(integer));
}
if let Ok(integer) = candidate.parse::<u64>() {
    return Some(Number::from(integer));
}
```

### Module structure / visibility
- Structure is fine if this stays crate-local. If the duplicated parser continues to exist in two crates, at least document one shared behavior contract so fixes do not diverge silently.

### Public API concerns (top-level visibility)
- None.

### Low-severity summary
7 lower-severity clippy items remain: private-module visibility plus `let...else` / duplicate-arm cleanup (clippy.txt lines 308-314).

---
## crates/pi-ai/src/unicode.rs
**Purpose:** Documents and centralizes the current no-op Unicode sanitization hook used by provider request builders.
**LOC:** 32

### Dead code (high-confidence only)
- None.

### DRY
- Reviewed, clean.

### Idiomatic Rust (medium/high severity only)
- Reviewed, clean.

### Module structure / visibility
- Reviewed, clean.

### Public API concerns (top-level visibility)
- None.

### Low-severity summary
1 lower-severity clippy item remains: a private-module visibility nit (clippy.txt line 322).
