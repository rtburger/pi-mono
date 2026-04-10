use httpmock::prelude::*;
use pi_ai::openai_completions::{
    OpenAiCompletionsFunctionChoice, OpenAiCompletionsToolChoice,
    OpenAiCompletionsToolChoiceFunction,
};
use pi_ai::{
    FauxResponse, RegisterFauxProviderOptions, SimpleStreamOptions, ThinkingLevel, complete_simple,
    register_faux_provider,
};
use pi_events::{AssistantContent, Context, Message, Model, ToolDefinition, UserContent};
use serde_json::json;

fn base_context() -> Context {
    Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::User {
            content: vec![UserContent::Text { text: "hi".into() }],
            timestamp: 1,
        }],
        tools: vec![],
    }
}

fn context_with_tool() -> Context {
    Context {
        tools: vec![ToolDefinition {
            name: "calculator".into(),
            description: "Calculate a result".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string" }
                },
                "required": ["expression"]
            }),
        }],
        ..base_context()
    }
}

fn openai_responses_model(base_url: String, max_tokens: u64) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens,
    }
}

fn openai_completions_model(base_url: String, max_tokens: u64) -> Model {
    Model {
        id: "gpt-5-mini".into(),
        name: "gpt-5-mini".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        context_window: 128_000,
        max_tokens,
    }
}

fn anthropic_model(base_url: String, max_tokens: u64) -> Model {
    Model {
        id: "claude-sonnet-4-20250514".into(),
        name: "claude-sonnet-4-20250514".into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url,
        reasoning: true,
        input: vec!["text".into()],
        context_window: 200_000,
        max_tokens,
    }
}

#[tokio::test]
async fn simple_openai_responses_clamps_xhigh_and_defaults_max_output_tokens() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"in_progress\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/responses")
            .header("authorization", "Bearer test-key")
            .body_contains("\"max_output_tokens\":32000")
            .body_contains("\"effort\":\"high\"");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        openai_responses_model(server.base_url(), 64_000),
        base_context(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            reasoning: Some(ThinkingLevel::Xhigh),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("resp_1"));
}

#[tokio::test]
async fn simple_openai_completions_passes_tool_choice_into_request_body() {
    let server = MockServer::start();
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .header("authorization", "Bearer test-key")
            .body_contains("\"max_completion_tokens\":16384")
            .body_contains(
                "\"tool_choice\":{\"type\":\"function\",\"function\":{\"name\":\"calculator\"}}",
            )
            .body_contains("\"tools\":[{");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        openai_completions_model(server.base_url(), 16_384),
        context_with_tool(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            tool_choice: Some(OpenAiCompletionsToolChoice::Function(
                OpenAiCompletionsFunctionChoice {
                    choice_type: "function".into(),
                    function: OpenAiCompletionsToolChoiceFunction {
                        name: "calculator".into(),
                    },
                },
            )),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("chatcmpl-1"));
}

#[tokio::test]
async fn simple_anthropic_adjusts_max_tokens_for_non_adaptive_thinking() {
    let server = MockServer::start();
    let sse = concat!(
        "event: message_start\n",
        "data: {\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\n",
        "data: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n"
    );

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/messages")
            .header("x-api-key", "test-key")
            .body_contains("\"max_tokens\":40000")
            .body_contains("\"thinking\":{\"type\":\"enabled\",\"budget_tokens\":16384}");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(sse);
    });

    let response = complete_simple(
        anthropic_model(server.base_url(), 40_000),
        base_context(),
        SimpleStreamOptions {
            api_key: Some("test-key".into()),
            reasoning: Some(ThinkingLevel::High),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(response.response_id.as_deref(), Some("msg_1"));
}

#[tokio::test]
async fn complete_simple_uses_registered_provider_dispatch() {
    let registration = register_faux_provider(RegisterFauxProviderOptions::default());
    registration.set_responses(vec![FauxResponse::text("Hello from faux")]);
    let model = registration.get_model(None).expect("faux model");

    let response = complete_simple(model, base_context(), SimpleStreamOptions::default())
        .await
        .unwrap();

    assert!(matches!(
        response.content.as_slice(),
        [AssistantContent::Text { text, .. }] if text == "Hello from faux"
    ));

    registration.unregister();
}
