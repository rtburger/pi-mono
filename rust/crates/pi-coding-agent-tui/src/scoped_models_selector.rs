use crate::selector_common::{
    ActionCallback, CancelCallback, SelectCallback, matches_binding, max_visible, render_hint_line,
    visible_window,
};
use crate::{KeybindingsManager, current_theme};
use pi_events::Model;
use pi_tui::{Component, Input, fuzzy_filter, truncate_to_width};
use std::{
    borrow::Cow,
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
};

type PersistCallback = SelectCallback<Vec<String>>;
type ProviderToggleCallback = ActionCallback<(String, Vec<String>, bool)>;
type ModelToggleCallback = ActionCallback<(String, bool)>;

#[derive(Debug, Clone, PartialEq)]
pub struct ScopedModelsConfig {
    pub all_models: Vec<Model>,
    pub enabled_model_ids: BTreeSet<String>,
    pub has_enabled_models_filter: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ModelItem {
    full_id: String,
    model: Model,
    enabled: bool,
}

pub struct ScopedModelsSelectorComponent {
    keybindings: KeybindingsManager,
    models_by_id: BTreeMap<String, Model>,
    all_ids: Vec<String>,
    enabled_ids: Option<Vec<String>>,
    filtered_items: Vec<ModelItem>,
    selected_index: usize,
    search_input: Input,
    on_model_toggle: Option<ModelToggleCallback>,
    on_persist: Option<PersistCallback>,
    on_enable_all: Option<SelectCallback<Vec<String>>>,
    on_clear_all: Option<CancelCallback>,
    on_toggle_provider: Option<ProviderToggleCallback>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
    is_dirty: bool,
}

impl ScopedModelsSelectorComponent {
    pub fn new(keybindings: &KeybindingsManager, config: ScopedModelsConfig) -> Self {
        let mut models_by_id = BTreeMap::new();
        let mut all_ids = Vec::new();

        for model in config.all_models {
            let full_id = full_model_id(&model);
            all_ids.push(full_id.clone());
            models_by_id.insert(full_id, model);
        }

        let mut selector = Self {
            keybindings: keybindings.clone(),
            models_by_id,
            all_ids,
            enabled_ids: config
                .has_enabled_models_filter
                .then(|| config.enabled_model_ids.into_iter().collect()),
            filtered_items: Vec::new(),
            selected_index: 0,
            search_input: Input::with_keybindings(keybindings.deref().clone()),
            on_model_toggle: None,
            on_persist: None,
            on_enable_all: None,
            on_clear_all: None,
            on_toggle_provider: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
            is_dirty: false,
        };
        selector.refresh();
        selector
    }

    pub fn set_on_model_toggle<F>(&mut self, callback: F)
    where
        F: FnMut((String, bool)) + Send + 'static,
    {
        self.on_model_toggle = Some(Box::new(callback));
    }

    pub fn set_on_persist<F>(&mut self, callback: F)
    where
        F: FnMut(Vec<String>) + Send + 'static,
    {
        self.on_persist = Some(Box::new(callback));
    }

    pub fn set_on_enable_all<F>(&mut self, callback: F)
    where
        F: FnMut(Vec<String>) + Send + 'static,
    {
        self.on_enable_all = Some(Box::new(callback));
    }

    pub fn set_on_clear_all<F>(&mut self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_clear_all = Some(Box::new(callback));
    }

    pub fn set_on_toggle_provider<F>(&mut self, callback: F)
    where
        F: FnMut((String, Vec<String>, bool)) + Send + 'static,
    {
        self.on_toggle_provider = Some(Box::new(callback));
    }

    pub fn set_on_cancel<F>(&mut self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(callback));
    }

    fn is_enabled(&self, id: &str) -> bool {
        self.enabled_ids
            .as_ref()
            .is_none_or(|ids| ids.iter().any(|entry| entry == id))
    }

    fn build_items(&self) -> Vec<ModelItem> {
        let ordered_ids = self.ordered_ids();
        ordered_ids
            .into_iter()
            .filter_map(|id| {
                self.models_by_id.get(&id).cloned().map(|model| ModelItem {
                    full_id: id.clone(),
                    enabled: self.is_enabled(&id),
                    model,
                })
            })
            .collect()
    }

