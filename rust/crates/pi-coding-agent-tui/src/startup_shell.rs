use crate::{BuiltInHeaderComponent, KeybindingsManager, StartupHeaderStyler};
use pi_tui::{Component, Input};
use std::ops::Deref;

pub struct StartupShellComponent {
    header: BuiltInHeaderComponent,
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

    pub fn is_focused(&self) -> bool {
        self.input.is_focused()
    }
}

impl Component for StartupShellComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = self.header.render(width);
        lines.extend(self.input.render(width));
        lines
    }

    fn invalidate(&mut self) {
        self.header.invalidate();
        self.input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        self.input.handle_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }
}
