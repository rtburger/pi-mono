use crate::current_theme;
use pi_coding_agent_core::{CustomMessage, CustomMessageContent};
use pi_events::UserContent;
use pi_tui::{Component, Container, Spacer, Text};

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
        let theme = current_theme();
        self.container.clear();
        self.container.add_child(Box::new(Spacer::new(1)));
        self.container.add_child(Box::new(Text::with_custom_bg_fn(
            self.rendered_text(),
            1,
            1,
            theme.background_fill("customMessageBg"),
        )));
    }

    fn rendered_text(&self) -> String {
        let theme = current_theme();
        let mut text = theme.fg(
            "customMessageLabel",
            format!("[{}]", self.message.custom_type),
        );
        let body = extract_text_content(&self.message.content);
        if !body.is_empty() {
            text.push_str("\n\n");
            text.push_str(&theme.fg("customMessageText", body));
        }
        text
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
