use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

const ESC: &str = "\x1b";
const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdinBufferEvent {
    Data(String),
    Paste(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdinBufferOptions {
    pub timeout: Duration,
}

impl Default for StdinBufferOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(10),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StdinBuffer {
    inner: Arc<Mutex<Inner>>,
    generation: Arc<AtomicU64>,
}

#[derive(Debug)]
struct Inner {
    buffer: String,
    paste_mode: bool,
    paste_buffer: String,
    timeout: Duration,
    listeners: Vec<Sender<StdinBufferEvent>>,
}

#[derive(Debug)]
struct ExtractedSequences {
    sequences: Vec<String>,
    remainder: String,
}

#[derive(Debug)]
struct ProcessOutcome {
    events: Vec<StdinBufferEvent>,
    schedule_timeout: bool,
    reprocess: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequenceStatus {
    Complete,
    Incomplete,
    NotEscape,
}

impl StdinBuffer {
    pub fn new(options: StdinBufferOptions) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                buffer: String::new(),
                paste_mode: false,
                paste_buffer: String::new(),
                timeout: options.timeout,
                listeners: Vec::new(),
            })),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn subscribe(&self) -> Receiver<StdinBufferEvent> {
        let (sender, receiver) = mpsc::channel();
        let mut inner = self.inner.lock().expect("stdin buffer mutex poisoned");
        inner.listeners.push(sender);
        receiver
    }

    pub fn process_str(&self, data: &str) {
        self.cancel_pending_timeout();

        let outcome = {
            let mut inner = self.inner.lock().expect("stdin buffer mutex poisoned");
            let str_data = data.to_owned();

            if str_data.is_empty() && inner.buffer.is_empty() {
                ProcessOutcome {
                    events: vec![StdinBufferEvent::Data(String::new())],
                    schedule_timeout: false,
                    reprocess: None,
                }
            } else {
                inner.buffer.push_str(&str_data);
                process_buffer_locked(&mut inner)
            }
        };

        self.emit_events(outcome.events);

        if outcome.schedule_timeout {
            self.schedule_timeout();
        }

        if let Some(remaining) = outcome.reprocess {
            self.process_str(&remaining);
        }
    }

    pub fn process_bytes(&self, data: &[u8]) {
        let string = if data.len() == 1 && data[0] > 127 {
            let byte = data[0] - 128;
            format!("\x1b{}", char::from(byte))
        } else {
            String::from_utf8_lossy(data).into_owned()
        };
        self.process_str(&string);
    }

    pub fn flush(&self) -> Vec<String> {
        self.cancel_pending_timeout();

        let mut inner = self.inner.lock().expect("stdin buffer mutex poisoned");
        if inner.buffer.is_empty() {
            return Vec::new();
        }

        let flushed = vec![inner.buffer.clone()];
        inner.buffer.clear();
        flushed
    }

    pub fn clear(&self) {
        self.cancel_pending_timeout();

        let mut inner = self.inner.lock().expect("stdin buffer mutex poisoned");
        inner.buffer.clear();
        inner.paste_mode = false;
        inner.paste_buffer.clear();
    }

    pub fn get_buffer(&self) -> String {
        self.inner
            .lock()
            .expect("stdin buffer mutex poisoned")
            .buffer
            .clone()
    }

    pub fn destroy(&self) {
        self.clear();
    }

    fn cancel_pending_timeout(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn schedule_timeout(&self) {
        let generation = self.generation.load(Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        let generation_counter = Arc::clone(&self.generation);

        let timeout = inner.lock().expect("stdin buffer mutex poisoned").timeout;
        thread::spawn(move || {
            thread::sleep(timeout);
            if generation_counter.load(Ordering::SeqCst) != generation {
                return;
            }

            let (events, listeners) = {
                let mut inner = inner.lock().expect("stdin buffer mutex poisoned");
                if generation_counter.load(Ordering::SeqCst) != generation
                    || inner.buffer.is_empty()
                {
                    (Vec::new(), Vec::new())
                } else {
                    let flushed = inner.buffer.clone();
                    inner.buffer.clear();
                    (
                        vec![StdinBufferEvent::Data(flushed)],
                        inner.listeners.clone(),
                    )
                }
            };

            if events.is_empty() || listeners.is_empty() {
                return;
            }

            emit_to_listeners(&inner, listeners, events);
        });
    }

    fn emit_events(&self, events: Vec<StdinBufferEvent>) {
        if events.is_empty() {
            return;
        }

        let listeners = {
            self.inner
                .lock()
                .expect("stdin buffer mutex poisoned")
                .listeners
                .clone()
        };
        emit_to_listeners(&self.inner, listeners, events);
    }
}

fn emit_to_listeners(
    inner: &Arc<Mutex<Inner>>,
    listeners: Vec<Sender<StdinBufferEvent>>,
    events: Vec<StdinBufferEvent>,
) {
    if listeners.is_empty() {
        return;
    }

    let mut active_listeners = Vec::new();
    for listener in listeners {
        let mut delivered_all = true;
        for event in &events {
            if listener.send(event.clone()).is_err() {
                delivered_all = false;
                break;
            }
        }
        if delivered_all {
            active_listeners.push(listener);
        }
    }

    let mut guard = inner.lock().expect("stdin buffer mutex poisoned");
    guard.listeners = active_listeners;
}

fn process_buffer_locked(inner: &mut Inner) -> ProcessOutcome {
    if inner.paste_mode {
        inner.paste_buffer.push_str(&inner.buffer);
        inner.buffer.clear();

        if let Some(end_index) = inner.paste_buffer.find(BRACKETED_PASTE_END) {
            let pasted_content = inner.paste_buffer[..end_index].to_owned();
            let remaining = inner.paste_buffer[end_index + BRACKETED_PASTE_END.len()..].to_owned();

            inner.paste_mode = false;
            inner.paste_buffer.clear();

            return ProcessOutcome {
                events: vec![StdinBufferEvent::Paste(pasted_content)],
                schedule_timeout: false,
                reprocess: (!remaining.is_empty()).then_some(remaining),
            };
        }

        return ProcessOutcome {
            events: Vec::new(),
            schedule_timeout: false,
            reprocess: None,
        };
    }

    if let Some(start_index) = inner.buffer.find(BRACKETED_PASTE_START) {
        let mut events = Vec::new();

        if start_index > 0 {
            let before_paste = inner.buffer[..start_index].to_owned();
            let extracted = extract_complete_sequences(&before_paste);
            events.extend(extracted.sequences.into_iter().map(StdinBufferEvent::Data));
        }

        inner.buffer = inner.buffer[start_index + BRACKETED_PASTE_START.len()..].to_owned();
        inner.paste_mode = true;
        inner.paste_buffer = inner.buffer.clone();
        inner.buffer.clear();

        if let Some(end_index) = inner.paste_buffer.find(BRACKETED_PASTE_END) {
            let pasted_content = inner.paste_buffer[..end_index].to_owned();
            let remaining = inner.paste_buffer[end_index + BRACKETED_PASTE_END.len()..].to_owned();

            inner.paste_mode = false;
            inner.paste_buffer.clear();
            events.push(StdinBufferEvent::Paste(pasted_content));

            return ProcessOutcome {
                events,
                schedule_timeout: false,
                reprocess: (!remaining.is_empty()).then_some(remaining),
            };
        }

        return ProcessOutcome {
            events,
            schedule_timeout: false,
            reprocess: None,
        };
    }

    let extracted = extract_complete_sequences(&inner.buffer);
    inner.buffer = extracted.remainder;

    ProcessOutcome {
        events: extracted
            .sequences
            .into_iter()
            .map(StdinBufferEvent::Data)
            .collect(),
        schedule_timeout: !inner.buffer.is_empty(),
        reprocess: None,
    }
}

fn extract_complete_sequences(buffer: &str) -> ExtractedSequences {
    let mut sequences = Vec::new();
    let mut pos = 0usize;

    while pos < buffer.len() {
        let remaining = &buffer[pos..];

        if remaining.starts_with(ESC) {
            let mut seq_end = 1usize;
            while seq_end <= remaining.len() {
                let candidate = &remaining[..seq_end];
                match is_complete_sequence(candidate) {
                    SequenceStatus::Complete => {
                        sequences.push(candidate.to_owned());
                        pos += seq_end;
                        break;
                    }
                    SequenceStatus::Incomplete => {
                        seq_end += 1;
                    }
                    SequenceStatus::NotEscape => {
                        sequences.push(candidate.to_owned());
                        pos += seq_end;
                        break;
                    }
                }
            }

            if seq_end > remaining.len() {
                return ExtractedSequences {
                    sequences,
                    remainder: remaining.to_owned(),
                };
            }
        } else {
            let character = remaining
                .chars()
                .next()
                .expect("remaining buffer should contain a character");
            sequences.push(character.to_string());
            pos += character.len_utf8();
        }
    }

    ExtractedSequences {
        sequences,
        remainder: String::new(),
    }
}

fn is_complete_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with(ESC) {
        return SequenceStatus::NotEscape;
    }

    if data.len() == 1 {
        return SequenceStatus::Incomplete;
    }

    let after_escape = &data[1..];

    if after_escape.starts_with('[') {
        if after_escape.starts_with("[M") {
            return if data.len() >= 6 {
                SequenceStatus::Complete
            } else {
                SequenceStatus::Incomplete
            };
        }
        return is_complete_csi_sequence(data);
    }

    if after_escape.starts_with(']') {
        return is_complete_osc_sequence(data);
    }

    if after_escape.starts_with('P') {
        return is_complete_dcs_sequence(data);
    }

    if after_escape.starts_with('_') {
        return is_complete_apc_sequence(data);
    }

    if after_escape.starts_with('O') {
        return if after_escape.len() >= 2 {
            SequenceStatus::Complete
        } else {
            SequenceStatus::Incomplete
        };
    }

    if after_escape.len() == 1 {
        return SequenceStatus::Complete;
    }

    SequenceStatus::Complete
}

fn is_complete_csi_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b[") {
        return SequenceStatus::Complete;
    }

    if data.len() < 3 {
        return SequenceStatus::Incomplete;
    }

    let payload = &data[2..];
    let Some(last_char) = payload.chars().last() else {
        return SequenceStatus::Incomplete;
    };
    let last_char_code = last_char as u32;

    if (0x40..=0x7e).contains(&last_char_code) {
        if payload.starts_with('<') {
            if is_complete_mouse_sgr_payload(payload) {
                return SequenceStatus::Complete;
            }

            if matches!(last_char, 'M' | 'm') {
                let middle = &payload[1..payload.len() - 1];
                let parts = middle.split(';').collect::<Vec<_>>();
                if parts.len() == 3 && parts.iter().all(|part| is_ascii_digits(part)) {
                    return SequenceStatus::Complete;
                }
            }

            return SequenceStatus::Incomplete;
        }

        return SequenceStatus::Complete;
    }

    SequenceStatus::Incomplete
}

fn is_complete_osc_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b]") {
        return SequenceStatus::Complete;
    }

    if data.ends_with("\x1b\\") || data.ends_with('\u{0007}') {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn is_complete_dcs_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1bP") {
        return SequenceStatus::Complete;
    }

    if data.ends_with("\x1b\\") {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn is_complete_apc_sequence(data: &str) -> SequenceStatus {
    if !data.starts_with("\x1b_") {
        return SequenceStatus::Complete;
    }

    if data.ends_with("\x1b\\") {
        SequenceStatus::Complete
    } else {
        SequenceStatus::Incomplete
    }
}

fn is_complete_mouse_sgr_payload(payload: &str) -> bool {
    if !matches!(payload.chars().last(), Some('M' | 'm')) {
        return false;
    }

    let middle = &payload[1..payload.len() - 1];
    let parts = middle.split(';').collect::<Vec<_>>();
    parts.len() == 3 && parts.iter().all(|part| is_ascii_digits(part))
}

fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.as_bytes().iter().all(u8::is_ascii_digit)
}
