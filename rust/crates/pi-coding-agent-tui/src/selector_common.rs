use crate::{KeybindingsManager, ThemedKeyHintStyler, key_hint};
use pi_tui::{matches_key, truncate_to_width};
use std::cell::Cell;

pub type CancelCallback = Box<dyn FnMut() + Send + 'static>;
pub type SelectCallback<T> = Box<dyn FnMut(T) + Send + 'static>;
pub type PreviewCallback<T> = Box<dyn FnMut(T) + Send + 'static>;
pub type ActionCallback<T> = Box<dyn FnMut(T) + Send + 'static>;

pub fn matches_binding(keybindings: &KeybindingsManager, data: &str, keybinding: &str) -> bool {
    keybindings
        .get_keys(keybinding)
        .iter()
        .any(|key| matches_key(data, key.as_str()))
}

pub fn max_visible(
    viewport_size: &Cell<Option<(usize, usize)>>,
    reserved_lines: usize,
    fallback: usize,
) -> usize {
    viewport_size
        .get()
        .map(|(_, height)| height.saturating_sub(reserved_lines).max(1))
        .unwrap_or(fallback)
}

pub fn visible_window(selected_index: usize, len: usize, max_visible: usize) -> (usize, usize) {
    if len == 0 {
        return (0, 0);
    }

    let start_index = selected_index
        .saturating_sub(max_visible / 2)
        .min(len.saturating_sub(max_visible));
    let end_index = (start_index + max_visible).min(len);
    (start_index, end_index)
}

pub fn render_hint_line(
    keybindings: &KeybindingsManager,
    width: usize,
    hints: &[(&str, &str)],
) -> String {
    let styler = ThemedKeyHintStyler;
    let hint = hints
        .iter()
        .filter_map(|(binding, description)| {
            let rendered = key_hint(keybindings, &styler, binding, description);
            (!rendered.is_empty()).then_some(rendered)
        })
        .collect::<Vec<_>>()
        .join("  ");
    truncate_to_width(&hint, width, "...", false)
}

pub fn framed_lines(
    width: usize,
    title: &str,
    mut body: Vec<String>,
    hint_line: Option<String>,
) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    lines.push("─".repeat(width));
    lines.push(truncate_to_width(title, width, "...", false));
    lines.append(&mut body);
    if let Some(hint_line) = hint_line {
        lines.push(hint_line);
    }
    lines.push("─".repeat(width));
    lines
}

pub fn cycle_index(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }

    if forward {
        if current + 1 >= len { 0 } else { current + 1 }
    } else if current == 0 {
        len - 1
    } else {
        current - 1
    }
}

pub fn sanitize_display_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() && character != '\n' && character != '\t' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
