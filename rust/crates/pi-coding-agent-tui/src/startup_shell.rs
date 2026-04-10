use crate::{
    BuiltInHeaderComponent, FooterComponent, FooterState, KeyHintStyler, KeybindingsManager,
    PendingMessagesComponent, StartupHeaderStyler, TranscriptComponent,
};
use pi_coding_agent_core::{FooterDataProvider, FooterDataSnapshot};
use pi_tui::{Component, ComponentId, Input, RenderHandle, matches_key, truncate_to_width};
use std::{
    cell::Cell,
    ops::Deref,
    sync::{Arc, Mutex},
};

type ActionCallback = Box<dyn FnMut() + Send + 'static>;
type ShortcutCallback = Box<dyn FnMut(String) -> bool + Send + 'static>;

#[derive(Clone)]
pub struct StatusHandle {
    status_message: Arc<Mutex<Option<String>>>,
    render_handle: Option<RenderHandle>,
}

impl StatusHandle {
    fn new(
        status_message: Arc<Mutex<Option<String>>>,
        render_handle: Option<RenderHandle>,
    ) -> Self {
        Self {
            status_message,
            render_handle,
        }
    }

    pub fn set_message(&self, message: impl Into<String>) {
        *self
            .status_message
            .lock()
            .expect("status message mutex poisoned") = Some(message.into());
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }

    pub fn clear(&self) {
        *self
            .status_message
            .lock()
            .expect("status message mutex poisoned") = None;
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }
}

