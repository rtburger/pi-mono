use pi_tui::{StdinBuffer, StdinBufferEvent, StdinBufferOptions};
use std::{sync::mpsc::Receiver, thread, time::Duration};

fn new_buffer() -> (StdinBuffer, Receiver<StdinBufferEvent>) {
    let buffer = StdinBuffer::new(StdinBufferOptions {
        timeout: Duration::from_millis(10),
    });
    let receiver = buffer.subscribe();
    (buffer, receiver)
}

fn drain_events(receiver: &Receiver<StdinBufferEvent>) -> Vec<StdinBufferEvent> {
    receiver.try_iter().collect()
}

fn drain_data(receiver: &Receiver<StdinBufferEvent>) -> Vec<String> {
    drain_events(receiver)
        .into_iter()
        .filter_map(|event| match event {
            StdinBufferEvent::Data(value) => Some(value),
            StdinBufferEvent::Paste(_) => None,
        })
        .collect()
}

fn drain_paste(receiver: &Receiver<StdinBufferEvent>) -> Vec<String> {
    drain_events(receiver)
        .into_iter()
        .filter_map(|event| match event {
            StdinBufferEvent::Paste(value) => Some(value),
            StdinBufferEvent::Data(_) => None,
        })
        .collect()
}

#[test]
fn regular_characters_pass_through_immediately() {
    let (buffer, receiver) = new_buffer();

    buffer.process_str("a");
    assert_eq!(drain_data(&receiver), vec!["a"]);

    buffer.process_str("abc");
    assert_eq!(drain_data(&receiver), vec!["a", "b", "c"]);

    buffer.process_str("hello 世界");
    assert_eq!(
        drain_data(&receiver),
        vec!["h", "e", "l", "l", "o", " ", "世", "界"]
    );
}

#[test]
fn complete_escape_sequences_emit_as_single_items() {
    let (buffer, receiver) = new_buffer();

    for sequence in ["\x1b[<35;20;5m", "\x1b[A", "\x1b[11~", "\x1ba", "\x1bOA"] {
        buffer.process_str(sequence);
    }

    assert_eq!(
        drain_data(&receiver),
        vec!["\x1b[<35;20;5m", "\x1b[A", "\x1b[11~", "\x1ba", "\x1bOA"]
    );
}

#[test]
fn partial_escape_sequences_are_buffered_until_complete_or_timeout() {
    let (buffer, receiver) = new_buffer();

    buffer.process_str("\x1b");
    assert!(drain_events(&receiver).is_empty());
    assert_eq!(buffer.get_buffer(), "\x1b");

    buffer.process_str("[<35");
    assert!(drain_events(&receiver).is_empty());
    assert_eq!(buffer.get_buffer(), "\x1b[<35");

    buffer.process_str(";20;5m");
    assert_eq!(drain_data(&receiver), vec!["\x1b[<35;20;5m"]);
    assert_eq!(buffer.get_buffer(), "");

    buffer.process_str("\x1b[<35");
    thread::sleep(Duration::from_millis(15));
    assert_eq!(drain_data(&receiver), vec!["\x1b[<35"]);
}

#[test]
fn mixed_content_and_kitty_batches_split_correctly() {
    let (buffer, receiver) = new_buffer();

    buffer.process_str("abc\x1b[A");
    assert_eq!(drain_data(&receiver), vec!["a", "b", "c", "\x1b[A"]);

    buffer.process_str("\x1b[Aabc");
    assert_eq!(drain_data(&receiver), vec!["\x1b[A", "a", "b", "c"]);

    buffer.process_str("\x1b[97u\x1b[97;1:3u\x1b[98u\x1b[98;1:3u");
    assert_eq!(
        drain_data(&receiver),
        vec!["\x1b[97u", "\x1b[97;1:3u", "\x1b[98u", "\x1b[98;1:3u"]
    );

    buffer.process_str("a\x1b[97;1:3u");
    assert_eq!(drain_data(&receiver), vec!["a", "\x1b[97;1:3u"]);

    buffer.process_str("\x1b[97ua");
    assert_eq!(drain_data(&receiver), vec!["\x1b[97u", "a"]);
}

#[test]
fn mouse_sequences_include_sgr_and_old_style_forms() {
    let (buffer, receiver) = new_buffer();

    buffer.process_str("\x1b[<0;10;5M\x1b[<0;10;5m\x1b[<35;20;5m");
    assert_eq!(
        drain_data(&receiver),
        vec!["\x1b[<0;10;5M", "\x1b[<0;10;5m", "\x1b[<35;20;5m"]
    );

    buffer.process_str("\x1b[<3");
    buffer.process_str("5;1");
    buffer.process_str("5;");
    buffer.process_str("10m");
    assert_eq!(drain_data(&receiver), vec!["\x1b[<35;15;10m"]);

    buffer.process_str("\x1b[M");
    assert_eq!(buffer.get_buffer(), "\x1b[M");
    buffer.process_str(" a");
    assert_eq!(buffer.get_buffer(), "\x1b[M a");
    buffer.process_str("bc");
    assert_eq!(drain_data(&receiver), vec!["\x1b[M ab", "c"]);
}

