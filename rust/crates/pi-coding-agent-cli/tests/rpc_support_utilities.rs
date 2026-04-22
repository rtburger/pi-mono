use pi_ai::{FauxModelDefinition, RegisterFauxProviderOptions, register_faux_provider};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
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
    let path = std::env::temp_dir().join(format!(
        "pi-coding-agent-cli-rpc-support-{prefix}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[tokio::test]
async fn rpc_bash_returns_exit_codes_and_session_stats_include_context_usage() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "rpc-support-faux".into(),
        models: vec![FauxModelDefinition {
            id: "rpc-support-faux-1".into(),
            name: Some("RPC Support Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    let model = faux
        .get_model(Some("rpc-support-faux-1"))
        .expect("expected faux model");

    let commands = [
        serde_json::json!({
            "id": "cmd-bash",
            "type": "bash",
            "command": "printf 'rpc bash output'; exit 7"
        }),
        serde_json::json!({
            "id": "cmd-stats",
            "type": "get_session_stats"
        }),
        serde_json::json!({
            "id": "cmd-messages",
            "type": "get_messages"
        }),
    ]
    .into_iter()
    .map(|command| serde_json::to_string(&command).unwrap())
    .collect::<Vec<_>>()
    .join("\n");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--no-session"),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(commands),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("cwd"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);

    let responses = result
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| {
            let command = value.get("command").and_then(Value::as_str)?.to_owned();
            Some((command, value))
        })
        .collect::<BTreeMap<_, _>>();

    let bash = responses.get("bash").expect("expected bash response");
    assert_eq!(bash.get("success").and_then(Value::as_bool), Some(true));
    assert_eq!(
        bash.pointer("/data/output").and_then(Value::as_str),
        Some("rpc bash output")
    );
    assert_eq!(
        bash.pointer("/data/exitCode").and_then(Value::as_i64),
        Some(7)
    );
    assert_eq!(
        bash.pointer("/data/cancelled").and_then(Value::as_bool),
        Some(false)
    );

    let stats = responses
        .get("get_session_stats")
        .expect("expected session stats response");
    assert_eq!(stats.get("success").and_then(Value::as_bool), Some(true));
    assert_eq!(
        stats.pointer("/data/totalMessages").and_then(Value::as_u64),
        Some(1)
    );
    assert!(
        stats.pointer("/data/contextUsage").is_some(),
        "stats: {stats}"
    );

    let messages = responses
        .get("get_messages")
        .expect("expected messages response");
    let message_list = messages
        .pointer("/data/messages")
        .and_then(Value::as_array)
        .expect("expected message list");
    assert_eq!(message_list.len(), 1, "messages: {message_list:?}");
    assert_eq!(
        message_list[0].get("role").and_then(Value::as_str),
        Some("bashExecution")
    );
    assert_eq!(
        message_list[0]
            .pointer("/payload/exitCode")
            .and_then(Value::as_i64),
        Some(7)
    );

    faux.unregister();
}

#[tokio::test]
async fn rpc_new_session_resets_in_memory_session_state() {
    let faux = register_faux_provider(RegisterFauxProviderOptions {
        provider: "rpc-session-transition-faux".into(),
        models: vec![FauxModelDefinition {
            id: "rpc-session-transition-faux-1".into(),
            name: Some("RPC Session Transition Faux".into()),
            reasoning: false,
        }],
        ..RegisterFauxProviderOptions::default()
    });
    let model = faux
        .get_model(Some("rpc-session-transition-faux-1"))
        .expect("expected faux model");

    let commands = [
        serde_json::json!({
            "id": "cmd-bash",
            "type": "bash",
            "command": "printf 'before new session'"
        }),
        serde_json::json!({
            "id": "cmd-new-session",
            "type": "new_session"
        }),
        serde_json::json!({
            "id": "cmd-stats",
            "type": "get_session_stats"
        }),
    ]
    .into_iter()
    .map(|command| serde_json::to_string(&command).unwrap())
    .collect::<Vec<_>>()
    .join("\n");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--no-session"),
            String::from("--provider"),
            model.provider.clone(),
            String::from("--model"),
            model.id.clone(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(commands),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            model.provider.as_str(),
            "token",
        )])),
        built_in_models: vec![model],
        models_json_path: None,
        agent_dir: None,
        cwd: unique_temp_dir("new-session-cwd"),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: Default::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);

    let responses = result
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| {
            let command = value.get("command").and_then(Value::as_str)?.to_owned();
            Some((command, value))
        })
        .collect::<BTreeMap<_, _>>();

    let new_session = responses
        .get("new_session")
        .expect("expected new_session response");
    assert_eq!(
        new_session
            .pointer("/data/cancelled")
            .and_then(Value::as_bool),
        Some(false)
    );

    let stats = responses
        .get("get_session_stats")
        .expect("expected session stats response");
    assert_eq!(stats.get("success").and_then(Value::as_bool), Some(true));
    assert_eq!(
        stats.pointer("/data/totalMessages").and_then(Value::as_u64),
        Some(0)
    );

    faux.unregister();
}
