use pi_tui::RenderHandle;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

type ExpireCallback = Arc<dyn Fn() + Send + Sync + 'static>;

struct DialogCountdownShared {
    remaining_seconds: AtomicU64,
    running: AtomicBool,
    completed: Arc<AtomicBool>,
    render_handle: Option<RenderHandle>,
    on_expire: ExpireCallback,
}

impl DialogCountdownShared {
    fn request_render(&self) {
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }
}

pub(crate) struct DialogCountdown {
    shared: Arc<DialogCountdownShared>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
}

impl DialogCountdown {
    pub(crate) fn new(
        timeout_ms: u64,
        render_handle: Option<RenderHandle>,
        completed: Arc<AtomicBool>,
        on_expire: ExpireCallback,
    ) -> Self {
        let initial_seconds = timeout_ms.div_ceil(1000).max(1);
        let shared = Arc::new(DialogCountdownShared {
            remaining_seconds: AtomicU64::new(initial_seconds),
            running: AtomicBool::new(true),
            completed,
            render_handle,
            on_expire,
        });

        let thread_shared = Arc::clone(&shared);
        let timeout = Duration::from_millis(timeout_ms.max(1));
        let thread_handle = thread::spawn(move || {
            let start = Instant::now();
            let mut last_reported_seconds = initial_seconds;
            thread_shared.request_render();

            loop {
                if !thread_shared.running.load(Ordering::SeqCst)
                    || thread_shared.completed.load(Ordering::SeqCst)
                {
                    break;
                }

                let elapsed = start.elapsed();
                if elapsed >= timeout {
                    thread_shared.remaining_seconds.store(0, Ordering::Relaxed);
                    thread_shared.request_render();
                    if !thread_shared.completed.swap(true, Ordering::SeqCst) {
                        (thread_shared.on_expire)();
                    }
                    break;
                }

                let remaining_ms = (timeout - elapsed).as_millis() as u64;
                let remaining_seconds = remaining_ms.div_ceil(1000).max(1);
                if remaining_seconds != last_reported_seconds {
                    thread_shared
                        .remaining_seconds
                        .store(remaining_seconds, Ordering::Relaxed);
                    thread_shared.request_render();
                    last_reported_seconds = remaining_seconds;
                }

                let sleep_for = (timeout - elapsed).min(Duration::from_millis(50));
                thread::sleep(sleep_for);
            }
        });

        Self {
            shared,
            thread_handle: Mutex::new(Some(thread_handle)),
        }
    }

    pub(crate) fn remaining_seconds(&self) -> Option<u64> {
        let remaining = self.shared.remaining_seconds.load(Ordering::Relaxed);
        (remaining > 0 && !self.shared.completed.load(Ordering::Relaxed)).then_some(remaining)
    }

    pub(crate) fn stop(&self) {
        self.shared.running.store(false, Ordering::SeqCst);
        let handle = self
            .thread_handle
            .lock()
            .expect("dialog countdown thread mutex poisoned")
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

impl Drop for DialogCountdown {
    fn drop(&mut self) {
        self.stop();
    }
}
