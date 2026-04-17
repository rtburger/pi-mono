use crate::{KeybindingsManager, current_theme, key_text, markdown_theme};
use pi_coding_agent_core::ParsedSkillBlock;
use pi_tui::{Box as TuiBox, Component, DefaultTextStyle, Markdown, Text};

pub struct SkillInvocationMessageComponent {
    skill_block: ParsedSkillBlock,
    expanded: bool,
    expand_key_text: String,
    box_component: TuiBox,
}

impl SkillInvocationMessageComponent {
    pub fn new(skill_block: ParsedSkillBlock, keybindings: &KeybindingsManager) -> Self {
        let mut component = Self {
            skill_block,
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

        if self.expanded {
            let theme = current_theme();
            let label = theme.fg("customMessageLabel", theme.bold("[skill]"));
            self.box_component
                .add_child(Box::new(Text::new(label, 0, 0)));
            self.box_component
                .add_child(Box::new(Markdown::with_default_text_style(
                    format!(
                        "**{}**\n\n{}",
                        self.skill_block.name, self.skill_block.content
                    ),
                    0,
                    0,
                    markdown_theme(),
                    DefaultTextStyle::new()
                        .with_color(|text| current_theme().fg("customMessageText", text)),
                )));
        } else {
            let theme = current_theme();
            let line = format!(
                "{}{}{}",
                theme.fg("customMessageLabel", theme.bold("[skill] ")),
                theme.fg("customMessageText", &self.skill_block.name),
                theme.fg("dim", format!(" ({} to expand)", self.expand_key_text),),
            );
            self.box_component
                .add_child(Box::new(Text::new(line, 0, 0)));
        }
    }
}

impl Component for SkillInvocationMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.box_component.render(width)
    }

    fn invalidate(&mut self) {
        self.box_component.invalidate();
        self.rebuild();
    }
}
