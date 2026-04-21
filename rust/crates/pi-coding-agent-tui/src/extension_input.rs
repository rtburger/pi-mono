use crate::KeybindingsManager;
use crate::dialog_countdown::DialogCountdown;
use crate::selector_common::{framed_lines, matches_binding, render_hint_line};
use pi_tui::{Component, Input, RenderHandle};
use std::{
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type SubmitCallback = Arc<dyn Fn(String) + Send + Sync + 'static>;
type CancelCallback = Arc<dyn Fn() + Send + Sync + 'static>;

pub struct ExtensionInputComponent {
    keybindings: KeybindingsManager,
    title: String,
    input: Input,
    on_submit: SubmitCallback,
    on_cancel: CancelCallback,
    completed: Arc<AtomicBool>,
    countdown: Option<DialogCountdown>,
}

impl ExtensionInputComponent {
    #[allow(clippy::too_many_arguments)]
    pub fn new<FSubmit, FCancel>(
        keybindings: &KeybindingsManager,
        title: impl Into<String>,
        _placeholder: Option<&str>,
        on_submit: FSubmit,
        on_cancel: FCancel,
        timeout_ms: Option<u64>,
        render_handle: Option<RenderHandle>,
    ) -> Self
    where
        FSubmit: Fn(String) + Send + Sync + 'static,
        FCancel: Fn() + Send + Sync + 'static,
    {
        let completed = Arc::new(AtomicBool::new(false));
        let on_submit = Arc::new(on_submit) as SubmitCallback;
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
            input: Input::with_keybindings(keybindings.deref().clone()),
            on_submit,
            on_cancel,
            completed,
            countdown,
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
        (self.on_submit)(self.input.get_value().to_owned());
    }

    fn cancel(&self) {
        if self.completed.swap(true, Ordering::SeqCst) {
            return;
        }
        (self.on_cancel)();
    }
}

impl Component for ExtensionInputComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let mut body = self.input.render(width);
        body.push(String::new());
        let hint_line = render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.input.submit", "submit"),
                ("tui.select.cancel", "cancel"),
            ],
        );
        framed_lines(width, &self.title_text(), body, Some(hint_line))
    }

    fn invalidate(&mut self) {
        self.input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if matches_binding(&self.keybindings, data, "tui.select.cancel") {
            self.cancel();
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.input.submit") || data == "\n" {
            self.submit();
            return;
        }

        self.input.handle_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.input.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.input.set_viewport_size(width, height.min(1));
    }
}
