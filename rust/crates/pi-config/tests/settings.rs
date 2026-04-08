use pi_config::{SettingsScope, load_image_settings};
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
fn defaults_image_settings_when_settings_are_missing() {
    let cwd = unique_temp_dir("defaults-cwd");
    let agent_dir = unique_temp_dir("defaults-agent");

    let loaded = load_image_settings(&cwd, &agent_dir);

    assert!(loaded.settings.auto_resize_images);
    assert!(!loaded.settings.block_images);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn project_settings_override_global_image_settings() {
    let cwd = unique_temp_dir("project-override-cwd");
    let agent_dir = unique_temp_dir("project-override-agent");
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"images":{"autoResize":false,"blockImages":true}}"#,
    )
    .unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(
        cwd.join(".pi").join("settings.json"),
        r#"{"images":{"autoResize":true,"blockImages":false}}"#,
    )
    .unwrap();

    let loaded = load_image_settings(&cwd, &agent_dir);

    assert!(loaded.settings.auto_resize_images);
    assert!(!loaded.settings.block_images);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn reports_invalid_json_as_scope_warning_and_uses_defaults() {
    let cwd = unique_temp_dir("invalid-json-cwd");
    let agent_dir = unique_temp_dir("invalid-json-agent");
    fs::write(agent_dir.join("settings.json"), "{").unwrap();

    let loaded = load_image_settings(&cwd, &agent_dir);

    assert!(loaded.settings.auto_resize_images);
    assert!(!loaded.settings.block_images);
    assert_eq!(loaded.warnings.len(), 1);
    assert_eq!(loaded.warnings[0].scope, SettingsScope::Global);
    assert!(!loaded.warnings[0].message.is_empty());
}
