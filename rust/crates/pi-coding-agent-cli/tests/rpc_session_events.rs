use pi_ai::{
    FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions, StreamOptions,
    register_faux_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn unique_name(prefix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{unique}")
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(unique_name(prefix));
    fs::create_dir_all(&path).unwrap();
    path
}

fn parse_jsonl(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect()
}

#[tokio::test]
async fn run_command_rpc_mode_emits_queue_updates_and_assistant_message_events() {
    let provider = unique_name("rpc-queue-provider");
    let model_id = unique_name("rpc-queue-model");
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: provider.clone(),
        models: vec![FauxModelDefinition {
            id: model_id.clone(),
            name: Some(model_id.clone()),
            reasoning: false,
        }],
        chunk_delay: Duration::from_millis(10),
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![
        FauxResponse::text("streamed first response"),
        FauxResponse::text("steered response"),
    ]);

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
        ],
        stdin_is_tty: true,
        stdin_content: Some(
            [
                r#"{"id":"req-1","type":"prompt","message":"start"}"#,
                r#"{"id":"req-2","type":"steer","message":"queued steer"}"#,
            ]
            .join("\n"),
        ),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![faux.get_model(Some(&model_id)).unwrap()],
        models_json_path: None,
        agent_dir: Some(unique_temp_dir("rpc-queue-agent")),
        cwd: unique_temp_dir("rpc-queue-cwd"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let events = parse_jsonl(&result.stdout);

    let queue_updates = events
        .iter()
        .filter(|event| event["type"] == "queue_update")
        .collect::<Vec<_>>();
    assert!(queue_updates.len() >= 2, "events: {events:?}");
    assert_eq!(
        queue_updates[0]["steering"],
        serde_json::json!(["queued steer"])
    );
    assert_eq!(queue_updates[0]["followUp"], serde_json::json!([]));
    assert_eq!(
        queue_updates.last().unwrap()["steering"],
        serde_json::json!([])
    );

    let message_update = events
        .iter()
        .find(|event| event["type"] == "message_update")
        .unwrap();
    assert!(message_update.get("assistantMessageEvent").is_some());
    assert!(message_update.get("assistantEvent").is_none());

    let steer_response = events
        .iter()
        .find(|event| event["type"] == "response" && event["id"] == "req-2")
        .unwrap();
    assert_eq!(steer_response["success"].as_bool(), Some(true));

    faux.unregister();
}
