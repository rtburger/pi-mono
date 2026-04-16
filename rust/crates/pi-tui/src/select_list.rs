use crate::{Component, KeybindingsManager, matches_key, truncate_to_width, visible_width};
use std::{collections::BTreeMap, sync::Arc};

const DEFAULT_PRIMARY_COLUMN_WIDTH: usize = 32;
const PRIMARY_COLUMN_GAP: usize = 2;
const MIN_DESCRIPTION_WIDTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone)]
pub struct SelectListTheme {
    selected_prefix: Arc<SelectListTextStyleFn>,
    selected_text: Arc<SelectListTextStyleFn>,
    description: Arc<SelectListTextStyleFn>,
    scroll_info: Arc<SelectListTextStyleFn>,
    no_match: Arc<SelectListTextStyleFn>,
}

type SelectListTextStyleFn = dyn Fn(&str) -> String + Send + Sync + 'static;
type SelectListTruncatePrimaryFn =
    dyn for<'a> Fn(&SelectListTruncatePrimaryContext<'a>) -> String + Send + Sync + 'static;
type SelectCallback = Box<dyn FnMut(SelectItem) + Send + 'static>;
type CancelCallback = Box<dyn FnMut() + Send + 'static>;

impl SelectListTheme {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_selected_prefix<F>(mut self, selected_prefix: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.selected_prefix = Arc::new(selected_prefix);
        self
    }

    pub fn with_selected_text<F>(mut self, selected_text: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.selected_text = Arc::new(selected_text);
        self
    }

    pub fn with_description<F>(mut self, description: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.description = Arc::new(description);
        self
    }

    pub fn with_scroll_info<F>(mut self, scroll_info: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.scroll_info = Arc::new(scroll_info);
        self
    }

    pub fn with_no_match<F>(mut self, no_match: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.no_match = Arc::new(no_match);
        self
    }
}

impl Default for SelectListTheme {
    fn default() -> Self {
        Self {
            selected_prefix: Arc::new(str::to_owned),
            selected_text: Arc::new(str::to_owned),
            description: Arc::new(str::to_owned),
            scroll_info: Arc::new(str::to_owned),
            no_match: Arc::new(str::to_owned),
        }
    }
}

#[derive(Clone, Default)]
pub struct SelectListLayoutOptions {
    pub min_primary_column_width: Option<usize>,
    pub max_primary_column_width: Option<usize>,
    pub truncate_primary: Option<Arc<SelectListTruncatePrimaryFn>>,
}

impl SelectListLayoutOptions {
    pub fn with_min_primary_column_width(mut self, width: usize) -> Self {
        self.min_primary_column_width = Some(width);
        self
    }

    pub fn with_max_primary_column_width(mut self, width: usize) -> Self {
        self.max_primary_column_width = Some(width);
        self
    }

    pub fn with_truncate_primary<F>(mut self, truncate_primary: F) -> Self
    where
        F: for<'a> Fn(&SelectListTruncatePrimaryContext<'a>) -> String + Send + Sync + 'static,
    {
        self.truncate_primary = Some(Arc::new(truncate_primary));
        self
    }
}

pub struct SelectListTruncatePrimaryContext<'a> {
    pub text: &'a str,
    pub max_width: usize,
    pub column_width: usize,
    pub item: &'a SelectItem,
    pub is_selected: bool,
}

pub struct SelectList {
    keybindings: KeybindingsManager,
    items: Vec<SelectItem>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    max_visible: usize,
    theme: SelectListTheme,
    layout: SelectListLayoutOptions,
    on_select: Option<SelectCallback>,
    on_cancel: Option<CancelCallback>,
    on_selection_change: Option<SelectCallback>,
}

impl SelectList {
    pub fn new(items: Vec<SelectItem>, max_visible: usize, theme: SelectListTheme) -> Self {
        Self::with_keybindings(
            KeybindingsManager::with_tui_defaults(BTreeMap::new()),
            items,
            max_visible,
            theme,
            SelectListLayoutOptions::default(),
        )
    }

