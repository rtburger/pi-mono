use crate::{
    Component, Input, KeybindingsManager, fuzzy_filter, matches_key, truncate_to_width,
    visible_width, wrap_text_with_ansi,
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub type SettingsSubmenuDone = Box<dyn FnMut(Option<String>) + Send + 'static>;
pub type SettingsSubmenuFactory =
    dyn Fn(String, SettingsSubmenuDone) -> Box<dyn Component> + Send + Sync + 'static;

type SettingsChangeCallback = Box<dyn FnMut(String, String) + Send + 'static>;
type SettingsCancelCallback = Box<dyn FnMut() + Send + 'static>;
type SettingsLabelStyleFn = dyn Fn(&str, bool) -> String + Send + Sync + 'static;
type SettingsTextStyleFn = dyn Fn(&str) -> String + Send + Sync + 'static;

pub struct SettingItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub current_value: String,
    pub values: Option<Vec<String>>,
    pub submenu: Option<Box<SettingsSubmenuFactory>>,
}

pub struct SettingsListTheme {
    label: Box<SettingsLabelStyleFn>,
    value: Box<SettingsLabelStyleFn>,
    description: Box<SettingsTextStyleFn>,
    cursor: String,
    hint: Box<SettingsTextStyleFn>,
}

impl SettingsListTheme {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_label<F>(mut self, label: F) -> Self
    where
        F: Fn(&str, bool) -> String + Send + Sync + 'static,
    {
        self.label = Box::new(label);
        self
    }

    pub fn with_value<F>(mut self, value: F) -> Self
    where
        F: Fn(&str, bool) -> String + Send + Sync + 'static,
    {
        self.value = Box::new(value);
        self
    }

    pub fn with_description<F>(mut self, description: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.description = Box::new(description);
        self
    }

    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = cursor.into();
        self
    }

    pub fn with_hint<F>(mut self, hint: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.hint = Box::new(hint);
        self
    }
}

