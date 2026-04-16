use crate::{CustomEditor, KeybindingsManager, ThemedKeyHintStyler, key_hint};
use pi_tui::{Component, matches_key, truncate_to_width};
use std::{
    cell::Cell,
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

type ActionCallback = Box<dyn FnMut() + Send + 'static>;

pub trait ExternalEditorHost: Send + Sync {
    fn stop(&self) {}
    fn start(&self) {}
    fn request_render(&self) {}
}

pub trait ExternalEditorCommandRunner: Send + Sync {
    fn run(&self, command: &str, file_path: &Path) -> io::Result<Option<i32>>;
}

#[derive(Default)]
pub struct SystemExternalEditorCommandRunner;

impl ExternalEditorCommandRunner for SystemExternalEditorCommandRunner {
    fn run(&self, command: &str, file_path: &Path) -> io::Result<Option<i32>> {
        let mut parts = command.split_whitespace();
        let Some(program) = parts.next() else {
            return Ok(None);
        };

        let status = Command::new(program).args(parts).arg(file_path).status()?;
        Ok(status.code())
    }
}

pub struct ExtensionEditorComponent {
    title: String,
    keybindings: KeybindingsManager,
    editor: CustomEditor,
    on_cancel: Option<ActionCallback>,
    on_external_editor: Option<ActionCallback>,
    external_editor_command: Option<String>,
    external_editor_runner: Arc<dyn ExternalEditorCommandRunner>,
    external_editor_host: Option<Arc<dyn ExternalEditorHost>>,
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
            external_editor_command: None,
            external_editor_runner: Arc::new(SystemExternalEditorCommandRunner),
            external_editor_host: None,
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

    pub fn set_external_editor_command(&mut self, command: impl Into<String>) {
        self.external_editor_command = Some(command.into());
    }

    pub fn clear_external_editor_command(&mut self) {
        self.external_editor_command = None;
    }

    pub fn set_external_editor_command_runner<R>(&mut self, runner: R)
    where
        R: ExternalEditorCommandRunner + 'static,
    {
        self.set_external_editor_command_runner_arc(Arc::new(runner));
    }

    pub fn set_external_editor_command_runner_arc(
        &mut self,
        runner: Arc<dyn ExternalEditorCommandRunner>,
    ) {
        self.external_editor_runner = runner;
    }

    pub fn clear_external_editor_command_runner(&mut self) {
        self.external_editor_runner = Arc::new(SystemExternalEditorCommandRunner);
    }

    pub fn set_external_editor_host<H>(&mut self, host: H)
    where
        H: ExternalEditorHost + 'static,
    {
        self.set_external_editor_host_arc(Arc::new(host));
    }

    pub fn set_external_editor_host_arc(&mut self, host: Arc<dyn ExternalEditorHost>) {
        self.external_editor_host = Some(host);
    }

    pub fn clear_external_editor_host(&mut self) {
        self.external_editor_host = None;
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn render_hint_line(&self, width: usize) -> String {
        let styler = ThemedKeyHintStyler;
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
        self.on_external_editor.is_some() || self.resolved_external_editor_command().is_some()
    }

    fn resolved_external_editor_command(&self) -> Option<String> {
        self.external_editor_command.clone().or_else(|| {
            env::var_os("VISUAL")
                .or_else(|| env::var_os("EDITOR"))
                .map(|value| value.to_string_lossy().into_owned())
        })
    }

    fn editor_height(&self, total_height: usize) -> usize {
        total_height.saturating_sub(8).max(1)
    }

    fn open_external_editor(&mut self) {
        let Some(command) = self.resolved_external_editor_command() else {
            return;
        };

        let temp_path = extension_editor_temp_path();
        if fs::write(&temp_path, self.editor.get_expanded_text()).is_err() {
            return;
        }

        if let Some(host) = &self.external_editor_host {
            host.stop();
        }

        let run_result = self.external_editor_runner.run(&command, &temp_path);
        if matches!(run_result, Ok(Some(0)))
            && let Ok(new_content) = fs::read_to_string(&temp_path)
        {
            let trimmed_content = new_content.strip_suffix('\n').unwrap_or(&new_content);
            self.editor.set_text(trimmed_content);
        }

        let _ = fs::remove_file(&temp_path);

        if let Some(host) = &self.external_editor_host {
            host.start();
            host.request_render();
        }
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
            } else {
                self.open_external_editor();
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

fn extension_editor_temp_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    env::temp_dir().join(format!(
        "pi-extension-editor-{}-{timestamp}.md",
        std::process::id()
    ))
}
