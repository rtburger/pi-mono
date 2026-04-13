use crate::{
    AssistantMessageComponent, BuiltInHeaderComponent, ClipboardImageSource, CustomEditor,
    DEFAULT_HIDDEN_THINKING_LABEL, ExtensionEditorComponent, FooterComponent, FooterState,
    KeyHintStyler, KeybindingsManager, PendingMessagesComponent, StartupHeaderStyler,
    ToolExecutionComponent, ToolExecutionOptions, ToolExecutionResult, TranscriptComponent,
    UserMessageComponent, paste_clipboard_image_into_shell,
};
use pi_coding_agent_core::{FooterDataProvider, FooterDataSnapshot};
use pi_events::{AssistantMessage, UserContent};
use pi_tui::{
    AutocompleteProvider, Component, ComponentId, EditorCursor, RenderHandle, matches_key,
    truncate_to_width,
};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type ActionCallback = Box<dyn FnMut() + Send + 'static>;
type ShortcutCallback = Box<dyn FnMut(String) -> bool + Send + 'static>;
type SubmitCallback = Box<dyn FnMut(String) + Send + 'static>;

const CLEAR_EXIT_WINDOW: Duration = Duration::from_millis(500);
const PROMPT_EXTENSION_EDITOR_TITLE: &str = "Edit message";

#[derive(Clone)]
pub struct StatusHandle {
    status_message: Arc<Mutex<Option<String>>>,
    render_handle: Option<RenderHandle>,
}

#[derive(Clone)]
struct ClipboardImagePasteConfig {
    source: Arc<dyn ClipboardImageSource + Send + Sync>,
    temp_dir: PathBuf,
}

struct SharedComponent<T> {
    inner: Rc<RefCell<T>>,
}

impl<T> Clone for SharedComponent<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T> SharedComponent<T> {
    fn new(component: T) -> Self {
        Self {
            inner: Rc::new(RefCell::new(component)),
        }
    }

    fn with_mut<R>(&self, visit: impl FnOnce(&mut T) -> R) -> R {
        visit(&mut self.inner.borrow_mut())
    }
}

impl<T: Component> Component for SharedComponent<T> {
    fn render(&self, width: usize) -> Vec<String> {
        self.inner.borrow().render(width)
    }

    fn invalidate(&mut self) {
        self.inner.borrow_mut().invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        self.inner.borrow_mut().handle_input(data);
    }

    fn wants_key_release(&self) -> bool {
        self.inner.borrow().wants_key_release()
    }

    fn set_focused(&mut self, focused: bool) {
        self.inner.borrow_mut().set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.inner.borrow().set_viewport_size(width, height);
    }
}

enum ShellUpdate {
    AppendUserMessage {
        text: String,
    },
    AppendToolResult {
        tool_call_id: String,
        tool_name: String,
        result: ToolExecutionResult,
    },
    StartAssistantMessage {
        message: AssistantMessage,
    },
    UpdateAssistantMessage {
        message: AssistantMessage,
    },
    FinishAssistantMessage {
        message: AssistantMessage,
    },
    StartToolExecution {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    UpdateToolExecution {
        tool_call_id: String,
        result: ToolExecutionResult,
        is_partial: bool,
    },
}

enum ExtensionEditorEvent {
    Submit(String),
    Cancel,
}

#[derive(Clone)]
pub(crate) struct ShellUpdateHandle {
    pending_updates: Arc<Mutex<VecDeque<ShellUpdate>>>,
    render_handle: Option<RenderHandle>,
}

impl ShellUpdateHandle {
    fn new(
        pending_updates: Arc<Mutex<VecDeque<ShellUpdate>>>,
        render_handle: Option<RenderHandle>,
    ) -> Self {
        Self {
            pending_updates,
            render_handle,
        }
    }

    fn push(&self, update: ShellUpdate) {
        self.pending_updates
            .lock()
            .expect("pending shell updates mutex poisoned")
            .push_back(update);
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }

