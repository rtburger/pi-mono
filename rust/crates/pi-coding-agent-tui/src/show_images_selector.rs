use crate::KeybindingsManager;
use crate::selector_common::{
    CancelCallback, SelectCallback, cycle_index, framed_lines, matches_binding, max_visible,
    render_hint_line, visible_window,
};
use pi_tui::{Component, truncate_to_width};
use std::cell::Cell;

pub struct ShowImagesSelectorComponent {
    keybindings: KeybindingsManager,
    options: [bool; 2],
    selected_index: usize,
    on_select: Option<SelectCallback<bool>>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl ShowImagesSelectorComponent {
    pub fn new(keybindings: &KeybindingsManager, current_value: bool) -> Self {
        Self {
            keybindings: keybindings.clone(),
            options: [true, false],
            selected_index: usize::from(!current_value),
            on_select: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        }
    }

    pub fn set_on_select<F>(&mut self, on_select: F)
    where
        F: FnMut(bool) + Send + 'static,
    {
        self.on_select = Some(Box::new(on_select));
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    fn render_option_lines(&self, width: usize) -> Vec<String> {
        let max_visible = max_visible(&self.viewport_size, 5, 5);
        let (start_index, end_index) =
            visible_window(self.selected_index, self.options.len(), max_visible);
        let mut lines = Vec::new();

        for (visible_index, value) in self.options[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let (label, description) = if *value {
                ("Yes", "Render images inline in the terminal")
            } else {
                ("No", "Use text placeholders instead")
            };
            let line = format!("{prefix}{label} — {description}");
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        lines
    }
}

impl Component for ShowImagesSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let body = self.render_option_lines(width);
        let hint_line = render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "select"),
                ("tui.select.cancel", "cancel"),
                ("tui.select.down", "navigate"),
            ],
        );
        framed_lines(width, "Show images", body, Some(hint_line))
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
            self.selected_index = cycle_index(self.selected_index, self.options.len(), false);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            self.selected_index = cycle_index(self.selected_index, self.options.len(), true);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm")
            && let Some(value) = self.options.get(self.selected_index).copied()
            && let Some(on_select) = &mut self.on_select
        {
            on_select(value);
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}
