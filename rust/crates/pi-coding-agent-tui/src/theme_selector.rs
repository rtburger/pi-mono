use crate::selector_common::{
    CancelCallback, PreviewCallback, SelectCallback, cycle_index, framed_lines, matches_binding,
    max_visible, render_hint_line, sanitize_display_text, visible_window,
};
use crate::{KeybindingsManager, ThemeInfo, get_available_themes_with_paths};
use pi_tui::{Component, truncate_to_width};
use std::cell::Cell;

pub struct ThemeSelectorComponent {
    keybindings: KeybindingsManager,
    current_theme: String,
    themes: Vec<ThemeInfo>,
    selected_index: usize,
    on_select: Option<SelectCallback<String>>,
    on_cancel: Option<CancelCallback>,
    on_preview: Option<PreviewCallback<String>>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl ThemeSelectorComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        current_theme: impl Into<String>,
        themes: Vec<ThemeInfo>,
    ) -> Self {
        let current_theme = current_theme.into();
        let selected_index = themes
            .iter()
            .position(|theme| theme.name == current_theme)
            .unwrap_or(0);
        Self {
            keybindings: keybindings.clone(),
            current_theme,
            themes,
            selected_index,
            on_select: None,
            on_cancel: None,
            on_preview: None,
            viewport_size: Cell::new(None),
        }
    }

    pub fn from_registered(
        keybindings: &KeybindingsManager,
        current_theme: impl Into<String>,
    ) -> Self {
        Self::new(
            keybindings,
            current_theme,
            get_available_themes_with_paths(),
        )
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

    pub fn set_on_preview<F>(&mut self, on_preview: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_preview = Some(Box::new(on_preview));
    }

    fn preview_selected_theme(&mut self) {
        if let Some(theme) = self.themes.get(self.selected_index)
            && let Some(on_preview) = &mut self.on_preview
        {
            on_preview(theme.name.clone());
        }
    }

    fn render_theme_lines(&self, width: usize) -> Vec<String> {
        if self.themes.is_empty() {
            return vec![truncate_to_width(
                "No themes available",
                width,
                "...",
                false,
            )];
        }

        let max_visible = max_visible(&self.viewport_size, 5, 10);
        let (start_index, end_index) =
            visible_window(self.selected_index, self.themes.len(), max_visible);
        let mut lines = Vec::new();

        for (visible_index, theme) in self.themes[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let current_suffix = if theme.name == self.current_theme {
                " (current)"
            } else {
                ""
            };
            let source_suffix = theme
                .path
                .as_deref()
                .map(|path| format!(" — {}", sanitize_display_text(path)))
                .unwrap_or_default();
            let line = format!("{prefix}{}{current_suffix}{source_suffix}", theme.name);
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        if start_index > 0 || end_index < self.themes.len() {
            lines.push(truncate_to_width(
                &format!("  ({}/{})", self.selected_index + 1, self.themes.len()),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for ThemeSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let body = self.render_theme_lines(width);
        let hint_line = render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "select"),
                ("tui.select.cancel", "cancel"),
                ("tui.select.down", "navigate"),
            ],
        );
        framed_lines(width, "Select theme", body, Some(hint_line))
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
            self.selected_index = cycle_index(self.selected_index, self.themes.len(), false);
            self.preview_selected_theme();
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            self.selected_index = cycle_index(self.selected_index, self.themes.len(), true);
            self.preview_selected_theme();
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
            self.selected_index =
                self.selected_index
                    .saturating_sub(max_visible(&self.viewport_size, 5, 10));
            self.preview_selected_theme();
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + max_visible(&self.viewport_size, 5, 10))
                .min(self.themes.len().saturating_sub(1));
            self.preview_selected_theme();
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm")
            && let Some(theme) = self.themes.get(self.selected_index)
            && let Some(on_select) = &mut self.on_select
        {
            on_select(theme.name.clone());
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}
