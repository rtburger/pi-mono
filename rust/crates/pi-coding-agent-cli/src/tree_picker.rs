use pi_coding_agent_tui::{KeybindingsManager, PlainKeyHintStyler, key_hint};
use pi_tui::{Component, Input, fuzzy_filter, matches_key, truncate_to_width};
use std::{borrow::Cow, cell::Cell, ops::Deref};

type SelectCallback = Box<dyn FnMut(String) + Send + 'static>;
type CancelCallback = Box<dyn FnMut() + Send + 'static>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreePickerItem {
    pub entry_id: String,
    pub display: String,
    pub search_text: String,
}

pub struct TreePickerComponent {
    keybindings: KeybindingsManager,
    search_input: Input,
    items: Vec<TreePickerItem>,
    filtered_items: Vec<TreePickerItem>,
    selected_index: usize,
    on_select: Option<SelectCallback>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl TreePickerComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        items: Vec<TreePickerItem>,
        initial_selected_id: Option<&str>,
    ) -> Self {
        let mut picker = Self {
            keybindings: keybindings.clone(),
            search_input: Input::with_keybindings(keybindings.deref().clone()),
            items,
            filtered_items: Vec::new(),
            selected_index: 0,
            on_select: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        };
        picker.refresh();
        picker.select_by_id(initial_selected_id);
        picker
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

    fn refresh(&mut self) {
        let previous_id = self
            .filtered_items
            .get(self.selected_index)
            .map(|item| item.entry_id.clone());
        let query = self.search_input.get_value().trim().to_owned();
        self.filtered_items = if query.is_empty() {
            self.items.clone()
        } else {
            fuzzy_filter(&self.items, &query, |item| {
                Cow::Borrowed(item.search_text.as_str())
            })
            .into_iter()
            .cloned()
            .collect()
        };

        if let Some(previous_id) = previous_id
            && let Some(index) = self
                .filtered_items
                .iter()
                .position(|item| item.entry_id == previous_id)
        {
            self.selected_index = index;
            return;
        }

        self.selected_index = self
            .selected_index
            .min(self.filtered_items.len().saturating_sub(1));
    }

    fn select_by_id(&mut self, entry_id: Option<&str>) {
        let Some(entry_id) = entry_id else {
            return;
        };
        if let Some(index) = self
            .filtered_items
            .iter()
            .position(|item| item.entry_id == entry_id)
        {
            self.selected_index = index;
        }
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn max_visible(&self) -> usize {
        self.viewport_size
            .get()
            .map(|(_, height)| height.saturating_sub(5).max(1))
            .unwrap_or(10)
    }

    fn render_hint_line(&self, width: usize) -> String {
        let styler = PlainKeyHintStyler;
        let hint = [
            key_hint(&self.keybindings, &styler, "tui.select.confirm", "select"),
            key_hint(&self.keybindings, &styler, "tui.select.cancel", "cancel"),
            key_hint(&self.keybindings, &styler, "tui.select.down", "navigate"),
        ]
        .into_iter()
        .filter(|hint| !hint.is_empty())
        .collect::<Vec<_>>()
        .join("  ");
        truncate_to_width(&hint, width, "...", false)
    }

    fn render_tree_lines(&self, width: usize) -> Vec<String> {
        if self.filtered_items.is_empty() {
            let message = if self.search_input.get_value().trim().is_empty() {
                "No entries in session"
            } else {
                "No matching entries"
            };
            return vec![truncate_to_width(message, width, "...", false)];
        }

        let max_visible = self.max_visible();
        let start_index = self
            .selected_index
            .saturating_sub(max_visible / 2)
            .min(self.filtered_items.len().saturating_sub(max_visible));
        let end_index = (start_index + max_visible).min(self.filtered_items.len());
        let mut lines = Vec::new();

        for (visible_index, item) in self.filtered_items[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            lines.push(truncate_to_width(
                &format!("{prefix}{}", item.display),
                width,
                "...",
                false,
            ));
        }

        if start_index > 0 || end_index < self.filtered_items.len() {
            lines.push(truncate_to_width(
                &format!(
                    "  ({}/{})",
                    self.selected_index + 1,
                    self.filtered_items.len()
                ),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for TreePickerComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(
            "Navigate session tree",
            width,
            "...",
            false,
        ));
        lines.extend(self.search_input.render(width));
        lines.extend(self.render_tree_lines(width));
        lines.push(self.render_hint_line(width));
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.search_input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if self.matches_binding(data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if self.matches_binding(data, "tui.select.up") {
            if self.filtered_items.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                self.filtered_items.len() - 1
            } else {
                self.selected_index - 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            if self.filtered_items.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index + 1 >= self.filtered_items.len() {
                0
            } else {
                self.selected_index + 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.pageUp") {
            self.selected_index = self.selected_index.saturating_sub(self.max_visible());
            return;
        }

        if self.matches_binding(data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + self.max_visible())
                .min(self.filtered_items.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") {
            if let Some(item) = self.filtered_items.get(self.selected_index)
                && let Some(on_select) = &mut self.on_select
            {
                on_select(item.entry_id.clone());
            }
            return;
        }

        self.search_input.handle_input(data);
        self.refresh();
    }

    fn set_focused(&mut self, focused: bool) {
        self.search_input.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.search_input.set_viewport_size(width, 1);
    }
}