pub struct StartupShellComponent {
    header: BuiltInHeaderComponent,
    transcript: TranscriptComponent,
    pending_messages: PendingMessagesComponent,
    input: Input,
    footer: FooterComponent,
    keybindings: KeybindingsManager,
    status_message: Arc<Mutex<Option<String>>>,
    on_escape: Option<ActionCallback>,
    on_exit: Option<ActionCallback>,
    on_paste_image: Option<ActionCallback>,
    on_extension_shortcut: Option<ShortcutCallback>,
    action_handlers: Vec<(String, ActionCallback)>,
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
            footer: FooterComponent::default(),
            keybindings: keybindings.clone(),
            status_message: Arc::new(Mutex::new(None)),
            on_escape: None,
            on_exit: None,
            on_paste_image: None,
            on_extension_shortcut: None,
            action_handlers: Vec::new(),
            viewport_size: Cell::new(None),
        }
    }

    fn transcript_viewport_height_for_width(&self, width: usize) -> Option<usize> {
        let (_, total_height) = self.viewport_size.get()?;
        let occupied_height = self.header.render(width).len()
            + self.pending_messages.render(width).len()
            + self.render_status(width).len()
            + self.input.render(width).len()
            + self.footer.render(width).len();
        Some(total_height.saturating_sub(occupied_height))
    }

    fn render_status(&self, width: usize) -> Vec<String> {
        self.status_message
            .lock()
            .expect("status message mutex poisoned")
            .as_ref()
            .map(|message| vec![truncate_to_width(message, width, "...", false)])
            .unwrap_or_default()
    }

    fn page_scroll_lines(&self) -> usize {
        let Some((width, _)) = self.viewport_size.get() else {
            return 0;
        };
        let page_lines = self
            .transcript_viewport_height_for_width(width)
            .unwrap_or(0);
        self.transcript.set_viewport_height(Some(page_lines));
        page_lines
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn invoke_registered_action(&mut self, action: &str) -> bool {
        if let Some((_, handler)) = self
            .action_handlers
            .iter_mut()
            .find(|(candidate, _)| candidate == action)
        {
            handler();
            return true;
        }
        false
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
        self.on_escape = Some(Box::new(on_escape));
    }

    pub fn clear_on_escape(&mut self) {
        self.on_escape = None;
    }

    pub fn set_on_exit<F>(&mut self, on_exit: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_exit = Some(Box::new(on_exit));
    }

    pub fn clear_on_exit(&mut self) {
        self.on_exit = None;
    }

    pub fn set_on_paste_image<F>(&mut self, on_paste_image: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_paste_image = Some(Box::new(on_paste_image));
    }

    pub fn clear_on_paste_image(&mut self) {
        self.on_paste_image = None;
    }

    pub fn set_on_extension_shortcut<F>(&mut self, on_extension_shortcut: F)
    where
        F: FnMut(String) -> bool + Send + 'static,
    {
        self.on_extension_shortcut = Some(Box::new(on_extension_shortcut));
    }

    pub fn clear_on_extension_shortcut(&mut self) {
        self.on_extension_shortcut = None;
    }

    pub fn on_action<F>(&mut self, action: impl Into<String>, handler: F)
    where
        F: FnMut() + Send + 'static,
    {
        let action = action.into();
        if let Some((_, existing_handler)) = self
            .action_handlers
            .iter_mut()
            .find(|(candidate, _)| candidate == &action)
        {
            *existing_handler = Box::new(handler);
            return;
        }
        self.action_handlers.push((action, Box::new(handler)));
    }

    pub fn clear_action(&mut self, action: &str) {
        self.action_handlers
            .retain(|(candidate, _)| candidate != action);
    }

    pub fn input_value(&self) -> &str {
        self.input.value()
    }

    pub fn set_input_value(&mut self, value: impl Into<String>) {
        self.input.set_value(value);
    }

    pub fn insert_input_text_at_cursor(&mut self, text: &str) {
        self.input.insert_text_at_cursor(text);
    }

    pub fn set_input_cursor(&mut self, cursor: usize) {
        self.input.set_cursor(cursor);
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

    pub fn set_status_message(&mut self, message: impl Into<String>) {
        *self
            .status_message
            .lock()
            .expect("status message mutex poisoned") = Some(message.into());
    }

    pub fn clear_status_message(&mut self) {
        *self
            .status_message
            .lock()
            .expect("status message mutex poisoned") = None;
    }

    pub fn status_handle(&self) -> StatusHandle {
        StatusHandle::new(Arc::clone(&self.status_message), None)
    }

    pub fn status_handle_with_render_handle(&self, render_handle: RenderHandle) -> StatusHandle {
        StatusHandle::new(Arc::clone(&self.status_message), Some(render_handle))
    }

    pub fn set_footer_state(&mut self, state: FooterState) {
        self.footer.set_state(state);
    }

    pub fn clear_footer(&mut self) {
        self.footer.clear_state();
    }

    pub fn bind_footer_data_provider(&mut self, provider: &FooterDataProvider) {
        self.footer.bind_data_provider(provider);
    }

    pub fn bind_footer_data_provider_with_render_handle(
        &mut self,
        provider: &FooterDataProvider,
        render_handle: RenderHandle,
    ) {
        self.footer
            .bind_data_provider_with_render_handle(provider, render_handle);
    }

    pub fn unbind_footer_data_provider(&mut self) {
        self.footer.unbind_data_provider();
    }

    pub fn apply_footer_data_snapshot(&mut self, snapshot: &FooterDataSnapshot) {
        self.footer.apply_data_snapshot(snapshot);
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
        let footer_lines = self.footer.render(width);
        let status_lines = self.render_status(width);
        let transcript_height = self.transcript_viewport_height_for_width(width);
        self.transcript.set_viewport_height(transcript_height);
        let transcript_lines = self.transcript.render(width);

        let mut lines = header_lines;
        lines.extend(transcript_lines);
        lines.extend(pending_lines);
        lines.extend(status_lines);
        lines.extend(input_lines);
        lines.extend(footer_lines);
        lines
    }

    fn invalidate(&mut self) {
        self.header.invalidate();
        self.transcript.invalidate();
        self.pending_messages.invalidate();
        self.input.invalidate();
        self.footer.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(on_extension_shortcut) = &mut self.on_extension_shortcut
            && on_extension_shortcut(data.to_owned())
        {
            return;
        }

        if self.matches_binding(data, "app.clipboard.pasteImage") {
            if let Some(on_paste_image) = &mut self.on_paste_image {
                on_paste_image();
            }
            return;
        }

        if self.matches_binding(data, "tui.editor.pageUp") {
            let page_lines = self.page_scroll_lines();
            if page_lines > 0 {
                self.transcript.scroll_up(page_lines);
            }
            return;
        }

        if self.matches_binding(data, "tui.editor.pageDown") {
            let page_lines = self.page_scroll_lines();
            if page_lines > 0 {
                self.transcript.scroll_down(page_lines);
            }
            return;
        }

        if self.matches_binding(data, "app.interrupt") {
            if let Some(on_escape) = &mut self.on_escape {
                on_escape();
                return;
            }
            if self.invoke_registered_action("app.interrupt") {
                return;
            }
        }

        if self.matches_binding(data, "app.exit") && self.input_value().is_empty() {
            if let Some(on_exit) = &mut self.on_exit {
                on_exit();
                return;
            }
            if self.invoke_registered_action("app.exit") {
                return;
            }
        }

        let matched_action = self
            .action_handlers
            .iter()
            .map(|(action, _)| action.as_str())
            .find(|action| {
                *action != "app.interrupt"
                    && *action != "app.exit"
                    && self.matches_binding(data, action)
            })
            .map(str::to_owned);
        if let Some(action) = matched_action {
            self.invoke_registered_action(&action);
            return;
        }

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
        self.footer.set_viewport_size(width, height);
    }
}