    fn ordered_ids(&self) -> Vec<String> {
        match &self.enabled_ids {
            Some(enabled_ids) => {
                let enabled_set = enabled_ids.iter().cloned().collect::<BTreeSet<_>>();
                let mut ordered = enabled_ids.clone();
                ordered.extend(
                    self.all_ids
                        .iter()
                        .filter(|id| !enabled_set.contains(*id))
                        .cloned(),
                );
                ordered
            }
            None => self.all_ids.clone(),
        }
    }

    fn refresh(&mut self) {
        let query = self.search_input.get_value().trim().to_owned();
        let items = self.build_items();
        self.filtered_items = if query.is_empty() {
            items
        } else {
            fuzzy_filter(&items, &query, |item| {
                Cow::Owned(format!(
                    "{} {} {}",
                    item.model.id, item.model.provider, item.model.name
                ))
            })
            .into_iter()
            .cloned()
            .collect()
        };
        self.selected_index = self
            .selected_index
            .min(self.filtered_items.len().saturating_sub(1));
    }

    fn enabled_count(&self) -> usize {
        self.enabled_ids
            .as_ref()
            .map_or(self.all_ids.len(), Vec::len)
    }

    fn footer_text(&self) -> String {
        let count_text = if self.enabled_ids.is_none() {
            String::from("all enabled")
        } else {
            format!("{}/{} enabled", self.enabled_count(), self.all_ids.len())
        };
        let hint = render_hint_line(
            &self.keybindings,
            usize::MAX / 2,
            &[
                ("tui.select.confirm", "toggle"),
                ("app.scopedModels.enableAll", "all"),
                ("app.scopedModels.clearAll", "clear"),
                ("app.scopedModels.toggleProvider", "provider"),
                ("app.scopedModels.moveDown", "reorder"),
                ("app.scopedModels.save", "save"),
            ],
        );
        if self.is_dirty {
            format!("{}  {} (unsaved)", hint.trim_end(), count_text)
        } else {
            format!("{}  {}", hint.trim_end(), count_text)
        }
    }

    fn render_model_lines(&self, width: usize) -> Vec<String> {
        if self.filtered_items.is_empty() {
            return vec![truncate_to_width("No matching models", width, "...", false)];
        }

        let theme = current_theme();
        let max_visible = max_visible(&self.viewport_size, 8, 15);
        let (start_index, end_index) =
            visible_window(self.selected_index, self.filtered_items.len(), max_visible);
        let mut lines = Vec::new();
        let all_enabled = self.enabled_ids.is_none();

        for (visible_index, item) in self.filtered_items[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                theme.fg("accent", "→ ")
            } else {
                String::from("  ")
            };
            let status = if all_enabled {
                String::new()
            } else if item.enabled {
                format!(" {}", theme.fg("success", "✓"))
            } else {
                format!(" {}", theme.fg("dim", "✗"))
            };
            let line = format!(
                "{prefix}{} [{}]{}",
                item.model.id, item.model.provider, status
            );
            lines.push(truncate_to_width(&line, width, "...", false));
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

        if let Some(selected) = self.filtered_items.get(self.selected_index) {
            lines.push(String::new());
            lines.push(truncate_to_width(
                &format!("  Model name: {}", selected.model.name),
                width,
                "...",
                false,
            ));
        }

        lines
    }

    fn toggle_selected_model(&mut self) {
        let Some(item) = self.filtered_items.get(self.selected_index).cloned() else {
            return;
        };

        let was_all_enabled = self.enabled_ids.is_none();
        self.enabled_ids = Some(match self.enabled_ids.take() {
            Some(mut enabled_ids) => {
                if let Some(index) = enabled_ids.iter().position(|id| *id == item.full_id) {
                    enabled_ids.remove(index);
                } else {
                    enabled_ids.push(item.full_id.clone());
                }
                enabled_ids
            }
            None => vec![item.full_id.clone()],
        });
        self.is_dirty = true;

        if was_all_enabled && let Some(on_clear_all) = &mut self.on_clear_all {
            on_clear_all();
        }

        let enabled = self.is_enabled(&item.full_id);
        if let Some(on_model_toggle) = &mut self.on_model_toggle {
            on_model_toggle((item.full_id, enabled));
        }
        self.refresh();
    }

    fn enable_all(&mut self) {
        self.enabled_ids = None;
        self.is_dirty = true;
        if let Some(on_enable_all) = &mut self.on_enable_all {
            on_enable_all(self.all_ids.clone());
        }
        self.refresh();
    }

