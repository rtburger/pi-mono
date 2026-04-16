use crate::{KeybindingsManager, current_theme, key_text};
use pi_coding_agent_core::ParsedSkillBlock;
use pi_tui::{Component, Container, Text};

pub struct SkillInvocationMessageComponent {
    skill_block: ParsedSkillBlock,
    expanded: bool,
    expand_key_text: String,
    container: Container,
}

impl SkillInvocationMessageComponent {
    pub fn new(skill_block: ParsedSkillBlock, keybindings: &KeybindingsManager) -> Self {
        let mut component = Self {
            skill_block,
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

        if self.expanded {
            self.container.add_child(Box::new(Text::new(
                current_theme().fg("accent", "[skill]"),
                0,
                0,
            )));
            self.container.add_child(Box::new(Text::new(
                current_theme().fg(
                    "text",
                    format!("{}\n\n{}", self.skill_block.name, self.skill_block.content),
                ),
                0,
                0,
            )));
        } else {
            self.container.add_child(Box::new(Text::new(
                current_theme().fg(
                    "dim",
                    format!(
                        "[skill] {} ({} to expand)",
                        self.skill_block.name, self.expand_key_text
                    ),
                ),
                0,
                0,
            )));
        }
    }
}

impl Component for SkillInvocationMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
        self.rebuild();
    }
}