    pub fn append_user_message(&self, text: impl Into<String>) {
        self.push(ShellUpdate::AppendUserMessage { text: text.into() });
    }

    pub fn append_tool_result(
        &self,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        result: ToolExecutionResult,
    ) {
        self.push(ShellUpdate::AppendToolResult {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            result,
        });
    }

    pub fn start_assistant_message(&self, message: AssistantMessage) {
        self.push(ShellUpdate::StartAssistantMessage { message });
    }

    pub fn update_assistant_message(&self, message: AssistantMessage) {
        self.push(ShellUpdate::UpdateAssistantMessage { message });
    }

    pub fn finish_assistant_message(&self, message: AssistantMessage) {
        self.push(ShellUpdate::FinishAssistantMessage { message });
    }

    pub fn start_tool_execution(
        &self,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        args: Value,
    ) {
        self.push(ShellUpdate::StartToolExecution {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            args,
        });
    }

    pub fn update_tool_execution(
        &self,
        tool_call_id: impl Into<String>,
        result: ToolExecutionResult,
        is_partial: bool,
    ) {
        self.push(ShellUpdate::UpdateToolExecution {
            tool_call_id: tool_call_id.into(),
            result,
            is_partial,
        });
    }
}

pub struct StartupShellComponent {
    header: BuiltInHeaderComponent,
    transcript: RefCell<TranscriptComponent>,
    pending_messages: PendingMessagesComponent,
    input: CustomEditor,
    footer: FooterComponent,
    keybindings: KeybindingsManager,
    status_message: Arc<Mutex<Option<String>>>,
    on_submit: Option<SubmitCallback>,
    on_escape: Option<ActionCallback>,
    on_exit: Option<ActionCallback>,
    on_paste_image: Option<ActionCallback>,
    on_extension_shortcut: Option<ShortcutCallback>,
    action_handlers: Vec<(String, ActionCallback)>,
    viewport_size: Cell<Option<(usize, usize)>>,
    last_clear_action: Cell<Option<Instant>>,
    clipboard_image_paste: Option<ClipboardImagePasteConfig>,
    pending_updates: Arc<Mutex<VecDeque<ShellUpdate>>>,
    current_assistant: RefCell<Option<SharedComponent<AssistantMessageComponent>>>,
    tool_components: RefCell<HashMap<String, SharedComponent<ToolExecutionComponent>>>,
    extension_editor: Option<ExtensionEditorComponent>,
    extension_editor_events: Arc<Mutex<VecDeque<ExtensionEditorEvent>>>,
    extension_editor_on_submit: Option<SubmitCallback>,
    extension_editor_on_cancel: Option<ActionCallback>,
    restore_prompt_from_extension_editor: bool,
    focused: bool,
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
            transcript: RefCell::new(TranscriptComponent::new()),
            pending_messages: PendingMessagesComponent::new(keybindings),
            input: CustomEditor::new(keybindings),
            footer: FooterComponent::default(),
            keybindings: keybindings.clone(),
            status_message: Arc::new(Mutex::new(None)),
            on_submit: None,
            on_escape: None,
            on_exit: None,
            on_paste_image: None,
            on_extension_shortcut: None,
            action_handlers: Vec::new(),
            viewport_size: Cell::new(None),
            last_clear_action: Cell::new(None),
            clipboard_image_paste: None,
            pending_updates: Arc::new(Mutex::new(VecDeque::new())),
            current_assistant: RefCell::new(None),
            tool_components: RefCell::new(HashMap::new()),
            extension_editor: None,
            extension_editor_events: Arc::new(Mutex::new(VecDeque::new())),
            extension_editor_on_submit: None,
            extension_editor_on_cancel: None,
            restore_prompt_from_extension_editor: false,
            focused: false,
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
        self.transcript
            .borrow()
            .set_viewport_height(Some(page_lines));
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

