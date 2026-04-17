use pi_ai::Transport;
use pi_config::{
    CompactionConfig, InMemorySettingsStorage, MarkdownConfig, PackageSource, Settings,
    SettingsManager, SettingsScope, TerminalConfig, load_resource_settings, load_runtime_settings,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-config-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn defaults_runtime_settings_when_settings_are_missing() {
    let cwd = unique_temp_dir("defaults-cwd");
    let agent_dir = unique_temp_dir("defaults-agent");

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.default_provider, None);
    assert_eq!(loaded.settings.default_model, None);
    assert_eq!(loaded.settings.default_thinking_level, None);
    assert_eq!(loaded.settings.transport, Transport::Sse);
    assert_eq!(loaded.settings.steering_mode, "one-at-a-time");
    assert_eq!(loaded.settings.follow_up_mode, "one-at-a-time");
    assert_eq!(loaded.settings.theme, None);
    assert!(loaded.settings.compaction.enabled);
    assert_eq!(loaded.settings.compaction.reserve_tokens, 16_384);
    assert_eq!(loaded.settings.compaction.keep_recent_tokens, 20_000);
    assert_eq!(loaded.settings.branch_summary.reserve_tokens, 16_384);
    assert!(!loaded.settings.branch_summary.skip_prompt);
    assert!(loaded.settings.retry.enabled);
    assert_eq!(loaded.settings.retry.max_retries, 3);
    assert_eq!(loaded.settings.retry.base_delay_ms, 2_000);
    assert_eq!(loaded.settings.retry.max_delay_ms, 60_000);
    assert!(!loaded.settings.hide_thinking_block);
    assert_eq!(loaded.settings.shell_path, None);
    assert!(!loaded.settings.quiet_startup);
    assert_eq!(loaded.settings.shell_command_prefix, None);
    assert!(!loaded.settings.collapse_changelog);
    assert!(loaded.settings.enable_skill_commands);
    assert!(loaded.settings.terminal.show_images);
    assert!(!loaded.settings.terminal.clear_on_shrink);
    assert!(loaded.settings.images.auto_resize_images);
    assert!(!loaded.settings.images.block_images);
    assert_eq!(loaded.settings.enabled_models, None);
    assert_eq!(loaded.settings.double_escape_action, "tree");
    assert_eq!(loaded.settings.tree_filter_mode, "default");
    assert_eq!(loaded.settings.thinking_budgets.minimal, None);
    assert_eq!(loaded.settings.thinking_budgets.low, None);
    assert_eq!(loaded.settings.thinking_budgets.medium, None);
    assert_eq!(loaded.settings.thinking_budgets.high, None);
    assert_eq!(loaded.settings.editor_padding_x, 0);
    assert_eq!(loaded.settings.autocomplete_max_visible, 5);
    assert!(!loaded.settings.show_hardware_cursor);
    assert_eq!(loaded.settings.markdown.code_block_indent, "  ");
    assert_eq!(loaded.settings.session_dir, None);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_extended_runtime_settings() {
    let cwd = unique_temp_dir("runtime-overrides-cwd");
    let agent_dir = unique_temp_dir("runtime-overrides-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{
  "defaultProvider": "anthropic",
  "defaultModel": "claude-global",
  "defaultThinkingLevel": "minimal",
  "transport": "websocket",
  "steeringMode": "all",
  "followUpMode": "all",
  "theme": "dark",
  "compaction": {
    "enabled": false,
    "reserveTokens": 8192,
    "keepRecentTokens": 1000
  },
  "branchSummary": {
    "reserveTokens": 2048,
    "skipPrompt": true
  },
  "retry": {
    "enabled": false,
    "maxRetries": 9,
    "baseDelayMs": 100,
    "maxDelayMs": 900
  },
  "hideThinkingBlock": true,
  "shellPath": "/bin/zsh",
  "quietStartup": true,
  "shellCommandPrefix": "source ~/.aliases",
  "collapseChangelog": true,
  "enableSkillCommands": false,
  "terminal": {
    "showImages": false,
    "clearOnShrink": true
  },
  "images": {
    "autoResize": false,
    "blockImages": true
  },
  "enabledModels": ["global-a", "global-b"],
  "doubleEscapeAction": "fork",
  "treeFilterMode": "labeled-only",
  "thinkingBudgets": {
    "low": 2048,
    "high": 8192
  },
  "editorPaddingX": 1,
  "autocompleteMaxVisible": 8,
  "showHardwareCursor": true,
  "markdown": {
    "codeBlockIndent": "\t"
  },
  "sessionDir": ".pi/global-sessions"
}"#,
    )
    .unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{
  "defaultProvider": "openai",
  "defaultModel": "gpt-project",
  "defaultThinkingLevel": "high",
  "transport": "auto",
  "followUpMode": "one-at-a-time",
  "theme": "light",
  "compaction": {
    "enabled": true,
    "keepRecentTokens": 4096
  },
  "branchSummary": {
    "reserveTokens": 4096
  },
  "retry": {
    "enabled": true,
    "maxRetries": 3
  },
  "quietStartup": false,
  "terminal": {
    "showImages": true
  },
  "enabledModels": ["project-a"],
  "treeFilterMode": "all",
  "editorPaddingX": 9,
  "autocompleteMaxVisible": 25,
  "sessionDir": "sessions/project"
}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.default_provider.as_deref(), Some("openai"));
    assert_eq!(
        loaded.settings.default_model.as_deref(),
        Some("gpt-project")
    );
    assert_eq!(
        loaded.settings.default_thinking_level.as_deref(),
        Some("high")
    );
    assert_eq!(loaded.settings.transport, Transport::Auto);
    assert_eq!(loaded.settings.steering_mode, "all");
    assert_eq!(loaded.settings.follow_up_mode, "one-at-a-time");
    assert_eq!(loaded.settings.theme.as_deref(), Some("light"));
    assert!(loaded.settings.compaction.enabled);
    assert_eq!(loaded.settings.compaction.reserve_tokens, 8192);
    assert_eq!(loaded.settings.compaction.keep_recent_tokens, 4096);
    assert_eq!(loaded.settings.branch_summary.reserve_tokens, 4096);
    assert!(loaded.settings.branch_summary.skip_prompt);
    assert!(loaded.settings.retry.enabled);
    assert_eq!(loaded.settings.retry.max_retries, 3);
    assert_eq!(loaded.settings.retry.base_delay_ms, 100);
    assert_eq!(loaded.settings.retry.max_delay_ms, 900);
    assert!(loaded.settings.hide_thinking_block);
    assert_eq!(loaded.settings.shell_path.as_deref(), Some("/bin/zsh"));
    assert!(!loaded.settings.quiet_startup);
    assert_eq!(
        loaded.settings.shell_command_prefix.as_deref(),
        Some("source ~/.aliases")
    );
    assert!(loaded.settings.collapse_changelog);
    assert!(!loaded.settings.enable_skill_commands);
    assert!(loaded.settings.terminal.show_images);
    assert!(loaded.settings.terminal.clear_on_shrink);
    assert!(!loaded.settings.images.auto_resize_images);
    assert!(loaded.settings.images.block_images);
    assert_eq!(
        loaded.settings.enabled_models,
        Some(vec![String::from("project-a")])
    );
    assert_eq!(loaded.settings.double_escape_action, "fork");
    assert_eq!(loaded.settings.tree_filter_mode, "all");
    assert_eq!(loaded.settings.thinking_budgets.low, Some(2048));
    assert_eq!(loaded.settings.thinking_budgets.high, Some(8192));
    assert_eq!(loaded.settings.editor_padding_x, 3);
    assert_eq!(loaded.settings.autocomplete_max_visible, 20);
    assert!(loaded.settings.show_hardware_cursor);
    assert_eq!(loaded.settings.markdown.code_block_indent, "\t");
    assert_eq!(
        loaded.settings.session_dir.as_deref(),
        Some("sessions/project")
    );
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_transport_and_migrate_legacy_websockets() {
    let cwd = unique_temp_dir("transport-cwd");
    let agent_dir = unique_temp_dir("transport-agent");
    fs::write(agent_dir.join("settings.json"), r#"{"websockets":true}"#).unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"transport":"auto"}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.transport, Transport::Auto);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn warns_for_invalid_transport_and_keeps_defaults() {
    let cwd = unique_temp_dir("invalid-transport-cwd");
    let agent_dir = unique_temp_dir("invalid-transport-agent");
    fs::write(agent_dir.join("settings.json"), r#"{"transport":"udp"}"#).unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.transport, Transport::Sse);
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].scope, SettingsScope::Global);
    assert!(
        loaded.warnings[0]
            .message
            .contains("Invalid transport setting \"udp\"")
    );
}

