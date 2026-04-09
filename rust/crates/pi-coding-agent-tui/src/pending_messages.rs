use crate::{KeyHintStyler, KeybindingsManager, key_text};
use pi_tui::{Component, TruncatedText};

#[derive(Debug, Clone, Default)]
pub struct PendingMessagesComponent {
    dequeue_key_text: String,
    lines: Vec<String>,
}

impl PendingMessagesComponent {
    pub fn new(keybindings: &KeybindingsManager) -> Self {
        Self {
            dequeue_key_text: key_text(keybindings, "app.message.dequeue"),
            lines: Vec::new(),
        }
    }

    pub fn set_messages<I, J, S, T>(
        &mut self,
        styler: &impl KeyHintStyler,
        steering: I,
        follow_up: J,
    ) where
        I: IntoIterator<Item = S>,
        J: IntoIterator<Item = T>,
        S: AsRef<str>,
        T: AsRef<str>,
    {
        self.lines.clear();

        for message in steering {
            self.lines
                .push(styler.dim(&format!("Steering: {}", message.as_ref())));
        }

        for message in follow_up {
            self.lines
                .push(styler.dim(&format!("Follow-up: {}", message.as_ref())));
        }

        if !self.lines.is_empty() {
            self.lines.push(styler.dim(&format!(
                "↳ {} to edit all queued messages",
                self.dequeue_key_text
            )));
        }
    }

    pub fn clear_messages(&mut self) {
        self.lines.clear();
    }

    pub fn has_messages(&self) -> bool {
        !self.lines.is_empty()
    }
}

impl Component for PendingMessagesComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if self.lines.is_empty() {
            return Vec::new();
        }

        let mut rendered = Vec::with_capacity(self.lines.len() + 1);
        rendered.push(String::new());

        for line in &self.lines {
            rendered.extend(TruncatedText::new(line, 1, 0).render(width));
        }

        rendered
    }

    fn invalidate(&mut self) {}
}
