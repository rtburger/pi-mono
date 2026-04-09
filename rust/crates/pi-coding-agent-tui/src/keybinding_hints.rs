use crate::KeybindingsManager;

pub trait KeyHintStyler {
    fn dim(&self, text: &str) -> String;
    fn muted(&self, text: &str) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlainKeyHintStyler;

impl KeyHintStyler for PlainKeyHintStyler {
    fn dim(&self, text: &str) -> String {
        text.to_owned()
    }

    fn muted(&self, text: &str) -> String {
        text.to_owned()
    }
}

pub fn key_text(keybindings: &KeybindingsManager, keybinding: &str) -> String {
    format_keys(&keybindings.get_keys(keybinding))
}

pub fn key_hint(
    keybindings: &KeybindingsManager,
    styler: &impl KeyHintStyler,
    keybinding: &str,
    description: &str,
) -> String {
    styler.dim(&key_text(keybindings, keybinding)) + &styler.muted(&format!(" {description}"))
}

pub fn raw_key_hint(styler: &impl KeyHintStyler, key: &str, description: &str) -> String {
    styler.dim(key) + &styler.muted(&format!(" {description}"))
}

fn format_keys(keys: &[pi_tui::KeyId]) -> String {
    match keys {
        [] => String::new(),
        [key] => key.to_string(),
        _ => keys
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/"),
    }
}