#[test]
fn reports_invalid_json_as_scope_warning_and_uses_defaults() {
    let cwd = unique_temp_dir("invalid-json-cwd");
    let agent_dir = unique_temp_dir("invalid-json-agent");
    fs::write(agent_dir.join("settings.json"), "{").unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.transport, Transport::Sse);
    assert_eq!(loaded.settings.theme, None);
    assert_eq!(loaded.settings.enabled_models, None);
    assert_eq!(loaded.settings.session_dir, None);
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].scope, SettingsScope::Global);
    assert!(!loaded.warnings[0].message.is_empty());
}

#[test]
fn load_resource_settings_reads_scoped_package_configuration() {
    let cwd = unique_temp_dir("resource-settings-cwd");
    let agent_dir = unique_temp_dir("resource-settings-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{
  "npmCommand": ["mise", "exec", "node@20", "--", "npm"],
  "packages": [
    "npm:global-pkg",
    {
      "source": "./global-local-pkg",
      "extensions": ["extensions"],
      "skills": []
    }
  ],
  "extensions": ["extensions/global.ts"],
  "skills": ["skills"],
  "prompts": ["prompts"],
  "themes": ["themes"]
}"#,
    )
    .unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{
  "packages": ["./project-pkg"],
  "extensions": ["extensions/project.ts"],
  "skills": ["skills/project"],
  "prompts": ["prompts/project.md"],
  "themes": ["themes/project.json"]
}"#,
    )
    .unwrap();

    let loaded = load_resource_settings(&cwd, &agent_dir);

    assert_eq!(
        loaded.global.npm_command,
        Some(vec![
            String::from("mise"),
            String::from("exec"),
            String::from("node@20"),
            String::from("--"),
            String::from("npm"),
        ])
    );
    assert_eq!(loaded.global.packages.len(), 2);
    assert_eq!(
        loaded.global.packages[0],
        PackageSource::Plain(String::from("npm:global-pkg"))
    );
    assert!(loaded.global.packages[1].is_filtered());
    assert_eq!(loaded.project.packages.len(), 1);
    assert_eq!(
        loaded.project.packages[0],
        PackageSource::Plain(String::from("./project-pkg"))
    );
    assert_eq!(
        loaded.global.extensions,
        vec![String::from("extensions/global.ts")]
    );
    assert_eq!(loaded.project.skills, vec![String::from("skills/project")]);
    assert_eq!(
        loaded.project.prompts,
        vec![String::from("prompts/project.md")]
    );
    assert_eq!(
        loaded.project.themes,
        vec![String::from("themes/project.json")]
    );
    assert!(loaded.warnings.is_empty());
}

