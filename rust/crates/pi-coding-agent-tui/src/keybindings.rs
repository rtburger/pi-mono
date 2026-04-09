use pi_tui::{
    KeyId, KeybindingDefinition, KeybindingsConfig, KeybindingsManager as TuiKeybindingsManager,
    TUI_KEYBINDINGS,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::LazyLock,
};

const KEYBINDINGS_FILE_NAME: &str = "keybindings.json";

pub static DEFAULT_APP_KEYBINDINGS: LazyLock<Vec<(String, KeybindingDefinition)>> =
    LazyLock::new(|| {
        let mut keybindings = TUI_KEYBINDINGS.iter().cloned().collect::<Vec<_>>();
        keybindings.extend([
            (
                "app.interrupt".to_owned(),
                definition(&["escape"], "Cancel or abort"),
            ),
            (
                "app.clear".to_owned(),
                definition(&["ctrl+c"], "Clear editor"),
            ),
            (
                "app.exit".to_owned(),
                definition(&["ctrl+d"], "Exit when editor is empty"),
            ),
            (
                "app.suspend".to_owned(),
                definition(&["ctrl+z"], "Suspend to background"),
            ),
            (
                "app.thinking.cycle".to_owned(),
                definition(&["shift+tab"], "Cycle thinking level"),
            ),
            (
                "app.model.cycleForward".to_owned(),
                definition(&["ctrl+p"], "Cycle to next model"),
            ),
            (
                "app.model.cycleBackward".to_owned(),
                definition(&["shift+ctrl+p"], "Cycle to previous model"),
            ),
            (
                "app.model.select".to_owned(),
                definition(&["ctrl+l"], "Open model selector"),
            ),
            (
                "app.tools.expand".to_owned(),
                definition(&["ctrl+o"], "Toggle tool output"),
            ),
            (
                "app.thinking.toggle".to_owned(),
                definition(&["ctrl+t"], "Toggle thinking blocks"),
            ),
            (
                "app.session.toggleNamedFilter".to_owned(),
                definition(&["ctrl+n"], "Toggle named session filter"),
            ),
            (
                "app.editor.external".to_owned(),
                definition(&["ctrl+g"], "Open external editor"),
            ),
            (
                "app.message.followUp".to_owned(),
                definition(&["alt+enter"], "Queue follow-up message"),
            ),
            (
                "app.message.dequeue".to_owned(),
                definition(&["alt+up"], "Restore queued messages"),
            ),
            (
                "app.clipboard.pasteImage".to_owned(),
                definition(&[default_paste_image_key()], "Paste image from clipboard"),
            ),
            (
                "app.session.new".to_owned(),
                definition(&[], "Start a new session"),
            ),
            (
                "app.session.tree".to_owned(),
                definition(&[], "Open session tree"),
            ),
            (
                "app.session.fork".to_owned(),
                definition(&[], "Fork current session"),
            ),
            (
                "app.session.resume".to_owned(),
                definition(&[], "Resume a session"),
            ),
            (
                "app.tree.foldOrUp".to_owned(),
                definition(&["ctrl+left", "alt+left"], "Fold tree branch or move up"),
            ),
            (
                "app.tree.unfoldOrDown".to_owned(),
                definition(
                    &["ctrl+right", "alt+right"],
                    "Unfold tree branch or move down",
                ),
            ),
            (
                "app.tree.editLabel".to_owned(),
                definition(&["shift+l"], "Edit tree label"),
            ),
            (
                "app.tree.toggleLabelTimestamp".to_owned(),
                definition(&["shift+t"], "Toggle tree label timestamps"),
            ),
            (
                "app.session.togglePath".to_owned(),
                definition(&["ctrl+p"], "Toggle session path display"),
            ),
            (
                "app.session.toggleSort".to_owned(),
                definition(&["ctrl+s"], "Toggle session sort mode"),
            ),
            (
                "app.session.rename".to_owned(),
                definition(&["ctrl+r"], "Rename session"),
            ),
            (
                "app.session.delete".to_owned(),
                definition(&["ctrl+d"], "Delete session"),
            ),
            (
                "app.session.deleteNoninvasive".to_owned(),
                definition(&["ctrl+backspace"], "Delete session when query is empty"),
            ),
        ]);
        keybindings
    });

