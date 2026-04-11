use crate::{CustomEditor, KeybindingsManager, PlainKeyHintStyler, key_hint};
use pi_tui::{Component, matches_key, truncate_to_width};
use std::cell::Cell;

type ActionCallback = Box<dyn FnMut() + Send + 'static>;

pub struct ExtensionEditorComponent {
    title: String,
    keybindings: KeybindingsManager,
    editor: CustomEditor,
    on_cancel: Option<ActionCallback>,
    on_external_editor: Option<ActionCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl ExtensionEditorComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        title: impl Into<String>,
        prefill: Option<&str>,
    ) -> Self {
        let mut editor = CustomEditor::new(keybindings);
        if let Some(prefill) = prefill {
            editor.set_text(prefill);
        }
        Self {
            title: title.into(),
            keybindings: keybindings.clone(),
            editor,
            on_cancel: None,
            on_external_editor: None,
            viewport_size: Cell::new(None),
        }
    }

    pub fn get_text(&self) -> String {
        self.editor.get_text()
    }

    pub fn set_text(&mut self, text: impl AsRef<str>) {
        self.editor.set_text(text);
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

    pub fn set_on_cancel<F>(&mut self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(callback));
    }

    pub fn clear_on_cancel(&mut self) {
        self.on_cancel = None;
    }

    pub fn set_on_external_editor<F>(&mut self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_external_editor = Some(Box::new(callback));
    }

    pub fn clear_on_external_editor(&mut self) {
        self.on_external_editor = None;
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn render_hint_line(&self, width: usize) -> String {
        let styler = PlainKeyHintStyler;
        let mut hint = key_hint(&self.keybindings, &styler, "tui.select.confirm", "submit");
        hint.push_str("  ");
        hint.push_str(&key_hint(
            &self.keybindings,
            &styler,
            "tui.input.newLine",
            "newline",
        ));
        hint.push_str("  ");
        hint.push_str(&key_hint(
            &self.keybindings,
            &styler,
            "tui.select.cancel",
            "cancel",
        ));
        if self.shows_external_editor_hint() {
            hint.push_str("  ");
            hint.push_str(&key_hint(
                &self.keybindings,
                &styler,
                "app.editor.external",
                "external editor",
            ));
        }
        truncate_to_width(&hint, width, "...", false)
    }

    fn shows_external_editor_hint(&self) -> bool {
        self.on_external_editor.is_some()
            || std::env::var_os("VISUAL").is_some()
            || std::env::var_os("EDITOR").is_some()
    }

    fn editor_height(&self, total_height: usize) -> usize {
        total_height.saturating_sub(8).max(1)
    }
}

impl Component for ExtensionEditorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let editor_height = self
            .viewport_size
            .get()
            .map(|(_, height)| self.editor_height(height))
            .unwrap_or(24);
        self.editor.set_viewport_size(width, editor_height);

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(String::new());
        lines.push(truncate_to_width(&self.title, width, "...", false));
        lines.push(String::new());
        lines.extend(self.editor.render(width));
        lines.push(String::new());
        lines.push(self.render_hint_line(width));
        lines.push(String::new());
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.editor.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if self.matches_binding(data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if self.matches_binding(data, "app.editor.external") {
            if let Some(on_external_editor) = &mut self.on_external_editor {
                on_external_editor();
            }
            return;
        }

        self.editor.handle_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.editor.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.editor
            .set_viewport_size(width, self.editor_height(height));
    }
}
