use parking_lot::{Mutex, MutexGuard};
use pi_agent::{AgentMessage, ThinkingLevel, Transport};
use pi_coding_agent_core::{SessionEntry, SessionInfo, SessionTreeNode};
use pi_coding_agent_tui::{
    ConfigResourceGroup, ConfigResourceItem, ConfigResourceSubgroup, ConfigResourceType,
    ConfigSelectorComponent, DeliveryMode, DoubleEscapeAction, KeybindingsManager,
    LoginDialogComponent, OAuthProviderItem, OAuthSelectorComponent, OAuthSelectorMode,
    ScopedModelsConfig, ScopedModelsSelectorComponent, SessionSelectorComponent, SettingsChange,
    SettingsConfig, SettingsSelectorComponent, ShowImagesSelectorComponent, ThemeInfo,
    ThemeSelectorComponent, ThinkingSelectorComponent, TreeFilterMode, TreeSelectorComponent,
    filter_and_sort_sessions_list, init_theme, parse_search_query,
};
use pi_events::{Message, Model, ModelCost, UserContent};
use pi_tui::Component;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
    time::{Duration, UNIX_EPOCH},
};

const KEY_DOWN: &str = "\x1b[B";
const KEY_ENTER: &str = "\n";
const KEY_ESCAPE: &str = "\x1b";
const KEY_UP: &str = "\x1b[A";
const CTRL_D: &str = "\x04";
const CTRL_S: &str = "\x13";
const CTRL_T: &str = "\x14";

fn selector_test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock()
}

fn keybindings() -> KeybindingsManager {
    KeybindingsManager::new(BTreeMap::new(), None)
}

fn keybindings_with(overrides: &[(&str, &[&str])]) -> KeybindingsManager {
    let config = overrides
        .iter()
        .map(|(binding, keys)| {
            (
                (*binding).to_owned(),
                keys.iter().map(|key| (*key).into()).collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    KeybindingsManager::new(config, None)
}

fn init_default_theme() {
    let _ = init_theme(None);
}

fn model(provider: &str, id: &str, name: &str) -> Model {
    Model {
        id: id.to_owned(),
        name: name.to_owned(),
        api: String::from("faux"),
        provider: provider.to_owned(),
        base_url: String::from("https://example.com"),
        reasoning: true,
        input: vec![String::from("text")],
        cost: ModelCost::default(),
        context_window: 200_000,
        max_tokens: 8_000,
        compat: None,
    }
}

fn session_info(
    path: &str,
    name: Option<&str>,
    first_message: &str,
    modified_offset_secs: u64,
) -> SessionInfo {
    SessionInfo {
        path: path.to_owned(),
        id: format!("id-{modified_offset_secs}"),
        cwd: String::from("/tmp/project"),
        name: name.map(str::to_owned),
        parent_session_path: None,
        created: UNIX_EPOCH + Duration::from_secs(10),
        modified: UNIX_EPOCH + Duration::from_secs(modified_offset_secs),
        message_count: 3,
        first_message: first_message.to_owned(),
        all_messages_text: format!("{first_message} body"),
    }
}

fn user_entry(id: &str, parent_id: Option<&str>, text: &str) -> SessionEntry {
    SessionEntry::Message {
        id: id.to_owned(),
        parent_id: parent_id.map(str::to_owned),
        timestamp: String::from("2024-01-01T00:00:00Z"),
        message: AgentMessage::from(Message::User {
            content: vec![UserContent::Text {
                text: text.to_owned(),
            }],
            timestamp: 1,
        }),
    }
}

fn tool_entry(id: &str, parent_id: Option<&str>, tool_name: &str) -> SessionEntry {
    SessionEntry::Message {
        id: id.to_owned(),
        parent_id: parent_id.map(str::to_owned),
        timestamp: String::from("2024-01-01T00:00:01Z"),
        message: AgentMessage::from(Message::ToolResult {
            tool_call_id: format!("call-{id}"),
            tool_name: tool_name.to_owned(),
            content: vec![UserContent::Text {
                text: String::from("ok"),
            }],
            details: None,
            is_error: false,
            timestamp: 2,
        }),
    }
}

#[test]
fn thinking_selector_selects_configured_level() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None));
    let mut selector = ThinkingSelectorComponent::new(
        &keybindings,
        ThinkingLevel::Low,
        vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::High],
    );
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |level| *selected.lock() = Some(level));
    }

    selector.handle_input(KEY_DOWN);
    selector.handle_input(KEY_ENTER);

    assert_eq!(*selected.lock(), Some(ThinkingLevel::High));
    assert!(
        selector
            .render(80)
            .join("\n")
            .contains("Select thinking level")
    );
}

