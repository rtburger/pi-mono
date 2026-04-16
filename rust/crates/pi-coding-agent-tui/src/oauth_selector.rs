use crate::selector_common::{
    CancelCallback, SelectCallback, cycle_index, framed_lines, matches_binding, max_visible,
    render_hint_line, visible_window,
};
use crate::{KeybindingsManager, current_theme};
use pi_tui::{Component, truncate_to_width};
use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthSelectorMode {
    Login,
    Logout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderItem {
    pub id: String,
    pub name: String,
    pub logged_in: bool,
}

pub struct OAuthSelectorComponent {
    keybindings: KeybindingsManager,
    mode: OAuthSelectorMode,
    providers: Vec<OAuthProviderItem>,
    selected_index: usize,
    on_select: Option<SelectCallback<String>>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl OAuthSelectorComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        mode: OAuthSelectorMode,
        providers: Vec<OAuthProviderItem>,
    ) -> Self {
        Self {
            keybindings: keybindings.clone(),
            mode,
            providers,
            selected_index: 0,
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

    fn title(&self) -> &'static str {
        match self.mode {
            OAuthSelectorMode::Login => "Select provider to login",
            OAuthSelectorMode::Logout => "Select provider to logout",
        }
    }

    fn empty_message(&self) -> &'static str {
        match self.mode {
            OAuthSelectorMode::Login => "No OAuth providers available",
            OAuthSelectorMode::Logout => "No OAuth providers logged in",
        }
    }

    fn render_provider_lines(&self, width: usize) -> Vec<String> {
        if self.providers.is_empty() {
            return vec![truncate_to_width(self.empty_message(), width, "...", false)];
        }

        let max_visible = max_visible(&self.viewport_size, 5, 10);
        let (start_index, end_index) =
            visible_window(self.selected_index, self.providers.len(), max_visible);
        let mut lines = Vec::new();
        let theme = current_theme();

        for (visible_index, provider) in self.providers[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                theme.fg("accent", "→ ")
            } else {
                String::from("  ")
            };
            let status = if provider.logged_in {
                format!(" {}", theme.fg("success", "✓ logged in"))
            } else {
                String::new()
            };
            let line = format!("{prefix}{}{}", provider.name, status);
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        lines
    }
}

impl Component for OAuthSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let body = self.render_provider_lines(width);
        let hint_line = render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "select"),
                ("tui.select.cancel", "cancel"),
                ("tui.select.down", "navigate"),
            ],
        );
        framed_lines(width, self.title(), body, Some(hint_line))
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
            self.selected_index = cycle_index(self.selected_index, self.providers.len(), false);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            self.selected_index = cycle_index(self.selected_index, self.providers.len(), true);
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
                .min(self.providers.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm")
            && let Some(provider) = self.providers.get(self.selected_index)
            && let Some(on_select) = &mut self.on_select
        {
            on_select(provider.id.clone());
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}
