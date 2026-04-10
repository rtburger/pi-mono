use crate::{KeyHintStyler, KeybindingsManager, key_text};
use pi_tui::{Component, TruncatedText};

#[derive(Debug, Clone, Default)]
pub struct PendingMessagesComponent {
    dequeue_key_text: String,
    steering_messages: Vec<String>,
    follow_up_messages: Vec<String>,
    lines: Vec<String>,
}

impl PendingMessagesComponent {
    pub fn new(keybindings: &KeybindingsManager) -> Self {
        Self {
            dequeue_key_text: key_text(keybindings, "app.message.dequeue"),
            steering_messages: Vec::new(),
            follow_up_messages: Vec::new(),
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
        self.clear_messages();

        for message in steering {
            let message = message.as_ref().to_owned();
            self.lines.push(styler.dim(&format!("Steering: {message}")));
            self.steering_messages.push(message);
        }

        for message in follow_up {
            let message = message.as_ref().to_owned();
            self.lines
                .push(styler.dim(&format!("Follow-up: {message}")));
            self.follow_up_messages.push(message);
        }

        if !self.lines.is_empty() {
            self.lines.push(styler.dim(&format!(
                "↳ {} to edit all queued messages",
                self.dequeue_key_text
            )));
        }
    }

    pub fn clear_messages(&mut self) {
        self.steering_messages.clear();
        self.follow_up_messages.clear();
        self.lines.clear();
    }

    pub fn has_messages(&self) -> bool {
        self.message_count() > 0
    }

    pub fn message_count(&self) -> usize {
        self.steering_messages.len() + self.follow_up_messages.len()
    }

    pub fn drain_messages(&mut self) -> Vec<String> {
        let mut messages = Vec::with_capacity(self.message_count());
        messages.append(&mut self.steering_messages);
        messages.append(&mut self.follow_up_messages);
        self.lines.clear();
        messages
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