#[test]
fn theme_selector_previews_and_selects_theme() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let previewed = Arc::new(Mutex::new(Vec::<String>::new()));
    let selected = Arc::new(Mutex::new(None::<String>));
    let mut selector = ThemeSelectorComponent::new(
        &keybindings,
        "dark",
        vec![
            ThemeInfo {
                name: String::from("dark"),
                path: None,
            },
            ThemeInfo {
                name: String::from("light"),
                path: None,
            },
        ],
    );
    {
        let previewed = Arc::clone(&previewed);
        selector.set_on_preview(move |theme| previewed.lock().push(theme));
    }
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |theme| *selected.lock() = Some(theme));
    }

    selector.handle_input(KEY_DOWN);
    selector.handle_input(KEY_ENTER);

    assert_eq!(previewed.lock().as_slice(), &[String::from("light")]);
    assert_eq!(*selected.lock(), Some(String::from("light")));
}

#[test]
fn show_images_selector_returns_boolean_choice() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None::<bool>));
    let mut selector = ShowImagesSelectorComponent::new(&keybindings, true);
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |value| *selected.lock() = Some(value));
    }

    selector.handle_input(KEY_DOWN);
    selector.handle_input(KEY_ENTER);

    assert_eq!(*selected.lock(), Some(false));
}

#[test]
fn oauth_selector_selects_provider_id() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None::<String>));
    let mut selector = OAuthSelectorComponent::new(
        &keybindings,
        OAuthSelectorMode::Login,
        vec![
            OAuthProviderItem {
                id: String::from("anthropic"),
                name: String::from("Anthropic"),
                logged_in: false,
            },
            OAuthProviderItem {
                id: String::from("openai"),
                name: String::from("OpenAI"),
                logged_in: true,
            },
        ],
    );
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |provider_id| *selected.lock() = Some(provider_id));
    }

    selector.handle_input(KEY_DOWN);
    selector.handle_input(KEY_ENTER);

    assert_eq!(*selected.lock(), Some(String::from("openai")));
}

#[test]
fn scoped_models_selector_toggles_and_persists_models() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let toggles = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let persisted = Arc::new(Mutex::new(None::<Vec<String>>));
    let mut selector = ScopedModelsSelectorComponent::new(
        &keybindings,
        ScopedModelsConfig {
            all_models: vec![
                model("anthropic", "claude", "Claude"),
                model("openai", "gpt-5", "GPT-5"),
            ],
            enabled_model_ids: BTreeSet::from([String::from("anthropic/claude")]),
            has_enabled_models_filter: true,
        },
    );
    {
        let toggles = Arc::clone(&toggles);
        selector.set_on_model_toggle(move |event| toggles.lock().push(event));
    }
    {
        let persisted = Arc::clone(&persisted);
        selector.set_on_persist(move |value| *persisted.lock() = Some(value));
    }

    selector.handle_input(KEY_DOWN);
    selector.handle_input(KEY_ENTER);
    selector.handle_input(CTRL_S);

    assert_eq!(
        toggles.lock().as_slice(),
        &[(String::from("openai/gpt-5"), true)]
    );
    assert_eq!(
        *persisted.lock(),
        Some(vec![
            String::from("anthropic/claude"),
            String::from("openai/gpt-5"),
        ])
    );
}

#[test]
fn session_search_supports_regex_and_phrase_queries() {
    let sessions = vec![
        session_info("/tmp/one.jsonl", Some("Alpha"), "fix node cve", 100),
        session_info("/tmp/two.jsonl", None, "refactor parser", 200),
    ];

    let regex = parse_search_query("re:node\\s+cve");
    let filtered = filter_and_sort_sessions_list(
        &sessions,
        "re:node\\s+cve",
        pi_coding_agent_tui::SessionSortMode::Relevance,
        pi_coding_agent_tui::SessionNameFilter::All,
    );
    assert!(regex.error.is_none(), "{regex:?}");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].path, "/tmp/one.jsonl");

    let phrase_filtered = filter_and_sort_sessions_list(
        &sessions,
        "\"node cve\"",
        pi_coding_agent_tui::SessionSortMode::Relevance,
        pi_coding_agent_tui::SessionNameFilter::All,
    );
    assert_eq!(phrase_filtered.len(), 1);
    assert_eq!(phrase_filtered[0].path, "/tmp/one.jsonl");
}

#[test]
fn session_selector_confirms_delete_for_selected_session() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let deleted = Arc::new(Mutex::new(None::<String>));
    let mut selector = SessionSelectorComponent::new(
        &keybindings,
        vec![session_info(
            "/tmp/current.jsonl",
            Some("Current"),
            "hello",
            100,
        )],
        vec![session_info("/tmp/all.jsonl", Some("Other"), "world", 200)],
        Some(String::from("/tmp/current.jsonl")),
    );
    {
        let deleted = Arc::clone(&deleted);
        selector.set_on_delete(move |path| *deleted.lock() = Some(path));
    }

    selector.handle_input(CTRL_D);
    selector.handle_input(KEY_ENTER);

    assert_eq!(*deleted.lock(), Some(String::from("/tmp/current.jsonl")));
}

