use crate::{KeybindingsManager, key_text};
use pi_coding_agent_core::CompactionSummaryMessage;
use pi_tui::{Component, Container, Spacer, Text};

pub struct CompactionSummaryMessageComponent {
    message: CompactionSummaryMessage,
    expanded: bool,
    expand_key_text: String,
    container: Container,
}

impl CompactionSummaryMessageComponent {
    pub fn new(message: CompactionSummaryMessage, keybindings: &KeybindingsManager) -> Self {
        let mut component = Self {
            message,
            expanded: false,
            expand_key_text: key_text(keybindings, "app.tools.expand"),
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
        self.container.clear();
        self.container.add_child(Box::new(Spacer::new(1)));
        self.container
            .add_child(Box::new(Text::new("[compaction]", 1, 0)));
        self.container.add_child(Box::new(Spacer::new(1)));

        let tokens_before = format_with_commas(self.message.tokens_before);
        if self.expanded {
            self.container.add_child(Box::new(Text::new(
                format!(
                    "Compacted from {tokens_before} tokens\n\n{}",
                    self.message.summary
                ),
                1,
                0,
            )));
        } else {
            self.container.add_child(Box::new(Text::new(
                format!(
                    "Compacted from {tokens_before} tokens ({} to expand)",
                    self.expand_key_text
                ),
                1,
                0,
            )));
        }

        self.container.add_child(Box::new(Spacer::new(1)));
    }
}

impl Component for CompactionSummaryMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
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
