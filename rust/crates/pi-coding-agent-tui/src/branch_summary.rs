use crate::{KeybindingsManager, current_theme, key_text, markdown_theme};
use pi_coding_agent_core::BranchSummaryMessage;
use pi_tui::{Box as TuiBox, Component, DefaultTextStyle, Markdown, Spacer, Text};

pub struct BranchSummaryMessageComponent {
    message: BranchSummaryMessage,
    expanded: bool,
    expand_key_text: String,
    box_component: TuiBox,
}

impl BranchSummaryMessageComponent {
    pub fn new(message: BranchSummaryMessage, keybindings: &KeybindingsManager) -> Self {
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
        let label = theme.fg("customMessageLabel", theme.bold("[branch]"));
        self.box_component
            .add_child(Box::new(Text::new(label, 0, 0)));
        self.box_component.add_child(Box::new(Spacer::new(1)));

        if self.expanded {
            self.box_component
                .add_child(Box::new(Markdown::with_default_text_style(
                    format!("**Branch Summary**\n\n{}", self.message.summary),
                    0,
                    0,
                    markdown_theme(),
                    DefaultTextStyle::new()
                        .with_color(|text| current_theme().fg("customMessageText", text)),
                )));
        } else {
            let line = format!(
                "{}{}{}",
                theme.fg("customMessageText", "Branch summary ("),
                theme.fg("dim", &self.expand_key_text),
                theme.fg("customMessageText", " to expand)"),
            );
            self.box_component
                .add_child(Box::new(Text::new(line, 0, 0)));
        }
    }
}

impl Component for BranchSummaryMessageComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.box_component.render(width)
    }

    fn invalidate(&mut self) {
        self.box_component.invalidate();
        self.rebuild();
    }
}
