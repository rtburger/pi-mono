use futures::stream;
use pi_ai::{
    AiProvider, AssistantEventStream, StreamOptions, register_provider, unregister_provider,
};
use pi_coding_agent_cli::{RunCommandOptions, run_command};
use pi_coding_agent_core::MemoryAuthStorage;
use pi_events::{
    AssistantContent, AssistantEvent, AssistantMessage, Context, Message, Model, StopReason, Usage,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default)]
struct RecordedRequest {
    context: Option<Context>,
}

#[derive(Clone)]
struct RecordingProvider {
    response_text: String,
    recorded: Arc<Mutex<RecordedRequest>>,
}

impl AiProvider for RecordingProvider {
    fn stream(
        &self,
        model: Model,
        context: Context,
        _options: StreamOptions,
    ) -> AssistantEventStream {
        *self.recorded.lock().unwrap() = RecordedRequest {
            context: Some(context),
        };

        let message = AssistantMessage {
            role: String::from("assistant"),
            content: vec![AssistantContent::Text {
                text: self.response_text.clone(),
                text_signature: None,
            }],
            api: model.api.clone(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            response_id: None,
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
            error_message: None,
            timestamp: 0,
        };

        Box::pin(stream::iter(vec![Ok(AssistantEvent::Done {
            reason: StopReason::Stop,
            message,
        })]))
    }
}

fn unique_name(prefix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{unique}-{counter}")
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(unique_name(prefix));
    fs::create_dir_all(&path).unwrap();
    path
}

fn register_recording_provider(response_text: &str) -> (String, Arc<Mutex<RecordedRequest>>) {
    let api = unique_name("resources-api");
    let recorded = Arc::new(Mutex::new(RecordedRequest::default()));
    register_provider(
        api.clone(),
        Arc::new(RecordingProvider {
            response_text: response_text.to_string(),
            recorded: recorded.clone(),
        }),
    );
    (api, recorded)
}

fn model(api: &str, provider: &str, id: &str) -> Model {
    Model {
        id: id.to_string(),
        name: id.to_string(),
        api: api.to_string(),
        provider: provider.to_string(),
        base_url: String::from("https://example.invalid/v1"),
        reasoning: false,
        input: vec![String::from("text")],
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

fn last_user_text(context: &Context) -> String {
    context
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::User { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|content| match content {
                        pi_events::UserContent::Text { text } => Some(text.as_str()),
                        pi_events::UserContent::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn run_command_print_mode_expands_prompt_templates_and_applies_tool_selection() {
    let provider = unique_name("resources-provider");
    let model_id = unique_name("resources-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("resources-print");
    let prompt_path = cwd.join("review.md");
    fs::write(&prompt_path, "Review $1 carefully\n").unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--no-tools"),
            String::from("--tools"),
            String::from("read,grep"),
            String::from("--prompt-template"),
            prompt_path.to_string_lossy().into_owned(),
            String::from("/review src/lib.rs"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    let tool_names = context
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tool_names, vec!["read", "grep"]);
    assert_eq!(last_user_text(&context), "Review src/lib.rs carefully\n");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_print_mode_preserves_explicit_no_tools_selection() {
    let provider = unique_name("no-tools-provider");
    let model_id = unique_name("no-tools-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("resources-no-tools");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--no-tools"),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    assert!(context.tools.is_empty(), "tools: {:?}", context.tools);
    let system_prompt = context.system_prompt.expect("expected system prompt");
    assert!(
        system_prompt.contains("Available tools:\n(none)"),
        "system prompt: {system_prompt}"
    );

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_print_mode_expands_skill_commands_from_project_resources() {
    let provider = unique_name("skill-provider");
    let model_id = unique_name("skill-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("resources-skill");
    let skill_dir = cwd.join(".pi").join("skills").join("review-code");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\ndescription: Review code safely\n---\n# Review Code\nRead the target file first.\n",
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("/skill:review-code src/lib.rs"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    let text = last_user_text(&context);
    assert!(text.contains("<skill name=\"review-code\""), "text: {text}");
    assert!(text.contains("Read the target file first."), "text: {text}");
    assert!(text.contains("src/lib.rs"), "text: {text}");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_print_mode_accepts_extension_flags_without_rejecting() {
    let provider = unique_name("extension-provider");
    let model_id = unique_name("extension-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("resources-extension-flags");
    let extension_path = cwd.join("demo-extension.ts");
    fs::write(&extension_path, "export default function () {}\n").unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--no-extensions"),
            String::from("--extension"),
            extension_path.to_string_lossy().into_owned(),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        !result
            .stderr
            .contains("Extension loading is not supported in the Rust CLI yet"),
        "stderr: {}",
        result.stderr
    );
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    assert_eq!(last_user_text(&context), "hello");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_warns_for_missing_extension_paths() {
    let provider = unique_name("missing-extension-provider");
    let model_id = unique_name("missing-extension-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("resources-missing-extension");
    let missing_extension_path = cwd.join("missing-extension.ts");

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--extension"),
            missing_extension_path.to_string_lossy().into_owned(),
            String::from("hello"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(&format!(
            "Warning: Extension path does not exist: {}",
            missing_extension_path.display()
        )),
        "stderr: {}",
        result.stderr
    );
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    assert_eq!(last_user_text(&context), "hello");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_print_mode_loads_prompt_templates_from_project_package_settings() {
    let provider = unique_name("package-prompt-provider");
    let model_id = unique_name("package-prompt-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("resources-package-prompt");
    let agent_dir = cwd.join("agent");
    let package_dir = cwd.join("demo-package");
    fs::create_dir_all(agent_dir.clone()).unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::create_dir_all(package_dir.join("prompts")).unwrap();
    fs::write(
        package_dir.join("prompts").join("review.md"),
        "Package review $1\n",
    )
    .unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        format!("{{\n  \"packages\": [\"{}\"]\n}}\n", package_dir.display()),
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("/review src/lib.rs"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    assert_eq!(last_user_text(&context), "Package review src/lib.rs\n");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_print_mode_loads_prompt_templates_from_temporary_extension_packages() {
    let provider = unique_name("temporary-package-provider");
    let model_id = unique_name("temporary-package-model");
    let (api, recorded) = register_recording_provider("done");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("resources-temporary-package-prompt");
    let agent_dir = cwd.join("agent");
    let package_dir = cwd.join("temporary-package");
    fs::create_dir_all(agent_dir.clone()).unwrap();
    fs::create_dir_all(package_dir.join("prompts")).unwrap();
    fs::write(
        package_dir.join("prompts").join("review.md"),
        "Temporary review $1\n",
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("-p"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
            String::from("--extension"),
            package_dir.to_string_lossy().into_owned(),
            String::from("/review src/main.rs"),
        ],
        stdin_is_tty: true,
        stdin_content: None,
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(agent_dir),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let request = recorded.lock().unwrap().clone();
    let context = request.context.expect("expected recorded context");
    assert_eq!(last_user_text(&context), "Temporary review src/main.rs\n");

    unregister_provider(&api);
}

#[tokio::test]
async fn run_command_rpc_get_commands_lists_prompt_templates_and_skills() {
    let provider = unique_name("rpc-resources-provider");
    let model_id = unique_name("rpc-resources-model");
    let (api, _recorded) = register_recording_provider("unused");
    let built_in_model = model(&api, &provider, &model_id);
    let cwd = unique_temp_dir("rpc-resources");
    let prompts_dir = cwd.join(".pi").join("prompts");
    let skills_dir = cwd.join(".pi").join("skills").join("review-code");
    fs::create_dir_all(&prompts_dir).unwrap();
    fs::create_dir_all(&skills_dir).unwrap();
    fs::write(prompts_dir.join("review.md"), "Review $1\n").unwrap();
    fs::write(
        skills_dir.join("SKILL.md"),
        "---\ndescription: Review code safely\n---\n# Review Code\n",
    )
    .unwrap();

    let result = run_command(RunCommandOptions {
        args: vec![
            String::from("--mode"),
            String::from("rpc"),
            String::from("--provider"),
            provider.clone(),
            String::from("--model"),
            model_id.clone(),
        ],
        stdin_is_tty: false,
        stdin_content: Some(String::from(
            "{\"id\":\"cmd-1\",\"type\":\"get_commands\"}\n",
        )),
        auth_source: Arc::new(MemoryAuthStorage::with_api_keys([(
            provider.as_str(),
            "token",
        )])),
        built_in_models: vec![built_in_model],
        models_json_path: None,
        agent_dir: Some(cwd.join("agent")),
        cwd: cwd.clone(),
        default_system_prompt: String::new(),
        version: String::from("0.1.0"),
        stream_options: StreamOptions::default(),
    })
    .await;

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let lines = result
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("expected json line"))
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "stdout: {}", result.stdout);
    let commands = lines[0]["data"]["commands"]
        .as_array()
        .expect("expected command array");
    let names = commands
        .iter()
        .filter_map(|command| command.get("name").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert!(names.contains(&"review"), "names: {names:?}");
    assert!(names.contains(&"skill:review-code"), "names: {names:?}");

    unregister_provider(&api);
}