    fn handle_default_clear_action(&mut self) {
        let now = Instant::now();
        let should_exit = self
            .last_clear_action
            .get()
            .is_some_and(|last| now.duration_since(last) < CLEAR_EXIT_WINDOW);
        if should_exit {
            self.last_clear_action.set(None);
            if let Some(on_exit) = &mut self.on_exit {
                on_exit();
                return;
            }
            if self.invoke_registered_action("app.exit") {
                return;
            }
        }

        self.clear_input();
        self.last_clear_action.set(Some(now));
    }

    fn handle_default_paste_image_action(&mut self) {
        let Some(config) = self.clipboard_image_paste.clone() else {
            return;
        };
        let _ = paste_clipboard_image_into_shell(self, config.source.as_ref(), &config.temp_dir);
    }

    fn handle_default_follow_up_action(&mut self) {
        if self.input_value().trim().is_empty() {
            return;
        }
        self.submit_current_input();
    }

    fn show_prompt_extension_editor(&mut self) {
        let current_input = self.input_value();
        let prefill = if current_input.is_empty() {
            None
        } else {
            Some(current_input.as_str())
        };

        self.show_extension_editor(PROMPT_EXTENSION_EDITOR_TITLE, prefill, |_| {}, || {});
        self.restore_prompt_from_extension_editor = true;
    }

    fn submit_current_input(&mut self) {
        let value = self.input_value();
        let mut on_submit = self.on_submit.take();
        if let Some(callback) = &mut on_submit {
            self.clear_input();
            self.last_clear_action.set(None);
            callback(value);
        }
        self.on_submit = on_submit;
    }

    fn drain_extension_editor_events(&mut self) {
        loop {
            let event = self
                .extension_editor_events
                .lock()
                .expect("extension editor events mutex poisoned")
                .pop_front();
            let Some(event) = event else {
                break;
            };

            match event {
                ExtensionEditorEvent::Submit(value) => {
                    let restore_prompt_from_extension_editor =
                        self.restore_prompt_from_extension_editor;
                    let mut on_submit = self.extension_editor_on_submit.take();
                    self.hide_extension_editor();
                    if restore_prompt_from_extension_editor {
                        self.set_input_value(value.clone());
                        self.set_input_cursor(value.len());
                    }
                    if let Some(callback) = &mut on_submit {
                        callback(value);
                    }
                }
                ExtensionEditorEvent::Cancel => {
                    let mut on_cancel = self.extension_editor_on_cancel.take();
                    self.hide_extension_editor();
                    if let Some(callback) = &mut on_cancel {
                        callback();
                    }
                }
            }
        }
    }

    fn drain_pending_updates(&self) {
        loop {
            let update = self
                .pending_updates
                .lock()
                .expect("pending shell updates mutex poisoned")
                .pop_front();
            let Some(update) = update else {
                break;
            };
            self.apply_shell_update(update);
        }
    }

    fn apply_shell_update(&self, update: ShellUpdate) {
        match update {
            ShellUpdate::AppendUserMessage { text } => {
                self.transcript
                    .borrow_mut()
                    .add_item(Box::new(UserMessageComponent::new(text)));
            }
            ShellUpdate::AppendToolResult {
                tool_call_id,
                tool_name,
                result,
            } => {
                let component = self.ensure_tool_component(&tool_call_id, &tool_name, Value::Null);
                component.with_mut(|component| {
                    component.mark_execution_started();
                    component.set_args_complete();
                    component.update_result(result, false);
                });
            }
            ShellUpdate::StartAssistantMessage { message } => {
                let component = self.ensure_assistant_component(message.clone());
                component.with_mut(|component| component.update_content(message));
            }
            ShellUpdate::UpdateAssistantMessage { message } => {
                let component = self.ensure_assistant_component(message.clone());
                component.with_mut(|component| component.update_content(message));
            }
            ShellUpdate::FinishAssistantMessage { message } => {
                let component = self.ensure_assistant_component(message.clone());
                component.with_mut(|component| component.update_content(message));
                self.current_assistant.borrow_mut().take();
            }
            ShellUpdate::StartToolExecution {
                tool_call_id,
                tool_name,
                args,
            } => {
                let component = self.ensure_tool_component(&tool_call_id, &tool_name, args.clone());
                component.with_mut(|component| {
                    component.update_args(args);
                    component.mark_execution_started();
                    component.set_args_complete();
                });
            }
            ShellUpdate::UpdateToolExecution {
                tool_call_id,
                result,
                is_partial,
            } => {
                if let Some(component) = self.tool_components.borrow().get(&tool_call_id).cloned() {
                    component.with_mut(|component| component.update_result(result, is_partial));
                }
            }
        }
    }

