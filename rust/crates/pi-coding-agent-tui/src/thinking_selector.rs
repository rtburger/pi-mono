use crate::KeybindingsManager;
use crate::selector_common::{
    CancelCallback, SelectCallback, cycle_index, framed_lines, matches_binding, max_visible,
    render_hint_line, visible_window,
};
use pi_agent::ThinkingLevel;
use pi_tui::{Component, truncate_to_width};
use std::cell::Cell;

pub struct ThinkingSelectorComponent {
    keybindings: KeybindingsManager,
    levels: Vec<ThinkingLevel>,
    selected_index: usize,
    on_select: Option<SelectCallback<ThinkingLevel>>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl ThinkingSelectorComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        current_level: ThinkingLevel,
        available_levels: Vec<ThinkingLevel>,
    ) -> Self {
        let selected_index = available_levels
            .iter()
            .position(|level| *level == current_level)
            .unwrap_or(0);
        Self {
            keybindings: keybindings.clone(),
            levels: available_levels,
            selected_index,
            on_select: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        }
    }

    pub fn set_on_select<F>(&mut self, on_select: F)
    where
        F: FnMut(ThinkingLevel) + Send + 'static,
    {
        self.on_select = Some(Box::new(on_select));
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    fn render_level_lines(&self, width: usize) -> Vec<String> {
        if self.levels.is_empty() {
            return vec![truncate_to_width(
                "No thinking levels available",
                width,
                "...",
                false,
            )];
        }

        let max_visible = max_visible(&self.viewport_size, 5, 10);
        let (start_index, end_index) =
            visible_window(self.selected_index, self.levels.len(), max_visible);
        let mut lines = Vec::new();

        for (visible_index, level) in self.levels[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let line = format!(
                "{prefix}{} — {}",
                level_label(*level),
                level_description(*level)
            );
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        if start_index > 0 || end_index < self.levels.len() {
            lines.push(truncate_to_width(
                &format!("  ({}/{})", self.selected_index + 1, self.levels.len()),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for ThinkingSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let body = self.render_level_lines(width);
        let hint_line = render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "select"),
                ("tui.select.cancel", "cancel"),
                ("tui.select.down", "navigate"),
            ],
        );
        framed_lines(width, "Select thinking level", body, Some(hint_line))
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
            self.selected_index = cycle_index(self.selected_index, self.levels.len(), false);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            self.selected_index = cycle_index(self.selected_index, self.levels.len(), true);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
            self.selected_index =
                self.selected_index
                    .saturating_sub(max_visible(&self.viewport_size, 5, 10));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + max_visible(&self.viewport_size, 5, 10))
                .min(self.levels.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm")
            && let Some(level) = self.levels.get(self.selected_index).copied()
            && let Some(on_select) = &mut self.on_select
        {
            on_select(level);
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}

fn level_label(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
    }
}

fn level_description(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "No reasoning",
        ThinkingLevel::Minimal => "Very brief reasoning (~1k tokens)",
        ThinkingLevel::Low => "Light reasoning (~2k tokens)",
        ThinkingLevel::Medium => "Moderate reasoning (~8k tokens)",
        ThinkingLevel::High => "Deep reasoning (~16k tokens)",
        ThinkingLevel::XHigh => "Maximum reasoning (~32k tokens)",
    }
}
