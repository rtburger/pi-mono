use pi_coding_agent_tui::{KeyId, KeybindingsManager, migrate_keybindings_file};
use serde_json::json;
use std::fs;
use tempfile::tempdir;

fn write_keybindings_file(agent_dir: &std::path::Path, config: serde_json::Value) {
    let path = agent_dir.join("keybindings.json");
    let content = format!("{}\n", serde_json::to_string_pretty(&config).unwrap());
    fs::write(path, content).unwrap();
}

fn read_keybindings_file(agent_dir: &std::path::Path) -> serde_json::Value {
    let path = agent_dir.join("keybindings.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn rewrites_old_key_names_to_namespaced_ids() {
    let agent_dir = tempdir().unwrap();
    write_keybindings_file(
        agent_dir.path(),
        json!({
            "cursorUp": ["up", "ctrl+p"],
            "expandTools": "ctrl+x"
        }),
    );

    assert!(migrate_keybindings_file(agent_dir.path().join("keybindings.json")).unwrap());

    assert_eq!(
        read_keybindings_file(agent_dir.path()),
        json!({
            "tui.editor.cursorUp": ["up", "ctrl+p"],
            "app.tools.expand": "ctrl+x"
        })
    );
}

#[test]
fn keeps_namespaced_value_when_old_and_new_names_both_exist() {
    let agent_dir = tempdir().unwrap();
    write_keybindings_file(
        agent_dir.path(),
        json!({
            "expandTools": "ctrl+x",
            "app.tools.expand": "ctrl+y"
        }),
    );

    assert!(migrate_keybindings_file(agent_dir.path().join("keybindings.json")).unwrap());

    assert_eq!(
        read_keybindings_file(agent_dir.path()),
        json!({
            "app.tools.expand": "ctrl+y"
        })
    );
}

#[test]
fn loads_old_key_names_in_memory_before_file_is_rewritten() {
    let agent_dir = tempdir().unwrap();
    write_keybindings_file(
        agent_dir.path(),
        json!({
            "selectConfirm": "enter",
            "interrupt": "ctrl+x"
        }),
    );

    let keybindings = KeybindingsManager::create(agent_dir.path());

    assert_eq!(
        keybindings.get_user_bindings(),
        [
            ("app.interrupt".to_owned(), vec![KeyId::from("ctrl+x")],),
            ("tui.select.confirm".to_owned(), vec![KeyId::from("enter")],),
        ]
        .into_iter()
        .collect()
    );

    let effective = keybindings.get_effective_config();
    assert_eq!(
        effective.get("tui.select.confirm"),
        Some(&vec![KeyId::from("enter")])
    );
    assert_eq!(
        effective.get("app.interrupt"),
        Some(&vec![KeyId::from("ctrl+x")])
    );
}

#[test]
fn migrated_file_keeps_known_bindings_in_default_order_before_extras() {
    let agent_dir = tempdir().unwrap();
    write_keybindings_file(
        agent_dir.path(),
        json!({
            "z.extra": "ctrl+z",
            "toggleThinking": "ctrl+t",
            "cursorUp": "up"
        }),
    );

    migrate_keybindings_file(agent_dir.path().join("keybindings.json")).unwrap();

    let content = fs::read_to_string(agent_dir.path().join("keybindings.json")).unwrap();
    let cursor_up = content.find("\"tui.editor.cursorUp\"").unwrap();
    let toggle_thinking = content.find("\"app.thinking.toggle\"").unwrap();
    let extra = content.find("\"z.extra\"").unwrap();

    assert!(cursor_up < toggle_thinking);
    assert!(toggle_thinking < extra);
}
