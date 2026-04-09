use crate::{
    BuiltInHeaderComponent, KeyHintStyler, KeybindingsManager, PendingMessagesComponent,
    StartupHeaderStyler,
};
use pi_tui::{Component, Input};
use std::ops::Deref;

pub struct StartupShellComponent {
    header: BuiltInHeaderComponent,
    pending_messages: PendingMessagesComponent,
    input: Input,
}

impl StartupShellComponent {
    pub fn new(
        app_name: &str,
        version: &str,
        keybindings: &KeybindingsManager,
        styler: &impl StartupHeaderStyler,
        quiet: bool,
        changelog_markdown: Option<&str>,
        show_condensed_changelog: bool,
    ) -> Self {
        Self {
            header: BuiltInHeaderComponent::new(
                app_name,
                version,
                keybindings,
                styler,
                quiet,
                changelog_markdown,
                show_condensed_changelog,
            ),
            pending_messages: PendingMessagesComponent::new(keybindings),
            input: Input::with_keybindings(keybindings.deref().clone()),
        }
    }

    pub fn set_on_submit<F>(&mut self, on_submit: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.input.set_on_submit(on_submit);
    }

    pub fn clear_on_submit(&mut self) {
        self.input.clear_on_submit();
    }

    pub fn set_on_escape<F>(&mut self, on_escape: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.input.set_on_escape(on_escape);
    }

    pub fn clear_on_escape(&mut self) {
        self.input.clear_on_escape();
    }

    pub fn input_value(&self) -> &str {
        self.input.value()
    }

    pub fn set_input_value(&mut self, value: impl Into<String>) {
        self.input.set_value(value);
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    pub fn set_pending_messages<I, J, S, T>(
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
        self.pending_messages
            .set_messages(styler, steering, follow_up);
    }

    pub fn clear_pending_messages(&mut self) {
        self.pending_messages.clear_messages();
    }

    pub fn has_pending_messages(&self) -> bool {
        self.pending_messages.has_messages()
    }

    pub fn is_focused(&self) -> bool {
        self.input.is_focused()
    }
}

impl Component for StartupShellComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = self.header.render(width);
        lines.extend(self.pending_messages.render(width));
        lines.extend(self.input.render(width));
        lines
    }

    fn invalidate(&mut self) {
        self.header.invalidate();
        self.pending_messages.invalidate();
        self.input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        self.input.handle_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }
}
