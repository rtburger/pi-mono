use crate::KeybindingsManager;
use crate::selector_common::{CancelCallback, SelectCallback, matches_binding, render_hint_line};
use pi_tui::{Component, Input, truncate_to_width};
use std::{cell::Cell, ops::Deref};

#[derive(Debug, Clone, PartialEq, Eq)]
enum LoginDialogMode {
    Idle,
    Auth {
        url: String,
        instructions: Option<String>,
    },
    Prompt {
        message: String,
        placeholder: Option<String>,
    },
    Waiting {
        message: String,
    },
}

pub struct LoginDialogComponent {
    keybindings: KeybindingsManager,
    provider_name: String,
    input: Input,
    mode: LoginDialogMode,
    progress_lines: Vec<String>,
    on_submit: Option<SelectCallback<String>>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl LoginDialogComponent {
    pub fn new(keybindings: &KeybindingsManager, provider_name: impl Into<String>) -> Self {
        Self {
            keybindings: keybindings.clone(),
            provider_name: provider_name.into(),
            input: Input::with_keybindings(keybindings.deref().clone()),
            mode: LoginDialogMode::Idle,
            progress_lines: Vec::new(),
            on_submit: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        }
    }

    pub fn set_on_submit<F>(&mut self, on_submit: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_submit = Some(Box::new(on_submit));
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    pub fn show_auth(&mut self, url: impl Into<String>, instructions: Option<&str>) {
        self.mode = LoginDialogMode::Auth {
            url: url.into(),
            instructions: instructions.map(str::to_owned),
        };
    }

    pub fn show_prompt(&mut self, message: impl Into<String>, placeholder: Option<&str>) {
        self.input.clear();
        self.mode = LoginDialogMode::Prompt {
            message: message.into(),
            placeholder: placeholder.map(str::to_owned),
        };
    }

    pub fn show_manual_input(&mut self, prompt: impl Into<String>) {
        self.show_prompt(prompt.into(), None);
    }

    pub fn show_waiting(&mut self, message: impl Into<String>) {
        self.mode = LoginDialogMode::Waiting {
            message: message.into(),
        };
    }

    pub fn show_progress(&mut self, message: impl Into<String>) {
        self.progress_lines.push(message.into());
    }

    fn render_body(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();

        match &self.mode {
            LoginDialogMode::Idle => {
                lines.push(truncate_to_width(
                    "Waiting for OAuth instructions",
                    width,
                    "...",
                    false,
                ));
            }
            LoginDialogMode::Auth { url, instructions } => {
                lines.push(truncate_to_width(url, width, "...", false));
                if let Some(instructions) = instructions {
                    lines.push(truncate_to_width(instructions, width, "...", false));
                }
            }
            LoginDialogMode::Prompt {
                message,
                placeholder,
            } => {
                lines.push(truncate_to_width(message, width, "...", false));
                if let Some(placeholder) = placeholder {
                    lines.push(truncate_to_width(
                        &format!("e.g. {placeholder}"),
                        width,
                        "...",
                        false,
                    ));
                }
                lines.extend(self.input.render(width));
            }
            LoginDialogMode::Waiting { message } => {
                lines.push(truncate_to_width(message, width, "...", false));
            }
        }

        if !self.progress_lines.is_empty() {
            lines.push(String::new());
            for progress_line in &self.progress_lines {
                lines.push(truncate_to_width(progress_line, width, "...", false));
            }
        }

        lines
    }
}

impl Component for LoginDialogComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(
            &format!("Login to {}", self.provider_name),
            width,
            "...",
            false,
        ));
        lines.extend(self.render_body(width));
        lines.push(render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "submit"),
                ("tui.select.cancel", "cancel"),
            ],
        ));
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if matches_binding(&self.keybindings, data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if matches!(self.mode, LoginDialogMode::Prompt { .. })
            && matches_binding(&self.keybindings, data, "tui.select.confirm")
        {
            if let Some(on_submit) = &mut self.on_submit {
                on_submit(self.input.get_value().to_owned());
            }
            return;
        }

        if matches!(self.mode, LoginDialogMode::Prompt { .. }) {
            self.input.handle_input(data);
        }
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}
