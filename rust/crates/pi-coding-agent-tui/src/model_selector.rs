use crate::{KeybindingsManager, ThemedKeyHintStyler, key_hint};
use pi_events::Model;
use pi_tui::{Component, Input, fuzzy_filter, matches_key, truncate_to_width};
use std::{borrow::Cow, cell::Cell, ops::Deref};

type SelectCallback = Box<dyn FnMut(Model) + Send + 'static>;
type CancelCallback = Box<dyn FnMut() + Send + 'static>;

pub struct ModelSelectorComponent {
    keybindings: KeybindingsManager,
    current_model: Option<Model>,
    search_input: Input,
    models: Vec<Model>,
    filtered_models: Vec<Model>,
    selected_index: usize,
    on_select: Option<SelectCallback>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
    focused: bool,
}

impl ModelSelectorComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        current_model: Option<Model>,
        mut models: Vec<Model>,
        initial_search: Option<&str>,
    ) -> Self {
        models.sort_by(|left, right| compare_models(&current_model, left, right));

        let mut search_input = Input::with_keybindings(keybindings.deref().clone());
        if let Some(initial_search) = initial_search {
            search_input.set_value(initial_search);
        }

        let mut selector = Self {
            keybindings: keybindings.clone(),
            current_model,
            search_input,
            models,
            filtered_models: Vec::new(),
            selected_index: 0,
            on_select: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
            focused: false,
        };
        selector.refresh();
        selector
    }

    pub fn set_on_select<F>(&mut self, on_select: F)
    where
        F: FnMut(Model) + Send + 'static,
    {
        self.on_select = Some(Box::new(on_select));
    }

    pub fn clear_on_select(&mut self) {
        self.on_select = None;
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    pub fn clear_on_cancel(&mut self) {
        self.on_cancel = None;
    }

    fn refresh(&mut self) {
        let query = self.search_input.get_value();
        self.filtered_models = if query.trim().is_empty() {
            self.models.clone()
        } else {
            fuzzy_filter(&self.models, query, |model| {
                Cow::Owned(format!(
                    "{} {} {}/{}",
                    model.id, model.provider, model.provider, model.id
                ))
            })
            .into_iter()
            .cloned()
            .collect()
        };

        self.selected_index = self
            .selected_index
            .min(self.filtered_models.len().saturating_sub(1));
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
        let styler = ThemedKeyHintStyler;
        let hint = format!(
            "{}  {}  {}",
            key_hint(&self.keybindings, &styler, "tui.select.confirm", "select"),
            key_hint(&self.keybindings, &styler, "tui.select.cancel", "cancel"),
            key_hint(&self.keybindings, &styler, "tui.select.down", "navigate"),
        );
        truncate_to_width(&hint, width, "...", false)
    }

    fn render_model_lines(&self, width: usize) -> Vec<String> {
        if self.filtered_models.is_empty() {
            return vec![truncate_to_width("No matching models", width, "...", false)];
        }

        let max_visible = self.max_visible();
        let start_index = self
            .selected_index
            .saturating_sub(max_visible / 2)
            .min(self.filtered_models.len().saturating_sub(max_visible));
        let end_index = (start_index + max_visible).min(self.filtered_models.len());
        let mut lines = Vec::new();

        for (visible_index, model) in self.filtered_models[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let current_suffix = if same_model_reference(self.current_model.as_ref(), Some(model)) {
                " ✓"
            } else {
                ""
            };
            let line = format!("{prefix}{} [{}]{current_suffix}", model.id, model.provider);
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        if start_index > 0 || end_index < self.filtered_models.len() {
            lines.push(truncate_to_width(
                &format!(
                    "  ({}/{})",
                    self.selected_index + 1,
                    self.filtered_models.len()
                ),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for ModelSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width("Select model", width, "...", false));
        lines.extend(self.search_input.render(width));
        lines.extend(self.render_model_lines(width));
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
            if self.filtered_models.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                self.filtered_models.len() - 1
            } else {
                self.selected_index - 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            if self.filtered_models.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index + 1 >= self.filtered_models.len() {
                0
            } else {
                self.selected_index + 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.pageUp") {
            let page = self.max_visible();
            self.selected_index = self.selected_index.saturating_sub(page);
            return;
        }

        if self.matches_binding(data, "tui.select.pageDown") {
            let page = self.max_visible();
            self.selected_index =
                (self.selected_index + page).min(self.filtered_models.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") {
            if let Some(model) = self.filtered_models.get(self.selected_index).cloned()
                && let Some(on_select) = &mut self.on_select
            {
                on_select(model);
            }
            return;
        }

        self.search_input.handle_input(data);
        self.refresh();
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.search_input.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.search_input.set_viewport_size(width, 1);
    }
}

fn compare_models(
    current_model: &Option<Model>,
    left: &Model,
    right: &Model,
) -> std::cmp::Ordering {
    let left_is_current = same_model_reference(current_model.as_ref(), Some(left));
    let right_is_current = same_model_reference(current_model.as_ref(), Some(right));
    left_is_current
        .cmp(&right_is_current)
        .reverse()
        .then_with(|| left.provider.cmp(&right.provider))
        .then_with(|| left.id.cmp(&right.id))
}

fn same_model_reference(left: Option<&Model>, right: Option<&Model>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left.provider == right.provider && left.id == right.id)
}
