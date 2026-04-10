use pi_ai::is_context_overflow;
use pi_events::{AssistantMessage, StopReason, Usage};

fn error_message(error_message: &str) -> AssistantMessage {
    AssistantMessage {
        role: "assistant".into(),
        content: Vec::new(),
        api: "openai-completions".into(),
        provider: "ollama".into(),
        model: "qwen3.5:35b".into(),
        response_id: None,
        usage: Usage::default(),
        stop_reason: StopReason::Error,
        error_message: Some(error_message.into()),
        timestamp: 0,
    }
}

#[test]
fn detects_explicit_ollama_prompt_too_long_errors() {
    let message =
        error_message("400 `prompt too long; exceeded max context length by 100918 tokens`");
    assert!(is_context_overflow(&message, Some(32_768)));
}

#[test]
fn does_not_treat_generic_non_overflow_ollama_errors_as_overflow() {
    let message = error_message("500 `model runner crashed unexpectedly`");
    assert!(!is_context_overflow(&message, Some(32_768)));
}

#[test]
fn does_not_treat_bedrock_throttling_too_many_tokens_as_overflow() {
    let message =
        error_message("Throttling error: Too many tokens, please wait before trying again.");
    assert!(!is_context_overflow(&message, Some(200_000)));
}

#[test]
fn does_not_treat_bedrock_service_unavailable_as_overflow() {
    let message = error_message("Service unavailable: The service is temporarily unavailable.");
    assert!(!is_context_overflow(&message, Some(200_000)));
}

#[test]
fn does_not_treat_generic_rate_limit_errors_as_overflow() {
    let message = error_message("Rate limit exceeded, please retry after 30 seconds.");
    assert!(!is_context_overflow(&message, Some(200_000)));
}

#[test]
fn does_not_treat_http_429_style_errors_as_overflow() {
    let message = error_message("Too many requests. Please slow down.");
    assert!(!is_context_overflow(&message, Some(200_000)));
}

#[test]
fn detects_openai_context_window_errors() {
    let message = error_message("Your input exceeds the context window of this model.");
    assert!(is_context_overflow(&message, Some(128_000)));
}

#[test]
fn detects_silent_overflow_from_usage_when_input_exceeds_context_window() {
    let mut message = error_message("not used");
    message.stop_reason = StopReason::Stop;
    message.error_message = None;
    message.usage.input = 110_000;
    message.usage.cache_read = 5_000;

    assert!(is_context_overflow(&message, Some(100_000)));
}

#[test]
fn does_not_detect_silent_overflow_without_context_window() {
    let mut message = error_message("not used");
    message.stop_reason = StopReason::Stop;
    message.error_message = None;
    message.usage.input = 110_000;
    message.usage.cache_read = 5_000;

    assert!(!is_context_overflow(&message, None));
}