#[test]
fn settings_manager_merges_scoped_settings_and_exposes_resolved_getters() {
    let manager = SettingsManager::in_memory_with_scopes(
        Settings {
            default_provider: Some(String::from("anthropic")),
            transport: Some(String::from("websocket")),
            compaction: Some(CompactionConfig {
                enabled: Some(false),
                reserve_tokens: Some(1234),
                keep_recent_tokens: None,
            }),
            terminal: Some(TerminalConfig {
                show_images: Some(false),
                clear_on_shrink: None,
            }),
            markdown: Some(MarkdownConfig {
                code_block_indent: Some(String::from("\t")),
            }),
            packages: Some(vec![PackageSource::Plain(String::from("global"))]),
            enable_skill_commands: Some(false),
            ..Settings::default()
        },
        Settings {
            default_model: Some(String::from("project-model")),
            theme: Some(String::from("light")),
            compaction: Some(CompactionConfig {
                enabled: None,
                reserve_tokens: None,
                keep_recent_tokens: Some(77),
            }),
            packages: Some(vec![PackageSource::Plain(String::from("project"))]),
            ..Settings::default()
        },
    );

    assert_eq!(manager.default_provider().as_deref(), Some("anthropic"));
    assert_eq!(manager.default_model().as_deref(), Some("project-model"));
    assert_eq!(manager.transport(), Transport::WebSocket);
    assert_eq!(manager.theme().as_deref(), Some("light"));
    assert!(!manager.compaction_enabled());
    assert_eq!(manager.compaction_reserve_tokens(), 1234);
    assert_eq!(manager.compaction_keep_recent_tokens(), 77);
    assert!(!manager.show_images());
    assert_eq!(manager.code_block_indent(), "\t");
    assert_eq!(
        manager.packages(),
        vec![PackageSource::Plain(String::from("project"))]
    );
    assert!(!manager.enable_skill_commands());
}

