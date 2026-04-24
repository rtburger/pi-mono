use crate::{
    AssistantMessageComponent, BashExecutionComponent, BashExecutionHandle, BuiltInHeaderComponent,
    ClipboardImageSource, CustomEditor, DEFAULT_HIDDEN_THINKING_LABEL, ExtensionEditorComponent,
    ExtensionInputComponent, ExtensionSelectorComponent, ExternalEditorCommandRunner,
    ExternalEditorHost, FooterComponent, FooterState, FooterStateHandle, KeyHintStyler,
    KeybindingsManager, ModelSelectorComponent, PendingMessagesComponent, StartupHeaderStyler,
    ThemedKeyHintStyler, ToolExecutionComponent, ToolExecutionOptions, ToolExecutionResult,
    TranscriptComponent, UserMessageComponent, current_theme, paste_clipboard_image_into_shell,
};
use pi_coding_agent_core::{BashExecutionMessage, FooterDataProvider, FooterDataSnapshot};
use pi_events::{AssistantMessage, Model, UserContent};
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
type ValueCallback = Box<dyn FnMut(String) + Send + 'static>;
type RemoteEditorInputCallback = Box<dyn FnMut(String, usize, usize) + Send + 'static>;
type RegisteredActionCallback = Box<dyn FnMut(&mut StartupShellComponent) + Send + 'static>;
type ShortcutCallback = Box<dyn FnMut(String) -> bool + Send + 'static>;
type SubmitCallback = Box<dyn FnMut(&mut StartupShellComponent, String) + Send + 'static>;
type ModelSelectorSelectCallback = Box<dyn FnMut(Model) + Send + 'static>;

const CLEAR_EXIT_WINDOW: Duration = Duration::from_millis(500);
const PROMPT_EXTENSION_EDITOR_TITLE: &str = "Edit message";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionWidgetPlacement {
    AboveEditor,
    BelowEditor,
}

struct RemoteEditorState {
    lines: Vec<String>,
    on_input: RemoteEditorInputCallback,
    focused: bool,
    viewport_size: Option<(usize, usize)>,
}

impl RemoteEditorState {
    fn new(lines: Vec<String>, on_input: RemoteEditorInputCallback) -> Self {
        Self {
            lines,
            on_input,
            focused: false,
            viewport_size: None,
        }
    }

    fn render(&self, width: usize) -> Vec<String> {
        render_static_lines(width, &self.lines)
    }

    fn handle_input(&mut self, data: &str) {
        let (width, height) = self.viewport_size.unwrap_or((80, 24));
        (self.on_input)(data.to_owned(), width, height);
    }

    fn set_lines(&mut self, lines: Vec<String>) {
        self.lines = lines;
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn set_viewport_size(&mut self, width: usize, height: usize) {
        self.viewport_size = Some((width, height));
    }
}

fn render_static_lines(width: usize, lines: &[String]) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    lines
        .iter()
        .map(|line| truncate_to_width(line, width, "...", false))
        .collect()
}

fn upsert_extension_widget_lines(
    widgets: &mut Vec<(String, Vec<String>)>,
    key: String,
    lines: Option<Vec<String>>,
) {
    if let Some(index) = widgets.iter().position(|(candidate, _)| candidate == &key) {
        if let Some(lines) = lines {
            widgets[index] = (key, lines);
        } else {
            widgets.remove(index);
        }
        return;
    }

    if let Some(lines) = lines {
        widgets.push((key, lines));
    }
}

fn render_extension_widgets(width: usize, widgets: &[(String, Vec<String>)]) -> Vec<String> {
    let mut lines = Vec::new();
    for (_, widget_lines) in widgets {
        lines.extend(render_static_lines(width, widget_lines));
    }
    lines
}

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
    AppendBashExecution {
        message: BashExecutionMessage,
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
    SetPendingMessages {
        steering: Vec<String>,
        follow_up: Vec<String>,
    },
    ClearPendingMessages,
    ShowExtensionInput {
        title: String,
        placeholder: Option<String>,
        timeout_ms: Option<u64>,
        on_submit: ValueCallback,
        on_cancel: ActionCallback,
    },
    ShowExtensionSelector {
        title: String,
        options: Vec<String>,
        timeout_ms: Option<u64>,
        on_select: ValueCallback,
        on_cancel: ActionCallback,
    },
    ShowExtensionEditor {
        title: String,
        prefill: Option<String>,
        on_submit: ValueCallback,
        on_cancel: ActionCallback,
    },
    SetExtensionHeaderLines {
        lines: Option<Vec<String>>,
    },
    SetExtensionFooterLines {
        lines: Option<Vec<String>>,
    },
    SetExtensionWidgetLines {
        key: String,
        placement: ExtensionWidgetPlacement,
        lines: Option<Vec<String>>,
    },
    ShowRemoteEditor {
        lines: Vec<String>,
        on_input: RemoteEditorInputCallback,
    },
    UpdateRemoteEditorLines {
        lines: Vec<String>,
    },
    HideRemoteEditor,
    SetInputValue {
        value: String,
        cursor: Option<usize>,
    },
}

enum ExtensionEditorEvent {
    Submit(String),
    Cancel,
}

enum ExtensionInputEvent {
    Submit(String),
    Cancel,
}

enum ExtensionSelectorEvent {
    Select(String),
    Cancel,
}

enum ModelSelectorEvent {
    Select(Model),
    Cancel,
}

#[derive(Clone)]
pub struct ShellUpdateHandle {
    pending_updates: Arc<Mutex<VecDeque<ShellUpdate>>>,
    input_text: Arc<Mutex<String>>,
    render_handle: Option<RenderHandle>,
}