#[test]
fn flush_clear_and_destroy_match_typescript_behavior() {
    let (buffer, receiver) = new_buffer();

    buffer.process_str("\x1b[<35");
    assert_eq!(buffer.flush(), vec!["\x1b[<35"]);
    assert_eq!(buffer.get_buffer(), "");
    assert!(drain_events(&receiver).is_empty());
    assert!(buffer.flush().is_empty());

    buffer.process_str("\x1b[<35");
    buffer.clear();
    assert_eq!(buffer.get_buffer(), "");
    assert!(drain_events(&receiver).is_empty());

    buffer.process_str("\x1b[<35");
    buffer.destroy();
    assert_eq!(buffer.get_buffer(), "");
    thread::sleep(Duration::from_millis(15));
    assert!(drain_events(&receiver).is_empty());
}

#[test]
fn bracketed_paste_emits_paste_events_and_preserves_surrounding_input() {
    let (buffer, receiver) = new_buffer();

    buffer.process_str("\x1b[200~hello world\x1b[201~");
    assert_eq!(drain_paste(&receiver), vec!["hello world"]);
    assert!(drain_data(&receiver).is_empty());

    buffer.process_str("\x1b[200~");
    buffer.process_str("hello ");
    buffer.process_str("world\x1b[201~");
    assert_eq!(drain_paste(&receiver), vec!["hello world"]);
    assert!(drain_data(&receiver).is_empty());

    buffer.process_str("a");
    buffer.process_str("\x1b[200~pasted\x1b[201~");
    buffer.process_str("b");
    let mixed_events = drain_events(&receiver);
    assert_eq!(
        mixed_events
            .iter()
            .filter_map(|event| match event {
                StdinBufferEvent::Data(value) => Some(value.clone()),
                StdinBufferEvent::Paste(_) => None,
            })
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(
        mixed_events
            .into_iter()
            .filter_map(|event| match event {
                StdinBufferEvent::Paste(value) => Some(value),
                StdinBufferEvent::Data(_) => None,
            })
            .collect::<Vec<_>>(),
        vec!["pasted"]
    );

    buffer.process_str("\x1b[200~line1\nline2\nline3\x1b[201~");
    buffer.process_str("\x1b[200~Hello 世界 🎉\x1b[201~");
    assert_eq!(
        drain_paste(&receiver),
        vec!["line1\nline2\nline3", "Hello 世界 🎉"]
    );
}

#[test]
fn empty_input_emits_empty_data_event_and_lone_escape_can_be_flushed() {
    let (buffer, receiver) = new_buffer();

    buffer.process_str("");
    assert_eq!(drain_data(&receiver), vec![""]);

    buffer.process_str("\x1b");
    assert!(drain_events(&receiver).is_empty());
    thread::sleep(Duration::from_millis(15));
    assert_eq!(drain_data(&receiver), vec!["\x1b"]);

    buffer.process_str("\x1b");
    assert_eq!(buffer.flush(), vec!["\x1b"]);
}

#[test]
fn process_bytes_handles_normal_utf8_and_single_high_byte_conversion() {
    let (buffer, receiver) = new_buffer();

    buffer.process_bytes("\x1b[A".as_bytes());
    assert_eq!(drain_data(&receiver), vec!["\x1b[A"]);

    buffer.process_bytes(&[0xE1]);
    assert_eq!(drain_data(&receiver), vec!["\x1ba"]);
}

#[test]
fn long_sequences_and_terminal_responses_are_buffered_as_complete_sequences() {
    let (buffer, receiver) = new_buffer();

    let long_sequence = format!("\x1b[{}H", "1;".repeat(50));
    buffer.process_str(&long_sequence);
    buffer.process_str("\x1b]11;rgb:0000/0000/0000\x07");
    buffer.process_str("\x1bP>|terminal\x1b\\");
    buffer.process_str("\x1b_Gi=1;OK\x1b\\");

    assert_eq!(
        drain_data(&receiver),
        vec![
            long_sequence,
            "\x1b]11;rgb:0000/0000/0000\x07".to_string(),
            "\x1bP>|terminal\x1b\\".to_string(),
            "\x1b_Gi=1;OK\x1b\\".to_string(),
        ]
    );
}
