use crate::{current_theme, markdown_theme};
use pi_coding_agent_core::{CustomMessage, CustomMessageContent};
use pi_events::UserContent;
use pi_tui::{Box as TuiBox, Component, Container, DefaultTextStyle, Markdown, Spacer, Text};

pub struct CustomMessageComponent {
    message: CustomMessage,
    expanded: bool,
    container: Container,
}

impl CustomMessageComponent {
    pub fn new(message: CustomMessage) -> Self {
        let mut component = Self {
            message,
            expanded: false,
            container: Container::new(),
        };
        component.rebuild();
        component
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        if self.expanded != expanded {
            self.expanded = expanded;
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        let mut box_component =
            TuiBox::with_bg_fn(1, 1, |text| current_theme().bg("customMessageBg", text));
        let theme = current_theme();
        let label = theme.fg(
            "customMessageLabel",
            theme.bold(format!("[{}]", self.message.custom_type)),
        );
        box_component.add_child(Box::new(Text::new(label, 0, 0)));
        box_component.add_child(Box::new(Spacer::new(1)));

        let body = extract_text_content(&self.message.content);
        box_component.add_child(Box::new(Markdown::with_default_text_style(
            body,
            0,
            0,
            markdown_theme(),
            DefaultTextStyle::new()
                .with_color(|text| current_theme().fg("customMessageText", text)),
        )));

        self.container.clear();
        self.container.add_child(Box::new(Spacer::new(1)));
        self.container.add_child(Box::new(box_component));
    }
}

impl Component for CustomMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
        self.rebuild();
    }
}

fn extract_text_content(content: &CustomMessageContent) -> String {
    match content {
        CustomMessageContent::Text(text) => text.clone(),
        CustomMessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                UserContent::Text { text } => Some(text.as_str()),
                UserContent::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