static LEGACY_KEYBINDING_NAME_MIGRATIONS: LazyLock<BTreeMap<&'static str, &'static str>> =
    LazyLock::new(|| {
        BTreeMap::from([
            ("cursorUp", "tui.editor.cursorUp"),
            ("cursorDown", "tui.editor.cursorDown"),
            ("cursorLeft", "tui.editor.cursorLeft"),
            ("cursorRight", "tui.editor.cursorRight"),
            ("cursorWordLeft", "tui.editor.cursorWordLeft"),
            ("cursorWordRight", "tui.editor.cursorWordRight"),
            ("cursorLineStart", "tui.editor.cursorLineStart"),
            ("cursorLineEnd", "tui.editor.cursorLineEnd"),
            ("jumpForward", "tui.editor.jumpForward"),
            ("jumpBackward", "tui.editor.jumpBackward"),
            ("pageUp", "tui.editor.pageUp"),
            ("pageDown", "tui.editor.pageDown"),
            ("deleteCharBackward", "tui.editor.deleteCharBackward"),
            ("deleteCharForward", "tui.editor.deleteCharForward"),
            ("deleteWordBackward", "tui.editor.deleteWordBackward"),
            ("deleteWordForward", "tui.editor.deleteWordForward"),
            ("deleteToLineStart", "tui.editor.deleteToLineStart"),
            ("deleteToLineEnd", "tui.editor.deleteToLineEnd"),
            ("yank", "tui.editor.yank"),
            ("yankPop", "tui.editor.yankPop"),
            ("undo", "tui.editor.undo"),
            ("newLine", "tui.input.newLine"),
            ("submit", "tui.input.submit"),
            ("tab", "tui.input.tab"),
            ("copy", "tui.input.copy"),
            ("selectUp", "tui.select.up"),
            ("selectDown", "tui.select.down"),
            ("selectPageUp", "tui.select.pageUp"),
            ("selectPageDown", "tui.select.pageDown"),
            ("selectConfirm", "tui.select.confirm"),
            ("selectCancel", "tui.select.cancel"),
            ("interrupt", "app.interrupt"),
            ("clear", "app.clear"),
            ("exit", "app.exit"),
            ("suspend", "app.suspend"),
            ("cycleThinkingLevel", "app.thinking.cycle"),
            ("cycleModelForward", "app.model.cycleForward"),
            ("cycleModelBackward", "app.model.cycleBackward"),
            ("selectModel", "app.model.select"),
            ("expandTools", "app.tools.expand"),
            ("toggleThinking", "app.thinking.toggle"),
            ("toggleSessionNamedFilter", "app.session.toggleNamedFilter"),
            ("externalEditor", "app.editor.external"),
            ("followUp", "app.message.followUp"),
            ("dequeue", "app.message.dequeue"),
            ("pasteImage", "app.clipboard.pasteImage"),
            ("newSession", "app.session.new"),
            ("tree", "app.session.tree"),
            ("fork", "app.session.fork"),
            ("resume", "app.session.resume"),
            ("treeFoldOrUp", "app.tree.foldOrUp"),
            ("treeUnfoldOrDown", "app.tree.unfoldOrDown"),
            ("treeEditLabel", "app.tree.editLabel"),
            ("treeToggleLabelTimestamp", "app.tree.toggleLabelTimestamp"),
            ("toggleSessionPath", "app.session.togglePath"),
            ("toggleSessionSort", "app.session.toggleSort"),
            ("renameSession", "app.session.rename"),
            ("deleteSession", "app.session.delete"),
            ("deleteSessionNoninvasive", "app.session.deleteNoninvasive"),
        ])
    });

#[derive(Debug, Clone, PartialEq)]
pub struct MigrateKeybindingsConfigResult {
    pub config: Map<String, Value>,
    pub migrated: bool,
}

#[derive(Debug, Clone)]
pub struct KeybindingsManager {
    inner: TuiKeybindingsManager,
    config_path: Option<PathBuf>,
}

impl KeybindingsManager {
    pub fn new(user_bindings: KeybindingsConfig, config_path: Option<PathBuf>) -> Self {
        Self {
            inner: TuiKeybindingsManager::new(DEFAULT_APP_KEYBINDINGS.as_slice(), user_bindings),
            config_path,
        }
    }

    pub fn create(agent_dir: impl AsRef<Path>) -> Self {
        let config_path = agent_dir.as_ref().join(KEYBINDINGS_FILE_NAME);
        let user_bindings = load_from_file(&config_path);
        Self::new(user_bindings, Some(config_path))
    }

    pub fn reload(&mut self) {
        let Some(config_path) = self.config_path.as_ref() else {
            return;
        };
        self.inner.set_user_bindings(load_from_file(config_path));
    }

    pub fn get_effective_config(&self) -> KeybindingsConfig {
        self.inner.get_resolved_bindings()
    }
}

