use pi_coding_agent_cli::{
    RpcClient, RpcClientError, RpcClientOptions, RpcExtensionUiResponse, RpcOutputEvent,
    RpcSessionEvent, RpcThinkingLevel,
};
use pi_events::AssistantEvent;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("rpc-client-fixture.mjs")
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "pi-coding-agent-cli-rpc-client-{prefix}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn build_client(extension_mode: &str, mode: &str) -> RpcClient {
    let provider = String::from("fixture-provider");
    let model = String::from("fixture-model");
    let mut env = BTreeMap::new();
    env.insert(
        String::from("RPC_FIXTURE_EXPECT_PROVIDER"),
        provider.clone(),
    );
    env.insert(String::from("RPC_FIXTURE_EXPECT_MODEL"), model.clone());
    env.insert(
        String::from("RPC_FIXTURE_EXPECT_ARGS"),
        String::from("--no-session"),
    );
    env.insert(
        String::from("RPC_FIXTURE_EXTENSION_MODE"),
        extension_mode.to_owned(),
    );
    env.insert(String::from("RPC_FIXTURE_MODE"), mode.to_owned());

    RpcClient::new(RpcClientOptions {
        program: Some(PathBuf::from("node")),
        cli_path: Some(fixture_path()),
        cwd: Some(unique_temp_dir("cwd")),
        env,
        provider: Some(provider),
        model: Some(model),
        args: vec![String::from("--no-session")],
    })
}

#[tokio::test]
async fn rpc_client_runs_commands_and_collects_events() {
    let mut client = build_client("off", "default");
    client.start().await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(client.stderr().contains("fixture stderr ready"));

    let initial_state = client.get_state().await.unwrap();
    assert_eq!(
        initial_state
            .model
            .as_ref()
            .map(|model| model.provider.as_str()),
        Some("fixture-provider")
    );
    assert_eq!(initial_state.thinking_level, RpcThinkingLevel::Off);
    assert_eq!(initial_state.message_count, 0);

    let models = client.get_available_models().await.unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "fixture-model");
    assert_eq!(models[1].id, "fixture-model-alt");

    client
        .set_thinking_level(RpcThinkingLevel::High)
        .await
        .unwrap();
    assert_eq!(
        client.get_state().await.unwrap().thinking_level,
        RpcThinkingLevel::High
    );

    let events = client
        .prompt_and_wait("hello from rust", None, Duration::from_secs(2))
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, RpcOutputEvent::Session(RpcSessionEvent::AgentStart)))
    );
    let message_update = events
        .iter()
        .find_map(|event| match event {
            RpcOutputEvent::Session(RpcSessionEvent::MessageUpdate {
                assistant_message_event,
                ..
            }) => Some(assistant_message_event),
            _ => None,
        })
        .unwrap();
    match message_update {
        AssistantEvent::TextDelta { delta, .. } => assert_eq!(delta, "fixture reply 1"),
        other => panic!("unexpected assistant event: {other:?}"),
    }

    client.prompt("second prompt", None).await.unwrap();
    client.wait_for_idle(Duration::from_secs(2)).await.unwrap();

    let bash = client.bash("echo hello").await.unwrap();
    assert_eq!(bash.output, "bash:echo hello");
    assert_eq!(bash.exit_code, 0);
    assert!(!bash.cancelled);

    let commands = client.get_commands().await.unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "fixture-command");

    let messages = client.get_messages().await.unwrap();
    assert_eq!(messages.len(), 2);

    let last_assistant_text = client.get_last_assistant_text().await.unwrap();
    assert_eq!(last_assistant_text.as_deref(), Some("fixture reply 2"));

    client.set_session_name("named session").await.unwrap();
    assert_eq!(
        client.get_state().await.unwrap().session_name.as_deref(),
        Some("named session")
    );

    let new_session = client.new_session(None).await.unwrap();
    assert!(!new_session.cancelled);
    assert_eq!(client.get_state().await.unwrap().message_count, 0);
    assert_eq!(client.get_last_assistant_text().await.unwrap(), None);

    client.stop().await.unwrap();
}

#[tokio::test]
async fn rpc_client_handles_extension_ui_requests() {
    let mut client = build_client("prompt", "default");
    client.start().await.unwrap();

    let mut receiver = client.subscribe_events();
    client.prompt("need extension ui", None).await.unwrap();

    let request_id = loop {
        match tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap()
        {
            RpcOutputEvent::ExtensionUiRequest(request) => break request.id,
            RpcOutputEvent::Session(_) | RpcOutputEvent::Unknown(_) => {}
        }
    };

    client
        .send_extension_ui_response(RpcExtensionUiResponse::value(request_id, "typed value"))
        .await
        .unwrap();

    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(
            event,
            RpcOutputEvent::Session(RpcSessionEvent::AgentEnd { .. })
        ) {
            break;
        }
    }

    assert_eq!(
        client.get_last_assistant_text().await.unwrap().as_deref(),
        Some("extension:typed value")
    );

    client.stop().await.unwrap();
}

#[tokio::test]
async fn rpc_client_reports_immediate_process_exit() {
    let mut client = build_client("off", "exit-immediately");
    let error = client.start().await.unwrap_err();

    match error {
        RpcClientError::ExitedImmediately { code, stderr } => {
            assert_eq!(code, Some(3));
            assert!(stderr.contains("fixture exiting immediately"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
