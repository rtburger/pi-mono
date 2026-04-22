use crate::selector_common::{
    framed_lines, matches_binding, render_hint_line, sanitize_display_text, visible_window,
};
use crate::{KeybindingsManager, current_theme};
use pi_tui::{Component, truncate_to_width};
use std::cell::Cell;

type SelectCallback = Box<dyn FnMut(String) + Send + 'static>;
type CancelCallback = Box<dyn FnMut() + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessageSelectorItem {
    pub id: String,
    pub text: String,
    pub is_root: bool,
}

pub struct UserMessageSelectorComponent {
    keybindings: KeybindingsManager,
    items: Vec<UserMessageSelectorItem>,
    selected_index: usize,
    on_select: Option<SelectCallback>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl UserMessageSelectorComponent {
    pub fn new(keybindings: &KeybindingsManager, items: Vec<UserMessageSelectorItem>) -> Self {
        Self {
            keybindings: keybindings.clone(),
            selected_index: items.len().saturating_sub(1),
            items,
            on_select: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        }
    }

    pub fn set_on_select<F>(&mut self, on_select: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_select = Some(Box::new(on_select));
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    fn max_visible_items(&self) -> usize {
        self.viewport_size
            .get()
            .map(|(_, height)| height.saturating_sub(4).max(3) / 3)
            .unwrap_or(10)
            .max(1)
    }

    fn render_message_lines(&self, width: usize) -> Vec<String> {
        if self.items.is_empty() {
            return vec![truncate_to_width(
                "No messages to fork from",
                width,
                "...",
                false,
            )];
        }

        let (start_index, end_index) = visible_window(
            self.selected_index,
            self.items.len(),
            self.max_visible_items(),
        );
        let theme = current_theme();
        let mut lines = Vec::new();

        for (visible_index, item) in self.items[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let is_selected = actual_index == self.selected_index;
            let prefix = if is_selected {
                theme.fg("accent", "› ")
            } else {
                String::from("  ")
            };
            let message = sanitize_display_text(&item.text);
            let message = truncate_to_width(&message, width.saturating_sub(2), "...", false);
            let message = if is_selected {
                theme.bold(&message)
            } else {
                message
            };
            lines.push(format!("{prefix}{message}"));

            let mut metadata = format!("  Message {} of {}", actual_index + 1, self.items.len());
            if item.is_root {
                metadata.push_str(" · root");
            }
            lines.push(theme.fg("muted", metadata));
            lines.push(String::new());
        }

        if start_index > 0 || end_index < self.items.len() {
            lines.push(current_theme().fg(
                "muted",
                format!("  ({}/{})", self.selected_index + 1, self.items.len()),
            ));
        }

        lines
    }
}

impl Component for UserMessageSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let body = self.render_message_lines(width);
        let hint_line = render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "select"),
                ("tui.select.cancel", "cancel"),
                ("tui.select.down", "navigate"),
            ],
        );
        framed_lines(
            width,
            "Fork session from user message",
            body,
            Some(hint_line),
        )
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        if matches_binding(&self.keybindings, data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.up") {
            if self.items.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                self.items.len() - 1
            } else {
                self.selected_index - 1
            };
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            if self.items.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index + 1 >= self.items.len() {
                0
            } else {
                self.selected_index + 1
            };
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
            self.selected_index = self.selected_index.saturating_sub(self.max_visible_items());
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + self.max_visible_items())
                .min(self.items.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm") || data == "\n" {
            if let Some(item) = self.items.get(self.selected_index)
                && let Some(on_select) = &mut self.on_select
            {
                on_select(item.id.clone());
            }
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}
