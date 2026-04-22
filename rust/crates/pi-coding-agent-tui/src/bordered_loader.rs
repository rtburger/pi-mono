use crate::{KeybindingsManager, ThemedKeyHintStyler, current_theme, key_hint};
use pi_tui::{CancellableLoader, Component, DynamicBorder, Loader, RenderHandle, Text};
use std::ops::Deref;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderedLoaderOptions {
    pub cancellable: bool,
}

impl Default for BorderedLoaderOptions {
    fn default() -> Self {
        Self { cancellable: true }
    }
}

enum LoaderKind {
    Cancellable(CancellableLoader),
    Plain(Loader),
}

pub struct BorderedLoader {
    keybindings: KeybindingsManager,
    loader: LoaderKind,
    passive_abort_tx: watch::Sender<bool>,
    passive_abort_rx: watch::Receiver<bool>,
}

impl BorderedLoader {
    pub fn new(
        keybindings: &KeybindingsManager,
        render_handle: Option<RenderHandle>,
        message: impl Into<String>,
    ) -> Self {
        Self::with_options(
            keybindings,
            render_handle,
            message,
            BorderedLoaderOptions::default(),
        )
    }

    pub fn with_options(
        keybindings: &KeybindingsManager,
        render_handle: Option<RenderHandle>,
        message: impl Into<String>,
        options: BorderedLoaderOptions,
    ) -> Self {
        let (passive_abort_tx, passive_abort_rx) = watch::channel(false);
        let loader = if options.cancellable {
            LoaderKind::Cancellable(CancellableLoader::with_keybindings(
                keybindings.deref().clone(),
                render_handle,
                |text| current_theme().fg("accent", text),
                |text| current_theme().fg("muted", text),
                message,
            ))
        } else {
            LoaderKind::Plain(Loader::with_optional_render_handle(
                render_handle,
                |text| current_theme().fg("accent", text),
                |text| current_theme().fg("muted", text),
                message,
            ))
        };

        Self {
            keybindings: keybindings.clone(),
            loader,
            passive_abort_tx,
            passive_abort_rx,
        }
    }

    pub fn signal(&self) -> watch::Receiver<bool> {
        match &self.loader {
            LoaderKind::Cancellable(loader) => loader.signal(),
            LoaderKind::Plain(_) => self.passive_abort_rx.clone(),
        }
    }

    pub fn aborted(&self) -> bool {
        match &self.loader {
            LoaderKind::Cancellable(loader) => loader.aborted(),
            LoaderKind::Plain(_) => *self.passive_abort_rx.borrow(),
        }
    }

    pub fn set_message(&self, message: impl Into<String>) {
        match &self.loader {
            LoaderKind::Cancellable(loader) => loader.set_message(message.into()),
            LoaderKind::Plain(loader) => loader.set_message(message.into()),
        }
    }

    pub fn set_on_abort<F>(&mut self, on_abort: F)
    where
        F: FnMut() + Send + 'static,
    {
        if let LoaderKind::Cancellable(loader) = &mut self.loader {
            loader.set_on_abort(on_abort);
        }
    }

    pub fn clear_on_abort(&mut self) {
        if let LoaderKind::Cancellable(loader) = &mut self.loader {
            loader.clear_on_abort();
        }
    }

    pub fn dispose(&self) {
        let _ = self.passive_abort_tx.send(false);
        match &self.loader {
            LoaderKind::Cancellable(loader) => loader.dispose(),
            LoaderKind::Plain(loader) => loader.dispose(),
        }
    }

    fn render_border(&self, width: usize) -> Vec<String> {
        DynamicBorder::with_color_fn(|text| current_theme().fg("border", text)).render(width)
    }

    fn render_loader(&self, width: usize) -> Vec<String> {
        match &self.loader {
            LoaderKind::Cancellable(loader) => loader.render(width),
            LoaderKind::Plain(loader) => loader.render(width),
        }
    }

    fn render_cancel_hint(&self, width: usize) -> Vec<String> {
        if !matches!(&self.loader, LoaderKind::Cancellable(_)) {
            return Vec::new();
        }

        let hint = key_hint(
            &self.keybindings,
            &ThemedKeyHintStyler,
            "tui.select.cancel",
            "cancel",
        );
        Text::new(hint, 1, 0).render(width)
    }
}

impl Drop for BorderedLoader {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl Component for BorderedLoader {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = self.render_border(width);
        lines.extend(self.render_loader(width));
        lines.extend(self.render_cancel_hint(width));
        lines.push(String::new());
        lines.extend(self.render_border(width));
        lines
    }

    fn invalidate(&mut self) {
        match &mut self.loader {
            LoaderKind::Cancellable(loader) => loader.invalidate(),
            LoaderKind::Plain(loader) => loader.invalidate(),
        }
    }

    fn handle_input(&mut self, data: &str) {
        if let LoaderKind::Cancellable(loader) = &mut self.loader {
            loader.handle_input(data);
        }
    }
}
