use pi_ai::anthropic_messages::{
    AnthropicOptions, build_anthropic_request_params, convert_anthropic_messages,
};
use pi_ai::openai_codex_responses::build_openai_codex_responses_request_params;
use pi_ai::openai_completions::{OpenAiCompletionsCompat, convert_openai_completions_messages};
use pi_ai::openai_responses::{
    OpenAiResponsesConvertOptions, ResponsesInputItem, convert_openai_responses_messages,
};
use pi_events::{Context, Message, Model, StopReason, Usage, UserContent};

fn anthropic_model() -> Model {
    Model {
        id: "claude-sonnet-4-5".into(),
        name: "claude-sonnet-4-5".into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url: "https://api.anthropic.com/v1".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 200_000,
        max_tokens: 8_192,
    }
}

fn openai_responses_model() -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn openai_completions_model() -> Model {
    Model {
        id: "gpt-4o-mini".into(),
        name: "gpt-4o-mini".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: false,
        input: vec!["text".into(), "image".into()],
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

fn openai_codex_model() -> Model {
    Model {
        id: "gpt-5.2-codex".into(),
        name: "gpt-5.2-codex".into(),
        api: "openai-codex-responses".into(),
        provider: "openai-codex".into(),
        base_url: "https://chatgpt.com/backend-api".into(),
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        context_window: 272_000,
        max_tokens: 128_000,
    }
}

fn empty_user_message(timestamp: u64) -> Message {
    Message::User {
        content: vec![],
        timestamp,
    }
}

fn text_user_message(text: &str, timestamp: u64) -> Message {
    Message::User {
        content: vec![UserContent::Text { text: text.into() }],
        timestamp,
    }
}

fn empty_assistant_message(api: &str, provider: &str, model: &str, timestamp: u64) -> Message {
    Message::Assistant {
        content: vec![],
        api: api.into(),
        provider: provider.into(),
        model: model.into(),
        response_id: None,
        usage: Usage::default(),
        stop_reason: StopReason::Stop,
        error_message: None,
        timestamp,
    }
}

fn user_roles(items: &[ResponsesInputItem]) -> Vec<&str> {
    items
        .iter()
        .map(|item| match item {
            ResponsesInputItem::Message { role, .. } => role.as_str(),
            other => panic!("expected only message items, got {other:?}"),
        })
        .collect()
}

#[test]
fn anthropic_empty_user_message_does_not_insert_synthetic_user_block() {
    let params = build_anthropic_request_params(
        &anthropic_model(),
        &Context {
            system_prompt: None,
            messages: vec![empty_user_message(1)],
            tools: vec![],
        },
        false,
        &AnthropicOptions::default(),
    );

    assert!(params.messages.is_empty());
}

#[test]
fn anthropic_empty_assistant_message_is_skipped_during_replay() {
    let messages = convert_anthropic_messages(
        &[
            text_user_message("Hello, how are you?", 1),
            empty_assistant_message("anthropic-messages", "anthropic", "claude-sonnet-4-5", 2),
            text_user_message("Please respond this time.", 3),
        ],
        &anthropic_model(),
        false,
        None,
    );

    let roles = messages
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["user", "user"]);
}

#[test]
fn openai_responses_empty_user_message_is_skipped() {
    let items = convert_openai_responses_messages(
        &openai_responses_model(),
        &Context {
            system_prompt: None,
            messages: vec![empty_user_message(1)],
            tools: vec![],
        },
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    assert!(items.is_empty());
}

#[test]
fn openai_responses_empty_assistant_message_is_skipped_during_replay() {
    let items = convert_openai_responses_messages(
        &openai_responses_model(),
        &Context {
            system_prompt: None,
            messages: vec![
                text_user_message("Hello, how are you?", 1),
                empty_assistant_message("openai-responses", "openai", "gpt-5-mini", 2),
                text_user_message("Please respond this time.", 3),
            ],
            tools: vec![],
        },
        &["openai", "openai-codex"],
        OpenAiResponsesConvertOptions {
            include_system_prompt: false,
        },
    );

    assert_eq!(user_roles(&items), vec!["user", "user"]);
}

#[test]
fn openai_completions_empty_user_message_is_skipped() {
    let messages = convert_openai_completions_messages(
        &openai_completions_model(),
        &Context {
            system_prompt: None,
            messages: vec![empty_user_message(1)],
            tools: vec![],
        },
        &OpenAiCompletionsCompat::default(),
    );

    assert!(messages.is_empty());
}

#[test]
fn openai_completions_empty_assistant_message_is_skipped_during_replay() {
    let messages = convert_openai_completions_messages(
        &openai_completions_model(),
        &Context {
            system_prompt: None,
            messages: vec![
                text_user_message("Hello, how are you?", 1),
                empty_assistant_message("openai-completions", "openai", "gpt-4o-mini", 2),
                text_user_message("Please respond this time.", 3),
            ],
            tools: vec![],
        },
        &OpenAiCompletionsCompat::default(),
    );

    let roles = messages
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["user", "user"]);
}

#[test]
fn openai_codex_empty_user_message_is_skipped() {
    let params = build_openai_codex_responses_request_params(
        &openai_codex_model(),
        &Context {
            system_prompt: None,
            messages: vec![empty_user_message(1)],
            tools: vec![],
        },
        &Default::default(),
    );

    assert!(params.input.is_empty());
    assert_eq!(params.instructions, None);
}

#[test]
fn openai_codex_empty_assistant_message_is_skipped_during_replay() {
    let params = build_openai_codex_responses_request_params(
        &openai_codex_model(),
        &Context {
            system_prompt: None,
            messages: vec![
                text_user_message("Hello, how are you?", 1),
                empty_assistant_message("openai-responses", "openai-codex", "gpt-5.2-codex", 2),
                text_user_message("Please respond this time.", 3),
            ],
            tools: vec![],
        },
        &Default::default(),
    );

    assert_eq!(user_roles(&params.input), vec!["user", "user"]);
}