    pub fn with_layout(
        items: Vec<SelectItem>,
        max_visible: usize,
        theme: SelectListTheme,
        layout: SelectListLayoutOptions,
    ) -> Self {
        Self::with_keybindings(
            KeybindingsManager::with_tui_defaults(BTreeMap::new()),
            items,
            max_visible,
            theme,
            layout,
        )
    }

    pub fn with_keybindings(
        keybindings: KeybindingsManager,
        items: Vec<SelectItem>,
        max_visible: usize,
        theme: SelectListTheme,
        layout: SelectListLayoutOptions,
    ) -> Self {
        let filtered_indices = (0..items.len()).collect();
        Self {
            keybindings,
            items,
            filtered_indices,
            selected_index: 0,
            max_visible,
            theme,
            layout,
            on_select: None,
            on_cancel: None,
            on_selection_change: None,
        }
    }

    pub fn set_filter(&mut self, filter: &str) {
        let filter = filter.trim().to_ascii_lowercase();
        if filter.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            self.filtered_indices = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    item.value
                        .to_ascii_lowercase()
                        .starts_with(&filter)
                        .then_some(index)
                })
                .collect();
        }
        self.selected_index = 0;
    }

    pub fn set_selected_index(&mut self, index: usize) {
        self.selected_index = index.min(self.filtered_indices.len().saturating_sub(1));
    }

    pub fn set_on_select<F>(&mut self, on_select: F)
    where
        F: FnMut(SelectItem) + Send + 'static,
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

    pub fn set_on_selection_change<F>(&mut self, on_selection_change: F)
    where
        F: FnMut(SelectItem) + Send + 'static,
    {
        self.on_selection_change = Some(Box::new(on_selection_change));
    }

    pub fn clear_on_selection_change(&mut self) {
        self.on_selection_change = None;
    }

    pub fn get_selected_item(&self) -> Option<SelectItem> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|index| self.items.get(*index))
            .cloned()
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn render_item(
        &self,
        item: &SelectItem,
        is_selected: bool,
        width: usize,
        primary_width: usize,
    ) -> String {
        let prefix = if is_selected { "→ " } else { "  " };
        let prefix_width = visible_width(prefix);
        let description_single_line = item
            .description
            .as_deref()
            .map(normalize_to_single_line)
            .filter(|description| !description.is_empty());

        if let Some(description_single_line) = description_single_line
            && width > 40
        {
            let effective_primary_width = primary_width
                .min(width.saturating_sub(prefix_width + 4))
                .max(1);
            let max_primary_width = effective_primary_width
                .saturating_sub(PRIMARY_COLUMN_GAP)
                .max(1);
            let truncated_value = self.truncate_primary(
                item,
                is_selected,
                max_primary_width,
                effective_primary_width,
            );
            let truncated_value_width = visible_width(&truncated_value);
            let spacing = " ".repeat(
                effective_primary_width
                    .saturating_sub(truncated_value_width)
                    .max(1),
            );
            let description_start = prefix_width + truncated_value_width + spacing.len();
            let remaining_width = width.saturating_sub(description_start + 2);

            if remaining_width > MIN_DESCRIPTION_WIDTH {
                let truncated_desc =
                    truncate_to_width(&description_single_line, remaining_width, "", false);
                if is_selected {
                    let styled_prefix = (self.theme.selected_prefix)(prefix);
                    let styled_body = (self.theme.selected_text)(&format!(
                        "{truncated_value}{spacing}{truncated_desc}"
                    ));
                    return format!("{styled_prefix}{styled_body}");
                }

                let desc_text = (self.theme.description)(&format!("{spacing}{truncated_desc}"));
                return format!("{prefix}{truncated_value}{desc_text}");
            }
        }

        let max_width = width.saturating_sub(prefix_width + 2);
        let truncated_value = self.truncate_primary(item, is_selected, max_width, max_width);
        if is_selected {
            let styled_prefix = (self.theme.selected_prefix)(prefix);
            let styled_body = (self.theme.selected_text)(&truncated_value);
            return format!("{styled_prefix}{styled_body}");
        }

        format!("{prefix}{truncated_value}")
    }

    fn get_primary_column_width(&self) -> usize {
        let (min_width, max_width) = self.get_primary_column_bounds();
        let widest_primary = self.filtered_indices.iter().fold(0usize, |widest, index| {
            let item = &self.items[*index];
            widest.max(visible_width(&self.get_display_value(item)) + PRIMARY_COLUMN_GAP)
        });
        widest_primary.clamp(min_width, max_width)
    }

    fn get_primary_column_bounds(&self) -> (usize, usize) {
        let raw_min = self.layout.min_primary_column_width.unwrap_or(
            self.layout
                .max_primary_column_width
                .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH),
        );
        let raw_max = self.layout.max_primary_column_width.unwrap_or(
            self.layout
                .min_primary_column_width
                .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH),
        );
        (raw_min.min(raw_max).max(1), raw_min.max(raw_max).max(1))
    }

    fn truncate_primary(
        &self,
        item: &SelectItem,
        is_selected: bool,
        max_width: usize,
        column_width: usize,
    ) -> String {
        let display_value = self.get_display_value(item);
        let truncated = if let Some(truncate_primary) = &self.layout.truncate_primary {
            truncate_primary(&SelectListTruncatePrimaryContext {
                text: &display_value,
                max_width,
                column_width,
                item,
                is_selected,
            })
        } else {
            truncate_to_width(&display_value, max_width, "", false)
        };

        truncate_to_width(&truncated, max_width, "", false)
    }

    fn get_display_value(&self, item: &SelectItem) -> String {
        if item.label.is_empty() {
            item.value.clone()
        } else {
            item.label.clone()
        }
    }

    fn notify_selection_change(&mut self) {
        let selected_item = self.get_selected_item();
        if let Some(selected_item) = selected_item
            && let Some(on_selection_change) = &mut self.on_selection_change
        {
            on_selection_change(selected_item);
        }
    }
}

