# Wave 2b deferred follow-ups

## `push_assistant_message`
- File: `src/openai_completions.rs`
- Status: extracted in Wave 2b with `#[allow(clippy::too_many_lines)]`
- Follow-up: split into smaller private helpers for:
  - assistant text shaping
  - assistant thinking shaping
  - assistant tool call shaping
- Tracking note: the current allow is intentional for Wave 2b mechanical extraction only and should be removed in a future Wave 3+ cleanup.
