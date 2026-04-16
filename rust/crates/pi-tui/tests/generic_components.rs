use pi_tui::{
    Box as TuiBox, CancellableLoader, Component, DefaultTextStyle, Loader, Markdown, MarkdownTheme,
    Text, visible_width,
};
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const KEY_ESCAPE: &str = "\x1b";

fn ansi(code: &str, text: &str) -> String {
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn markdown_theme() -> MarkdownTheme {
    MarkdownTheme::new()
        .with_heading(|text| ansi("36", text))
        .with_link(|text| ansi("34", text))
        .with_link_url(|text| ansi("90", text))
        .with_code(|text| ansi("33", text))
        .with_code_block(|text| ansi("32", text))
        .with_code_block_border(|text| ansi("90", text))
        .with_quote(|text| ansi("35", text))
        .with_quote_border(|text| ansi("35", text))
        .with_hr(|text| ansi("90", text))
        .with_list_bullet(|text| ansi("36", text))
        .with_bold(|text| ansi("1", text))
        .with_italic(|text| ansi("3", text))
        .with_strikethrough(|text| ansi("9", text))
        .with_underline(|text| ansi("4", text))
}

#[test]
fn box_component_applies_padding_and_background() {
    let mut component = TuiBox::with_bg_fn(1, 1, |text| ansi("48;5;240", text));
    component.add_child(std::boxed::Box::new(Text::new("hello", 0, 0)));

    let first = component.render(10);

    assert_eq!(first.len(), 3);
    assert!(first.iter().all(|line| visible_width(line) == 10));
    assert!(first[1].contains("hello"), "lines: {first:?}");

    component.set_bg_fn(|text| ansi("48;5;25", text));
    let recolored = component.render(10);

    assert_ne!(first, recolored);
    assert!(recolored.iter().all(|line| visible_width(line) == 10));
}

#[test]
fn loader_animates_and_updates_messages() {
    let loader = Loader::without_render_handle(
        |text| ansi("36", text),
        |text| ansi("90", text),
        "Loading...",
    );

    let first = loader.render(24);
    thread::sleep(Duration::from_millis(120));
    let second = loader.render(24);
    loader.set_message("Still loading...");
    let third = loader.render(24);
    loader.stop();

    assert_eq!(first.len(), 2);
    assert_ne!(
        first[1], second[1],
        "spinner did not advance: {first:?} vs {second:?}"
    );
    assert!(third[1].contains("Still loading..."), "lines: {third:?}");
}

#[test]
fn cancellable_loader_aborts_on_escape() {
    let aborted = Arc::new(Mutex::new(false));
    let mut loader = CancellableLoader::without_render_handle(
        |text| ansi("36", text),
        |text| ansi("90", text),
        "Working...",
    );
    {
        let aborted = Arc::clone(&aborted);
        loader.set_on_abort(move || *aborted.lock().expect("aborted mutex poisoned") = true);
    }

    let signal = loader.signal();
    assert!(!*signal.borrow());
    assert!(!loader.aborted());

    loader.handle_input(KEY_ESCAPE);
    loader.stop();

    assert!(*loader.signal().borrow());
    assert!(loader.aborted());
    assert!(*aborted.lock().expect("aborted mutex poisoned"));
}

#[test]
fn markdown_renders_block_elements_and_full_width_background() {
    let markdown = Markdown::with_default_text_style(
        "# Title\n\nSome **bold** [site](https://example.com).\n\n- one\n- two\n\n> quoted `code`\n\n```rust\nfn main() {}\n```\n\n| Name | Value |\n| --- | --- |\n| alpha | beta |",
        1,
        1,
        markdown_theme(),
        DefaultTextStyle::new()
            .with_color(|text| ansi("37", text))
            .with_bg_color(|text| ansi("48;5;236", text)),
    );

    let lines = markdown.render(60);

    assert!(lines.iter().all(|line| visible_width(line) == 60));
    assert!(
        lines.iter().any(|line| line.contains("Title")),
        "lines: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("site") && line.contains("https://example.com")),
        "lines: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("→")),
        "lines: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("│ ")),
        "lines: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("```rust")),
        "lines: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("┌─")),
        "lines: {lines:?}"
    );
}

#[test]
fn markdown_renders_html_as_plain_text_and_avoids_duplicate_autolink_urls() {
    let markdown = Markdown::new(
        "<span>inline</span> [https://example.com](https://example.com)",
        0,
        0,
        markdown_theme(),
    );

    let lines = markdown.render(80);

    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("<span>inline</span>"));
    assert_eq!(
        lines[0].matches("https://example.com").count(),
        1,
        "lines: {lines:?}"
    );
}
