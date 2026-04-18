use pi_agent::ThinkingLevel;
use pi_coding_agent_tui::{
    ColorMode, LoadThemesOptions, current_theme, current_theme_name, get_available_themes,
    get_available_themes_with_paths, get_resolved_theme_colors, get_theme_by_name,
    get_theme_export_colors, init_theme, is_light_theme, load_theme_from_path_with_mode,
    load_themes, set_registered_themes, set_theme_instance,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
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

fn theme_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("theme test lock poisoned")
}

#[test]
fn load_themes_reads_default_directories_and_explicit_paths() {
    let _guard = theme_test_guard();
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
    let _guard = theme_test_guard();
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

#[test]
fn selected_custom_theme_hot_reloads_from_disk() {
    let _guard = theme_test_guard();
    let temp_dir = unique_temp_dir("watch");
    let custom_path = temp_dir.join("watched.json");
    let previous_colorterm = std::env::var_os("COLORTERM");
    unsafe { std::env::set_var("COLORTERM", "truecolor") };

    let result = (|| {
        let initial_source = include_str!("../src/theme/dark.json")
            .replace("\"name\": \"dark\"", "\"name\": \"watched\"")
            .replace("\"accent\": \"#8abeb7\"", "\"accent\": \"#123456\"");
        fs::write(&custom_path, initial_source).unwrap();

        let loaded = load_themes(LoadThemesOptions {
            cwd: temp_dir.clone(),
            agent_dir: None,
            theme_paths: vec![custom_path.to_string_lossy().into_owned()],
            include_defaults: false,
        });
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);

        set_registered_themes(loaded.themes);
        let selected = init_theme(Some("watched"));
        assert!(selected.success, "{selected:?}");
        assert!(
            current_theme()
                .fg("accent", "x")
                .starts_with("\u{1b}[38;2;18;52;86m"),
            "theme: {}",
            current_theme().fg("accent", "x")
        );

        let updated_source = include_str!("../src/theme/dark.json")
            .replace("\"name\": \"dark\"", "\"name\": \"watched\"")
            .replace("\"accent\": \"#8abeb7\"", "\"accent\": \"#abcdef\"");
        fs::write(&custom_path, updated_source).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if current_theme()
                .fg("accent", "x")
                .starts_with("\u{1b}[38;2;171;205;239m")
            {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        assert_eq!(current_theme_name(), "watched");
        assert!(
            current_theme()
                .fg("accent", "x")
                .starts_with("\u{1b}[38;2;171;205;239m"),
            "theme: {}",
            current_theme().fg("accent", "x")
        );
    })();

    let _ = init_theme(Some("dark"));
    set_registered_themes(Vec::new());
    match previous_colorterm {
        Some(value) => unsafe { std::env::set_var("COLORTERM", value) },
        None => unsafe { std::env::remove_var("COLORTERM") },
    }

    result
}

#[test]
fn theme_helpers_expose_registered_metadata_and_css_colors() {
    let _guard = theme_test_guard();
    let temp_dir = unique_temp_dir("helpers");
    let custom_path = temp_dir.join("indexed.json");
    let custom_path_string = custom_path.to_string_lossy().into_owned();
    let custom_source = include_str!("../src/theme/dark.json")
        .replace("\"name\": \"dark\"", "\"name\": \"indexed\"")
        .replace("\"accent\": \"#8abeb7\"", "\"accent\": \"#808080\"")
        .replace("\"pageBg\": \"#18181e\"", "\"pageBg\": 16");
    fs::write(&custom_path, custom_source).unwrap();

    let theme = load_theme_from_path_with_mode(&custom_path, ColorMode::Ansi256).unwrap();
    assert_eq!(theme.name(), "indexed");
    assert_eq!(theme.inverse("x"), "\u{1b}[7mx\u{1b}[27m");
    assert!(theme.fg("accent", "x").starts_with("\u{1b}[38;5;244m"));
    assert!(theme.get_thinking_border_color(ThinkingLevel::High)("x").contains('x'));
    assert!(theme.get_bash_mode_border_color()("x").contains('x'));

    set_registered_themes(vec![theme.clone()]);
    set_theme_instance(theme.clone());
    assert_eq!(current_theme_name(), "<in-memory>");
    assert_eq!(current_theme().name(), "indexed");

    let available = get_available_themes();
    assert!(available.contains(&String::from("dark")));
    assert!(available.contains(&String::from("light")));
    assert!(available.contains(&String::from("indexed")));

    let theme_info = get_available_themes_with_paths()
        .into_iter()
        .find(|info| info.name == "indexed")
        .unwrap();
    assert_eq!(
        theme_info.path.as_deref(),
        Some(custom_path_string.as_str())
    );

    let resolved = get_resolved_theme_colors(Some("indexed")).unwrap();
    assert_eq!(resolved.get("accent").map(String::as_str), Some("#808080"));
    assert_eq!(resolved.get("text").map(String::as_str), Some("#e5e5e7"));

    let export = get_theme_export_colors(Some("indexed")).unwrap();
    assert_eq!(export.page_bg.as_deref(), Some("#000000"));

    let light_export = get_theme_export_colors(Some("light")).unwrap();
    assert_eq!(light_export.page_bg.as_deref(), Some("#f8f8f8"));
    assert!(is_light_theme(Some("light")));
    assert!(!is_light_theme(Some("indexed")));

    assert!(get_theme_by_name("indexed").is_some());

    let _ = init_theme(Some("dark"));
    set_registered_themes(Vec::new());
}
