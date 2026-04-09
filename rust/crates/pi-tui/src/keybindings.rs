use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt,
    sync::LazyLock,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(String);

impl KeyId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for KeyId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for KeyId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for KeyId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingDefinition {
    pub default_keys: Vec<KeyId>,
    pub description: Option<String>,
}

impl KeybindingDefinition {
    pub fn new(default_keys: impl IntoIterator<Item = KeyId>) -> Self {
        Self {
            default_keys: default_keys.into_iter().collect(),
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

pub type KeybindingsConfig = BTreeMap<String, Vec<KeyId>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingConflict {
    pub key: KeyId,
    pub keybindings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct KeybindingsManager {
    definition_order: Vec<String>,
    definitions: HashMap<String, KeybindingDefinition>,
    user_bindings: KeybindingsConfig,
    keys_by_id: HashMap<String, Vec<KeyId>>,
    conflicts: Vec<KeybindingConflict>,
}

impl KeybindingsManager {
    pub fn new(
        definitions: &[(String, KeybindingDefinition)],
        user_bindings: KeybindingsConfig,
    ) -> Self {
        let definition_order = definitions
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let definitions = definitions
            .iter()
            .map(|(id, definition)| (id.clone(), definition.clone()))
            .collect::<HashMap<_, _>>();

        let mut manager = Self {
            definition_order,
            definitions,
            user_bindings,
            keys_by_id: HashMap::new(),
            conflicts: Vec::new(),
        };
        manager.rebuild();
        manager
    }

    pub fn with_tui_defaults(user_bindings: KeybindingsConfig) -> Self {
        Self::new(TUI_KEYBINDINGS.as_slice(), user_bindings)
    }

    pub fn matches(&self, key: &KeyId, keybinding: &str) -> bool {
        self.keys_by_id
            .get(keybinding)
            .is_some_and(|keys| keys.iter().any(|candidate| candidate == key))
    }

    pub fn get_keys(&self, keybinding: &str) -> Vec<KeyId> {
        self.keys_by_id.get(keybinding).cloned().unwrap_or_default()
    }

    pub fn get_definition(&self, keybinding: &str) -> Option<&KeybindingDefinition> {
        self.definitions.get(keybinding)
    }

    pub fn get_conflicts(&self) -> Vec<KeybindingConflict> {
        self.conflicts.clone()
    }

    pub fn set_user_bindings(&mut self, user_bindings: KeybindingsConfig) {
        self.user_bindings = user_bindings;
        self.rebuild();
    }

    pub fn get_user_bindings(&self) -> KeybindingsConfig {
        self.user_bindings.clone()
    }

    pub fn get_resolved_bindings(&self) -> KeybindingsConfig {
        self.definition_order
            .iter()
            .map(|id| (id.clone(), self.get_keys(id)))
            .collect()
    }

    fn rebuild(&mut self) {
        self.keys_by_id.clear();
        self.conflicts.clear();

        let mut user_claims = BTreeMap::<KeyId, BTreeSet<String>>::new();
        for (keybinding, keys) in &self.user_bindings {
            if !self.definitions.contains_key(keybinding) {
                continue;
            }
            for key in normalize_keys(keys.iter().cloned()) {
                user_claims
                    .entry(key)
                    .or_default()
                    .insert(keybinding.clone());
            }
        }

        self.conflicts = user_claims
            .into_iter()
            .filter_map(|(key, keybindings)| {
                if keybindings.len() <= 1 {
                    return None;
                }
                Some(KeybindingConflict {
                    key,
                    keybindings: keybindings.into_iter().collect(),
                })
            })
            .collect();

        for keybinding in &self.definition_order {
            let Some(definition) = self.definitions.get(keybinding) else {
                continue;
            };
            let keys = self
                .user_bindings
                .get(keybinding)
                .cloned()
                .unwrap_or_else(|| definition.default_keys.clone());
            self.keys_by_id
                .insert(keybinding.clone(), normalize_keys(keys));
        }
    }
}

fn normalize_keys(keys: impl IntoIterator<Item = KeyId>) -> Vec<KeyId> {
    let mut seen = HashSet::<KeyId>::new();
    let mut normalized = Vec::new();

    for key in keys {
        if seen.insert(key.clone()) {
            normalized.push(key);
        }
    }

    normalized
}

fn definition(default_keys: &[&str], description: &str) -> KeybindingDefinition {
    KeybindingDefinition::new(default_keys.iter().copied().map(KeyId::from))
        .with_description(description)
}

pub static TUI_KEYBINDINGS: LazyLock<Vec<(String, KeybindingDefinition)>> = LazyLock::new(|| {
    vec![
        (
            "tui.editor.cursorUp".to_owned(),
            definition(&["up"], "Move cursor up"),
        ),
        (
            "tui.editor.cursorDown".to_owned(),
            definition(&["down"], "Move cursor down"),
        ),
        (
            "tui.editor.cursorLeft".to_owned(),
            definition(&["left", "ctrl+b"], "Move cursor left"),
        ),
        (
            "tui.editor.cursorRight".to_owned(),
            definition(&["right", "ctrl+f"], "Move cursor right"),
        ),
        (
            "tui.editor.cursorWordLeft".to_owned(),
            definition(&["alt+left", "ctrl+left", "alt+b"], "Move cursor word left"),
        ),
        (
            "tui.editor.cursorWordRight".to_owned(),
            definition(
                &["alt+right", "ctrl+right", "alt+f"],
                "Move cursor word right",
            ),
        ),
        (
            "tui.editor.cursorLineStart".to_owned(),
            definition(&["home", "ctrl+a"], "Move to line start"),
        ),
        (
            "tui.editor.cursorLineEnd".to_owned(),
            definition(&["end", "ctrl+e"], "Move to line end"),
        ),
        (
            "tui.editor.jumpForward".to_owned(),
            definition(&["ctrl+]"], "Jump forward to character"),
        ),
        (
            "tui.editor.jumpBackward".to_owned(),
            definition(&["ctrl+alt+]"], "Jump backward to character"),
        ),
        (
            "tui.editor.pageUp".to_owned(),
            definition(&["pageUp"], "Page up"),
        ),
        (
            "tui.editor.pageDown".to_owned(),
            definition(&["pageDown"], "Page down"),
        ),
        (
            "tui.editor.deleteCharBackward".to_owned(),
            definition(&["backspace"], "Delete character backward"),
        ),
        (
            "tui.editor.deleteCharForward".to_owned(),
            definition(&["delete", "ctrl+d"], "Delete character forward"),
        ),
        (
            "tui.editor.deleteWordBackward".to_owned(),
            definition(&["ctrl+w", "alt+backspace"], "Delete word backward"),
        ),
        (
            "tui.editor.deleteWordForward".to_owned(),
            definition(&["alt+d", "alt+delete"], "Delete word forward"),
        ),
        (
            "tui.editor.deleteToLineStart".to_owned(),
            definition(&["ctrl+u"], "Delete to line start"),
        ),
        (
            "tui.editor.deleteToLineEnd".to_owned(),
            definition(&["ctrl+k"], "Delete to line end"),
        ),
        (
            "tui.editor.yank".to_owned(),
            definition(&["ctrl+y"], "Yank"),
        ),
        (
            "tui.editor.yankPop".to_owned(),
            definition(&["alt+y"], "Yank pop"),
        ),
        (
            "tui.editor.undo".to_owned(),
            definition(&["ctrl+-"], "Undo"),
        ),
        (
            "tui.input.newLine".to_owned(),
            definition(&["shift+enter"], "Insert newline"),
        ),
        (
            "tui.input.submit".to_owned(),
            definition(&["enter"], "Submit input"),
        ),
        (
            "tui.input.tab".to_owned(),
            definition(&["tab"], "Tab / autocomplete"),
        ),
        (
            "tui.input.copy".to_owned(),
            definition(&["ctrl+c"], "Copy selection"),
        ),
        (
            "tui.select.up".to_owned(),
            definition(&["up"], "Move selection up"),
        ),
        (
            "tui.select.down".to_owned(),
            definition(&["down"], "Move selection down"),
        ),
        (
            "tui.select.pageUp".to_owned(),
            definition(&["pageUp"], "Selection page up"),
        ),
        (
            "tui.select.pageDown".to_owned(),
            definition(&["pageDown"], "Selection page down"),
        ),
        (
            "tui.select.confirm".to_owned(),
            definition(&["enter"], "Confirm selection"),
        ),
        (
            "tui.select.cancel".to_owned(),
            definition(&["escape", "ctrl+c"], "Cancel selection"),
        ),
    ]
});
