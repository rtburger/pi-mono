use pi_tui::{Component, Spacer, Text, visible_width};

#[test]
fn text_returns_no_lines_for_blank_text() {
    let text = Text::new("   ", 1, 1);

    assert_eq!(text.render(20), Vec::<String>::new());
}

#[test]
fn spacer_renders_requested_empty_lines() {
    let spacer = Spacer::new(3);

    assert_eq!(spacer.render(20), vec!["", "", ""]);
}

#[test]
fn text_wraps_with_padding_and_full_width_lines() {
    let text = Text::new("hello world from rust", 1, 0);
    let lines = text.render(10);

    assert!(lines.len() > 1);
    assert!(lines.iter().all(|line| visible_width(line) == 10));
    assert!(lines[0].contains("hello"));
    assert!(lines.iter().any(|line| line.contains("rust")));
}

#[test]
fn text_adds_vertical_padding() {
    let text = Text::new("body", 0, 2);
    let lines = text.render(6);

    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "      ");
    assert_eq!(lines[1], "      ");
    assert_eq!(lines[2], "body  ");
    assert_eq!(lines[3], "      ");
    assert_eq!(lines[4], "      ");
}

#[test]
fn text_replaces_tabs_with_three_spaces_before_wrapping() {
    let text = Text::new("a\tb", 0, 0);
    let lines = text.render(10);

    assert_eq!(lines, vec!["a   b     "]);
}

#[test]
fn text_applies_background_to_content_and_padding() {
    let text = Text::with_custom_bg_fn("hello", 1, 1, |line| format!("\x1b[44m{line}\x1b[0m"));
    let lines = text.render(10);

    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|line| line.starts_with("\x1b[44m")));
    assert!(lines.iter().all(|line| line.ends_with("\x1b[0m")));
    assert!(lines.iter().all(|line| visible_width(line) == 10));
}
