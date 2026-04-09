use crate::{
    StdinBuffer, StdinBufferEvent, StdinBufferOptions, TuiError, set_kitty_protocol_active,
};
use std::{
    env,
    io::{self, Write as _},
    sync::mpsc::Receiver,
    time::Duration,
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

pub struct ProcessTerminal {
    backend: Box<dyn TerminalBackend>,
    input_handler: Option<Box<dyn FnMut(String) + Send>>,
    resize_handler: Option<Box<dyn FnMut() + Send>>,
    kitty_protocol_active: bool,
    modify_other_keys_active: bool,
    stdin_buffer: StdinBuffer,
    #[cfg_attr(not(test), allow(dead_code))]
    stdin_events: Receiver<StdinBufferEvent>,
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
        let stdin_buffer = StdinBuffer::new(StdinBufferOptions {
            timeout: DEFAULT_STDIN_TIMEOUT,
        });
        let stdin_events = stdin_buffer.subscribe();

        Self {
            backend,
            input_handler: None,
            resize_handler: None,
            kitty_protocol_active: false,
            modify_other_keys_active: false,
            stdin_buffer,
            stdin_events,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn process_input_data(&mut self, data: &str) -> Result<(), TuiError> {
        self.stdin_buffer.process_str(data);

        while let Ok(event) = self.stdin_events.try_recv() {
            match event {
                StdinBufferEvent::Data(sequence) => {
                    if !self.kitty_protocol_active && is_kitty_protocol_response(&sequence) {
                        self.kitty_protocol_active = true;
                        set_kitty_protocol_active(true);
                        self.backend.write(KITTY_ENABLE_FLAGS)?;
                        continue;
                    }

                    if let Some(handler) = &mut self.input_handler {
                        handler(sequence);
                    }
                }
                StdinBufferEvent::Paste(content) => {
                    if let Some(handler) = &mut self.input_handler {
                        handler(format!("\x1b[200~{content}\x1b[201~"));
                    }
                }
            }
        }

        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn enable_modify_other_keys_fallback(&mut self) -> Result<(), TuiError> {
        if !self.kitty_protocol_active && !self.modify_other_keys_active {
            self.backend.write(MODIFY_OTHER_KEYS_ENABLE)?;
            self.modify_other_keys_active = true;
        }
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn notify_resize(&mut self) {
        if let Some(handler) = &mut self.resize_handler {
            handler();
        }
    }

    fn disable_input_protocols(&mut self) -> Result<(), TuiError> {
        if self.kitty_protocol_active {
            self.backend.write(KITTY_DISABLE)?;
            self.kitty_protocol_active = false;
            set_kitty_protocol_active(false);
        }

        if self.modify_other_keys_active {
            self.backend.write(MODIFY_OTHER_KEYS_DISABLE)?;
            self.modify_other_keys_active = false;
        }

        Ok(())
    }
}

impl Terminal for ProcessTerminal {
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send>,
        on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        self.input_handler = Some(on_input);
        self.resize_handler = Some(on_resize);
        self.backend.write(BRACKETED_PASTE_ENABLE)?;
        self.backend.write(KITTY_QUERY)?;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        self.backend.write(BRACKETED_PASTE_DISABLE)?;
        self.disable_input_protocols()?;
        self.stdin_buffer.destroy();
        self.input_handler = None;
        self.resize_handler = None;
        Ok(())
    }

    fn drain_input(&mut self, _max: Duration, _idle: Duration) -> Result<(), TuiError> {
        self.disable_input_protocols()
    }

    fn write(&mut self, data: &str) -> Result<(), TuiError> {
        self.backend.write(data)?;
        Ok(())
    }

    fn columns(&self) -> u16 {
        self.backend.columns()
    }

    fn rows(&self) -> u16 {
        self.backend.rows()
    }

    fn kitty_protocol_active(&self) -> bool {
        self.kitty_protocol_active
    }

    fn move_by(&mut self, lines: i32) -> Result<(), TuiError> {
        if lines > 0 {
            self.backend.write(&format!("\x1b[{lines}B"))?;
        } else if lines < 0 {
            self.backend.write(&format!("\x1b[{}A", -lines))?;
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), TuiError> {
        self.backend.write("\x1b[?25l")?;
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), TuiError> {
        self.backend.write("\x1b[?25h")?;
        Ok(())
    }

    fn clear_line(&mut self) -> Result<(), TuiError> {
        self.backend.write("\x1b[K")?;
        Ok(())
    }

    fn clear_from_cursor(&mut self) -> Result<(), TuiError> {
        self.backend.write("\x1b[J")?;
        Ok(())
    }

    fn clear_screen(&mut self) -> Result<(), TuiError> {
        self.backend.write("\x1b[2J\x1b[H")?;
        Ok(())
    }

    fn set_title(&mut self, title: &str) -> Result<(), TuiError> {
        self.backend.write(&format!("\x1b]0;{title}\x07"))?;
        Ok(())
    }
}

trait TerminalBackend: Send {
    fn write(&mut self, data: &str) -> io::Result<()>;
    fn columns(&self) -> u16;
    fn rows(&self) -> u16;
}

struct StdoutBackend;

impl TerminalBackend for StdoutBackend {
    fn write(&mut self, data: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(data.as_bytes())?;
        stdout.flush()
    }

    fn columns(&self) -> u16 {
        env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_COLUMNS)
    }

    fn rows(&self) -> u16 {
        env::var("LINES")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_ROWS)
    }
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
