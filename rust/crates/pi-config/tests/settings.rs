use pi_config::{SettingsScope, load_runtime_settings};
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
    assert_eq!(loaded.settings.thinking_budgets.minimal, None);
    assert_eq!(loaded.settings.thinking_budgets.low, None);
    assert_eq!(loaded.settings.thinking_budgets.medium, None);
    assert_eq!(loaded.settings.thinking_budgets.high, None);
    assert_eq!(loaded.settings.autocomplete_max_visible, 5);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_global_runtime_settings() {
    let cwd = unique_temp_dir("project-override-cwd");
    let agent_dir = unique_temp_dir("project-override-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"images":{"autoResize":false,"blockImages":true},"thinkingBudgets":{"low":2048,"high":16384}}"#,
    )
    .unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"images":{"autoResize":true,"blockImages":false},"thinkingBudgets":{"high":4096}}"#,
    )
    .unwrap();

    let loaded = load_runtime_settings(&cwd, &agent_dir);

    assert!(loaded.settings.images.auto_resize_images);
    assert!(!loaded.settings.images.block_images);
    assert_eq!(loaded.settings.thinking_budgets.low, Some(2048));
    assert_eq!(loaded.settings.thinking_budgets.high, Some(4096));
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
    assert_eq!(loaded.settings.thinking_budgets, Default::default());
    assert_eq!(loaded.settings.autocomplete_max_visible, 5);
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].scope, SettingsScope::Global);
    assert!(!loaded.warnings[0].message.is_empty());
}
