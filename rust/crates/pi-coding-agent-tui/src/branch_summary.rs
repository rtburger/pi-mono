use crate::{KeybindingsManager, current_theme, key_text};
use pi_coding_agent_core::BranchSummaryMessage;
use pi_tui::{Component, Container, Spacer, Text};

pub struct BranchSummaryMessageComponent {
    message: BranchSummaryMessage,
    expanded: bool,
    expand_key_text: String,
    container: Container,
}

impl BranchSummaryMessageComponent {
    pub fn new(message: BranchSummaryMessage, keybindings: &KeybindingsManager) -> Self {
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
        self.container.add_child(Box::new(Text::new(
            current_theme().fg("accent", "[branch]"),
            1,
            0,
        )));
        self.container.add_child(Box::new(Spacer::new(1)));

        if self.expanded {
            self.container.add_child(Box::new(Text::new(
                current_theme().fg(
                    "text",
                    format!("Branch Summary\n\n{}", self.message.summary),
                ),
                1,
                0,
            )));
        } else {
            self.container.add_child(Box::new(Text::new(
                current_theme().fg(
                    "dim",
                    format!("Branch summary ({} to expand)", self.expand_key_text),
                ),
                1,
                0,
            )));
        }

        self.container.add_child(Box::new(Spacer::new(1)));
    }
}

impl Component for BranchSummaryMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
        self.rebuild();
    }
}