#[test]
fn settings_manager_persists_global_and_project_updates() {
    let storage = InMemorySettingsStorage::new();
    let mut manager = SettingsManager::from_storage(storage.clone());

    manager.set_default_provider("anthropic");
    manager.set_default_model("claude-sonnet");
    manager.set_show_images(false);
    manager.set_code_block_indent(Some(String::from("\t")));
    manager.set_session_dir(Some(String::from(".pi/sessions")));
    manager.set_project_theme_paths(vec![String::from("themes/project.json")]);
    manager.flush();

    let global: Value = serde_json::from_str(&storage.raw(SettingsScope::Global).unwrap()).unwrap();
    let project: Value =
        serde_json::from_str(&storage.raw(SettingsScope::Project).unwrap()).unwrap();

    assert_eq!(
        global["defaultProvider"],
        Value::String(String::from("anthropic"))
    );
    assert_eq!(
        global["defaultModel"],
        Value::String(String::from("claude-sonnet"))
    );
    assert_eq!(global["terminal"]["showImages"], Value::Bool(false));
    assert_eq!(
        global["markdown"]["codeBlockIndent"],
        Value::String(String::from("\t"))
    );
    assert_eq!(
        global["sessionDir"],
        Value::String(String::from(".pi/sessions"))
    );
    assert_eq!(
        project["themes"],
        Value::Array(vec![Value::String(String::from("themes/project.json"))])
    );
}

#[test]
fn settings_manager_migrates_legacy_fields_and_persists_new_shape() {
    let storage = InMemorySettingsStorage::new();
    storage.set_raw(
        SettingsScope::Global,
        Some(
            String::from(
                r#"{"queueMode":"all","websockets":true,"skills":{"enableSkillCommands":false,"customDirectories":["skills/global"]}}"#,
            ),
        ),
    );

    let mut manager = SettingsManager::from_storage(storage.clone());

    assert_eq!(manager.steering_mode(), "all");
    assert_eq!(manager.transport(), Transport::WebSocket);
    assert_eq!(manager.skill_paths(), vec![String::from("skills/global")]);
    assert!(!manager.enable_skill_commands());

    manager.set_theme("light");
    manager.flush();

    let persisted: Value =
        serde_json::from_str(&storage.raw(SettingsScope::Global).unwrap()).unwrap();
    assert_eq!(
        persisted["steeringMode"],
        Value::String(String::from("all"))
    );
    assert_eq!(
        persisted["transport"],
        Value::String(String::from("websocket"))
    );
    assert_eq!(
        persisted["skills"],
        Value::Array(vec![Value::String(String::from("skills/global"))])
    );
    assert_eq!(persisted["enableSkillCommands"], Value::Bool(false));
    assert_eq!(persisted["theme"], Value::String(String::from("light")));
    assert!(persisted.get("queueMode").is_none());
    assert!(persisted.get("websockets").is_none());
}

#[test]
fn settings_manager_keeps_invalid_json_untouched_and_records_errors() {
    let storage = InMemorySettingsStorage::new();
    storage.set_raw(SettingsScope::Global, Some(String::from("{")));

    let mut manager = SettingsManager::from_storage(storage.clone());
    let errors = manager.drain_errors();

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].scope, SettingsScope::Global);

    manager.set_theme("light");
    manager.flush();

    assert_eq!(storage.raw(SettingsScope::Global).as_deref(), Some("{"));
    assert_eq!(manager.theme().as_deref(), Some("light"));
}