    fn ensure_assistant_component(
        &self,
        message: AssistantMessage,
    ) -> SharedComponent<AssistantMessageComponent> {
        if let Some(component) = self.current_assistant.borrow().clone() {
            return component;
        }

        let component = SharedComponent::new(AssistantMessageComponent::new(
            Some(message),
            false,
            DEFAULT_HIDDEN_THINKING_LABEL,
        ));
        self.transcript
            .borrow_mut()
            .add_item(Box::new(component.clone()));
        *self.current_assistant.borrow_mut() = Some(component.clone());
        component
    }

    fn ensure_tool_component(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: Value,
    ) -> SharedComponent<ToolExecutionComponent> {
        if let Some(component) = self.tool_components.borrow().get(tool_call_id).cloned() {
            return component;
        }

        let component = SharedComponent::new(ToolExecutionComponent::new(
            tool_name.to_owned(),
            tool_call_id.to_owned(),
            args,
            ToolExecutionOptions::default(),
            &self.keybindings,
        ));
        self.transcript
            .borrow_mut()
            .add_item(Box::new(component.clone()));
        self.tool_components
            .borrow_mut()
            .insert(tool_call_id.to_owned(), component.clone());
        component
    }

    pub fn set_on_submit<F>(&mut self, on_submit: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_submit = Some(Box::new(on_submit));
    }

