use crate::KeybindingsManager;
use crate::selector_common::{
    ActionCallback, CancelCallback, matches_binding, max_visible, sanitize_display_text,
    visible_window,
};
use pi_tui::{Component, Input, truncate_to_width};
use std::{cell::Cell, ops::Deref};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigResourceType {
    Extensions,
    Skills,
    Prompts,
    Themes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigResourceItem {
    pub id: String,
    pub path: String,
    pub display_name: String,
    pub enabled: bool,
    pub resource_type: ConfigResourceType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigResourceSubgroup {
    pub label: String,
    pub items: Vec<ConfigResourceItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigResourceGroup {
    pub label: String,
    pub subgroups: Vec<ConfigResourceSubgroup>,
}

#[derive(Debug, Clone)]
enum FlatEntry {
    Group(String),
    Subgroup(String),
    Item(ConfigResourceItem),
}

pub struct ConfigSelectorComponent {
    keybindings: KeybindingsManager,
    search_input: Input,
    groups: Vec<ConfigResourceGroup>,
    filtered_entries: Vec<FlatEntry>,
    selected_index: usize,
    on_toggle: Option<ActionCallback<(String, bool)>>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl ConfigSelectorComponent {
    pub fn new(keybindings: &KeybindingsManager, groups: Vec<ConfigResourceGroup>) -> Self {
        let mut selector = Self {
            keybindings: keybindings.clone(),
            search_input: Input::with_keybindings(keybindings.deref().clone()),
            groups,
            filtered_entries: Vec::new(),
            selected_index: 0,
            on_toggle: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        };
        selector.refresh();
        selector
    }

    pub fn set_on_toggle<F>(&mut self, on_toggle: F)
    where
        F: FnMut((String, bool)) + Send + 'static,
    {
        self.on_toggle = Some(Box::new(on_toggle));
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    fn flatten_groups(&self) -> Vec<FlatEntry> {
        let mut entries = Vec::new();
        for group in &self.groups {
            entries.push(FlatEntry::Group(group.label.clone()));
            for subgroup in &group.subgroups {
                entries.push(FlatEntry::Subgroup(subgroup.label.clone()));
                for item in &subgroup.items {
                    entries.push(FlatEntry::Item(item.clone()));
                }
            }
        }
        entries
    }

    fn refresh(&mut self) {
        let query = self.search_input.get_value().trim().to_lowercase();
        let flat_entries = self.flatten_groups();
        if query.is_empty() {
            self.filtered_entries = flat_entries;
        } else {
            let mut next = Vec::new();
            let mut pending_group = None::<String>;
            let mut pending_subgroup = None::<String>;
            for entry in flat_entries {
                match &entry {
                    FlatEntry::Group(label) => {
                        pending_group = Some(label.clone());
                        pending_subgroup = None;
                    }
                    FlatEntry::Subgroup(label) => {
                        pending_subgroup = Some(label.clone());
                    }
                    FlatEntry::Item(item) => {
                        let haystack = format!(
                            "{} {} {} {}",
                            item.display_name,
                            item.path,
                            item.id,
                            resource_type_label(item.resource_type),
                        )
                        .to_lowercase();
                        if haystack.contains(&query) {
                            if let Some(group) = pending_group.take() {
                                next.push(FlatEntry::Group(group));
                            }
                            if let Some(subgroup) = pending_subgroup.take() {
                                next.push(FlatEntry::Subgroup(subgroup));
                            }
                            next.push(entry.clone());
                        }
                    }
                }
            }
            self.filtered_entries = next;
        }

        if let Some(index) = self
            .filtered_entries
            .iter()
            .position(|entry| matches!(entry, FlatEntry::Item(_)))
        {
            self.selected_index = self.selected_index.min(index.max(self.selected_index));
            if !matches!(
                self.filtered_entries.get(self.selected_index),
                Some(FlatEntry::Item(_))
            ) {
                self.selected_index = index;
            }
        } else {
            self.selected_index = 0;
        }
    }

    fn render_entries(&self, width: usize) -> Vec<String> {
        if self.filtered_entries.is_empty() {
            return vec![truncate_to_width(
                "No matching resources",
                width,
                "...",
                false,
            )];
        }

        let max_visible = max_visible(&self.viewport_size, 8, 15);
        let (start_index, end_index) = visible_window(
            self.selected_index,
            self.filtered_entries.len(),
            max_visible,
        );
        let mut lines = Vec::new();

        for (visible_index, entry) in self.filtered_entries[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            match entry {
                FlatEntry::Group(label) => {
                    lines.push(truncate_to_width(label, width, "...", false));
                }
                FlatEntry::Subgroup(label) => {
                    lines.push(truncate_to_width(
                        &format!("  {label}"),
                        width,
                        "...",
                        false,
                    ));
                }
                FlatEntry::Item(item) => {
                    let cursor = if actual_index == self.selected_index {
                        "> "
                    } else {
                        "  "
                    };
                    let checkbox = if item.enabled { "[x]" } else { "[ ]" };
                    let line = format!(
                        "{cursor}    {checkbox} {} ({})",
                        sanitize_display_text(&item.display_name),
                        resource_type_label(item.resource_type)
                    );
                    lines.push(truncate_to_width(&line, width, "...", false));
                }
            }
        }

        lines
    }
}

impl Component for ConfigSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(
            "Resource configuration",
            width,
            "...",
            false,
        ));
        lines.extend(self.search_input.render(width));
        lines.extend(self.render_entries(width));
        lines.push(truncate_to_width(
            "Enter to toggle · Esc to close",
            width,
            "...",
            false,
        ));
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
            self.selected_index = self.selected_index.saturating_sub(1);
            while self.selected_index > 0
                && !matches!(
                    self.filtered_entries.get(self.selected_index),
                    Some(FlatEntry::Item(_))
                )
            {
                self.selected_index = self.selected_index.saturating_sub(1);
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            self.selected_index =
                (self.selected_index + 1).min(self.filtered_entries.len().saturating_sub(1));
            while self.selected_index + 1 < self.filtered_entries.len()
                && !matches!(
                    self.filtered_entries.get(self.selected_index),
                    Some(FlatEntry::Item(_))
                )
            {
                self.selected_index += 1;
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
            self.selected_index =
                self.selected_index
                    .saturating_sub(max_visible(&self.viewport_size, 8, 15));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + max_visible(&self.viewport_size, 8, 15))
                .min(self.filtered_entries.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm") {
            if let Some(FlatEntry::Item(item)) = self.filtered_entries.get_mut(self.selected_index)
            {
                item.enabled = !item.enabled;
                if let Some(on_toggle) = &mut self.on_toggle {
                    on_toggle((item.id.clone(), item.enabled));
                }
                for group in &mut self.groups {
                    for subgroup in &mut group.subgroups {
                        for group_item in &mut subgroup.items {
                            if group_item.id == item.id {
                                group_item.enabled = item.enabled;
                            }
                        }
                    }
                }
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
    }
}

fn resource_type_label(resource_type: ConfigResourceType) -> &'static str {
    match resource_type {
        ConfigResourceType::Extensions => "extensions",
        ConfigResourceType::Skills => "skills",
        ConfigResourceType::Prompts => "prompts",
        ConfigResourceType::Themes => "themes",
    }
}
