use crate::{KeybindingsManager, current_theme, key_text, markdown_theme};
use pi_coding_agent_core::CompactionSummaryMessage;
use pi_tui::{Box as TuiBox, Component, DefaultTextStyle, Markdown, Spacer, Text};

pub struct CompactionSummaryMessageComponent {
    message: CompactionSummaryMessage,
    expanded: bool,
    expand_key_text: String,
    box_component: TuiBox,
}

impl CompactionSummaryMessageComponent {
    pub fn new(message: CompactionSummaryMessage, keybindings: &KeybindingsManager) -> Self {
        let mut component = Self {
            message,
            expanded: false,
            expand_key_text: key_text(keybindings, "app.tools.expand"),
            box_component: TuiBox::with_bg_fn(1, 1, |text| {
                current_theme().bg("customMessageBg", text)
            }),
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
        self.box_component
            .set_bg_fn(|text| current_theme().bg("customMessageBg", text));
        self.box_component.clear();

        let theme = current_theme();
        let tokens_before = format_with_commas(self.message.tokens_before);
        let label = theme.fg("customMessageLabel", theme.bold("[compaction]"));
        self.box_component
            .add_child(Box::new(Text::new(label, 0, 0)));
        self.box_component.add_child(Box::new(Spacer::new(1)));

        if self.expanded {
            self.box_component
                .add_child(Box::new(Markdown::with_default_text_style(
                    format!(
                        "**Compacted from {tokens_before} tokens**\n\n{}",
                        self.message.summary
                    ),
                    0,
                    0,
                    markdown_theme(),
                    DefaultTextStyle::new()
                        .with_color(|text| current_theme().fg("customMessageText", text)),
                )));
        } else {
            let line = format!(
                "{}{}{}",
                theme.fg(
                    "customMessageText",
                    format!("Compacted from {tokens_before} tokens ("),
                ),
                theme.fg("dim", &self.expand_key_text),
                theme.fg("customMessageText", " to expand)"),
            );
            self.box_component
                .add_child(Box::new(Text::new(line, 0, 0)));
        }
    }
}

impl Component for CompactionSummaryMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.box_component.render(width)
    }

    fn invalidate(&mut self) {
        self.box_component.invalidate();
        self.rebuild();
    }
}

fn format_with_commas(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(digit);
    }

    formatted
}

#[cfg(test)]
mod tests {
    use super::format_with_commas;

    #[test]
    fn formats_large_numbers_with_grouping_commas() {
        assert_eq!(format_with_commas(0), "0");
        assert_eq!(format_with_commas(12), "12");
        assert_eq!(format_with_commas(1_234), "1,234");
        assert_eq!(format_with_commas(12_345_678), "12,345,678");
    }
}
