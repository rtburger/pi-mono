use crate::{KeybindingsManager, current_theme, key_text};
use pi_coding_agent_core::BashExecutionMessage;
use pi_tui::{Component, Container, RenderHandle, Spacer, Text};
use std::sync::{Arc, Mutex};

const PREVIEW_LINES: usize = 20;

#[derive(Debug, Clone)]
enum BashExecutionStatus {
    Running,
    Complete {
        exit_code: Option<i32>,
        cancelled: bool,
        truncated: bool,
        full_output_path: Option<String>,
    },
    Error(String),
}

#[derive(Debug)]
struct BashExecutionState {
    command: String,
    output: String,
    exclude_from_context: bool,
    interrupt_key_text: String,
    expand_key_text: String,
    expanded: bool,
    status: BashExecutionStatus,
}

impl BashExecutionState {
    fn new(
        command: impl Into<String>,
        interrupt_key_text: impl Into<String>,
        expand_key_text: impl Into<String>,
        exclude_from_context: bool,
    ) -> Self {
        Self {
            command: command.into(),
            output: String::new(),
            exclude_from_context,
            interrupt_key_text: interrupt_key_text.into(),
            expand_key_text: expand_key_text.into(),
            expanded: false,
            status: BashExecutionStatus::Running,
        }
    }
}

#[derive(Clone)]
pub struct BashExecutionHandle {
    state: Arc<Mutex<BashExecutionState>>,
    render_handle: Option<RenderHandle>,
}

impl BashExecutionHandle {
    fn new(state: Arc<Mutex<BashExecutionState>>) -> Self {
        Self {
            state,
            render_handle: None,
        }
    }

    pub fn set_render_handle(&mut self, render_handle: RenderHandle) {
        self.render_handle = Some(render_handle);
    }

    pub fn set_output(&self, output: impl AsRef<str>) {
        self.state
            .lock()
            .expect("bash execution mutex poisoned")
            .output = normalize_output(output.as_ref());
        self.request_render();
    }

    pub fn append_output(&self, chunk: impl AsRef<str>) {
        let chunk = normalize_output(chunk.as_ref());
        self.state
            .lock()
            .expect("bash execution mutex poisoned")
            .output
            .push_str(&chunk);
        self.request_render();
    }

    pub fn set_complete(
        &self,
        exit_code: Option<i32>,
        cancelled: bool,
        truncated: bool,
        full_output_path: Option<String>,
    ) {
        self.state
            .lock()
            .expect("bash execution mutex poisoned")
            .status = BashExecutionStatus::Complete {
            exit_code,
            cancelled,
            truncated,
            full_output_path,
        };
        self.request_render();
    }

    pub fn set_complete_from_message(&self, message: &BashExecutionMessage) {
        self.set_output(&message.output);
        self.set_complete(
            message.exit_code.map(|exit_code| exit_code as i32),
            message.cancelled,
            message.truncated,
            message.full_output_path.clone(),
        );
    }

    pub fn set_error(&self, error: impl Into<String>) {
        self.state
            .lock()
            .expect("bash execution mutex poisoned")
            .status = BashExecutionStatus::Error(error.into());
        self.request_render();
    }

    pub fn set_expanded(&self, expanded: bool) {
        self.state
            .lock()
            .expect("bash execution mutex poisoned")
            .expanded = expanded;
        self.request_render();
    }

    pub fn expanded(&self) -> bool {
        self.state
            .lock()
            .expect("bash execution mutex poisoned")
            .expanded
    }

    fn request_render(&self) {
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }
}

pub struct BashExecutionComponent {
    state: Arc<Mutex<BashExecutionState>>,
}

