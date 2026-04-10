use pi_tui::{Component, Input, visible_width};
use std::sync::{Arc, Mutex};

#[test]
fn submits_value_including_backslash_on_enter() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_clone = Arc::clone(&submitted);

    let mut input = Input::new();
    input.set_on_submit(move |value| {
        *submitted_clone.lock().expect("submitted mutex poisoned") = Some(value);
    });

    input.handle_input("h");
    input.handle_input("e");
    input.handle_input("l");
    input.handle_input("l");
    input.handle_input("o");
    input.handle_input("\\");
    input.handle_input("\r");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("hello\\")
    );
}

#[test]
fn inserts_backslash_as_regular_character() {
    let mut input = Input::new();

    input.handle_input("\\");
    input.handle_input("x");

    assert_eq!(input.get_value(), "\\x");
}

#[test]
fn escape_triggers_cancel_callback() {
    let escaped = Arc::new(Mutex::new(false));
    let escaped_clone = Arc::clone(&escaped);

    let mut input = Input::new();
    input.set_on_escape(move || {
        *escaped_clone.lock().expect("escaped mutex poisoned") = true;
    });

    input.handle_input("\x1b");

    assert!(*escaped.lock().expect("escaped mutex poisoned"));
}

#[test]
fn bracketed_paste_strips_newlines_and_expands_tabs() {
    let mut input = Input::new();

    input.handle_input("\x1b[200~hello\nworld\t!\x1b[201~");

    assert_eq!(input.get_value(), "helloworld    !");
}

#[test]
fn ctrl_w_deletes_previous_word() {
    let mut input = Input::new();
    input.set_value("foo bar baz");
    input.handle_input("\x05");

    input.handle_input("\x17");

    assert_eq!(input.get_value(), "foo bar ");
}

#[test]
fn public_insert_text_at_cursor_inserts_at_current_cursor_position() {
    let mut input = Input::new();
    input.set_value("ac");
    input.set_cursor(1);

    input.insert_text_at_cursor("b");

    assert_eq!(input.get_value(), "abc");
    assert_eq!(input.cursor(), 2);
}

#[test]
fn render_does_not_overflow_with_wide_text() {
    let width = 93;
    let cases = [
        "가나다라마바사아자차카타파하 한글 텍스트가 터미널 너비를 초과하면 크래시가 발생합니다",
        "これはテスト文章です。日本語のテキストが正しく表示されるかどうかを確認するためのサンプルです。",
        "ＡＢＣＤＥＦＧＨＩＪＫＬＭＮＯＰＱＲＳＴＵＶＷＸＹＺ０１２３４５６７８９",
    ];

    for text in cases {
        for position in ["start", "middle", "end"] {
            let mut input = Input::new();
            input.set_value(text);
            input.set_focused(true);

            match position {
                "start" => {}
                "middle" => {
                    for _ in 0..10 {
                        input.handle_input("\x1b[C");
                    }
                }
                "end" => input.handle_input("\x05"),
                _ => unreachable!("unexpected cursor position"),
            }

            let lines = input.render(width);
            let line = &lines[0];
            assert!(
                visible_width(line) <= width,
                "rendered line overflowed for {position}: {line:?}"
            );
        }
    }
}

#[test]
fn keeps_cursor_visible_when_horizontally_scrolling_wide_text() {
    let mut input = Input::new();
    let width = 20;
    let text = "가나다라마바사아자차카타파하";
    input.set_value(text);
    input.set_focused(true);
    input.handle_input("\x01");
    for _ in 0..5 {
        input.handle_input("\x1b[C");
    }

    let lines = input.render(width);
    let line = &lines[0];
    assert!(visible_width(line) <= width);
}