    pub fn clear_on_submit(&mut self) {
        self.on_submit = None;
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

    pub fn set_clipboard_image_source<S>(&mut self, source: S, temp_dir: impl Into<PathBuf>)
    where
        S: ClipboardImageSource + Send + Sync + 'static,
    {
        self.clipboard_image_paste = Some(ClipboardImagePasteConfig {
            source: Arc::new(source),
            temp_dir: temp_dir.into(),
        });
    }

    pub fn clear_clipboard_image_source(&mut self) {
        self.clipboard_image_paste = None;
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

    pub fn input_value(&self) -> String {
        self.input.get_text()
    }

    pub fn set_input_value(&mut self, value: impl Into<String>) {
        self.input.set_text(value.into());
    }

    pub fn insert_input_text_at_cursor(&mut self, text: &str) {
        self.input.insert_text_at_cursor(text);
    }

    pub fn set_input_cursor(&mut self, cursor: usize) {
        let text = self.input.get_text();
        self.input
            .set_cursor(editor_cursor_from_offset(&text, cursor));
    }

    pub fn clear_input(&mut self) {
        self.input.set_text("");
    }

    pub fn set_autocomplete_max_visible(&mut self, max_visible: usize) {
        self.input.set_autocomplete_max_visible(max_visible);
    }

    pub fn set_autocomplete_provider(&mut self, provider: Arc<dyn AutocompleteProvider>) {
        self.input.set_autocomplete_provider(provider);
    }

    pub fn clear_autocomplete_provider(&mut self) {
        self.input.clear_autocomplete_provider();
    }

    pub fn show_extension_editor<F, G>(
        &mut self,
        title: impl Into<String>,
        prefill: Option<&str>,
        on_submit: F,
        on_cancel: G,
    ) where
        F: FnMut(String) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.hide_extension_editor();

        let mut editor = ExtensionEditorComponent::new(&self.keybindings, title, prefill);
        let events = Arc::clone(&self.extension_editor_events);
        editor.set_on_submit(move |value| {
            events
                .lock()
                .expect("extension editor events mutex poisoned")
                .push_back(ExtensionEditorEvent::Submit(value));
        });

        let events = Arc::clone(&self.extension_editor_events);
        editor.set_on_cancel(move || {
            events
                .lock()
                .expect("extension editor events mutex poisoned")
                .push_back(ExtensionEditorEvent::Cancel);
        });

        if let Some((width, height)) = self.viewport_size.get() {
            editor.set_viewport_size(width, height);
        }

        self.input.set_focused(false);
        editor.set_focused(self.focused);
        self.extension_editor = Some(editor);
        self.extension_editor_on_submit = Some(Box::new(on_submit));
        self.extension_editor_on_cancel = Some(Box::new(on_cancel));
        self.restore_prompt_from_extension_editor = false;
    }

    pub fn hide_extension_editor(&mut self) {
        self.extension_editor = None;
        self.extension_editor_on_submit = None;
        self.extension_editor_on_cancel = None;
        self.restore_prompt_from_extension_editor = false;
        self.input.set_focused(self.focused);
    }

    pub fn is_showing_extension_editor(&self) -> bool {
        self.extension_editor.is_some()
    }

    pub fn add_transcript_item(&mut self, component: Box<dyn Component>) -> ComponentId {
        self.transcript.borrow_mut().add_item(component)
    }

    pub fn remove_transcript_item(&mut self, id: ComponentId) -> bool {
        self.transcript.borrow_mut().remove_item(id)
    }

    pub fn clear_transcript(&mut self) {
        self.pending_updates
            .lock()
            .expect("pending shell updates mutex poisoned")
            .clear();
        self.current_assistant.borrow_mut().take();
        self.tool_components.borrow_mut().clear();
        self.transcript.borrow_mut().clear_items();
    }

    pub fn transcript_item_count(&self) -> usize {
        self.transcript.borrow().item_count()
    }

    pub fn transcript_scroll_offset(&self) -> usize {
        self.transcript.borrow().scroll_offset()
    }

    pub fn set_transcript_scroll_offset(&mut self, offset: usize) {
        self.transcript.borrow_mut().set_scroll_offset(offset);
    }

    pub fn scroll_transcript_up(&mut self, lines: usize) {
        self.transcript.borrow_mut().scroll_up(lines);
    }

    pub fn scroll_transcript_down(&mut self, lines: usize) {
        self.transcript.borrow_mut().scroll_down(lines);
    }

    pub fn scroll_transcript_to_bottom(&mut self) {
        self.transcript.borrow_mut().scroll_to_bottom();
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

    pub(crate) fn update_handle_with_render_handle(
        &self,
        render_handle: RenderHandle,
    ) -> ShellUpdateHandle {
        ShellUpdateHandle::new(Arc::clone(&self.pending_updates), Some(render_handle))
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
        self.focused
    }
}

impl Component for StartupShellComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.drain_pending_updates();
        let header_lines = self.header.render(width);
        let pending_lines = self.pending_messages.render(width);
        let input_lines = if let Some(extension_editor) = &self.extension_editor {
            extension_editor.render(width)
        } else {
            self.input.render(width)
        };
        let footer_lines = self.footer.render(width);
        let status_lines = self.render_status(width);
        let transcript_height = self.transcript_viewport_height_for_width(width);
        self.transcript
            .borrow()
            .set_viewport_height(transcript_height);
        let transcript_lines = self.transcript.borrow().render(width);

        let mut lines = header_lines;
        lines.extend(transcript_lines);
        lines.extend(pending_lines);
        lines.extend(status_lines);
        lines.extend(input_lines);
        lines.extend(footer_lines);
        lines
    }