impl BashExecutionComponent {
    pub fn new(
        command: impl Into<String>,
        keybindings: &KeybindingsManager,
        exclude_from_context: bool,
    ) -> (Self, BashExecutionHandle) {
        let state = Arc::new(Mutex::new(BashExecutionState::new(
            command,
            key_text(keybindings, "app.interrupt"),
            key_text(keybindings, "app.tools.expand"),
            exclude_from_context,
        )));
        let handle = BashExecutionHandle::new(Arc::clone(&state));
        (Self { state }, handle)
    }

    pub fn from_message(
        message: &BashExecutionMessage,
        keybindings: &KeybindingsManager,
    ) -> (Self, BashExecutionHandle) {
        let (component, handle) = Self::new(
            message.command.clone(),
            keybindings,
            message.exclude_from_context,
        );
        handle.set_complete_from_message(message);
        (component, handle)
    }
}

impl Component for BashExecutionComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let state = self.state.lock().expect("bash execution mutex poisoned");
        let theme = current_theme();
        let header_key = if state.exclude_from_context {
            "dim"
        } else {
            "bashMode"
        };
        let label = if state.exclude_from_context {
            "[bash no context]"
        } else {
            "[bash]"
        };

        let all_lines = state.output.lines().collect::<Vec<_>>();
        let preview_start = all_lines.len().saturating_sub(PREVIEW_LINES);
        let hidden_line_count = preview_start;
        let visible_lines = if state.expanded {
            &all_lines[..]
        } else {
            &all_lines[preview_start..]
        };

        let mut status_lines = Vec::new();
        if hidden_line_count > 0 {
            if state.expanded {
                status_lines.push(format!("({} to collapse)", state.expand_key_text));
            } else {
                status_lines.push(format!(
                    "... {hidden_line_count} more lines ({} to expand)",
                    state.expand_key_text
                ));
            }
        }

        match &state.status {
            BashExecutionStatus::Running => {
                status_lines.push(format!(
                    "Running... ({} to cancel)",
                    state.interrupt_key_text
                ));
            }
            BashExecutionStatus::Complete {
                exit_code,
                cancelled,
                truncated,
                full_output_path,
            } => {
                if *cancelled {
                    status_lines.push(String::from("(cancelled)"));
                } else if let Some(exit_code) = exit_code.filter(|exit_code| *exit_code != 0) {
                    status_lines.push(format!("(exit {exit_code})"));
                }
                if *truncated && let Some(full_output_path) = full_output_path.as_deref() {
                    status_lines.push(format!("Output truncated. Full output: {full_output_path}"));
                }
            }
            BashExecutionStatus::Error(error) => {
                status_lines.push(format!("Error: {error}"));
            }
        }

        let mut body_lines = Vec::new();
        if visible_lines.is_empty() {
            if !matches!(state.status, BashExecutionStatus::Running) {
                body_lines.push(theme.fg("muted", "(no output)"));
            }
        } else {
            body_lines.extend(visible_lines.iter().map(|line| theme.fg("muted", line)));
        }

        let styled_status_lines = status_lines
            .into_iter()
            .map(|line| {
                let key = if line.starts_with("Error:") {
                    "error"
                } else if line.contains("cancelled") || line.contains("truncated") {
                    "warning"
                } else {
                    "muted"
                };
                theme.fg(key, line)
            })
            .collect::<Vec<_>>();

        let mut content = String::new();
        content.push_str(&theme.fg(header_key, label));
        content.push_str("\n\n");
        content.push_str(&theme.fg(header_key, theme.bold(format!("$ {}", state.command))));

        if !body_lines.is_empty() {
            content.push_str("\n\n");
            content.push_str(&body_lines.join("\n"));
        }

        if !styled_status_lines.is_empty() {
            content.push_str("\n\n");
            content.push_str(&styled_status_lines.join("\n"));
        }

        let mut container = Container::new();
        container.add_child(Box::new(Spacer::new(1)));
        container.add_child(Box::new(Text::new(content, 1, 0)));
        container.add_child(Box::new(Spacer::new(1)));
        container.render(width)
    }

    fn invalidate(&mut self) {}
}

fn normalize_output(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
