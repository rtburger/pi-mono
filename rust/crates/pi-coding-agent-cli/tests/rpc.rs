use pi_ai::{
    FauxModelDefinition, FauxResponse, RegisterFauxProviderOptions, register_faux_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use serde_json::Value;
use std::{
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
    let path = std::env::temp_dir().join(format!(
        "pi-coding-agent-cli-rpc-{prefix}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[tokio::test]
async fn run_command_rpc_mode_emits_response_before_agent_events() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "rpc-mode-events-faux".into(),
        models: vec![FauxModelDefinition {
            id: "rpc-mode-events-faux-1".into(),
            name: Some("RPC Mode Events Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    faux.set_responses(vec![FauxResponse::text("rpc done")]);
    let model = faux
        .get_model(Some("rpc-mode-events-faux-1"))
        .expect("expected faux model");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"req-1\",\"type\":\"prompt\",\"message\":\"hello\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("events"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected valid json line"))
        .collect::<Vec<_>>();

    assert!(lines.len() >= 3, "stdout: {}", result.stdout);
    assert_eq!(
        lines[0].get("type").and_then(Value::as_str),
        Some("response"),
        "stdout: {}",
        result.stdout
    );
    assert_eq!(
        lines[0].get("command").and_then(Value::as_str),
        Some("prompt")
    );
    assert_eq!(
        lines[1].get("type").and_then(Value::as_str),
        Some("agent_start"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        lines
            .iter()
            .any(|line| line.get("type").and_then(Value::as_str) == Some("agent_end")),
        "stdout: {}",
        result.stdout
    );

    faux.unregister();
}

#[tokio::test]
async fn run_command_rpc_mode_updates_and_reports_session_state() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "rpc-mode-state-faux".into(),
        models: vec![
            FauxModelDefinition {
                id: "rpc-mode-state-faux-1".into(),
                name: Some("RPC Mode State Faux 1".into()),
                reasoning: false,
            },
            FauxModelDefinition {
                id: "rpc-mode-state-faux-2".into(),
                name: Some("RPC Mode State Faux 2".into()),
                reasoning: true,
            },
        ],
        ..RegisterFauxProviderOptions::default()
    });
    let initial_model = faux
        .get_model(Some("rpc-mode-state-faux-1"))
        .expect("expected initial faux model");
    let second_model = faux
        .get_model(Some("rpc-mode-state-faux-2"))
        .expect("expected second faux model");

    let stdin_content = [
        "{\"id\":\"req-1\",\"type\":\"get_state\"}",
        "{\"id\":\"req-2\",\"type\":\"set_model\",\"provider\":\"rpc-mode-state-faux\",\"modelId\":\"rpc-mode-state-faux-2\"}",
        "{\"id\":\"req-3\",\"type\":\"set_thinking_level\",\"level\":\"high\"}",
        "{\"id\":\"req-4\",\"type\":\"get_state\"}",
    ]
    .join("\n")
        + "\n";

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            initial_model.provider.clone(),
            String::from("--model"),
            initial_model.id.clone(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(stdin_content),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            initial_model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![initial_model.clone(), second_model.clone()],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("state"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(result.stderr.is_empty(), "stderr: {}", result.stderr);

    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("expected valid json line"))
        .collect::<Vec<_>>();

    assert_eq!(lines.len(), 4, "stdout: {}", result.stdout);

    let initial_state = lines[0]
        .get("data")
        .and_then(|data| data.get("model"))
        .and_then(|model| model.get("id"))
        .and_then(Value::as_str);
    assert_eq!(initial_state, Some(initial_model.id.as_str()));

    let set_model_id = lines[1]
        .get("data")
        .and_then(|data| data.get("id"))
        .and_then(Value::as_str);
    assert_eq!(set_model_id, Some(second_model.id.as_str()));

    let final_state = &lines[3]["data"];
    assert_eq!(
        final_state
            .get("model")
            .and_then(|model| model.get("id"))
            .and_then(Value::as_str),
        Some(second_model.id.as_str())
    );
    assert_eq!(
        final_state.get("thinkingLevel").and_then(Value::as_str),
        Some("high")
    );

    faux.unregister();
}

#[tokio::test]
async fn run_command_rpc_mode_rejects_file_arguments() {
    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("@README.md"),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::new()),
        auth_source: Arc::new(MemoryAuthStorage::default()),
        built_in_models: Vec::new(),
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("file-args"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty(), "stdout: {}", result.stdout);
    assert!(
        result
            .stderr
            .contains("@file arguments are not supported in RPC mode"),
        "stderr: {}",
        result.stderr
    );
}
