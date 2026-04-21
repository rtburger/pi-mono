use crate::KeybindingsManager;
use crate::dialog_countdown::DialogCountdown;
use crate::selector_common::{
    cycle_index, framed_lines, matches_binding, max_visible, render_hint_line,
    sanitize_display_text, visible_window,
};
use pi_tui::{Component, RenderHandle, truncate_to_width};
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type SelectCallback = Arc<dyn Fn(String) + Send + Sync + 'static>;
type CancelCallback = Arc<dyn Fn() + Send + Sync + 'static>;

pub struct ExtensionSelectorComponent {
    keybindings: KeybindingsManager,
    title: String,
    options: Vec<String>,
    selected_index: usize,
    on_select: SelectCallback,
    on_cancel: CancelCallback,
    completed: Arc<AtomicBool>,
    countdown: Option<DialogCountdown>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl ExtensionSelectorComponent {
    #[allow(clippy::too_many_arguments)]
    pub fn new<FSelect, FCancel>(
        keybindings: &KeybindingsManager,
        title: impl Into<String>,
        options: Vec<String>,
        on_select: FSelect,
        on_cancel: FCancel,
        timeout_ms: Option<u64>,
        render_handle: Option<RenderHandle>,
    ) -> Self
    where
        FSelect: Fn(String) + Send + Sync + 'static,
        FCancel: Fn() + Send + Sync + 'static,
    {
        let completed = Arc::new(AtomicBool::new(false));
        let on_select = Arc::new(on_select) as SelectCallback;
        let on_cancel = Arc::new(on_cancel) as CancelCallback;
        let countdown = timeout_ms
            .filter(|timeout_ms| *timeout_ms > 0)
            .map(|timeout_ms| {
                DialogCountdown::new(
                    timeout_ms,
                    render_handle,
                    Arc::clone(&completed),
                    Arc::clone(&on_cancel),
                )
            });

        Self {
            keybindings: keybindings.clone(),
            title: title.into(),
            options,
            selected_index: 0,
            on_select,
            on_cancel,
            completed,
            countdown,
            viewport_size: Cell::new(None),
        }
    }

    fn title_text(&self) -> String {
        match self
            .countdown
            .as_ref()
            .and_then(DialogCountdown::remaining_seconds)
        {
            Some(remaining_seconds) => format!("{} ({remaining_seconds}s)", self.title),
            None => self.title.clone(),
        }
    }

    fn submit(&self) {
        if self.completed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(option) = self.options.get(self.selected_index) {
            (self.on_select)(option.clone());
        }
    }

    fn cancel(&self) {
        if self.completed.swap(true, Ordering::SeqCst) {
            return;
        }
        (self.on_cancel)();
    }

    fn render_option_lines(&self, width: usize) -> Vec<String> {
        if self.options.is_empty() {
            return vec![truncate_to_width(
                "No options available",
                width,
                "...",
                false,
            )];
        }

        let max_visible = max_visible(&self.viewport_size, 5, 10);
        let (start_index, end_index) =
            visible_window(self.selected_index, self.options.len(), max_visible);
        let mut lines = Vec::new();

        for (visible_index, option) in self.options[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            lines.push(truncate_to_width(
                &format!("{prefix}{}", sanitize_display_text(option)),
                width,
                "...",
                false,
            ));
        }

        if start_index > 0 || end_index < self.options.len() {
            lines.push(truncate_to_width(
                &format!("  ({}/{})", self.selected_index + 1, self.options.len()),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for ExtensionSelectorComponent {
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
        framed_lines(width, &self.title_text(), body, Some(hint_line))
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        if matches_binding(&self.keybindings, data, "tui.select.cancel") {
            self.cancel();
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

        if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
            self.selected_index =
                self.selected_index
                    .saturating_sub(max_visible(&self.viewport_size, 5, 10));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + max_visible(&self.viewport_size, 5, 10))
                .min(self.options.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm") || data == "\n" {
            self.submit();
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}
