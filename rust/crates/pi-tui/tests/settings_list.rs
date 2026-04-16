use pi_tui::{
    Component, SettingItem, SettingsList, SettingsListOptions, SettingsListTheme,
    SettingsSubmenuDone,
};
use std::sync::{Arc, Mutex};

const KEY_ENTER: &str = "\n";
const KEY_ESCAPE: &str = "\x1b";

fn themed_settings_list(items: Vec<SettingItem>, options: SettingsListOptions) -> SettingsList {
    SettingsList::with_keybindings(
        pi_tui::KeybindingsManager::with_tui_defaults(std::collections::BTreeMap::new()),
        items,
        5,
        SettingsListTheme::new()
            .with_label(|text, selected| {
                if selected {
                    format!("[{text}]")
                } else {
                    text.to_owned()
                }
            })
            .with_value(|text, selected| {
                if selected {
                    format!("<{text}>")
                } else {
                    text.to_owned()
                }
            })
            .with_description(|text| format!("({text})"))
            .with_cursor("=> ")
            .with_hint(|text| format!("!{text}!")),
        options,
    )
}

struct TestSubmenu {
    current_value: String,
    done: Option<SettingsSubmenuDone>,
}

impl TestSubmenu {
    fn new(current_value: String, done: SettingsSubmenuDone) -> Self {
        Self {
            current_value,
            done: Some(done),
        }
    }
}

impl Component for TestSubmenu {
    fn render(&self, _width: usize) -> Vec<String> {
        vec![format!("submenu:{}", self.current_value)]
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        if data == KEY_ENTER {
            if let Some(done) = &mut self.done {
                done(Some(String::from("manual")));
            }
        } else if data == KEY_ESCAPE
            && let Some(done) = &mut self.done
        {
            done(None);
        }
    }
}

#[test]
fn settings_list_cycles_values_and_emits_change() {
    let changes = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let mut list = themed_settings_list(
        vec![SettingItem {
            id: String::from("theme"),
            label: String::from("Theme"),
            description: Some(String::from("Current color theme")),
            current_value: String::from("dark"),
            values: Some(vec![String::from("dark"), String::from("light")]),
            submenu: None,
        }],
        SettingsListOptions::default(),
    );
    {
        let changes = Arc::clone(&changes);
        list.set_on_change(move |id, value| {
            changes.lock().expect("changes mutex").push((id, value))
        });
    }

    list.handle_input(" ");
    let lines = list.render(80);

    assert!(
        lines.iter().any(|line| line.contains("light")),
        "lines: {lines:?}"
    );
    assert_eq!(
        changes.lock().expect("changes mutex").as_slice(),
        &[(String::from("theme"), String::from("light"))]
    );
}

#[test]
fn settings_list_search_filters_items_and_keeps_selected_description() {
    let mut list = themed_settings_list(
        vec![
            SettingItem {
                id: String::from("theme"),
                label: String::from("Theme"),
                description: Some(String::from("Select the active theme")),
                current_value: String::from("dark"),
                values: Some(vec![String::from("dark"), String::from("light")]),
                submenu: None,
            },
            SettingItem {
                id: String::from("transport"),
                label: String::from("Transport"),
                description: Some(String::from("Choose the preferred transport")),
                current_value: String::from("sse"),
                values: Some(vec![String::from("sse"), String::from("websocket")]),
                submenu: None,
            },
        ],
        SettingsListOptions {
            enable_search: true,
        },
    );

    list.set_focused(true);
    list.handle_input("t");
    list.handle_input("h");
    let lines = list.render(80);

    assert!(
        lines.iter().any(|line| line.contains("Theme")),
        "lines: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("Transport")),
        "lines: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Select the active theme")),
        "lines: {lines:?}"
    );
}

#[test]
fn settings_list_submenu_updates_value_and_returns_to_main_list() {
    let changes = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let mut list = themed_settings_list(
        vec![SettingItem {
            id: String::from("editor"),
            label: String::from("Editor"),
            description: Some(String::from("Open submenu")),
            current_value: String::from("default"),
            values: None,
            submenu: Some(Box::new(|current_value, done| {
                Box::new(TestSubmenu::new(current_value, done))
            })),
        }],
        SettingsListOptions::default(),
    );
    {
        let changes = Arc::clone(&changes);
        list.set_on_change(move |id, value| {
            changes.lock().expect("changes mutex").push((id, value))
        });
    }

    list.handle_input(KEY_ENTER);
    assert_eq!(list.render(80), vec![String::from("submenu:default")]);

    list.handle_input(KEY_ENTER);
    let lines = list.render(80);

    assert!(
        lines.iter().any(|line| line.contains("manual")),
        "lines: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Open submenu")),
        "lines: {lines:?}"
    );
    assert_eq!(
        changes.lock().expect("changes mutex").as_slice(),
        &[(String::from("editor"), String::from("manual"))]
    );
}