impl Default for SettingsListTheme {
    fn default() -> Self {
        Self {
            label: Box::new(|text, _| text.to_owned()),
            value: Box::new(|text, _| text.to_owned()),
            description: Box::new(str::to_owned),
            cursor: String::from("→ "),
            hint: Box::new(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SettingsListOptions {
    pub enable_search: bool,
}

struct SubmenuState {
    component: Box<dyn Component>,
    item_index: usize,
    display_index: usize,
    completion: Arc<Mutex<Option<Option<String>>>>,
}

pub struct SettingsList {
    keybindings: KeybindingsManager,
    items: Vec<SettingItem>,
    filtered_indices: Vec<usize>,
    theme: SettingsListTheme,
    selected_index: usize,
    max_visible: usize,
    on_change: Option<SettingsChangeCallback>,
    on_cancel: Option<SettingsCancelCallback>,
    search_input: Option<Input>,
    search_enabled: bool,
    submenu: Option<SubmenuState>,
    focused: bool,
}

impl SettingsList {
    pub fn new(items: Vec<SettingItem>, max_visible: usize, theme: SettingsListTheme) -> Self {
        Self::with_keybindings(
            KeybindingsManager::with_tui_defaults(BTreeMap::new()),
            items,
            max_visible,
            theme,
            SettingsListOptions::default(),
        )
    }

    pub fn with_keybindings(
        keybindings: KeybindingsManager,
        items: Vec<SettingItem>,
        max_visible: usize,
        theme: SettingsListTheme,
        options: SettingsListOptions,
    ) -> Self {
        let mut search_input = options
            .enable_search
            .then(|| Input::with_keybindings(keybindings.clone()));
        if let Some(search_input) = &mut search_input {
            search_input.set_focused(false);
        }

        let mut settings_list = Self {
            keybindings,
            filtered_indices: Vec::new(),
            items,
            theme,
            selected_index: 0,
            max_visible,
            on_change: None,
            on_cancel: None,
            search_input,
            search_enabled: options.enable_search,
            submenu: None,
            focused: false,
        };
        settings_list.refresh_filter(false);
        settings_list
    }

    pub fn set_on_change<F>(&mut self, on_change: F)
    where
        F: FnMut(String, String) + Send + 'static,
    {
        self.on_change = Some(Box::new(on_change));
    }

    pub fn clear_on_change(&mut self) {
        self.on_change = None;
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

    pub fn update_value(&mut self, id: &str, new_value: &str) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.current_value = new_value.to_owned();
        }
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn refresh_filter(&mut self, reset_selection: bool) {
        if !self.search_enabled {
            self.filtered_indices = (0..self.items.len()).collect();
            if reset_selection {
                self.selected_index = 0;
            }
            return;
        }

        let query = self
            .search_input
            .as_ref()
            .map(|input| input.get_value().trim())
            .unwrap_or_default();

        if query.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            let indices = (0..self.items.len()).collect::<Vec<_>>();
            self.filtered_indices = fuzzy_filter(&indices, query, |index| {
                Cow::Owned(self.items[*index].label.clone())
            })
            .into_iter()
            .copied()
            .collect();
        }

        if reset_selection {
            self.selected_index = 0;
        } else {
            self.selected_index = self
                .selected_index
                .min(self.filtered_indices.len().saturating_sub(1));
        }
    }

    fn activate_item(&mut self) {
        let Some(&item_index) = self.filtered_indices.get(self.selected_index) else {
            return;
        };

        if let Some(submenu) = self.open_submenu(item_index) {
            self.submenu = Some(submenu);
            return;
        }

        let (item_id, new_value) = {
            let item = &self.items[item_index];
            let Some(values) = item.values.as_ref() else {
                return;
            };
            if values.is_empty() {
                return;
            }
            let next_index = item
                .values
                .as_ref()
                .and_then(|values| values.iter().position(|value| value == &item.current_value))
                .map_or(0, |current_index| (current_index + 1) % values.len());
            (item.id.clone(), values[next_index].clone())
        };

        self.items[item_index].current_value = new_value.clone();
        if let Some(on_change) = &mut self.on_change {
            on_change(item_id, new_value);
        }
    }

    fn open_submenu(&mut self, item_index: usize) -> Option<SubmenuState> {
        let item = self.items.get(item_index)?;
        let factory = item.submenu.as_ref()?;
        let completion = Arc::new(Mutex::new(None));
        let completion_done = Arc::clone(&completion);
        let mut component = factory(
            item.current_value.clone(),
            Box::new(move |selected_value| {
                *completion_done
                    .lock()
                    .expect("submenu completion mutex poisoned") = Some(selected_value);
            }),
        );
        component.set_focused(self.focused);
        Some(SubmenuState {
            component,
            item_index,
            display_index: self.selected_index,
            completion,
        })
    }

    fn close_submenu(&mut self) {
        if let Some(submenu) = self.submenu.take() {
            self.selected_index = submenu.display_index;
        }
    }

    fn consume_submenu_completion(&mut self) {
        let Some(submenu) = &self.submenu else {
            return;
        };

        let completion = submenu
            .completion
            .lock()
            .expect("submenu completion mutex poisoned")
            .take();
        let Some(selected_value) = completion else {
            return;
        };

        let item_index = submenu.item_index;
        if let Some(selected_value) = selected_value {
            let item_id = self.items[item_index].id.clone();
            self.items[item_index].current_value = selected_value.clone();
            if let Some(on_change) = &mut self.on_change {
                on_change(item_id, selected_value);
            }
        }

        self.close_submenu();
    }

    fn add_hint_line(&self, lines: &mut Vec<String>, width: usize) {
        lines.push(String::new());
        let hint = if self.search_enabled {
            "  Type to search · Enter/Space to change · Esc to cancel"
        } else {
            "  Enter/Space to change · Esc to cancel"
        };
        lines.push(truncate_to_width(
            &(self.theme.hint)(hint),
            width,
            "",
            false,
        ));
    }

    fn render_main_list(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some(search_input) = &self.search_input {
            lines.extend(search_input.render(width));
            lines.push(String::new());
        }

        if self.items.is_empty() {
            lines.push((self.theme.hint)("  No settings available"));
            if self.search_enabled {
                self.add_hint_line(&mut lines, width);
            }
            return lines;
        }

        if self.filtered_indices.is_empty() {
            lines.push(truncate_to_width(
                &(self.theme.hint)("  No matching settings"),
                width,
                "",
                false,
            ));
            self.add_hint_line(&mut lines, width);
            return lines;
        }

        let start_index = self
            .selected_index
            .saturating_sub(self.max_visible / 2)
            .min(self.filtered_indices.len().saturating_sub(self.max_visible));
        let end_index = (start_index + self.max_visible).min(self.filtered_indices.len());
        let max_label_width = self
            .items
            .iter()
            .map(|item| visible_width(&item.label))
            .max()
            .unwrap_or(0)
            .min(30);

        for visible_index in start_index..end_index {
            let item = &self.items[self.filtered_indices[visible_index]];
            let is_selected = visible_index == self.selected_index;
            let prefix = if is_selected {
                self.theme.cursor.as_str()
            } else {
                "  "
            };
            let prefix_width = visible_width(prefix);
            let label_width = visible_width(&item.label);
            let label_padded = format!(
                "{}{}",
                item.label,
                " ".repeat(max_label_width.saturating_sub(label_width))
            );
            let label_text = (self.theme.label)(&label_padded, is_selected);
            let separator = "  ";
            let used_width = prefix_width + max_label_width + visible_width(separator);
            let value_max_width = width.saturating_sub(used_width + 2);
            let value_text = (self.theme.value)(
                &truncate_to_width(&item.current_value, value_max_width, "", false),
                is_selected,
            );
            lines.push(truncate_to_width(
                &format!("{prefix}{label_text}{separator}{value_text}"),
                width,
                "",
                false,
            ));
        }

        if start_index > 0 || end_index < self.filtered_indices.len() {
            let scroll_text = format!(
                "  ({}/{})",
                self.selected_index + 1,
                self.filtered_indices.len()
            );
            lines.push((self.theme.hint)(&truncate_to_width(
                &scroll_text,
                width.saturating_sub(2),
                "",
                false,
            )));
        }

        if let Some(selected_index) = self.filtered_indices.get(self.selected_index)
            && let Some(description) = self.items[*selected_index].description.as_deref()
            && !description.is_empty()
        {
            lines.push(String::new());
            for line in wrap_text_with_ansi(description, width.saturating_sub(4).max(1)) {
                lines.push((self.theme.description)(&format!("  {line}")));
            }
        }

        self.add_hint_line(&mut lines, width);
        lines
    }
}

impl Component for SettingsList {
    fn render(&self, width: usize) -> Vec<String> {
        if let Some(submenu) = &self.submenu {
            return submenu.component.render(width);
        }

        self.render_main_list(width)
    }

    fn invalidate(&mut self) {
        if let Some(search_input) = &mut self.search_input {
            search_input.invalidate();
        }
        if let Some(submenu) = &mut self.submenu {
            submenu.component.invalidate();
        }
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(submenu) = &mut self.submenu {
            submenu.component.handle_input(data);
            self.consume_submenu_completion();
            return;
        }

        if self.matches_binding(data, "tui.select.up") {
            if self.filtered_indices.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                self.filtered_indices.len() - 1
            } else {
                self.selected_index - 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            if self.filtered_indices.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index + 1 >= self.filtered_indices.len() {
                0
            } else {
                self.selected_index + 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") || data == " " {
            self.activate_item();
            return;
        }

        if self.matches_binding(data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if self.search_enabled
            && let Some(search_input) = &mut self.search_input
        {
            let sanitized = data.replace(' ', "");
            if sanitized.is_empty() {
                return;
            }
            search_input.handle_input(&sanitized);
            self.refresh_filter(true);
        }
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if let Some(search_input) = &mut self.search_input {
            search_input.set_focused(focused);
        }
        if let Some(submenu) = &mut self.submenu {
            submenu.component.set_focused(focused);
        }
    }
}
