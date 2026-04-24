use parking_lot::Mutex;
use pi_ai::{
    FauxContentBlock, FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions,
    StreamOptions, register_faux_provider,
};
use pi_coding_agent_cli::{PrintModeOptions, PrintOutputMode, run_print_mode};
use pi_coding_agent_core::{
    AgentSession, AgentSessionOptions, CodingAgentCoreOptions, MemoryAuthStorage, RetrySettings,
    SessionBootstrapOptions, SessionManager, create_agent_session,
};
use pi_events::{StopReason, UserContent};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-coding-agent-cli-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn create_session(
    provider: &str,
    model_id: &str,
    responses: Vec<FauxResponse>,
    cwd: Option<PathBuf>,
) -> (AgentSession, pi_ai::FauxRegistration) {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: provider.into(),
        models: vec![FauxModelDefinition {
            id: model_id.into(),
            name: Some(model_id.into()),
            reasoning: true,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(responses);
    let model = faux.get_model(Some(model_id)).unwrap();
    let auth = Arc::new(MemoryAuthStorage::with_api_keys([(
        model.provider.clone(),
        "test-token",
    )]));

    let cwd = cwd.unwrap_or_else(|| unique_temp_dir("print-mode-session"));
    let session_manager = Arc::new(Mutex::new(SessionManager::in_memory(
        cwd.to_string_lossy().as_ref(),
    )));
    let created = create_agent_session(AgentSessionOptions {
        core: CodingAgentCoreOptions {
            auth_source: auth,
            built_in_models: vec![model],
            models_json_path: None,
            cwd: Some(cwd),
            tools: None,
            system_prompt: String::new(),
            bootstrap: SessionBootstrapOptions::default(),
            stream_options: StreamOptions::default(),
        },
        session_manager: Some(session_manager),
    })
    .unwrap();

    (created.session, faux)
}

#[tokio::test]
async fn print_mode_writes_final_text_blocks_with_newlines() {
    let (session, faux) = create_session(
        "print-mode-faux",
        "print-mode-faux-1",
        vec![FauxResponse {
            content: vec![
                FauxContentBlock::Text(String::from("hello")),
                FauxContentBlock::Text(String::from("world")),
            ],
            stop_reason: StopReason::Stop,
            error_message: None,
        }],
        None,
    );

    let result = run_print_mode(
        &session,
        PrintModeOptions {
            mode: PrintOutputMode::Text,
            initial_message: Some(String::from("Say hello")),
            ..PrintModeOptions::default()
        },
    )
    .await;

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "hello\nworld\n");
    assert!(result.stderr.is_empty());

    faux.unregister();
}

#[tokio::test]
async fn print_mode_serializes_agent_events_in_json_mode() {
    let temp_dir = unique_temp_dir("json-mode");
    let (session, faux) = create_session(
        "json-mode-faux",
        "json-mode-faux-1",
        vec![
            FauxResponse {
                content: vec![FauxContentBlock::ToolCall {
                    id: String::from("tool-1"),
                    name: String::from("write"),
                    arguments: BTreeMap::from([
                        (
                            String::from("path"),
                            Value::String(String::from("notes.txt")),
                        ),
                        (
                            String::from("content"),
                            Value::String(String::from("hello")),
                        ),
                    ]),
                }],
                stop_reason: StopReason::ToolUse,
                error_message: None,
            },
            FauxResponse::text("done"),
        ],
        Some(temp_dir.clone()),
    );

    let result = run_print_mode(
        &session,
        PrintModeOptions {
            mode: PrintOutputMode::Json,
            messages: vec![String::from("create file")],
            ..PrintModeOptions::default()
        },
    )
    .await;

    assert_eq!(result.exit_code, 0);
    assert!(result.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(temp_dir.join("notes.txt")).unwrap(),
        "hello"
    );

    let events = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events[0]["type"].as_str(), Some("session"));
    let event_types = events[1..]
        .iter()
        .map(|event| event["type"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(event_types.first().copied(), Some("agent_start"));
    assert!(event_types.contains(&"tool_execution_start"));
    assert!(event_types.contains(&"tool_execution_end"));
    assert_eq!(event_types.last().copied(), Some("agent_end"));

    let message_update = events
        .iter()
        .find(|event| event["type"] == "message_update")
        .unwrap();
    assert!(message_update.get("assistantMessageEvent").is_some());
    assert!(message_update.get("assistantEvent").is_none());

    let tool_end = events
        .iter()
        .find(|event| event["type"] == "tool_execution_end")
        .unwrap();
    assert_eq!(tool_end["toolName"], Value::String(String::from("write")));
    assert_eq!(
        tool_end["result"]["content"],
        serde_json::to_value(vec![UserContent::Text {
            text: String::from("Successfully wrote 5 bytes to notes.txt"),
        }])
        .unwrap()
    );

    faux.unregister();
}

#[tokio::test]
async fn print_mode_returns_non_zero_for_assistant_errors() {
    let (session, faux) = create_session(
        "error-mode-faux",
        "error-mode-faux-1",
        vec![FauxResponse {
            content: Vec::new(),
            stop_reason: StopReason::Error,
            error_message: Some(String::from("provider failure")),
        }],
        None,
    );

    let result = run_print_mode(
        &session,
        PrintModeOptions {
            mode: PrintOutputMode::Text,
            initial_message: Some(String::from("Hi")),
            ..PrintModeOptions::default()
        },
    )
    .await;

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty());
    assert_eq!(result.stderr, "provider failure\n");

    faux.unregister();
}

#[tokio::test]
async fn print_mode_json_includes_retry_events() {
    let (session, faux) = create_session(
        "retry-mode-faux",
        "retry-mode-faux-1",
        vec![
            FauxResponse {
                content: Vec::new(),
                stop_reason: StopReason::Error,
                error_message: Some(String::from("503 overloaded")),
            },
            FauxResponse::text("recovered"),
        ],
        None,
    );
    session.set_retry_settings(RetrySettings {
        enabled: true,
        max_retries: 1,
        base_delay_ms: 1,
        max_retry_delay_ms: None,
    });

    let result = run_print_mode(
        &session,
        PrintModeOptions {
            mode: PrintOutputMode::Json,
            initial_message: Some(String::from("retry once")),
            ..PrintModeOptions::default()
        },
    )
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let events = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let event_types = events
        .iter()
        .map(|event| event["type"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(event_types[0], "session");
    assert!(
        event_types.contains(&"auto_retry_start"),
        "events: {event_types:?}"
    );
    assert!(
        event_types.contains(&"auto_retry_end"),
        "events: {event_types:?}"
    );

    let retry_start = events
        .iter()
        .find(|event| event["type"] == "auto_retry_start")
        .unwrap();
    assert_eq!(retry_start["maxAttempts"].as_u64(), Some(1));
    assert_eq!(retry_start["delayMs"].as_u64(), Some(1));

    let retry_end = events
        .iter()
        .find(|event| event["type"] == "auto_retry_end")
        .unwrap();
    assert_eq!(retry_end["success"].as_bool(), Some(true));

    let message_update = events
        .iter()
        .find(|event| event["type"] == "message_update")
        .unwrap();
    assert!(message_update.get("assistantMessageEvent").is_some());
    assert!(message_update.get("assistantEvent").is_none());

    faux.unregister();
}
