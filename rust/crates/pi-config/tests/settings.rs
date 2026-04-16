use pi_config::{PackageSource, SettingsScope, load_resource_settings, load_runtime_settings};
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

    assert!(loaded.settings.images.auto_resize_images);
    assert!(!loaded.settings.images.block_images);
    assert!(loaded.settings.compaction.enabled);
    assert_eq!(loaded.settings.compaction.reserve_tokens, 16_384);
    assert_eq!(loaded.settings.compaction.keep_recent_tokens, 20_000);
    assert_eq!(loaded.settings.thinking_budgets.minimal, None);
    assert_eq!(loaded.settings.thinking_budgets.low, None);
    assert_eq!(loaded.settings.thinking_budgets.medium, None);
    assert_eq!(loaded.settings.thinking_budgets.high, None);
    assert_eq!(loaded.settings.theme, None);
    assert_eq!(loaded.settings.editor_padding_x, 0);
    assert_eq!(loaded.settings.autocomplete_max_visible, 5);
    assert_eq!(loaded.settings.enabled_models, None);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_global_runtime_settings() {
    let cwd = unique_temp_dir("project-override-cwd");
    let agent_dir = unique_temp_dir("project-override-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"images":{"autoResize":false,"blockImages":true},"compaction":{"enabled":false,"reserveTokens":8192},"thinkingBudgets":{"low":2048,"high":16384}}"#,
    )
    .unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"images":{"autoResize":true,"blockImages":false},"compaction":{"enabled":true,"keepRecentTokens":4096},"thinkingBudgets":{"high":4096}}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert!(loaded.settings.images.auto_resize_images);
    assert!(!loaded.settings.images.block_images);
    assert!(loaded.settings.compaction.enabled);
    assert_eq!(loaded.settings.compaction.reserve_tokens, 8192);
    assert_eq!(loaded.settings.compaction.keep_recent_tokens, 4096);
    assert_eq!(loaded.settings.thinking_budgets.low, Some(2048));
    assert_eq!(loaded.settings.thinking_budgets.high, Some(4096));
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_theme() {
    let cwd = unique_temp_dir("theme-cwd");
    let agent_dir = unique_temp_dir("theme-agent");
    fs::write(agent_dir.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"theme":"light"}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.theme.as_deref(), Some("light"));
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_and_clamp_editor_padding_x() {
    let cwd = unique_temp_dir("editor-padding-cwd");
    let agent_dir = unique_temp_dir("editor-padding-agent");
    fs::write(agent_dir.join("settings.json"), r#"{"editorPaddingX":1}"#).unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"editorPaddingX":9}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.editor_padding_x, 3);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_and_clamp_autocomplete_max_visible() {
    let cwd = unique_temp_dir("autocomplete-max-visible-cwd");
    let agent_dir = unique_temp_dir("autocomplete-max-visible-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"autocompleteMaxVisible":8}"#,
    )
    .unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"autocompleteMaxVisible":25}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(loaded.settings.autocomplete_max_visible, 20);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn reports_invalid_json_as_scope_warning_and_uses_defaults() {
    let cwd = unique_temp_dir("invalid-json-cwd");
    let agent_dir = unique_temp_dir("invalid-json-agent");
    fs::write(agent_dir.join("settings.json"), "{").unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert!(loaded.settings.images.auto_resize_images);
    assert!(!loaded.settings.images.block_images);
    assert_eq!(loaded.settings.compaction.reserve_tokens, 16_384);
    assert_eq!(loaded.settings.compaction.keep_recent_tokens, 20_000);
    assert_eq!(loaded.settings.thinking_budgets, Default::default());
    assert_eq!(loaded.settings.theme, None);
    assert_eq!(loaded.settings.editor_padding_x, 0);
    assert_eq!(loaded.settings.autocomplete_max_visible, 5);
    assert_eq!(loaded.settings.enabled_models, None);
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].scope, SettingsScope::Global);
    assert!(!loaded.warnings[0].message.is_empty());
}

#[test]
fn project_settings_override_enabled_models() {
    let cwd = unique_temp_dir("enabled-models-cwd");
    let agent_dir = unique_temp_dir("enabled-models-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"enabledModels":["global-a","global-b"]}"#,
    )
    .unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"enabledModels":["project-a"]}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert_eq!(
        loaded.settings.enabled_models,
        Some(vec![String::from("project-a")])
    );
    assert!(loaded.warnings.is_empty());
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
