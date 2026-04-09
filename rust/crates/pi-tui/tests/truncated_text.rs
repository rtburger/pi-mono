use pi_tui::{Component, TruncatedText, visible_width};

const RED: &str = "\x1b[31m";
const BLUE: &str = "\x1b[34m";
const FG_RESET: &str = "\x1b[39m";

#[test]
fn truncated_text_pads_output_lines_to_exact_width() {
    let text = TruncatedText::new("Hello world", 1, 0);
    let lines = text.render(50);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 50);
}

#[test]
fn truncated_text_pads_vertical_padding_lines_to_width() {
    let text = TruncatedText::new("Hello", 0, 2);
    let lines = text.render(40);

    assert_eq!(lines.len(), 5);
    assert!(lines.iter().all(|line| visible_width(line) == 40));
}

#[test]
fn truncated_text_truncates_long_text_and_pads_to_width() {
    let long_text =
        "This is a very long piece of text that will definitely exceed the available width";
    let text = TruncatedText::new(long_text, 1, 0);
    let lines = text.render(30);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 30);
    assert!(lines[0].contains("..."));
}

#[test]
fn truncated_text_preserves_ansi_codes_in_output() {
    let styled_text = format!("{RED}Hello{FG_RESET} {BLUE}world{FG_RESET}");
    let text = TruncatedText::new(styled_text, 1, 0);
    let lines = text.render(40);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 40);
    assert!(lines[0].contains("\x1b["));
}

#[test]
fn truncated_text_adds_reset_before_ellipsis_when_truncating_styled_text() {
    let long_styled_text =
        format!("{RED}This is a very long red text that will be truncated{FG_RESET}");
    let text = TruncatedText::new(long_styled_text, 1, 0);
    let lines = text.render(20);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 20);
    assert!(lines[0].contains("\x1b[0m..."));
}

#[test]
fn truncated_text_does_not_add_ellipsis_when_text_fits() {
    let text = TruncatedText::new("Hello world", 1, 0);
    let lines = text.render(30);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 30);
    assert!(!lines[0].contains("..."));
}

#[test]
fn truncated_text_handles_empty_text() {
    let text = TruncatedText::new("", 1, 0);
    let lines = text.render(30);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 30);
}

#[test]
fn truncated_text_stops_at_newline_and_only_shows_first_line() {
    let text = TruncatedText::new("First line\nSecond line\nThird line", 1, 0);
    let lines = text.render(40);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 40);
    assert!(lines[0].contains("First line"));
    assert!(!lines[0].contains("Second line"));
    assert!(!lines[0].contains("Third line"));
}

#[test]
fn truncated_text_truncates_only_the_first_line_when_multiline_input_is_long() {
    let text = TruncatedText::new(
        "This is a very long first line that needs truncation\nSecond line",
        1,
        0,
    );
    let lines = text.render(25);

    assert_eq!(lines.len(), 1);
    assert_eq!(visible_width(&lines[0]), 25);
    assert!(lines[0].contains("..."));
    assert!(!lines[0].contains("Second line"));
}