impl ShellUpdateHandle {
    fn new(
        pending_updates: Arc<Mutex<VecDeque<ShellUpdate>>>,
        input_text: Arc<Mutex<String>>,
        render_handle: Option<RenderHandle>,
    ) -> Self {
        Self {
            pending_updates,
            input_text,
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

    pub fn append_bash_execution(&self, message: BashExecutionMessage) {
        self.push(ShellUpdate::AppendBashExecution { message });
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

    pub fn set_pending_messages(&self, steering: Vec<String>, follow_up: Vec<String>) {
        self.push(ShellUpdate::SetPendingMessages {
            steering,
            follow_up,
        });
    }

    pub fn clear_pending_messages(&self) {
        self.push(ShellUpdate::ClearPendingMessages);
    }

    pub fn show_extension_input<F, G>(
        &self,
        title: impl Into<String>,
        placeholder: Option<String>,
        timeout_ms: Option<u64>,
        on_submit: F,
        on_cancel: G,
    ) where
        F: FnMut(String) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.push(ShellUpdate::ShowExtensionInput {
            title: title.into(),
            placeholder,
            timeout_ms,
            on_submit: Box::new(on_submit),
            on_cancel: Box::new(on_cancel),
        });
    }

    pub fn show_extension_selector<F, G>(
        &self,
        title: impl Into<String>,
        options: Vec<String>,
        timeout_ms: Option<u64>,
        on_select: F,
        on_cancel: G,
    ) where
        F: FnMut(String) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.push(ShellUpdate::ShowExtensionSelector {
            title: title.into(),
            options,
            timeout_ms,
            on_select: Box::new(on_select),
            on_cancel: Box::new(on_cancel),
        });
    }

    pub fn show_extension_editor<F, G>(
        &self,
        title: impl Into<String>,
        prefill: Option<String>,
        on_submit: F,
        on_cancel: G,
    ) where
        F: FnMut(String) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.push(ShellUpdate::ShowExtensionEditor {
            title: title.into(),
            prefill,
            on_submit: Box::new(on_submit),
            on_cancel: Box::new(on_cancel),
        });
    }

    pub fn set_extension_header_lines(&self, lines: Option<Vec<String>>) {
        self.push(ShellUpdate::SetExtensionHeaderLines { lines });
    }

    pub fn set_extension_footer_lines(&self, lines: Option<Vec<String>>) {
        self.push(ShellUpdate::SetExtensionFooterLines { lines });
    }

    pub fn set_extension_widget_lines(
        &self,
        key: impl Into<String>,
        placement: ExtensionWidgetPlacement,
        lines: Option<Vec<String>>,
    ) {
        self.push(ShellUpdate::SetExtensionWidgetLines {
            key: key.into(),
            placement,
            lines,
        });
    }

    pub fn show_remote_editor<F>(&self, lines: Vec<String>, on_input: F)
    where
        F: FnMut(String, usize, usize) + Send + 'static,
    {
        self.push(ShellUpdate::ShowRemoteEditor {
            lines,
            on_input: Box::new(on_input),
        });
    }

    pub fn update_remote_editor_lines(&self, lines: Vec<String>) {
        self.push(ShellUpdate::UpdateRemoteEditorLines { lines });
    }

    pub fn hide_remote_editor(&self) {
        self.push(ShellUpdate::HideRemoteEditor);
    }

    pub fn set_input_value(&self, value: impl Into<String>, cursor: Option<usize>) {
        let value = value.into();
        *self
            .input_text
            .lock()
            .expect("shell input text mutex poisoned") = value.clone();
        self.push(ShellUpdate::SetInputValue { value, cursor });
    }

    pub fn current_input_value(&self) -> String {
        self.input_text
            .lock()
            .expect("shell input text mutex poisoned")
            .clone()
    }

    pub fn request_render(&self) {
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }
}

