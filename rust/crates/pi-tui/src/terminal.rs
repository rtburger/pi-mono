use crate::{
    StdinBuffer, StdinBufferEvent, StdinBufferOptions, TuiError, set_kitty_protocol_active,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size};
use std::{
    env,
    io::{self, Read as _, Write as _},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::Receiver,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BRACKETED_PASTE_ENABLE: &str = "\x1b[?2004h";
const BRACKETED_PASTE_DISABLE: &str = "\x1b[?2004l";
const KITTY_QUERY: &str = "\x1b[?u";
#[cfg_attr(not(test), allow(dead_code))]
const KITTY_ENABLE_FLAGS: &str = "\x1b[>7u";
const KITTY_DISABLE: &str = "\x1b[<u";
#[cfg_attr(not(test), allow(dead_code))]
const MODIFY_OTHER_KEYS_ENABLE: &str = "\x1b[>4;2m";
const MODIFY_OTHER_KEYS_DISABLE: &str = "\x1b[>4;0m";
const DEFAULT_COLUMNS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_STDIN_TIMEOUT: Duration = Duration::from_millis(10);
const MODIFY_OTHER_KEYS_FALLBACK_DELAY: Duration = Duration::from_millis(150);

pub trait Terminal {
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send>,
        on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError>;
    fn stop(&mut self) -> Result<(), TuiError>;
    fn drain_input(&mut self, max: Duration, idle: Duration) -> Result<(), TuiError>;
    fn write(&mut self, data: &str) -> Result<(), TuiError>;
    fn columns(&self) -> u16;
    fn rows(&self) -> u16;
    fn kitty_protocol_active(&self) -> bool;
    fn move_by(&mut self, lines: i32) -> Result<(), TuiError>;
    fn hide_cursor(&mut self) -> Result<(), TuiError>;
    fn show_cursor(&mut self) -> Result<(), TuiError>;
    fn clear_line(&mut self) -> Result<(), TuiError>;
    fn clear_from_cursor(&mut self) -> Result<(), TuiError>;
    fn clear_screen(&mut self) -> Result<(), TuiError>;
    fn set_title(&mut self, title: &str) -> Result<(), TuiError>;
}

impl<T> Terminal for Box<T>
where
    T: Terminal + ?Sized,
{
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send>,
        on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        self.as_mut().start(on_input, on_resize)
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        self.as_mut().stop()
    }

    fn drain_input(&mut self, max: Duration, idle: Duration) -> Result<(), TuiError> {
        self.as_mut().drain_input(max, idle)
    }

    fn write(&mut self, data: &str) -> Result<(), TuiError> {
        self.as_mut().write(data)
    }

    fn columns(&self) -> u16 {
        self.as_ref().columns()
    }

    fn rows(&self) -> u16 {
        self.as_ref().rows()
    }

    fn kitty_protocol_active(&self) -> bool {
        self.as_ref().kitty_protocol_active()
    }

    fn move_by(&mut self, lines: i32) -> Result<(), TuiError> {
        self.as_mut().move_by(lines)
    }

    fn hide_cursor(&mut self) -> Result<(), TuiError> {
        self.as_mut().hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), TuiError> {
        self.as_mut().show_cursor()
    }

    fn clear_line(&mut self) -> Result<(), TuiError> {
        self.as_mut().clear_line()
    }

    fn clear_from_cursor(&mut self) -> Result<(), TuiError> {
        self.as_mut().clear_from_cursor()
    }

    fn clear_screen(&mut self) -> Result<(), TuiError> {
        self.as_mut().clear_screen()
    }

    fn set_title(&mut self, title: &str) -> Result<(), TuiError> {
        self.as_mut().set_title(title)
    }
}

pub struct ProcessTerminal {
    backend: Arc<Mutex<Box<dyn TerminalBackend>>>,
    input_handler: SharedInputHandler,
    resize_handler: SharedResizeHandler,
    kitty_protocol_active: Arc<AtomicBool>,
    modify_other_keys_active: Arc<AtomicBool>,
    suppress_input: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    last_input_at_ms: Arc<AtomicU64>,
    stdin_buffer: StdinBuffer,
    #[cfg_attr(not(test), allow(dead_code))]
    stdin_events: Receiver<StdinBufferEvent>,
    uses_process_io: bool,
    raw_mode_enabled: bool,
}

