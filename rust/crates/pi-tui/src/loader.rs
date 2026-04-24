use crate::{Component, KeybindingsManager, RenderHandle, Text, matches_key};
use parking_lot::Mutex;
use std::{
    boxed::Box as StdBox,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::watch;

type TextStyleFn = dyn Fn(&str) -> String + Send + Sync + 'static;
type AbortCallback = dyn FnMut() + Send + 'static;

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

struct LoaderShared {
    frame_index: AtomicUsize,
    message: Mutex<String>,
    running: AtomicBool,
    render_handle: Option<RenderHandle>,
}

impl LoaderShared {
    fn request_render(&self) {
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }
}

pub struct Loader {
    shared: Arc<LoaderShared>,
    spinner_color_fn: StdBox<TextStyleFn>,
    message_color_fn: StdBox<TextStyleFn>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
}

impl Loader {
    pub fn new<FSpinner, FMessage>(
        render_handle: RenderHandle,
        spinner_color_fn: FSpinner,
        message_color_fn: FMessage,
        message: impl Into<String>,
    ) -> Self
    where
        FSpinner: Fn(&str) -> String + Send + Sync + 'static,
        FMessage: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self::with_optional_render_handle(
            Some(render_handle),
            spinner_color_fn,
            message_color_fn,
            message,
        )
    }

    pub fn without_render_handle<FSpinner, FMessage>(
        spinner_color_fn: FSpinner,
        message_color_fn: FMessage,
        message: impl Into<String>,
    ) -> Self
    where
        FSpinner: Fn(&str) -> String + Send + Sync + 'static,
        FMessage: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self::with_optional_render_handle(None, spinner_color_fn, message_color_fn, message)
    }

    pub fn with_optional_render_handle<FSpinner, FMessage>(
        render_handle: Option<RenderHandle>,
        spinner_color_fn: FSpinner,
        message_color_fn: FMessage,
        message: impl Into<String>,
    ) -> Self
    where
        FSpinner: Fn(&str) -> String + Send + Sync + 'static,
        FMessage: Fn(&str) -> String + Send + Sync + 'static,
    {
        let loader = Self {
            shared: Arc::new(LoaderShared {
                frame_index: AtomicUsize::new(0),
                message: Mutex::new(message.into()),
                running: AtomicBool::new(false),
                render_handle,
            }),
            spinner_color_fn: StdBox::new(spinner_color_fn),
            message_color_fn: StdBox::new(message_color_fn),
            thread_handle: Mutex::new(None),
        };
        loader.start();
        loader
    }

    pub fn start(&self) {
        let mut thread_handle = self.thread_handle.lock();
        if thread_handle.is_some() {
            return;
        }

        self.shared.running.store(true, Ordering::SeqCst);
        self.shared.request_render();

        let shared = Arc::clone(&self.shared);
        *thread_handle = Some(thread::spawn(move || {
            while shared.running.load(Ordering::SeqCst) {
                thread::sleep(SPINNER_INTERVAL);
                if !shared.running.load(Ordering::SeqCst) {
                    break;
                }
                let next = (shared.frame_index.load(Ordering::Relaxed) + 1) % SPINNER_FRAMES.len();
                shared.frame_index.store(next, Ordering::Relaxed);
                shared.request_render();
            }
        }));
    }

    pub fn stop(&self) {
        self.shared.running.store(false, Ordering::SeqCst);
        let handle = self.thread_handle.lock().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    pub fn dispose(&self) {
        self.stop();
    }

    pub fn set_message(&self, message: impl Into<String>) {
        *self.shared.message.lock() = message.into();
        self.shared.request_render();
    }

    pub fn message(&self) -> String {
        self.shared.message.lock().clone()
    }

    fn formatted_text(&self) -> String {
        let frame =
            SPINNER_FRAMES[self.shared.frame_index.load(Ordering::Relaxed) % SPINNER_FRAMES.len()];
        let message = self.shared.message.lock().clone();
        format!(
            "{} {}",
            (self.spinner_color_fn)(frame),
            (self.message_color_fn)(&message)
        )
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Component for Loader {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![String::new()];
        let text = Text::new(self.formatted_text(), 1, 0);
        lines.extend(text.render(width));
        lines
    }

    fn invalidate(&mut self) {}
}

pub struct CancellableLoader {
    loader: Loader,
    keybindings: KeybindingsManager,
    abort_tx: watch::Sender<bool>,
    abort_rx: watch::Receiver<bool>,
    on_abort: Option<StdBox<AbortCallback>>,
}

impl CancellableLoader {
    pub fn new<FSpinner, FMessage>(
        render_handle: RenderHandle,
        spinner_color_fn: FSpinner,
        message_color_fn: FMessage,
        message: impl Into<String>,
    ) -> Self
    where
        FSpinner: Fn(&str) -> String + Send + Sync + 'static,
        FMessage: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self::with_keybindings(
            KeybindingsManager::with_tui_defaults(Default::default()),
            Some(render_handle),
            spinner_color_fn,
            message_color_fn,
            message,
        )
    }

    pub fn without_render_handle<FSpinner, FMessage>(
        spinner_color_fn: FSpinner,
        message_color_fn: FMessage,
        message: impl Into<String>,
    ) -> Self
    where
        FSpinner: Fn(&str) -> String + Send + Sync + 'static,
        FMessage: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self::with_keybindings(
            KeybindingsManager::with_tui_defaults(Default::default()),
            None,
            spinner_color_fn,
            message_color_fn,
            message,
        )
    }

    pub fn with_keybindings<FSpinner, FMessage>(
        keybindings: KeybindingsManager,
        render_handle: Option<RenderHandle>,
        spinner_color_fn: FSpinner,
        message_color_fn: FMessage,
        message: impl Into<String>,
    ) -> Self
    where
        FSpinner: Fn(&str) -> String + Send + Sync + 'static,
        FMessage: Fn(&str) -> String + Send + Sync + 'static,
    {
        let (abort_tx, abort_rx) = watch::channel(false);
        Self {
            loader: Loader::with_optional_render_handle(
                render_handle,
                spinner_color_fn,
                message_color_fn,
                message,
            ),
            keybindings,
            abort_tx,
            abort_rx,
            on_abort: None,
        }
    }

    pub fn start(&self) {
        self.loader.start();
    }

    pub fn stop(&self) {
        self.loader.stop();
    }

    pub fn dispose(&self) {
        self.loader.dispose();
    }

    pub fn set_message(&self, message: impl Into<String>) {
        self.loader.set_message(message);
    }

    pub fn signal(&self) -> watch::Receiver<bool> {
        self.abort_rx.clone()
    }

    pub fn aborted(&self) -> bool {
        *self.abort_rx.borrow()
    }

    pub fn set_on_abort<F>(&mut self, on_abort: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_abort = Some(StdBox::new(on_abort));
    }

    pub fn clear_on_abort(&mut self) {
        self.on_abort = None;
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn abort(&mut self) {
        if self.aborted() {
            return;
        }
        let _ = self.abort_tx.send(true);
        if let Some(on_abort) = &mut self.on_abort {
            on_abort();
        }
    }
}

impl Component for CancellableLoader {
    fn render(&self, width: usize) -> Vec<String> {
        self.loader.render(width)
    }

    fn invalidate(&mut self) {
        self.loader.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if self.matches_binding(data, "tui.select.cancel") {
            self.abort();
        }
    }
}
