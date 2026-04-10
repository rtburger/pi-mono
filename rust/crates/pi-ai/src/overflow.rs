use pi_events::{AssistantMessage, StopReason};
use regex::Regex;
use std::sync::OnceLock;

pub fn is_context_overflow(message: &AssistantMessage, context_window: Option<u64>) -> bool {
    if message.stop_reason == StopReason::Error
        && let Some(error_message) = message.error_message.as_deref()
    {
        let is_non_overflow = non_overflow_patterns()
            .iter()
            .any(|pattern| pattern.is_match(error_message));
        if !is_non_overflow
            && overflow_patterns()
                .iter()
                .any(|pattern| pattern.is_match(error_message))
        {
            return true;
        }
    }

    if let Some(context_window) = context_window
        && message.stop_reason == StopReason::Stop
    {
        let input_tokens = message.usage.input.saturating_add(message.usage.cache_read);
        if input_tokens > context_window {
            return true;
        }
    }

    false
}

pub fn overflow_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            [
                r"prompt is too long",
                r"request_too_large",
                r"input is too long for requested model",
                r"exceeds the context window",
                r"input token count.*exceeds the maximum",
                r"maximum prompt length is \d+",
                r"reduce the length of the messages",
                r"maximum context length is \d+ tokens",
                r"exceeds the limit of \d+",
                r"exceeds the available context size",
                r"greater than the context length",
                r"context window exceeds limit",
                r"exceeded model token limit",
                r"too large for model with \d+ maximum context length",
                r"model_context_window_exceeded",
                r"prompt too long; exceeded (?:max )?context length",
                r"context[_ ]length[_ ]exceeded",
                r"too many tokens",
                r"token limit exceeded",
                r"^4(?:00|13)\s*(?:status code)?\s*\(no body\)",
            ]
            .into_iter()
            .map(|pattern| Regex::new(&format!("(?i){pattern}")).unwrap())
            .collect()
        })
        .as_slice()
}

fn non_overflow_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            [
                r"^(Throttling error|Service unavailable):",
                r"rate limit",
                r"too many requests",
            ]
            .into_iter()
            .map(|pattern| Regex::new(&format!("(?i){pattern}")).unwrap())
            .collect()
        })
        .as_slice()
}
