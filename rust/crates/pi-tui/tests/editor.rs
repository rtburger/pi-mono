use pi_tui::{
    CURSOR_MARKER, Component, Editor, EditorCursor, EditorOptions, visible_width, word_wrap_line,
};
use std::sync::{Arc, Mutex};

#[test]
fn backslash_enter_inserts_newline_instead_of_submitting() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_clone = Arc::clone(&submitted);

    let mut editor = Editor::new();
    editor.set_on_submit(move |value| {
        *submitted_clone.lock().expect("submitted mutex poisoned") = Some(value);
    });

    editor.handle_input("\\");
    editor.handle_input("\r");

    assert_eq!(editor.get_text(), "\n");
    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        None
    );
}

#[test]
fn submit_resets_editor_and_emits_trimmed_text() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_clone = Arc::clone(&submitted);

    let mut editor = Editor::new();
    editor.set_on_submit(move |value| {
        *submitted_clone.lock().expect("submitted mutex poisoned") = Some(value);
    });

    editor.handle_input(" ");
    editor.handle_input("h");
    editor.handle_input("i");
    editor.handle_input("\n");
    editor.handle_input("t");
    editor.handle_input("h");
    editor.handle_input("e");
    editor.handle_input("r");
    editor.handle_input("e");
    editor.handle_input(" ");
    editor.handle_input("\r");

    assert_eq!(editor.get_text(), "");
    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("hi\nthere")
    );
}

#[test]
fn backspace_at_start_of_line_merges_with_previous_line() {
    let mut editor = Editor::new();
    editor.set_text("foo\nbar");

    editor.handle_input("\x1b[D");
    editor.handle_input("\x1b[D");
    editor.handle_input("\x1b[D");
    editor.handle_input("\x7f");

    assert_eq!(editor.get_text(), "foobar");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 3 });
}

#[test]
fn history_navigation_supports_multiline_entries() {
    let mut editor = Editor::new();
    editor.add_to_history("older entry");
    editor.add_to_history("line1\nline2\nline3");

    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_text(), "line1\nline2\nline3");

    editor.handle_input("\x1b[A");
    editor.handle_input("\x1b[A");
    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_text(), "older entry");

    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_text(), "line1\nline2\nline3");

    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_text(), "");
}

#[test]
fn word_wrap_line_preserves_boundary_whitespace_rules() {
    let chunks = word_wrap_line("hello world test", 11);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].text, "hello ");
    assert_eq!(chunks[1].text, "world test");

    let chunks = word_wrap_line("hello world test", 12);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].text, "hello world ");
    assert_eq!(chunks[1].text, "test");
}

#[test]
fn bracketed_paste_preserves_newlines_and_expands_tabs() {
    let mut editor = Editor::new();

    editor.handle_input("\x1b[200~foo\tbar\nbaz\x1b[201~");

    assert_eq!(editor.get_text(), "foo    bar\nbaz");
}

#[test]
fn render_wraps_wide_text_without_overflow_and_emits_cursor_marker() {
    let mut editor = Editor::with_options(EditorOptions { padding_x: 1 });
    editor.set_text("日本語テスト");
    editor.set_focused(true);
    editor.set_viewport_size(11, 24);

    let lines = editor.render(11);

    assert!(lines.iter().any(|line| line.contains(CURSOR_MARKER)));
    for line in &lines {
        assert!(
            visible_width(line) <= 11,
            "rendered line overflowed width: {line:?}"
        );
    }
}
