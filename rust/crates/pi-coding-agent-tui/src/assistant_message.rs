use crate::current_theme;
use pi_events::{AssistantContent, AssistantMessage, StopReason};
use pi_tui::{Component, Container, Spacer, Text};

pub const DEFAULT_HIDDEN_THINKING_LABEL: &str = "Thinking...";

pub struct AssistantMessageComponent {
    content_container: Container,
    hide_thinking_block: bool,
    hidden_thinking_label: String,
    last_message: Option<AssistantMessage>,
}

impl AssistantMessageComponent {
    pub fn new(
        message: Option<AssistantMessage>,
        hide_thinking_block: bool,
        hidden_thinking_label: impl Into<String>,
    ) -> Self {
        let mut component = Self {
            content_container: Container::new(),
            hide_thinking_block,
            hidden_thinking_label: hidden_thinking_label.into(),
            last_message: None,
        };

        if let Some(message) = message {
            component.update_content(message);
        }

        component
    }

    pub fn set_hide_thinking_block(&mut self, hide: bool) {
        if self.hide_thinking_block != hide {
            self.hide_thinking_block = hide;
            if let Some(message) = self.last_message.clone() {
                self.rebuild(&message);
            }
        }
    }

    pub fn set_hidden_thinking_label(&mut self, label: impl Into<String>) {
        self.hidden_thinking_label = label.into();
        if let Some(message) = self.last_message.clone() {
            self.rebuild(&message);
        }
    }

    pub fn update_content(&mut self, message: AssistantMessage) {
        self.last_message = Some(message.clone());
        self.rebuild(&message);
    }

    fn rebuild(&mut self, message: &AssistantMessage) {
        self.content_container.clear();

        let has_visible_content = message.content.iter().any(is_visible_content);
        if has_visible_content {
            self.content_container.add_child(Box::new(Spacer::new(1)));
        }

        for (index, content) in message.content.iter().enumerate() {
            match content {
                AssistantContent::Text { text, .. } if !text.trim().is_empty() => {
                    self.content_container.add_child(Box::new(Text::new(
                        current_theme().fg("text", text.trim()),
                        1,
                        0,
                    )));
                }
                AssistantContent::Thinking { thinking, .. } if !thinking.trim().is_empty() => {
                    let has_visible_content_after = message
                        .content
                        .iter()
                        .skip(index + 1)
                        .any(is_visible_content);

                    if self.hide_thinking_block {
                        self.content_container.add_child(Box::new(Text::new(
                            current_theme().fg("thinkingText", &self.hidden_thinking_label),
                            1,
                            0,
                        )));
                    } else {
                        self.content_container.add_child(Box::new(Text::new(
                            current_theme().fg("thinkingText", thinking.trim()),
                            1,
                            0,
                        )));
                    }

                    if has_visible_content_after {
                        self.content_container.add_child(Box::new(Spacer::new(1)));
                    }
                }
                _ => {}
            }
        }

        let has_tool_calls = message
            .content
            .iter()
            .any(|content| matches!(content, AssistantContent::ToolCall { .. }));

        if !has_tool_calls {
            match message.stop_reason {
                StopReason::Aborted => {
                    let abort_message = match message.error_message.as_deref() {
                        Some("Request was aborted") | None => "Operation aborted".to_string(),
                        Some(message) => message.to_string(),
                    };
                    self.content_container.add_child(Box::new(Spacer::new(1)));
                    self.content_container.add_child(Box::new(Text::new(
                        current_theme().fg("warning", abort_message),
                        1,
                        0,
                    )));
                }
                StopReason::Error => {
                    let error_message = message.error_message.as_deref().unwrap_or("Unknown error");
                    self.content_container.add_child(Box::new(Spacer::new(1)));
                    self.content_container.add_child(Box::new(Text::new(
                        current_theme().fg("error", format!("Error: {error_message}")),
                        1,
                        0,
                    )));
                }
                _ => {}
            }
        }
    }
}

impl Component for AssistantMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.content_container.render(width)
    }

    fn invalidate(&mut self) {
        self.content_container.invalidate();
        if let Some(message) = self.last_message.clone() {
            self.rebuild(&message);
        }
    }
}

fn is_visible_content(content: &AssistantContent) -> bool {
    match content {
        AssistantContent::Text { text, .. } => !text.trim().is_empty(),
        AssistantContent::Thinking { thinking, .. } => !thinking.trim().is_empty(),
        AssistantContent::ToolCall { .. } => false,
    }
}
