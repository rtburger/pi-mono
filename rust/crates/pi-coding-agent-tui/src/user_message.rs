use crate::current_theme;
use pi_tui::{Component, Container, Spacer, Text};

const OSC133_ZONE_START: &str = "\x1b]133;A\x07";
const OSC133_ZONE_END: &str = "\x1b]133;B\x07";
const OSC133_ZONE_FINAL: &str = "\x1b]133;C\x07";

pub struct UserMessageComponent {
    container: Container,
}

impl UserMessageComponent {
    pub fn new(text: impl Into<String>) -> Self {
        let theme = current_theme();
        let mut container = Container::new();
        container.add_child(Box::new(Spacer::new(1)));
        container.add_child(Box::new(Text::with_custom_bg_fn(
            theme.fg("userMessageText", text.into()),
            1,
            1,
            theme.background_fill("userMessageBg"),
        )));
        Self { container }
    }
}

impl Component for UserMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = self.container.render(width);
        if lines.is_empty() {
            return lines;
        }

        lines[0] = format!("{OSC133_ZONE_START}{}", lines[0]);
        let last_index = lines.len() - 1;
        lines[last_index].push_str(OSC133_ZONE_END);
        lines[last_index].push_str(OSC133_ZONE_FINAL);
        lines
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
    }
}