    fn invalidate(&mut self) {
        self.drain_pending_updates();
        self.header.invalidate();
        self.transcript.borrow_mut().invalidate();
        self.pending_messages.invalidate();
        if let Some(extension_editor) = &mut self.extension_editor {
            extension_editor.invalidate();
        } else {
            self.input.invalidate();
        }
        self.footer.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(extension_editor) = &mut self.extension_editor {
            extension_editor.handle_input(data);
            self.drain_extension_editor_events();
            return;
        }

        if let Some(on_extension_shortcut) = &mut self.on_extension_shortcut
            && on_extension_shortcut(data.to_owned())
        {
            return;
        }

        if self.matches_binding(data, "app.clipboard.pasteImage") {
            if let Some(on_paste_image) = &mut self.on_paste_image {
                on_paste_image();
            } else {
                self.handle_default_paste_image_action();
            }
            return;
        }

        if self.matches_binding(data, "tui.editor.pageUp") {
            let page_lines = self.page_scroll_lines();
            if page_lines > 0 {
                self.transcript.borrow_mut().scroll_up(page_lines);
            }
            return;
        }

        if self.matches_binding(data, "tui.editor.pageDown") {
            let page_lines = self.page_scroll_lines();
            if page_lines > 0 {
                self.transcript.borrow_mut().scroll_down(page_lines);
            }
            return;
        }

        if self.matches_binding(data, "app.clear") {
            if self.invoke_registered_action("app.clear") {
                return;
            }
            self.handle_default_clear_action();
            return;
        }

        if self.matches_binding(data, "app.interrupt") {
            if !self.input.is_showing_autocomplete() {
                if let Some(on_escape) = &mut self.on_escape {
                    on_escape();
                    return;
                }
                if self.invoke_registered_action("app.interrupt") {
                    return;
                }
            }
            self.input.handle_input(data);
            return;
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

        if self.matches_binding(data, "app.message.followUp") {
            if self.invoke_registered_action("app.message.followUp") {
                return;
            }
            self.handle_default_follow_up_action();
            return;
        }

        if self.matches_binding(data, "app.editor.external") {
            if self.invoke_registered_action("app.editor.external") {
                return;
            }
            self.show_prompt_extension_editor();
            return;
        }

        if self.matches_binding(data, "tui.input.submit") || data == "\n" {
            if self.input.is_showing_autocomplete() {
                self.input.handle_input(data);
                return;
            }
            self.submit_current_input();
            return;
        }

        let matched_action = self
            .action_handlers
            .iter()
            .map(|(action, _)| action.as_str())
            .find(|action| {
                *action != "app.clear"
                    && *action != "app.interrupt"
                    && *action != "app.exit"
                    && *action != "app.message.followUp"
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
        self.focused = focused;
        self.input
            .set_focused(focused && self.extension_editor.is_none());
        if let Some(extension_editor) = &mut self.extension_editor {
            extension_editor.set_focused(focused);
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.header.set_viewport_size(width, height);
        self.transcript.borrow().set_viewport_size(width, height);
        self.pending_messages.set_viewport_size(width, height);
        self.input.set_viewport_size(width, height);
        if let Some(extension_editor) = &self.extension_editor {
            extension_editor.set_viewport_size(width, height);
        }
        self.footer.set_viewport_size(width, height);
    }
}

fn editor_cursor_from_offset(text: &str, cursor: usize) -> EditorCursor {
    let clamped = cursor.min(text.len());
    let mut remaining = clamped;

    for (line_index, line) in text.split('\n').enumerate() {
        if remaining <= line.len() {
            return EditorCursor {
                line: line_index,
                col: remaining,
            };
        }

        remaining = remaining.saturating_sub(line.len() + 1);
    }

    let lines = text.split('\n').collect::<Vec<_>>();
    let last_line = lines.len().saturating_sub(1);
    let last_col = lines.last().map(|line| line.len()).unwrap_or(0);
    EditorCursor {
        line: last_line,
        col: last_col,
    }
}

fn format_user_message_text(content: &[UserContent]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            UserContent::Text { text } => Some(text.replace('\r', "")),
            UserContent::Image { mime_type, .. } => Some(format!("[Image: [{mime_type}]]")),
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn tool_result_from_user_content(
    content: Vec<UserContent>,
    details: Value,
    is_error: bool,
) -> ToolExecutionResult {
    ToolExecutionResult {
        content,
        details,
        is_error,
    }
}

pub(crate) fn user_message_text(content: &[UserContent]) -> String {
    format_user_message_text(content)
}
