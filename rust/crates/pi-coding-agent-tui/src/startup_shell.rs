use crate::{
    BuiltInHeaderComponent, KeyHintStyler, KeybindingsManager, PendingMessagesComponent,
    StartupHeaderStyler, TranscriptComponent,
};
use pi_tui::{Component, ComponentId, Input};
use std::{cell::Cell, ops::Deref};

pub struct StartupShellComponent {
    header: BuiltInHeaderComponent,
    transcript: TranscriptComponent,
    pending_messages: PendingMessagesComponent,
    input: Input,
    viewport_size: Cell<Option<(usize, usize)>>,
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
            transcript: TranscriptComponent::new(),
            pending_messages: PendingMessagesComponent::new(keybindings),
            input: Input::with_keybindings(keybindings.deref().clone()),
            viewport_size: Cell::new(None),
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

    pub fn add_transcript_item(&mut self, component: Box<dyn Component>) -> ComponentId {
        self.transcript.add_item(component)
    }

    pub fn remove_transcript_item(&mut self, id: ComponentId) -> bool {
        self.transcript.remove_item(id)
    }

    pub fn clear_transcript(&mut self) {
        self.transcript.clear_items();
    }

    pub fn transcript_item_count(&self) -> usize {
        self.transcript.item_count()
    }

    pub fn transcript_scroll_offset(&self) -> usize {
        self.transcript.scroll_offset()
    }

    pub fn set_transcript_scroll_offset(&mut self, offset: usize) {
        self.transcript.set_scroll_offset(offset);
    }

    pub fn scroll_transcript_up(&mut self, lines: usize) {
        self.transcript.scroll_up(lines);
    }

    pub fn scroll_transcript_down(&mut self, lines: usize) {
        self.transcript.scroll_down(lines);
    }

    pub fn scroll_transcript_to_bottom(&mut self) {
        self.transcript.scroll_to_bottom();
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
        let header_lines = self.header.render(width);
        let pending_lines = self.pending_messages.render(width);
        let input_lines = self.input.render(width);
        let transcript_height = self.viewport_size.get().map(|(_, total_height)| {
            total_height
                .saturating_sub(header_lines.len() + pending_lines.len() + input_lines.len())
        });
        self.transcript.set_viewport_height(transcript_height);
        let transcript_lines = self.transcript.render(width);

        let mut lines = header_lines;
        lines.extend(transcript_lines);
        lines.extend(pending_lines);
        lines.extend(input_lines);
        lines
    }

    fn invalidate(&mut self) {
        self.header.invalidate();
        self.transcript.invalidate();
        self.pending_messages.invalidate();
        self.input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        self.input.handle_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.header.set_viewport_size(width, height);
        self.transcript.set_viewport_size(width, height);
        self.pending_messages.set_viewport_size(width, height);
        self.input.set_viewport_size(width, height);
    }
}