impl Default for ProcessTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessTerminal {
    pub fn new() -> Self {
        Self::with_backend(Box::new(StdoutBackend))
    }

    fn with_backend(backend: Box<dyn TerminalBackend>) -> Self {
        let uses_process_io = backend.uses_process_io();
        let stdin_buffer = StdinBuffer::new(StdinBufferOptions {
            timeout: DEFAULT_STDIN_TIMEOUT,
        });
        let stdin_events = stdin_buffer.subscribe();

        Self {
            backend: Arc::new(Mutex::new(backend)),
            input_handler: Arc::new(Mutex::new(None)),
            resize_handler: Arc::new(Mutex::new(None)),
            kitty_protocol_active: Arc::new(AtomicBool::new(false)),
            modify_other_keys_active: Arc::new(AtomicBool::new(false)),
            suppress_input: Arc::new(AtomicBool::new(false)),
            stop_requested: Arc::new(AtomicBool::new(false)),
            last_input_at_ms: Arc::new(AtomicU64::new(now_millis())),
            stdin_buffer,
            stdin_events,
            uses_process_io,
            raw_mode_enabled: false,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn process_input_data(&mut self, data: &str) -> Result<(), TuiError> {
        self.stdin_buffer.process_str(data);
        self.forward_stdin_events(&self.stdin_events)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn enable_modify_other_keys_fallback(&mut self) -> Result<(), TuiError> {
        if !self.kitty_protocol_active.load(Ordering::Relaxed)
            && !self.modify_other_keys_active.load(Ordering::Relaxed)
        {
            write_backend(&self.backend, MODIFY_OTHER_KEYS_ENABLE)?;
            self.modify_other_keys_active.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn notify_resize(&mut self) {
        notify_resize_handler(&self.resize_handler);
    }

    fn disable_input_protocols(&mut self) -> Result<(), TuiError> {
        if self.kitty_protocol_active.swap(false, Ordering::Relaxed) {
            write_backend(&self.backend, KITTY_DISABLE)?;
            set_kitty_protocol_active(false);
        }

        if self.modify_other_keys_active.swap(false, Ordering::Relaxed) {
            write_backend(&self.backend, MODIFY_OTHER_KEYS_DISABLE)?;
        }

        Ok(())
    }

    fn forward_stdin_events(&self, receiver: &Receiver<StdinBufferEvent>) -> Result<(), TuiError> {
        while let Ok(event) = receiver.try_recv() {
            match event {
                StdinBufferEvent::Data(sequence) => {
                    if !self.kitty_protocol_active.load(Ordering::Relaxed)
                        && is_kitty_protocol_response(&sequence)
                    {
                        self.kitty_protocol_active.store(true, Ordering::Relaxed);
                        set_kitty_protocol_active(true);
                        write_backend(&self.backend, KITTY_ENABLE_FLAGS)?;
                        continue;
                    }

                    if !self.suppress_input.load(Ordering::Relaxed) {
                        forward_input(&self.input_handler, sequence);
                    }
                }
                StdinBufferEvent::Paste(content) => {
                    if !self.suppress_input.load(Ordering::Relaxed) {
                        forward_input(&self.input_handler, format!("\x1b[200~{content}\x1b[201~"));
                    }
                }
            }
        }

        Ok(())
    }

    fn start_process_input_loop(&self) {
        if !self.uses_process_io {
            return;
        }

        self.stop_requested.store(false, Ordering::Relaxed);
        self.last_input_at_ms.store(now_millis(), Ordering::Relaxed);

        let backend = Arc::clone(&self.backend);
        let input_handler = Arc::clone(&self.input_handler);
        let kitty_protocol_active = Arc::clone(&self.kitty_protocol_active);
        let suppress_input = Arc::clone(&self.suppress_input);
        let stop_requested = Arc::clone(&self.stop_requested);
        let last_input_at_ms = Arc::clone(&self.last_input_at_ms);

        thread::spawn(move || {
            let stdin_buffer = StdinBuffer::new(StdinBufferOptions {
                timeout: DEFAULT_STDIN_TIMEOUT,
            });
            let stdin_events = stdin_buffer.subscribe();
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let mut buffer = [0u8; 4096];

            loop {
                if stop_requested.load(Ordering::Relaxed) {
                    break;
                }

                match stdin.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        last_input_at_ms.store(now_millis(), Ordering::Relaxed);
                        stdin_buffer.process_bytes(&buffer[..read]);
                        while let Ok(event) = stdin_events.try_recv() {
                            match event {
                                StdinBufferEvent::Data(sequence) => {
                                    if !kitty_protocol_active.load(Ordering::Relaxed)
                                        && is_kitty_protocol_response(&sequence)
                                    {
                                        kitty_protocol_active.store(true, Ordering::Relaxed);
                                        set_kitty_protocol_active(true);
                                        let _ = write_backend(&backend, KITTY_ENABLE_FLAGS);
                                        continue;
                                    }

                                    if !suppress_input.load(Ordering::Relaxed) {
                                        forward_input(&input_handler, sequence);
                                    }
                                }
                                StdinBufferEvent::Paste(content) => {
                                    if !suppress_input.load(Ordering::Relaxed) {
                                        forward_input(
                                            &input_handler,
                                            format!("\x1b[200~{content}\x1b[201~"),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
    }

    fn start_modify_other_keys_fallback_timer(&self) {
        if !self.uses_process_io {
            return;
        }

        let backend = Arc::clone(&self.backend);
        let kitty_protocol_active = Arc::clone(&self.kitty_protocol_active);
        let modify_other_keys_active = Arc::clone(&self.modify_other_keys_active);
        let stop_requested = Arc::clone(&self.stop_requested);

        thread::spawn(move || {
            thread::sleep(MODIFY_OTHER_KEYS_FALLBACK_DELAY);
            if stop_requested.load(Ordering::Relaxed)
                || kitty_protocol_active.load(Ordering::Relaxed)
                || modify_other_keys_active.load(Ordering::Relaxed)
            {
                return;
            }

            if write_backend(&backend, MODIFY_OTHER_KEYS_ENABLE).is_ok() {
                modify_other_keys_active.store(true, Ordering::Relaxed);
            }
        });
    }
}

impl Terminal for ProcessTerminal {
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send>,
        on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        *self
            .input_handler
            .lock()
            .expect("input handler mutex poisoned") = Some(on_input);
        *self
            .resize_handler
            .lock()
            .expect("resize handler mutex poisoned") = Some(on_resize);
        self.suppress_input.store(false, Ordering::Relaxed);
        self.stop_requested.store(false, Ordering::Relaxed);

        if self.uses_process_io {
            enable_raw_mode()?;
            self.raw_mode_enabled = true;
        }

        write_backend(&self.backend, BRACKETED_PASTE_ENABLE)?;
        write_backend(&self.backend, KITTY_QUERY)?;

        self.start_process_input_loop();
        self.start_modify_other_keys_fallback_timer();

        Ok(())
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        self.stop_requested.store(true, Ordering::Relaxed);
        self.suppress_input.store(true, Ordering::Relaxed);
        write_backend(&self.backend, BRACKETED_PASTE_DISABLE)?;
        self.disable_input_protocols()?;
        self.stdin_buffer.destroy();
        *self
            .input_handler
            .lock()
            .expect("input handler mutex poisoned") = None;
        *self
            .resize_handler
            .lock()
            .expect("resize handler mutex poisoned") = None;

        if self.raw_mode_enabled {
            disable_raw_mode()?;
            self.raw_mode_enabled = false;
        }

        Ok(())
    }

    fn drain_input(&mut self, max: Duration, idle: Duration) -> Result<(), TuiError> {
        self.disable_input_protocols()?;
        self.suppress_input.store(true, Ordering::Relaxed);

        let start = std::time::Instant::now();
        let idle_ms = duration_to_millis(idle);
        while start.elapsed() < max {
            let last_input = self.last_input_at_ms.load(Ordering::Relaxed);
            let now = now_millis();
            if now.saturating_sub(last_input) >= idle_ms {
                break;
            }
            thread::sleep(idle.min(Duration::from_millis(10)));
        }

        self.suppress_input.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn write(&mut self, data: &str) -> Result<(), TuiError> {
        write_backend(&self.backend, data)?;
        Ok(())
    }

    fn columns(&self) -> u16 {
        self.backend
            .lock()
            .expect("terminal backend mutex poisoned")
            .columns()
    }

    fn rows(&self) -> u16 {
        self.backend
            .lock()
            .expect("terminal backend mutex poisoned")
            .rows()
    }

    fn kitty_protocol_active(&self) -> bool {
        self.kitty_protocol_active.load(Ordering::Relaxed)
    }

    fn move_by(&mut self, lines: i32) -> Result<(), TuiError> {
        if lines > 0 {
            write_backend(&self.backend, &format!("\x1b[{lines}B"))?;
        } else if lines < 0 {
            write_backend(&self.backend, &format!("\x1b[{}A", -lines))?;
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), TuiError> {
        write_backend(&self.backend, "\x1b[?25l")?;
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), TuiError> {
        write_backend(&self.backend, "\x1b[?25h")?;
        Ok(())
    }

    fn clear_line(&mut self) -> Result<(), TuiError> {
        write_backend(&self.backend, "\x1b[K")?;
        Ok(())
    }

    fn clear_from_cursor(&mut self) -> Result<(), TuiError> {
        write_backend(&self.backend, "\x1b[J")?;
        Ok(())
    }

    fn clear_screen(&mut self) -> Result<(), TuiError> {
        write_backend(&self.backend, "\x1b[2J\x1b[H")?;
        Ok(())
    }

    fn set_title(&mut self, title: &str) -> Result<(), TuiError> {
        write_backend(&self.backend, &format!("\x1b]0;{title}\x07"))?;
        Ok(())
    }
}

type SharedInputHandler = Arc<Mutex<Option<Box<dyn FnMut(String) + Send>>>>;
type SharedResizeHandler = Arc<Mutex<Option<Box<dyn FnMut() + Send>>>>;

trait TerminalBackend: Send {
    fn write(&mut self, data: &str) -> io::Result<()>;
    fn columns(&self) -> u16;
    fn rows(&self) -> u16;
    fn uses_process_io(&self) -> bool {
        false
    }
}

struct StdoutBackend;

impl TerminalBackend for StdoutBackend {
    fn write(&mut self, data: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(data.as_bytes())?;
        stdout.flush()
    }

    fn columns(&self) -> u16 {
        terminal_size()
            .ok()
            .map(|(columns, _)| columns)
            .or_else(|| {
                env::var("COLUMNS")
                    .ok()
                    .and_then(|value| value.parse::<u16>().ok())
            })
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_COLUMNS)
    }

    fn rows(&self) -> u16 {
        terminal_size()
            .ok()
            .map(|(_, rows)| rows)
            .or_else(|| {
                env::var("LINES")
                    .ok()
                    .and_then(|value| value.parse::<u16>().ok())
            })
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_ROWS)
    }

    fn uses_process_io(&self) -> bool {
        true
    }
}

fn write_backend(backend: &Arc<Mutex<Box<dyn TerminalBackend>>>, data: &str) -> io::Result<()> {
    backend
        .lock()
        .expect("terminal backend mutex poisoned")
        .write(data)
}

fn forward_input(handler: &SharedInputHandler, data: String) {
    if let Some(handler) = &mut *handler.lock().expect("input handler mutex poisoned") {
        handler(data);
    }
}

fn notify_resize_handler(handler: &SharedResizeHandler) {
    if let Some(handler) = &mut *handler.lock().expect("resize handler mutex poisoned") {
        handler();
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn duration_to_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg_attr(not(test), allow(dead_code))]
fn is_kitty_protocol_response(sequence: &str) -> bool {
    if !sequence.starts_with("\x1b[?") || !sequence.ends_with('u') {
        return false;
    }

    let flags = &sequence[3..sequence.len() - 1];
    !flags.is_empty() && flags.as_bytes().iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockBackend {
        writes: Arc<Mutex<Vec<String>>>,
        columns: u16,
        rows: u16,
    }

    impl MockBackend {
        fn new(columns: u16, rows: u16) -> Self {
            Self {
                writes: Arc::new(Mutex::new(Vec::new())),
                columns,
                rows,
            }
        }

        fn writes(&self) -> Vec<String> {
            self.writes.lock().expect("writes mutex poisoned").clone()
        }
    }

    impl TerminalBackend for MockBackend {
        fn write(&mut self, data: &str) -> io::Result<()> {
            self.writes
                .lock()
                .expect("writes mutex poisoned")
                .push(data.to_string());
            Ok(())
        }

        fn columns(&self) -> u16 {
            self.columns
        }

        fn rows(&self) -> u16 {
            self.rows
        }
    }

    fn new_test_terminal() -> (
        ProcessTerminal,
        MockBackend,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<u32>>,
    ) {
        let backend = MockBackend::new(120, 40);
        let input_events = Arc::new(Mutex::new(Vec::new()));
        let resize_count = Arc::new(Mutex::new(0u32));
        (
            ProcessTerminal::with_backend(Box::new(backend.clone())),
            backend,
            input_events,
            resize_count,
        )
    }

    #[test]
    fn start_writes_bracketed_paste_and_kitty_query() {
        let (mut terminal, backend, input_events, resize_count) = new_test_terminal();
        terminal
            .start(
                Box::new(move |data| {
                    input_events
                        .lock()
                        .expect("input mutex poisoned")
                        .push(data)
                }),
                Box::new(move || *resize_count.lock().expect("resize mutex poisoned") += 1),
            )
            .expect("start should succeed");

        assert_eq!(backend.writes(), vec![BRACKETED_PASTE_ENABLE, KITTY_QUERY]);
        assert_eq!(terminal.columns(), 120);
        assert_eq!(terminal.rows(), 40);
        assert!(!terminal.kitty_protocol_active());
    }

    #[test]
    fn kitty_response_enables_protocol_without_forwarding_input() {
        let (mut terminal, backend, input_events, resize_count) = new_test_terminal();
        terminal
            .start(
                Box::new(move |data| {
                    input_events
                        .lock()
                        .expect("input mutex poisoned")
                        .push(data)
                }),
                Box::new(move || *resize_count.lock().expect("resize mutex poisoned") += 1),
            )
            .expect("start should succeed");

        terminal
            .process_input_data("\x1b[?1u")
            .expect("processing input should succeed");

        assert!(terminal.kitty_protocol_active());
        assert_eq!(
            backend.writes(),
            vec![BRACKETED_PASTE_ENABLE, KITTY_QUERY, KITTY_ENABLE_FLAGS]
        );
    }

    #[test]
    fn normal_input_and_paste_are_forwarded_through_handlers() {
        let (mut terminal, _backend, input_events, resize_count) = new_test_terminal();
        let input_events_for_handler = Arc::clone(&input_events);
        terminal
            .start(
                Box::new(move |data| {
                    input_events_for_handler
                        .lock()
                        .expect("input mutex poisoned")
                        .push(data)
                }),
                Box::new(move || *resize_count.lock().expect("resize mutex poisoned") += 1),
            )
            .expect("start should succeed");

        terminal
            .process_input_data("a\x1b[A")
            .expect("processing input should succeed");
        terminal
            .process_input_data("\x1b[200~hello world\x1b[201~")
            .expect("processing paste should succeed");

        assert_eq!(
            input_events.lock().expect("input mutex poisoned").clone(),
            vec![
                "a".to_string(),
                "\x1b[A".to_string(),
                "\x1b[200~hello world\x1b[201~".to_string()
            ]
        );
    }

    #[test]
    fn modify_other_keys_fallback_and_drain_disable_protocols() {
        let (mut terminal, backend, input_events, resize_count) = new_test_terminal();
        terminal
            .start(
                Box::new(move |data| {
                    input_events
                        .lock()
                        .expect("input mutex poisoned")
                        .push(data)
                }),
                Box::new(move || *resize_count.lock().expect("resize mutex poisoned") += 1),
            )
            .expect("start should succeed");
        terminal
            .enable_modify_other_keys_fallback()
            .expect("fallback enable should succeed");
        terminal
            .process_input_data("\x1b[?1u")
            .expect("processing input should succeed");
        terminal
            .drain_input(Duration::from_secs(1), Duration::from_millis(50))
            .expect("drain should succeed");

        assert!(!terminal.kitty_protocol_active());
        assert_eq!(
            backend.writes(),
            vec![
                BRACKETED_PASTE_ENABLE,
                KITTY_QUERY,
                MODIFY_OTHER_KEYS_ENABLE,
                KITTY_ENABLE_FLAGS,
                KITTY_DISABLE,
                MODIFY_OTHER_KEYS_DISABLE,
            ]
        );
    }

    #[test]
    fn stop_disables_protocols_and_bracketed_paste() {
        let (mut terminal, backend, input_events, resize_count) = new_test_terminal();
        terminal
            .start(
                Box::new(move |data| {
                    input_events
                        .lock()
                        .expect("input mutex poisoned")
                        .push(data)
                }),
                Box::new(move || *resize_count.lock().expect("resize mutex poisoned") += 1),
            )
            .expect("start should succeed");
        terminal
            .process_input_data("\x1b[?1u")
            .expect("processing input should succeed");
        terminal.stop().expect("stop should succeed");

        assert_eq!(
            backend.writes(),
            vec![
                BRACKETED_PASTE_ENABLE,
                KITTY_QUERY,
                KITTY_ENABLE_FLAGS,
                BRACKETED_PASTE_DISABLE,
                KITTY_DISABLE,
            ]
        );
    }

    #[test]
    fn ansi_helpers_and_resize_notification_emit_expected_sequences() {
        let (mut terminal, backend, input_events, resize_count) = new_test_terminal();
        let input_events_for_handler = Arc::clone(&input_events);
        let resize_count_for_handler = Arc::clone(&resize_count);
        terminal
            .start(
                Box::new(move |data| {
                    input_events_for_handler
                        .lock()
                        .expect("input mutex poisoned")
                        .push(data)
                }),
                Box::new(move || {
                    *resize_count_for_handler
                        .lock()
                        .expect("resize mutex poisoned") += 1
                }),
            )
            .expect("start should succeed");

        terminal.move_by(3).expect("move down should succeed");
        terminal.move_by(-2).expect("move up should succeed");
        terminal.move_by(0).expect("zero move should succeed");
        terminal.hide_cursor().expect("hide cursor should succeed");
        terminal.show_cursor().expect("show cursor should succeed");
        terminal.clear_line().expect("clear line should succeed");
        terminal
            .clear_from_cursor()
            .expect("clear from cursor should succeed");
        terminal
            .clear_screen()
            .expect("clear screen should succeed");
        terminal.set_title("Pi").expect("set title should succeed");
        terminal.notify_resize();

        assert_eq!(
            backend.writes(),
            vec![
                BRACKETED_PASTE_ENABLE,
                KITTY_QUERY,
                "\x1b[3B",
                "\x1b[2A",
                "\x1b[?25l",
                "\x1b[?25h",
                "\x1b[K",
                "\x1b[J",
                "\x1b[2J\x1b[H",
                "\x1b]0;Pi\x07",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
        assert_eq!(*resize_count.lock().expect("resize mutex poisoned"), 1);
    }

    #[test]
    fn kitty_response_detection_matches_typescript_shape() {
        assert!(is_kitty_protocol_response("\x1b[?1u"));
        assert!(is_kitty_protocol_response("\x1b[?42u"));
        assert!(!is_kitty_protocol_response("\x1b[97u"));
        assert!(!is_kitty_protocol_response("\x1b[?u"));
        assert!(!is_kitty_protocol_response("\x1b[?1A"));
    }
}
