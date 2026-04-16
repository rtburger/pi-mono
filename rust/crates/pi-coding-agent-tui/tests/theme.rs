use pi_coding_agent_tui::{
    LoadThemesOptions, current_theme, current_theme_name, init_theme, load_themes,
    set_registered_themes,
};
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
    let path = std::env::temp_dir().join(format!("pi-coding-agent-theme-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn load_themes_reads_default_directories_and_explicit_paths() {
    let temp_dir = unique_temp_dir("load");
    let agent_dir = temp_dir.join("agent");
    let cwd = temp_dir.join("project");
    fs::create_dir_all(agent_dir.join("themes")).unwrap();
    fs::create_dir_all(cwd.join(".pi").join("themes")).unwrap();

    let custom_source =
        include_str!("../src/theme/dark.json").replace("\"name\": \"dark\"", "\"name\": \"ocean\"");
    let project_source = include_str!("../src/theme/light.json")
        .replace("\"name\": \"light\"", "\"name\": \"paper\"");
    let explicit_source = include_str!("../src/theme/dark.json")
        .replace("\"name\": \"dark\"", "\"name\": \"forest\"");
    let explicit_path = temp_dir.join("forest.json");

    fs::write(agent_dir.join("themes").join("ocean.json"), custom_source).unwrap();
    fs::write(
        cwd.join(".pi").join("themes").join("paper.json"),
        project_source,
    )
    .unwrap();
    fs::write(&explicit_path, explicit_source).unwrap();

    let loaded = load_themes(LoadThemesOptions {
        cwd: cwd.clone(),
        agent_dir: Some(agent_dir.clone()),
        theme_paths: vec![explicit_path.to_string_lossy().into_owned()],
        include_defaults: true,
    });

    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    let names = loaded
        .themes
        .iter()
        .map(|theme| theme.name().to_owned())
        .collect::<Vec<_>>();
    assert!(names.contains(&String::from("ocean")), "names: {names:?}");
    assert!(names.contains(&String::from("paper")), "names: {names:?}");
    assert!(names.contains(&String::from("forest")), "names: {names:?}");
}

#[test]
fn init_theme_uses_registered_themes_and_falls_back_to_dark() {
    let temp_dir = unique_temp_dir("init");
    let custom_path = temp_dir.join("custom.json");
    let custom_source = include_str!("../src/theme/dark.json")
        .replace("\"name\": \"dark\"", "\"name\": \"custom\"")
        .replace("\"accent\": \"#8abeb7\"", "\"accent\": \"#123456\"");
    fs::write(&custom_path, custom_source).unwrap();

    let loaded = load_themes(LoadThemesOptions {
        cwd: temp_dir.clone(),
        agent_dir: None,
        theme_paths: vec![custom_path.to_string_lossy().into_owned()],
        include_defaults: false,
    });
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);

    set_registered_themes(loaded.themes.clone());
    let selected = init_theme(Some("custom"));
    assert!(selected.success, "{selected:?}");
    assert_eq!(current_theme_name(), "custom");
    assert_eq!(current_theme().name(), "custom");
    assert!(current_theme().fg("accent", "x").contains("x"));

    let fallback = init_theme(Some("missing-theme"));
    assert!(!fallback.success, "{fallback:?}");
    assert_eq!(fallback.applied_theme_name, "dark");
    assert_eq!(current_theme_name(), "dark");
    assert_eq!(current_theme().name(), "dark");

    set_registered_themes(Vec::new());
}