pub struct StartupShellComponent {
    header: BuiltInHeaderComponent,
    transcript: RefCell<TranscriptComponent>,
    pending_messages: RefCell<PendingMessagesComponent>,
    input: RefCell<CustomEditor>,
    input_text: Arc<Mutex<String>>,
    footer: FooterComponent,
    keybindings: KeybindingsManager,
    status_message: Arc<Mutex<Option<String>>>,
    on_submit: Option<SubmitCallback>,
    on_escape: Option<ActionCallback>,
    on_exit: Option<ActionCallback>,
    on_paste_image: Option<ActionCallback>,
    on_extension_shortcut: Option<ShortcutCallback>,
    action_handlers: Vec<(String, RegisteredActionCallback)>,
    viewport_size: Cell<Option<(usize, usize)>>,
    render_handle: Option<RenderHandle>,
    last_clear_action: Cell<Option<Instant>>,
    clipboard_image_paste: Option<ClipboardImagePasteConfig>,
    pending_updates: Arc<Mutex<VecDeque<ShellUpdate>>>,
    current_assistant: RefCell<Option<SharedComponent<AssistantMessageComponent>>>,
    assistant_components: RefCell<Vec<SharedComponent<AssistantMessageComponent>>>,
    tool_components: RefCell<HashMap<String, SharedComponent<ToolExecutionComponent>>>,
    bash_components: RefCell<Vec<BashExecutionHandle>>,
    tool_output_expanded: Cell<bool>,
    show_images: Cell<bool>,
    hide_thinking_blocks: Cell<bool>,
    extension_input: RefCell<Option<ExtensionInputComponent>>,
    extension_input_events: Arc<Mutex<VecDeque<ExtensionInputEvent>>>,
    extension_input_on_submit: RefCell<Option<SubmitCallback>>,
    extension_input_on_cancel: RefCell<Option<ActionCallback>>,
    extension_selector: RefCell<Option<ExtensionSelectorComponent>>,
    extension_selector_events: Arc<Mutex<VecDeque<ExtensionSelectorEvent>>>,
    extension_selector_on_select: RefCell<Option<SubmitCallback>>,
    extension_selector_on_cancel: RefCell<Option<ActionCallback>>,
    extension_editor: RefCell<Option<ExtensionEditorComponent>>,
    extension_editor_events: Arc<Mutex<VecDeque<ExtensionEditorEvent>>>,
    extension_editor_on_submit: RefCell<Option<SubmitCallback>>,
    extension_editor_on_cancel: RefCell<Option<ActionCallback>>,
    extension_editor_command: Option<String>,
    extension_editor_runner: Option<Arc<dyn ExternalEditorCommandRunner>>,
    extension_editor_host: Option<Arc<dyn ExternalEditorHost>>,
    extension_header_lines: RefCell<Option<Vec<String>>>,
    extension_footer_lines: RefCell<Option<Vec<String>>>,
    extension_widgets_above: RefCell<Vec<(String, Vec<String>)>>,
    extension_widgets_below: RefCell<Vec<(String, Vec<String>)>>,
    remote_editor: RefCell<Option<RemoteEditorState>>,
    model_selector: RefCell<Option<ModelSelectorComponent>>,
    model_selector_events: Arc<Mutex<VecDeque<ModelSelectorEvent>>>,
    model_selector_on_select: RefCell<Option<ModelSelectorSelectCallback>>,
    model_selector_on_cancel: RefCell<Option<ActionCallback>>,
    restore_prompt_from_extension_editor: Cell<bool>,
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
    ) -> Self {
        let input_text = Arc::new(Mutex::new(String::new()));
        let mut input = CustomEditor::new(keybindings);
        {
            let input_text = Arc::clone(&input_text);
            input.set_on_change(move |text| {
                *input_text.lock().expect("shell input text mutex poisoned") = text;
            });
        }

        Self {
            header: BuiltInHeaderComponent::new(app_name, version, keybindings, styler, quiet),
            transcript: RefCell::new(TranscriptComponent::new()),
            pending_messages: RefCell::new(PendingMessagesComponent::new(keybindings)),
            input: RefCell::new(input),
            input_text,
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
            render_handle: None,
            last_clear_action: Cell::new(None),
            clipboard_image_paste: None,
            pending_updates: Arc::new(Mutex::new(VecDeque::new())),
            current_assistant: RefCell::new(None),
            assistant_components: RefCell::new(Vec::new()),
            tool_components: RefCell::new(HashMap::new()),
            bash_components: RefCell::new(Vec::new()),
            tool_output_expanded: Cell::new(false),
            show_images: Cell::new(true),
            hide_thinking_blocks: Cell::new(false),
            extension_input: RefCell::new(None),
            extension_input_events: Arc::new(Mutex::new(VecDeque::new())),
            extension_input_on_submit: RefCell::new(None),
            extension_input_on_cancel: RefCell::new(None),
            extension_selector: RefCell::new(None),
            extension_selector_events: Arc::new(Mutex::new(VecDeque::new())),
            extension_selector_on_select: RefCell::new(None),
            extension_selector_on_cancel: RefCell::new(None),
            extension_editor: RefCell::new(None),
            extension_editor_events: Arc::new(Mutex::new(VecDeque::new())),
            extension_editor_on_submit: RefCell::new(None),
            extension_editor_on_cancel: RefCell::new(None),
            extension_editor_command: None,
            extension_editor_runner: None,
            extension_editor_host: None,
            extension_header_lines: RefCell::new(None),
            extension_footer_lines: RefCell::new(None),
            extension_widgets_above: RefCell::new(Vec::new()),
            extension_widgets_below: RefCell::new(Vec::new()),
            remote_editor: RefCell::new(None),
            model_selector: RefCell::new(None),
            model_selector_events: Arc::new(Mutex::new(VecDeque::new())),
            model_selector_on_select: RefCell::new(None),
            model_selector_on_cancel: RefCell::new(None),
            restore_prompt_from_extension_editor: Cell::new(false),
            focused: false,
        }
    }

    fn active_prompt_component_height_for_width(&self, width: usize) -> usize {
        if let Some(model_selector) = self.model_selector.borrow().as_ref() {
            return model_selector.render(width).len();
        }
        if let Some(extension_editor) = self.extension_editor.borrow().as_ref() {
            return extension_editor.render(width).len();
        }
        if let Some(extension_selector) = self.extension_selector.borrow().as_ref() {
            return extension_selector.render(width).len();
        }
        if let Some(extension_input) = self.extension_input.borrow().as_ref() {
            return extension_input.render(width).len();
        }
        if let Some(remote_editor) = self.remote_editor.borrow().as_ref() {
            return remote_editor.render(width).len();
        }
        self.input.borrow().render(width).len()
    }

    fn header_lines_for_width(&self, width: usize) -> Vec<String> {
        self.extension_header_lines
            .borrow()
            .as_ref()
            .map(|lines| render_static_lines(width, lines))
            .unwrap_or_else(|| self.header.render(width))
    }

    fn footer_lines_for_width(&self, width: usize) -> Vec<String> {
        self.extension_footer_lines
            .borrow()
            .as_ref()
            .map(|lines| render_static_lines(width, lines))
            .unwrap_or_else(|| self.footer.render(width))
    }

    fn widget_lines_above_for_width(&self, width: usize) -> Vec<String> {
        render_extension_widgets(width, &self.extension_widgets_above.borrow())
    }

    fn widget_lines_below_for_width(&self, width: usize) -> Vec<String> {
        render_extension_widgets(width, &self.extension_widgets_below.borrow())
    }

    fn transcript_viewport_height_for_width(&self, width: usize) -> Option<usize> {
        let (_, total_height) = self.viewport_size.get()?;
        let occupied_height = self.header_lines_for_width(width).len()
            + self.pending_messages.borrow().render(width).len()
            + self.render_status(width).len()
            + self.widget_lines_above_for_width(width).len()
            + self.active_prompt_component_height_for_width(width)
            + self.widget_lines_below_for_width(width).len()
            + self.footer_lines_for_width(width).len();
        Some(total_height.saturating_sub(occupied_height))
    }

    fn render_status(&self, width: usize) -> Vec<String> {
        self.status_message
            .lock()
            .expect("status message mutex poisoned")
            .as_ref()
            .map(|message| {
                let styled = current_theme().fg("dim", message);
                vec![truncate_to_width(&styled, width, "...", false)]
            })
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
        let Some(index) = self
            .action_handlers
            .iter()
            .position(|(candidate, _)| candidate == action)
        else {
            return false;
        };

        let (action_name, mut handler) = self.action_handlers.remove(index);
        handler(self);
        self.action_handlers.insert(index, (action_name, handler));
        true
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

    fn should_focus_input(&self) -> bool {
        self.focused
            && self.extension_input.borrow().is_none()
            && self.extension_selector.borrow().is_none()
            && self.extension_editor.borrow().is_none()
            && self.remote_editor.borrow().is_none()
            && self.model_selector.borrow().is_none()
    }

    fn show_prompt_extension_editor(&mut self) {
        let current_input = self.input_value();
        let prefill = if current_input.is_empty() {
            None
        } else {
            Some(current_input.as_str())
        };

        self.show_extension_editor(PROMPT_EXTENSION_EDITOR_TITLE, prefill, |_| {}, || {});
        self.restore_prompt_from_extension_editor.set(true);
    }

    pub(crate) fn submit_current_input(&mut self) {
        let value = self.input_value();
        let mut on_submit = self.on_submit.take();
        if let Some(callback) = &mut on_submit {
            self.clear_input();
            self.last_clear_action.set(None);
            callback(self, value);
        }
        self.on_submit = on_submit;
    }

    fn drain_extension_input_events(&mut self) {
        loop {
            let event = self
                .extension_input_events
                .lock()
                .expect("extension input events mutex poisoned")
                .pop_front();
            let Some(event) = event else {
                break;
            };

            match event {
                ExtensionInputEvent::Submit(value) => {
                    let mut on_submit = self.extension_input_on_submit.borrow_mut().take();
                    self.hide_extension_input();
                    if let Some(callback) = &mut on_submit {
                        callback(self, value);
                    }
                }
                ExtensionInputEvent::Cancel => {
                    let mut on_cancel = self.extension_input_on_cancel.borrow_mut().take();
                    self.hide_extension_input();
                    if let Some(callback) = &mut on_cancel {
                        callback();
                    }
                }
            }
        }
    }

    fn drain_extension_selector_events(&mut self) {
        loop {
            let event = self
                .extension_selector_events
                .lock()
                .expect("extension selector events mutex poisoned")
                .pop_front();
            let Some(event) = event else {
                break;
            };

            match event {
                ExtensionSelectorEvent::Select(value) => {
                    let mut on_select = self.extension_selector_on_select.borrow_mut().take();
                    self.hide_extension_selector();
                    if let Some(callback) = &mut on_select {
                        callback(self, value);
                    }
                }
                ExtensionSelectorEvent::Cancel => {
                    let mut on_cancel = self.extension_selector_on_cancel.borrow_mut().take();
                    self.hide_extension_selector();
                    if let Some(callback) = &mut on_cancel {
                        callback();
                    }
                }
            }
        }
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
                        self.restore_prompt_from_extension_editor.get();
                    let mut on_submit = self.extension_editor_on_submit.borrow_mut().take();
                    self.hide_extension_editor();
                    if restore_prompt_from_extension_editor {
                        self.set_input_value(value.clone());
                        self.set_input_cursor(value.len());
                    }
                    if let Some(callback) = &mut on_submit {
                        callback(self, value);
                    }
                }
                ExtensionEditorEvent::Cancel => {
                    let mut on_cancel = self.extension_editor_on_cancel.borrow_mut().take();
                    self.hide_extension_editor();
                    if let Some(callback) = &mut on_cancel {
                        callback();
                    }
                }
            }
        }
    }

    fn drain_model_selector_events(&mut self) {
        loop {
            let event = self
                .model_selector_events
                .lock()
                .expect("model selector events mutex poisoned")
                .pop_front();
            let Some(event) = event else {
                break;
            };

            match event {
                ModelSelectorEvent::Select(model) => {
                    let mut on_select = self.model_selector_on_select.borrow_mut().take();
                    self.hide_model_selector();
                    if let Some(callback) = &mut on_select {
                        callback(model);
                    }
                }
                ModelSelectorEvent::Cancel => {
                    let mut on_cancel = self.model_selector_on_cancel.borrow_mut().take();
                    self.hide_model_selector();
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
            ShellUpdate::AppendBashExecution { message } => {
                self.append_bash_execution_message(message);
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
            ShellUpdate::SetPendingMessages {
                steering,
                follow_up,
            } => {
                self.pending_messages.borrow_mut().set_messages(
                    &ThemedKeyHintStyler,
                    steering,
                    follow_up,
                );
            }
            ShellUpdate::ClearPendingMessages => {
                self.pending_messages.borrow_mut().clear_messages();
            }
            ShellUpdate::ShowExtensionInput {
                title,
                placeholder,
                timeout_ms,
                mut on_submit,
                mut on_cancel,
            } => {
                self.show_extension_input(
                    title,
                    placeholder.as_deref(),
                    timeout_ms,
                    move |value| on_submit(value),
                    move || on_cancel(),
                );
            }
            ShellUpdate::ShowExtensionSelector {
                title,
                options,
                timeout_ms,
                mut on_select,
                mut on_cancel,
            } => {
                self.show_extension_selector(
                    title,
                    options,
                    timeout_ms,
                    move |value| on_select(value),
                    move || on_cancel(),
                );
            }
            ShellUpdate::ShowExtensionEditor {
                title,
                prefill,
                mut on_submit,
                mut on_cancel,
            } => {
                self.show_extension_editor(
                    title,
                    prefill.as_deref(),
                    move |value| on_submit(value),
                    move || on_cancel(),
                );
            }
            ShellUpdate::SetExtensionHeaderLines { lines } => {
                *self.extension_header_lines.borrow_mut() = lines;
            }
            ShellUpdate::SetExtensionFooterLines { lines } => {
                *self.extension_footer_lines.borrow_mut() = lines;
            }
            ShellUpdate::SetExtensionWidgetLines {
                key,
                placement,
                lines,
            } => match placement {
                ExtensionWidgetPlacement::AboveEditor => {
                    let mut widgets = self.extension_widgets_above.borrow_mut();
                    upsert_extension_widget_lines(&mut widgets, key, lines);
                }
                ExtensionWidgetPlacement::BelowEditor => {
                    let mut widgets = self.extension_widgets_below.borrow_mut();
                    upsert_extension_widget_lines(&mut widgets, key, lines);
                }
            },
            ShellUpdate::ShowRemoteEditor { lines, on_input } => {
                let mut remote_editor = RemoteEditorState::new(lines, on_input);
                if let Some((width, height)) = self.viewport_size.get() {
                    remote_editor.set_viewport_size(width, height);
                }
                remote_editor.set_focused(self.focused);
                self.input.borrow_mut().set_focused(false);
                *self.remote_editor.borrow_mut() = Some(remote_editor);
            }
            ShellUpdate::UpdateRemoteEditorLines { lines } => {
                if let Some(remote_editor) = self.remote_editor.borrow_mut().as_mut() {
                    remote_editor.set_lines(lines);
                }
            }
            ShellUpdate::HideRemoteEditor => {
                *self.remote_editor.borrow_mut() = None;
                self.input
                    .borrow_mut()
                    .set_focused(self.should_focus_input());
            }
            ShellUpdate::SetInputValue { value, cursor } => {
                self.set_input_value(value.clone());
                self.set_input_cursor(cursor.unwrap_or(value.len()));
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
            self.hide_thinking_blocks.get(),
            DEFAULT_HIDDEN_THINKING_LABEL,
        ));
        self.transcript
            .borrow_mut()
            .add_item(Box::new(component.clone()));
        self.assistant_components
            .borrow_mut()
            .push(component.clone());
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

        let mut options = ToolExecutionOptions::default();
        options.show_images = self.show_images.get();
        let component = SharedComponent::new(ToolExecutionComponent::new(
            tool_name.to_owned(),
            tool_call_id.to_owned(),
            args,
            options,
            &self.keybindings,
        ));
        component.with_mut(|component| component.set_expanded(self.tool_output_expanded.get()));
        self.transcript
            .borrow_mut()
            .add_item(Box::new(component.clone()));
        self.tool_components
            .borrow_mut()
            .insert(tool_call_id.to_owned(), component.clone());
        component
    }

    fn append_bash_execution_message(&self, message: BashExecutionMessage) {
        let (component, handle) = BashExecutionComponent::from_message(&message, &self.keybindings);
        handle.set_expanded(self.tool_output_expanded.get());
        self.transcript.borrow_mut().add_item(Box::new(component));
        self.bash_components.borrow_mut().push(handle);
    }

    pub fn set_on_submit<F>(&mut self, mut on_submit: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.set_on_submit_with_shell(move |_shell, value| on_submit(value));
    }

    pub fn set_on_submit_with_shell<F>(&mut self, on_submit: F)
    where
        F: FnMut(&mut StartupShellComponent, String) + Send + 'static,
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

    pub fn on_action<F>(&mut self, action: impl Into<String>, mut handler: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_action_with_shell(action, move |_shell| handler());
    }

    pub fn on_action_with_shell<F>(&mut self, action: impl Into<String>, handler: F)
    where
        F: FnMut(&mut StartupShellComponent) + Send + 'static,
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
        self.input.borrow().get_text()
    }

    fn sync_input_text(&self) {
        *self
            .input_text
            .lock()
            .expect("shell input text mutex poisoned") = self.input.borrow().get_text();
    }

    pub fn set_input_value(&self, value: impl Into<String>) {
        self.input.borrow_mut().set_text(value.into());
        self.sync_input_text();
    }

    pub fn insert_input_text_at_cursor(&self, text: &str) {
        self.input.borrow_mut().insert_text_at_cursor(text);
        self.sync_input_text();
    }

    pub fn set_input_cursor(&self, cursor: usize) {
        let text = self.input.borrow().get_text();
        self.input
            .borrow_mut()
            .set_cursor(editor_cursor_from_offset(&text, cursor));
    }

    pub fn clear_input(&self) {
        self.input.borrow_mut().set_text("");
        self.sync_input_text();
    }

    pub fn set_input_padding_x(&self, padding_x: usize) {
        self.input.borrow_mut().set_padding_x(padding_x);
    }

    pub fn set_autocomplete_max_visible(&self, max_visible: usize) {
        self.input
            .borrow_mut()
            .set_autocomplete_max_visible(max_visible);
    }

    pub fn set_autocomplete_provider(&self, provider: Arc<dyn AutocompleteProvider>) {
        self.input.borrow_mut().set_autocomplete_provider(provider);
    }

    pub fn clear_autocomplete_provider(&self) {
        self.input.borrow_mut().clear_autocomplete_provider();
    }

    pub fn set_extension_editor_command(&mut self, command: impl Into<String>) {
        self.extension_editor_command = Some(command.into());
    }

    pub fn clear_extension_editor_command(&mut self) {
        self.extension_editor_command = None;
    }

    pub fn set_extension_editor_command_runner<R>(&mut self, runner: R)
    where
        R: ExternalEditorCommandRunner + 'static,
    {
        self.set_extension_editor_command_runner_arc(Arc::new(runner));
    }

    pub fn set_extension_editor_command_runner_arc(
        &mut self,
        runner: Arc<dyn ExternalEditorCommandRunner>,
    ) {
        self.extension_editor_runner = Some(runner);
    }

    pub fn clear_extension_editor_command_runner(&mut self) {
        self.extension_editor_runner = None;
    }

    pub fn set_extension_editor_host<H>(&mut self, host: H)
    where
        H: ExternalEditorHost + 'static,
    {
        self.set_extension_editor_host_arc(Arc::new(host));
    }

    pub fn set_extension_editor_host_arc(&mut self, host: Arc<dyn ExternalEditorHost>) {
        self.extension_editor_host = Some(host);
    }

    pub fn clear_extension_editor_host(&mut self) {
        self.extension_editor_host = None;
    }

    pub fn show_extension_input<F, G>(
        &self,
        title: impl Into<String>,
        placeholder: Option<&str>,
        timeout_ms: Option<u64>,
        mut on_submit: F,
        on_cancel: G,
    ) where
        F: FnMut(String) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.hide_model_selector();
        self.hide_extension_editor();
        self.hide_extension_selector();
        self.hide_extension_input();

        let events = Arc::clone(&self.extension_input_events);
        let cancel_events = Arc::clone(&events);
        let mut input = ExtensionInputComponent::new(
            &self.keybindings,
            title,
            placeholder,
            move |value| {
                events
                    .lock()
                    .expect("extension input events mutex poisoned")
                    .push_back(ExtensionInputEvent::Submit(value));
            },
            move || {
                cancel_events
                    .lock()
                    .expect("extension input events mutex poisoned")
                    .push_back(ExtensionInputEvent::Cancel);
            },
            timeout_ms,
            self.render_handle.clone(),
        );

        if let Some((width, height)) = self.viewport_size.get() {
            input.set_viewport_size(width, height);
        }

        self.input.borrow_mut().set_focused(false);
        input.set_focused(self.focused);
        *self.extension_input.borrow_mut() = Some(input);
        *self.extension_input_on_submit.borrow_mut() =
            Some(Box::new(move |_shell, value| on_submit(value)));
        *self.extension_input_on_cancel.borrow_mut() = Some(Box::new(on_cancel));
    }

    pub fn hide_extension_input(&self) {
        *self.extension_input.borrow_mut() = None;
        self.extension_input_on_submit.borrow_mut().take();
        self.extension_input_on_cancel.borrow_mut().take();
        self.input
            .borrow_mut()
            .set_focused(self.should_focus_input());
    }

    pub fn is_showing_extension_input(&self) -> bool {
        self.extension_input.borrow().is_some()
    }

    pub fn show_extension_selector<F, G>(
        &self,
        title: impl Into<String>,
        options: Vec<String>,
        timeout_ms: Option<u64>,
        mut on_select: F,
        on_cancel: G,
    ) where
        F: FnMut(String) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.hide_model_selector();
        self.hide_extension_editor();
        self.hide_extension_input();
        self.hide_extension_selector();

        let events = Arc::clone(&self.extension_selector_events);
        let cancel_events = Arc::clone(&events);
        let mut selector = ExtensionSelectorComponent::new(
            &self.keybindings,
            title,
            options,
            move |value| {
                events
                    .lock()
                    .expect("extension selector events mutex poisoned")
                    .push_back(ExtensionSelectorEvent::Select(value));
            },
            move || {
                cancel_events
                    .lock()
                    .expect("extension selector events mutex poisoned")
                    .push_back(ExtensionSelectorEvent::Cancel);
            },
            timeout_ms,
            self.render_handle.clone(),
        );

        if let Some((width, height)) = self.viewport_size.get() {
            selector.set_viewport_size(width, height);
        }

        self.input.borrow_mut().set_focused(false);
        selector.set_focused(self.focused);
        *self.extension_selector.borrow_mut() = Some(selector);
        *self.extension_selector_on_select.borrow_mut() =
            Some(Box::new(move |_shell, value| on_select(value)));
        *self.extension_selector_on_cancel.borrow_mut() = Some(Box::new(on_cancel));
    }

    pub fn hide_extension_selector(&self) {
        *self.extension_selector.borrow_mut() = None;
        self.extension_selector_on_select.borrow_mut().take();
        self.extension_selector_on_cancel.borrow_mut().take();
        self.input
            .borrow_mut()
            .set_focused(self.should_focus_input());
    }

    pub fn is_showing_extension_selector(&self) -> bool {
        self.extension_selector.borrow().is_some()
    }

    pub fn show_extension_editor<F, G>(
        &self,
        title: impl Into<String>,
        prefill: Option<&str>,
        mut on_submit: F,
        on_cancel: G,
    ) where
        F: FnMut(String) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.hide_model_selector();
        self.hide_extension_selector();
        self.hide_extension_input();
        self.hide_extension_editor();

        let mut editor = ExtensionEditorComponent::new(&self.keybindings, title, prefill);
        if let Some(command) = &self.extension_editor_command {
            editor.set_external_editor_command(command.clone());
        }
        if let Some(runner) = &self.extension_editor_runner {
            editor.set_external_editor_command_runner_arc(Arc::clone(runner));
        }
        if let Some(host) = &self.extension_editor_host {
            editor.set_external_editor_host_arc(Arc::clone(host));
        }
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

        self.input.borrow_mut().set_focused(false);
        editor.set_focused(self.focused);
        *self.extension_editor.borrow_mut() = Some(editor);
        *self.extension_editor_on_submit.borrow_mut() =
            Some(Box::new(move |_shell, value| on_submit(value)));
        *self.extension_editor_on_cancel.borrow_mut() = Some(Box::new(on_cancel));
        self.restore_prompt_from_extension_editor.set(false);
    }

    pub fn hide_extension_editor(&self) {
        *self.extension_editor.borrow_mut() = None;
        self.extension_editor_on_submit.borrow_mut().take();
        self.extension_editor_on_cancel.borrow_mut().take();
        self.restore_prompt_from_extension_editor.set(false);
        self.input
            .borrow_mut()
            .set_focused(self.should_focus_input());
    }

    pub fn is_showing_extension_editor(&self) -> bool {
        self.extension_editor.borrow().is_some()
    }

    pub fn show_model_selector<F, G>(
        &self,
        current_model: Option<Model>,
        models: Vec<Model>,
        initial_search: Option<&str>,
        on_select: F,
        on_cancel: G,
    ) where
        F: FnMut(Model) + Send + 'static,
        G: FnMut() + Send + 'static,
    {
        self.hide_extension_input();
        self.hide_extension_selector();
        self.hide_extension_editor();
        self.hide_model_selector();

        let mut selector =
            ModelSelectorComponent::new(&self.keybindings, current_model, models, initial_search);
        let events = Arc::clone(&self.model_selector_events);
        selector.set_on_select(move |model| {
            events
                .lock()
                .expect("model selector events mutex poisoned")
                .push_back(ModelSelectorEvent::Select(model));
        });

        let events = Arc::clone(&self.model_selector_events);
        selector.set_on_cancel(move || {
            events
                .lock()
                .expect("model selector events mutex poisoned")
                .push_back(ModelSelectorEvent::Cancel);
        });

        if let Some((width, height)) = self.viewport_size.get() {
            selector.set_viewport_size(width, height);
        }

        self.input.borrow_mut().set_focused(false);
        selector.set_focused(self.focused);
        *self.model_selector.borrow_mut() = Some(selector);
        *self.model_selector_on_select.borrow_mut() = Some(Box::new(on_select));
        *self.model_selector_on_cancel.borrow_mut() = Some(Box::new(on_cancel));
    }

    pub fn hide_model_selector(&self) {
        *self.model_selector.borrow_mut() = None;
        self.model_selector_on_select.borrow_mut().take();
        self.model_selector_on_cancel.borrow_mut().take();
        self.input
            .borrow_mut()
            .set_focused(self.should_focus_input());
    }

    pub fn is_showing_model_selector(&self) -> bool {
        self.model_selector.borrow().is_some()
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
        self.assistant_components.borrow_mut().clear();
        self.tool_components.borrow_mut().clear();
        self.bash_components.borrow_mut().clear();
        self.transcript.borrow_mut().clear_items();
    }

    pub fn start_bash_execution(
        &mut self,
        command: impl Into<String>,
        exclude_from_context: bool,
        render_handle: RenderHandle,
    ) -> BashExecutionHandle {
        let (component, mut handle) =
            BashExecutionComponent::new(command, &self.keybindings, exclude_from_context);
        handle.set_render_handle(render_handle);
        handle.set_expanded(self.tool_output_expanded.get());
        self.transcript.borrow_mut().add_item(Box::new(component));
        self.bash_components.borrow_mut().push(handle.clone());
        handle
    }

    pub fn tools_expanded(&self) -> bool {
        self.tool_output_expanded.get()
    }

    pub fn set_tools_expanded(&self, expanded: bool) {
        self.tool_output_expanded.set(expanded);
        for component in self.tool_components.borrow().values() {
            component.with_mut(|component| component.set_expanded(expanded));
        }
        for handle in self.bash_components.borrow().iter() {
            handle.set_expanded(expanded);
        }
    }

    pub fn show_images(&self) -> bool {
        self.show_images.get()
    }

    pub fn set_show_images(&self, show_images: bool) {
        self.show_images.set(show_images);
        for component in self.tool_components.borrow().values() {
            component.with_mut(|component| component.set_show_images(show_images));
        }
    }

    pub fn hide_thinking_blocks(&self) -> bool {
        self.hide_thinking_blocks.get()
    }

    pub fn set_hide_thinking_blocks(&self, hide: bool) {
        self.hide_thinking_blocks.set(hide);
        for component in self.assistant_components.borrow().iter() {
            component.with_mut(|component| component.set_hide_thinking_block(hide));
        }
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
            .borrow_mut()
            .set_messages(styler, steering, follow_up);
    }

    pub fn clear_pending_messages(&mut self) {
        self.pending_messages.borrow_mut().clear_messages();
    }

    pub fn has_pending_messages(&self) -> bool {
        self.pending_messages.borrow().has_messages()
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

    pub fn set_render_handle(&mut self, render_handle: RenderHandle) {
        self.render_handle = Some(render_handle);
    }

    pub fn clear_render_handle(&mut self) {
        self.render_handle = None;
    }

    pub fn update_handle_with_render_handle(
        &self,
        render_handle: RenderHandle,
    ) -> ShellUpdateHandle {
        ShellUpdateHandle::new(
            Arc::clone(&self.pending_updates),
            Arc::clone(&self.input_text),
            Some(render_handle),
        )
    }

    pub fn set_footer_state(&mut self, state: FooterState) {
        self.footer.set_state(state);
    }

    pub fn footer_state_handle(&self) -> FooterStateHandle {
        self.footer.state_handle()
    }

    pub fn footer_state_handle_with_render_handle(
        &self,
        render_handle: RenderHandle,
    ) -> FooterStateHandle {
        self.footer.state_handle_with_render_handle(render_handle)
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
        let header_lines = self.header_lines_for_width(width);
        let pending_lines = self.pending_messages.borrow().render(width);
        let widget_lines_above = self.widget_lines_above_for_width(width);
        let input_lines = if let Some(model_selector) = self.model_selector.borrow().as_ref() {
            model_selector.render(width)
        } else if let Some(extension_editor) = self.extension_editor.borrow().as_ref() {
            extension_editor.render(width)
        } else if let Some(extension_selector) = self.extension_selector.borrow().as_ref() {
            extension_selector.render(width)
        } else if let Some(extension_input) = self.extension_input.borrow().as_ref() {
            extension_input.render(width)
        } else if let Some(remote_editor) = self.remote_editor.borrow().as_ref() {
            remote_editor.render(width)
        } else {
            self.input.borrow().render(width)
        };
        let widget_lines_below = self.widget_lines_below_for_width(width);
        let footer_lines = self.footer_lines_for_width(width);
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
        lines.extend(widget_lines_above);
        lines.extend(input_lines);
        lines.extend(widget_lines_below);
        lines.extend(footer_lines);
        lines
    }

    fn invalidate(&mut self) {
        self.drain_pending_updates();
        self.header.invalidate();
        self.transcript.borrow_mut().invalidate();
        self.pending_messages.borrow_mut().invalidate();
        if let Some(model_selector) = self.model_selector.borrow_mut().as_mut() {
            model_selector.invalidate();
        } else if let Some(extension_editor) = self.extension_editor.borrow_mut().as_mut() {
            extension_editor.invalidate();
        } else if let Some(extension_selector) = self.extension_selector.borrow_mut().as_mut() {
            extension_selector.invalidate();
        } else if let Some(extension_input) = self.extension_input.borrow_mut().as_mut() {
            extension_input.invalidate();
        } else {
            self.input.borrow_mut().invalidate();
        }
        self.footer.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if self.model_selector.borrow().is_some() {
            if let Some(model_selector) = self.model_selector.borrow_mut().as_mut() {
                model_selector.handle_input(data);
            }
            self.drain_model_selector_events();
            return;
        }

        if self.extension_editor.borrow().is_some() {
            if let Some(extension_editor) = self.extension_editor.borrow_mut().as_mut() {
                extension_editor.handle_input(data);
            }
            self.drain_extension_editor_events();
            return;
        }

        if self.extension_selector.borrow().is_some() {
            if let Some(extension_selector) = self.extension_selector.borrow_mut().as_mut() {
                extension_selector.handle_input(data);
            }
            self.drain_extension_selector_events();
            return;
        }

        if self.extension_input.borrow().is_some() {
            if let Some(extension_input) = self.extension_input.borrow_mut().as_mut() {
                extension_input.handle_input(data);
            }
            self.drain_extension_input_events();
            return;
        }

        if self.remote_editor.borrow().is_some() {
            if let Some(remote_editor) = self.remote_editor.borrow_mut().as_mut() {
                remote_editor.handle_input(data);
            }
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
            if !self.input.borrow().is_showing_autocomplete() {
                if let Some(on_escape) = &mut self.on_escape {
                    on_escape();
                    return;
                }
                if self.invoke_registered_action("app.interrupt") {
                    return;
                }
            }
            self.input.borrow_mut().handle_input(data);
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
            if self.input.borrow().is_showing_autocomplete() {
                self.input.borrow_mut().handle_input(data);
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

        self.input.borrow_mut().handle_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.input
            .borrow_mut()
            .set_focused(self.should_focus_input());
        if let Some(model_selector) = self.model_selector.borrow_mut().as_mut() {
            model_selector.set_focused(focused);
        }
        if let Some(extension_editor) = self.extension_editor.borrow_mut().as_mut() {
            extension_editor.set_focused(focused);
        }
        if let Some(extension_selector) = self.extension_selector.borrow_mut().as_mut() {
            extension_selector.set_focused(focused);
        }
        if let Some(extension_input) = self.extension_input.borrow_mut().as_mut() {
            extension_input.set_focused(focused);
        }
        if let Some(remote_editor) = self.remote_editor.borrow_mut().as_mut() {
            remote_editor.set_focused(focused);
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.header.set_viewport_size(width, height);
        self.transcript.borrow().set_viewport_size(width, height);
        self.pending_messages
            .borrow()
            .set_viewport_size(width, height);
        self.input.borrow().set_viewport_size(width, height);
        if let Some(model_selector) = self.model_selector.borrow().as_ref() {
            model_selector.set_viewport_size(width, height);
        }
        if let Some(extension_editor) = self.extension_editor.borrow().as_ref() {
            extension_editor.set_viewport_size(width, height);
        }
        if let Some(extension_selector) = self.extension_selector.borrow().as_ref() {
            extension_selector.set_viewport_size(width, height);
        }
        if let Some(extension_input) = self.extension_input.borrow().as_ref() {
            extension_input.set_viewport_size(width, height);
        }
        if let Some(remote_editor) = self.remote_editor.borrow_mut().as_mut() {
            remote_editor.set_viewport_size(width, height);
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
