use pi_ai::openai_completions::{
    OpenAiCompletionsCompat, OpenAiCompletionsMessageContent, convert_openai_completions_messages,
    normalize_openai_completions_tool_call_id,
};
use pi_events::{AssistantContent, Context, Message, Model, StopReason, Usage, UserContent};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Debug, Deserialize)]
struct Fixture {
    raw_id: String,
    normalized_call_id: String,
}

fn model() -> Model {
    Model {
        id: "gpt-4o-mini".into(),
        name: "gpt-4o-mini".into(),
        api: "openai-completions".into(),
        provider: "openai".into(),
        base_url: "https://api.example.test/v1".into(),
        reasoning: false,
        input: vec!["text".into()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 128_000,
        max_tokens: 16_384,
        compat: None,
    }
}

fn usage() -> Usage {
    Usage::default()
}

fn fixture() -> Fixture {
    serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("openai_completions_foreign_pipe_tool_call.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn normalizes_foreign_pipe_separated_tool_call_ids_for_openai_completions() {
    let fixture = fixture();
    let context = Context {
        system_prompt: None,
        messages: vec![
            Message::Assistant {
                content: vec![AssistantContent::ToolCall {
                    id: fixture.raw_id.clone(),
                    name: "echo".into(),
                    arguments: BTreeMap::from([("message".into(), json!("hello"))]),
                    thought_signature: None,
                }],
                api: "openai-codex-responses".into(),
                provider: "openai-codex".into(),
                model: "gpt-5.2-codex".into(),
                response_id: None,
                usage: usage(),
                stop_reason: StopReason::ToolUse,
                error_message: None,
                timestamp: 1,
            },
            Message::User {
                content: vec![UserContent::Text {
                    text: "Say hi".into(),
                }],
                timestamp: 2,
            },
        ],
        tools: vec![],
    };

    let messages = convert_openai_completions_messages(
        &model(),
        &context,
        &OpenAiCompletionsCompat::default(),
    );

    assert_eq!(messages.len(), 3);
    assert_eq!(
        normalize_openai_completions_tool_call_id(&model(), &fixture.raw_id),
        fixture.normalized_call_id
    );

    match &messages[0].tool_calls {
        Some(tool_calls) => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].id, fixture.normalized_call_id);
            assert_eq!(tool_calls[0].function.name, "echo");
            assert_eq!(tool_calls[0].function.arguments, r#"{"message":"hello"}"#);
        }
        other => panic!("expected assistant tool_calls, got {other:?}"),
    }

    assert_eq!(messages[1].role, "tool");
    assert_eq!(
        messages[1].tool_call_id.as_deref(),
        Some(fixture.normalized_call_id.as_str())
    );
    match &messages[1].content {
        OpenAiCompletionsMessageContent::Text(text) => {
            assert_eq!(text, "No result provided");
        }
        other => panic!("expected synthetic tool result text, got {other:?}"),
    }

    assert_eq!(messages[2].role, "user");
}