impl Deref for KeybindingsManager {
    type Target = TuiKeybindingsManager;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for KeybindingsManager {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

pub fn migrate_keybindings_config(
    raw_config: Map<String, Value>,
) -> MigrateKeybindingsConfigResult {
    let existing_keys = raw_config.keys().cloned().collect::<BTreeSet<_>>();
    let mut config = Map::new();
    let mut migrated = false;

    for (key, value) in raw_config {
        let next_key = legacy_keybinding_name(key.as_str()).unwrap_or(key.as_str());
        if next_key != key {
            migrated = true;
        }
        if next_key != key && existing_keys.contains(next_key) {
            migrated = true;
            continue;
        }
        config.insert(next_key.to_owned(), value);
    }

    MigrateKeybindingsConfigResult { config, migrated }
}

pub fn migrate_keybindings_file(path: impl AsRef<Path>) -> io::Result<bool> {
    let Some(raw_config) = load_raw_config(path.as_ref()) else {
        return Ok(false);
    };

    let migrated = migrate_keybindings_config(raw_config);
    if !migrated.migrated {
        return Ok(false);
    }

    write_ordered_config(path.as_ref(), &migrated.config)?;
    Ok(true)
}

fn load_from_file(path: &Path) -> KeybindingsConfig {
    let Some(raw_config) = load_raw_config(path) else {
        return KeybindingsConfig::default();
    };

    let migrated = migrate_keybindings_config(raw_config);
    to_keybindings_config(&migrated.config)
}

fn load_raw_config(path: &Path) -> Option<Map<String, Value>> {
    let raw = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&raw).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

fn to_keybindings_config(raw_config: &Map<String, Value>) -> KeybindingsConfig {
    let mut config = BTreeMap::new();

    for (key, value) in raw_config {
        if let Some(keys) = raw_value_to_key_ids(value) {
            config.insert(key.clone(), keys);
        }
    }

    config
}

fn raw_value_to_key_ids(value: &Value) -> Option<Vec<KeyId>> {
    match value {
        Value::String(key) => Some(vec![KeyId::from(key.clone())]),
        Value::Array(keys) => keys
            .iter()
            .map(|entry| match entry {
                Value::String(key) => Some(KeyId::from(key.clone())),
                _ => None,
            })
            .collect::<Option<Vec<_>>>(),
        _ => None,
    }
}

fn write_ordered_config(path: &Path, config: &Map<String, Value>) -> io::Result<()> {
    let entries = ordered_entries(config);
    if entries.is_empty() {
        return fs::write(path, "{}\n");
    }

    let mut rendered = String::from("{\n");
    for (index, (key, value)) in entries.iter().enumerate() {
        rendered.push_str("  ");
        rendered.push_str(&json_to_string(key)?);
        rendered.push_str(": ");
        rendered.push_str(&indent_json_value(value)?);
        if index + 1 != entries.len() {
            rendered.push(',');
        }
        rendered.push('\n');
    }
    rendered.push_str("}\n");

    fs::write(path, rendered)
}

fn ordered_entries<'a>(config: &'a Map<String, Value>) -> Vec<(String, &'a Value)> {
    let mut entries = Vec::new();

    for (keybinding, _) in DEFAULT_APP_KEYBINDINGS.iter() {
        if let Some(value) = config.get(keybinding) {
            entries.push((keybinding.clone(), value));
        }
    }

    let mut extras = config
        .iter()
        .filter(|(key, _)| {
            !DEFAULT_APP_KEYBINDINGS
                .iter()
                .any(|(known_key, _)| known_key == *key)
        })
        .map(|(key, value)| (key.clone(), value))
        .collect::<Vec<_>>();
    extras.sort_by(|left, right| left.0.cmp(&right.0));
    entries.extend(extras);

    entries
}

fn indent_json_value(value: &Value) -> io::Result<String> {
    let pretty = json_to_string_pretty(value)?;
    let mut lines = pretty.lines();
    let Some(first_line) = lines.next() else {
        return Ok(String::new());
    };

    let mut rendered = first_line.to_owned();
    for line in lines {
        rendered.push('\n');
        rendered.push_str("  ");
        rendered.push_str(line);
    }

    Ok(rendered)
}

fn json_to_string(value: &str) -> io::Result<String> {
    serde_json::to_string(value).map_err(json_to_io_error)
}

fn json_to_string_pretty(value: &Value) -> io::Result<String> {
    serde_json::to_string_pretty(value).map_err(json_to_io_error)
}

fn json_to_io_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn legacy_keybinding_name(key: &str) -> Option<&'static str> {
    LEGACY_KEYBINDING_NAME_MIGRATIONS.get(key).copied()
}

fn definition(default_keys: &[&str], description: &str) -> KeybindingDefinition {
    KeybindingDefinition::new(default_keys.iter().copied().map(KeyId::from))
        .with_description(description)
}

fn default_paste_image_key() -> &'static str {
    if cfg!(windows) { "alt+v" } else { "ctrl+v" }
}