#[test]
fn tree_selector_filters_and_selects_entry() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None::<String>));
    let mut selector = TreeSelectorComponent::new(
        &keybindings,
        vec![SessionTreeNode {
            entry: user_entry("user-1", None, "check logs"),
            children: vec![SessionTreeNode {
                entry: tool_entry("tool-1", Some("user-1"), "read"),
                children: Vec::new(),
                label: Some(String::from("tool")),
                label_timestamp: None,
            }],
            label: None,
            label_timestamp: None,
        }],
        Some(String::from("tool-1")),
        None,
        TreeFilterMode::All,
    );
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |entry_id| *selected.lock() = Some(entry_id));
    }

    selector.handle_input("r");
    selector.handle_input("e");
    selector.handle_input("a");
    selector.handle_input("d");
    selector.handle_input(KEY_ENTER);

    assert_eq!(*selected.lock(), Some(String::from("tool-1")));
}

#[test]
fn tree_selector_prioritizes_active_branch_and_supports_filter_shortcuts() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let selected = Arc::new(Mutex::new(None::<String>));
    let mut selector = TreeSelectorComponent::new(
        &keybindings,
        vec![SessionTreeNode {
            entry: user_entry("root-user", None, "root"),
            children: vec![SessionTreeNode {
                entry: SessionEntry::Message {
                    id: String::from("root-assistant"),
                    parent_id: Some(String::from("root-user")),
                    timestamp: String::from("2024-01-01T00:00:01Z"),
                    message: AgentMessage::from(Message::Assistant {
                        content: Vec::new(),
                        api: String::from("faux:test"),
                        provider: String::from("faux"),
                        model: String::from("model"),
                        response_id: None,
                        usage: Default::default(),
                        stop_reason: pi_events::StopReason::ToolUse,
                        error_message: None,
                        timestamp: 2,
                    }),
                },
                children: vec![
                    SessionTreeNode {
                        entry: user_entry("primary", Some("root-assistant"), "primary branch"),
                        children: Vec::new(),
                        label: None,
                        label_timestamp: None,
                    },
                    SessionTreeNode {
                        entry: user_entry("active", Some("root-assistant"), "active branch"),
                        children: Vec::new(),
                        label: None,
                        label_timestamp: None,
                    },
                ],
                label: None,
                label_timestamp: None,
            }],
            label: None,
            label_timestamp: None,
        }],
        Some(String::from("active")),
        None,
        TreeFilterMode::All,
    );
    {
        let selected = Arc::clone(&selected);
        selector.set_on_select(move |entry_id| *selected.lock() = Some(entry_id));
    }

    selector.handle_input(KEY_UP);
    selector.handle_input(KEY_ENTER);

    assert_eq!(*selected.lock(), Some(String::from("root-assistant")));

    let mut selector = TreeSelectorComponent::new(
        &keybindings,
        vec![SessionTreeNode {
            entry: user_entry("user-1", None, "check logs"),
            children: vec![SessionTreeNode {
                entry: tool_entry("tool-1", Some("user-1"), "read"),
                children: Vec::new(),
                label: None,
                label_timestamp: None,
            }],
            label: None,
            label_timestamp: None,
        }],
        Some(String::from("tool-1")),
        None,
        TreeFilterMode::All,
    );

    assert!(selector.render(80).join("\n").contains("tool read ok"));
    selector.handle_input(CTRL_T);
    assert!(!selector.render(80).join("\n").contains("tool read ok"));
}

#[test]
fn tree_selector_edits_labels_with_configurable_keybinding() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings_with(&[("app.tree.editLabel", &["ctrl+r"])]);
    let changes = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let mut selector = TreeSelectorComponent::new(
        &keybindings,
        vec![SessionTreeNode {
            entry: user_entry("user-1", None, "check logs"),
            children: Vec::new(),
            label: None,
            label_timestamp: None,
        }],
        Some(String::from("user-1")),
        None,
        TreeFilterMode::All,
    );
    {
        let changes = Arc::clone(&changes);
        selector.set_on_label_change(move |change| changes.lock().push(change));
    }

    selector.handle_input("\x12");
    selector.handle_input("i");
    selector.handle_input("m");
    selector.handle_input("p");
    selector.handle_input("o");
    selector.handle_input("r");
    selector.handle_input("t");
    selector.handle_input("a");
    selector.handle_input("n");
    selector.handle_input("t");
    selector.handle_input(KEY_ENTER);

    assert_eq!(
        changes.lock().as_slice(),
        &[(String::from("user-1"), Some(String::from("important")))]
    );
    assert!(selector.render(80).join("\n").contains("[important]"));
}

