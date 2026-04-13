use crate::KeybindingsManager;
use pi_tui::{AutocompleteProvider, Component, Editor, EditorCursor, EditorOptions, matches_key};
use std::ops::Deref;
use std::sync::Arc;

type ActionCallback = Box<dyn FnMut() + Send + 'static>;
type ShortcutCallback = Box<dyn FnMut(String) -> bool + Send + 'static>;

pub struct CustomEditor {
    editor: Editor,
    keybindings: KeybindingsManager,
    action_handlers: Vec<(String, ActionCallback)>,
    on_escape: Option<ActionCallback>,
    on_ctrl_d: Option<ActionCallback>,
    on_paste_image: Option<ActionCallback>,
    on_extension_shortcut: Option<ShortcutCallback>,
}

impl CustomEditor {
    pub fn new(keybindings: &KeybindingsManager) -> Self {
        Self::with_options(keybindings, EditorOptions::default())
    }

    pub fn with_options(keybindings: &KeybindingsManager, options: EditorOptions) -> Self {
        Self {
            editor: Editor::with_keybindings_and_options(keybindings.deref().clone(), options),
            keybindings: keybindings.clone(),
            action_handlers: Vec::new(),
            on_escape: None,
            on_ctrl_d: None,
            on_paste_image: None,
            on_extension_shortcut: None,
        }
    }

    pub fn get_text(&self) -> String {
        self.editor.get_text()
    }

    pub fn get_expanded_text(&self) -> String {
        self.editor.get_expanded_text()
    }

    pub fn set_text(&mut self, text: impl AsRef<str>) {
        self.editor.set_text(text);
    }

    pub fn insert_text_at_cursor(&mut self, text: impl AsRef<str>) {
        self.editor.insert_text_at_cursor(text);
    }

    pub fn set_cursor(&mut self, cursor: EditorCursor) {
        self.editor.set_cursor(cursor);
    }

    pub fn add_to_history(&mut self, text: impl AsRef<str>) {
        self.editor.add_to_history(text);
    }

    pub fn padding_x(&self) -> usize {
        self.editor.padding_x()
    }

    pub fn set_padding_x(&mut self, padding_x: usize) {
        self.editor.set_padding_x(padding_x);
    }

    pub fn autocomplete_max_visible(&self) -> usize {
        self.editor.autocomplete_max_visible()
    }

    pub fn set_autocomplete_max_visible(&mut self, max_visible: usize) {
        self.editor.set_autocomplete_max_visible(max_visible);
    }

    pub fn set_autocomplete_provider(&mut self, provider: Arc<dyn AutocompleteProvider>) {
        self.editor.set_autocomplete_provider(provider);
    }

    pub fn clear_autocomplete_provider(&mut self) {
        self.editor.clear_autocomplete_provider();
    }

    pub fn is_showing_autocomplete(&self) -> bool {
        self.editor.is_showing_autocomplete()
    }

    pub fn set_on_submit<F>(&mut self, callback: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.editor.set_on_submit(callback);
    }

    pub fn clear_on_submit(&mut self) {
        self.editor.clear_on_submit();
    }

    pub fn set_on_change<F>(&mut self, callback: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.editor.set_on_change(callback);
    }

    pub fn clear_on_change(&mut self) {
        self.editor.clear_on_change();
    }

    pub fn set_on_escape<F>(&mut self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_escape = Some(Box::new(callback));
    }

    pub fn clear_on_escape(&mut self) {
        self.on_escape = None;
    }

    pub fn set_on_ctrl_d<F>(&mut self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_ctrl_d = Some(Box::new(callback));
    }

    pub fn clear_on_ctrl_d(&mut self) {
        self.on_ctrl_d = None;
    }

    pub fn set_on_paste_image<F>(&mut self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_paste_image = Some(Box::new(callback));
    }

    pub fn clear_on_paste_image(&mut self) {
        self.on_paste_image = None;
    }

    pub fn set_on_extension_shortcut<F>(&mut self, callback: F)
    where
        F: FnMut(String) -> bool + Send + 'static,
    {
        self.on_extension_shortcut = Some(Box::new(callback));
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
}

impl Component for CustomEditor {
    fn render(&self, width: usize) -> Vec<String> {
        self.editor.render(width)
    }

    fn invalidate(&mut self) {
        self.editor.invalidate();
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

        if self.matches_binding(data, "app.interrupt") {
            if !self.is_showing_autocomplete() {
                if let Some(on_escape) = &mut self.on_escape {
                    on_escape();
                    return;
                }
                if self.invoke_registered_action("app.interrupt") {
                    return;
                }
            }
            self.editor.handle_input(data);
            return;
        }

        if self.matches_binding(data, "app.exit") {
            if self.get_text().is_empty() {
                if let Some(on_ctrl_d) = &mut self.on_ctrl_d {
                    on_ctrl_d();
                } else {
                    let _ = self.invoke_registered_action("app.exit");
                }
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

        self.editor.handle_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.editor.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.editor.set_viewport_size(width, height);
    }
}