    fn clear_all(&mut self) {
        self.enabled_ids = Some(Vec::new());
        self.is_dirty = true;
        if let Some(on_clear_all) = &mut self.on_clear_all {
            on_clear_all();
        }
        self.refresh();
    }

    fn toggle_selected_provider(&mut self) {
        let Some(item) = self.filtered_items.get(self.selected_index) else {
            return;
        };
        let provider = item.model.provider.clone();
        let provider_ids = self
            .all_ids
            .iter()
            .filter(|id| {
                self.models_by_id
                    .get(*id)
                    .is_some_and(|model| model.provider == provider)
            })
            .cloned()
            .collect::<Vec<_>>();
        let all_provider_models_enabled = provider_ids.iter().all(|id| self.is_enabled(id));

        if all_provider_models_enabled {
            self.enabled_ids = Some(match self.enabled_ids.take() {
                Some(enabled_ids) => enabled_ids
                    .into_iter()
                    .filter(|id| !provider_ids.iter().any(|provider_id| provider_id == id))
                    .collect(),
                None => self
                    .all_ids
                    .iter()
                    .filter(|id| !provider_ids.iter().any(|provider_id| provider_id == *id))
                    .cloned()
                    .collect(),
            });
        } else if let Some(mut enabled_ids) = self.enabled_ids.take() {
            for provider_id in &provider_ids {
                if !enabled_ids.iter().any(|id| id == provider_id) {
                    enabled_ids.push(provider_id.clone());
                }
            }
            self.enabled_ids = Some(enabled_ids);
        } else {
            self.enabled_ids = None;
        }

        self.is_dirty = true;
        if let Some(on_toggle_provider) = &mut self.on_toggle_provider {
            on_toggle_provider((provider, provider_ids, !all_provider_models_enabled));
        }
        self.refresh();
    }

    fn move_selected(&mut self, up: bool) {
        let Some(item) = self.filtered_items.get(self.selected_index) else {
            return;
        };
        let Some(mut enabled_ids) = self.enabled_ids.clone() else {
            return;
        };
        let Some(current_index) = enabled_ids.iter().position(|id| *id == item.full_id) else {
            return;
        };

        if up {
            if current_index == 0 {
                return;
            }
            enabled_ids.swap(current_index - 1, current_index);
            self.selected_index = self.selected_index.saturating_sub(1);
        } else {
            if current_index + 1 >= enabled_ids.len() {
                return;
            }
            enabled_ids.swap(current_index, current_index + 1);
            self.selected_index =
                (self.selected_index + 1).min(self.filtered_items.len().saturating_sub(1));
        }

        self.enabled_ids = Some(enabled_ids);
        self.is_dirty = true;
        self.refresh();
    }

    fn persist_selection(&mut self) {
        if let Some(on_persist) = &mut self.on_persist {
            let value = self
                .enabled_ids
                .clone()
                .unwrap_or_else(|| self.all_ids.clone());
            on_persist(value);
            self.is_dirty = false;
        }
    }
}

impl Component for ScopedModelsSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(
            "Configure scoped models",
            width,
            "...",
            false,
        ));
        lines.push(truncate_to_width(
            "Session-only. Save to persist in settings.",
            width,
            "...",
            false,
        ));
        lines.extend(self.search_input.render(width));
        lines.extend(self.render_model_lines(width));
        lines.push(truncate_to_width(&self.footer_text(), width, "...", false));
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.search_input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if matches_binding(&self.keybindings, data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.up") {
            if self.filtered_items.is_empty() {
                return;
            }
            self.selected_index = self.selected_index.saturating_sub(1);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            if self.filtered_items.is_empty() {
                return;
            }
            self.selected_index =
                (self.selected_index + 1).min(self.filtered_items.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "app.scopedModels.moveUp") {
            self.move_selected(true);
            return;
        }

        if matches_binding(&self.keybindings, data, "app.scopedModels.moveDown") {
            self.move_selected(false);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm") {
            self.toggle_selected_model();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.scopedModels.enableAll") {
            self.enable_all();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.scopedModels.clearAll") {
            self.clear_all();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.scopedModels.toggleProvider") {
            self.toggle_selected_provider();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.scopedModels.save") {
            self.persist_selection();
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
    }
}

fn full_model_id(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}