#[test]
fn settings_selector_toggles_boolean_and_restores_theme_preview_on_cancel() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let changes = Arc::new(Mutex::new(Vec::<SettingsChange>::new()));
    let previews = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut selector = SettingsSelectorComponent::new(
        &keybindings,
        SettingsConfig {
            supports_images: false,
            auto_compact: false,
            show_images: false,
            auto_resize_images: true,
            block_images: false,
            enable_skill_commands: true,
            steering_mode: DeliveryMode::OneAtATime,
            follow_up_mode: DeliveryMode::All,
            transport: Transport::Sse,
            thinking_level: ThinkingLevel::Low,
            available_thinking_levels: vec![ThinkingLevel::Off, ThinkingLevel::Low],
            current_theme: String::from("dark"),
            available_themes: vec![String::from("dark"), String::from("light")],
            hide_thinking_block: false,
            double_escape_action: DoubleEscapeAction::Tree,
            tree_filter_mode: TreeFilterMode::Default,
            show_hardware_cursor: false,
            editor_padding_x: 0,
            autocomplete_max_visible: 5,
            quiet_startup: false,
            clear_on_shrink: false,
        },
    );
    {
        let changes = Arc::clone(&changes);
        selector.set_on_change(move |change| changes.lock().push(change));
    }
    {
        let previews = Arc::clone(&previews);
        selector.set_on_theme_preview(move |theme| previews.lock().push(theme));
    }

    selector.handle_input(KEY_ENTER);
    for _ in 0..8 {
        selector.handle_input(KEY_DOWN);
    }
    selector.handle_input(KEY_ENTER);
    selector.handle_input(KEY_DOWN);
    selector.handle_input(KEY_ESCAPE);

    assert_eq!(
        changes.lock().as_slice(),
        &[SettingsChange::AutoCompact(true)]
    );
    assert_eq!(
        previews.lock().as_slice(),
        &[String::from("light"), String::from("dark")]
    );
}

#[test]
fn config_selector_toggles_selected_resource() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let toggled = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
    let mut selector = ConfigSelectorComponent::new(
        &keybindings,
        vec![ConfigResourceGroup {
            label: String::from("User"),
            subgroups: vec![ConfigResourceSubgroup {
                label: String::from("Themes"),
                items: vec![ConfigResourceItem {
                    id: String::from("theme:dark"),
                    path: String::from("/tmp/dark.json"),
                    display_name: String::from("dark.json"),
                    enabled: false,
                    resource_type: ConfigResourceType::Themes,
                }],
            }],
        }],
    );
    {
        let toggled = Arc::clone(&toggled);
        selector.set_on_toggle(move |event| toggled.lock().push(event));
    }

    selector.handle_input(KEY_ENTER);

    assert_eq!(
        toggled.lock().as_slice(),
        &[(String::from("theme:dark"), true)]
    );
}

#[test]
fn login_dialog_submits_prompt_value() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let submitted = Arc::new(Mutex::new(None::<String>));
    let mut dialog = LoginDialogComponent::new(&keybindings, "Anthropic");
    {
        let submitted = Arc::clone(&submitted);
        dialog.set_on_submit(move |value| *submitted.lock() = Some(value));
    }
    dialog.show_prompt("Paste verification code", Some("abc123"));

    dialog.handle_input("a");
    dialog.handle_input("b");
    dialog.handle_input("c");
    dialog.handle_input(KEY_ENTER);

    assert_eq!(*submitted.lock(), Some(String::from("abc")));
    assert!(dialog.render(80).join("\n").contains("Login to Anthropic"));
}

#[test]
fn login_dialog_preserves_auth_url_when_prompting_for_manual_input() {
    let _guard = selector_test_guard();
    init_default_theme();
    let keybindings = keybindings();
    let mut dialog = LoginDialogComponent::new(&keybindings, "Anthropic");

    dialog.show_auth(
        "https://example.com/login",
        Some("Complete login in your browser."),
    );
    dialog.show_manual_input("Paste redirect URL below, or complete login in browser:");
    dialog.show_progress("Exchanging authorization code for tokens...");

    let rendered = dialog.render(80).join("\n");
    assert!(
        rendered.contains("https://example.com/login"),
        "output: {rendered}"
    );
    assert!(
        rendered.contains("Complete login in your browser."),
        "output: {rendered}"
    );
    assert!(
        rendered.contains("Paste redirect URL below, or complete login in browser:"),
        "output: {rendered}"
    );
    assert!(
        rendered.contains("Exchanging authorization code for tokens..."),
        "output: {rendered}"
    );
}