impl Component for SelectList {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        if self.filtered_indices.is_empty() {
            return vec![(self.theme.no_match)("  No matching commands")];
        }

        let primary_width = self.get_primary_column_width();
        let start_index = self
            .selected_index
            .saturating_sub(self.max_visible / 2)
            .min(self.filtered_indices.len().saturating_sub(self.max_visible));
        let end_index = (start_index + self.max_visible).min(self.filtered_indices.len());
        let mut lines = Vec::new();

        for visible_index in start_index..end_index {
            let item = &self.items[self.filtered_indices[visible_index]];
            lines.push(self.render_item(
                item,
                visible_index == self.selected_index,
                width,
                primary_width,
            ));
        }

        if start_index > 0 || end_index < self.filtered_indices.len() {
            let scroll_text = format!(
                "  ({}/{})",
                self.selected_index + 1,
                self.filtered_indices.len()
            );
            lines.push((self.theme.scroll_info)(&truncate_to_width(
                &scroll_text,
                width.saturating_sub(2),
                "",
                false,
            )));
        }

        lines
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        if self.filtered_indices.is_empty() {
            if self.matches_binding(data, "tui.select.cancel")
                && let Some(on_cancel) = &mut self.on_cancel
            {
                on_cancel();
            }
            return;
        }

        if self.matches_binding(data, "tui.select.up") {
            self.selected_index = if self.selected_index == 0 {
                self.filtered_indices.len() - 1
            } else {
                self.selected_index - 1
            };
            self.notify_selection_change();
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            self.selected_index = if self.selected_index + 1 >= self.filtered_indices.len() {
                0
            } else {
                self.selected_index + 1
            };
            self.notify_selection_change();
            return;
        }

        if self.matches_binding(data, "tui.select.pageUp") {
            self.selected_index = self.selected_index.saturating_sub(self.max_visible.max(1));
            self.notify_selection_change();
            return;
        }

        if self.matches_binding(data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + self.max_visible.max(1))
                .min(self.filtered_indices.len().saturating_sub(1));
            self.notify_selection_change();
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") {
            if let Some(item) = self.get_selected_item()
                && let Some(on_select) = &mut self.on_select
            {
                on_select(item);
            }
            return;
        }

        if self.matches_binding(data, "tui.select.cancel")
            && let Some(on_cancel) = &mut self.on_cancel
        {
            on_cancel();
        }
    }
}

fn normalize_to_single_line(text: &str) -> String {
    text.lines().collect::<Vec<_>>().join(" ").trim().to_owned()
}
