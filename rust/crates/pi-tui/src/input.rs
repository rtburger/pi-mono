use crate::{
    keybindings::KeybindingsManager,
    keys::{decode_kitty_printable, matches_key},
    tui::{CURSOR_MARKER, Component},
    utils::{is_punctuation_char, is_whitespace_char, slice_by_column, visible_width},
};
use std::collections::BTreeMap;
use unicode_segmentation::UnicodeSegmentation;

const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";
const CURSOR_ON: &str = "\x1b[7m";
const CURSOR_OFF: &str = "\x1b[27m";
const PROMPT: &str = "> ";

pub struct Input {
    value: String,
    cursor: usize,
    on_submit: Option<Box<dyn FnMut(String) + Send + 'static>>,
    on_escape: Option<Box<dyn FnMut() + Send + 'static>>,
    focused: bool,
    paste_buffer: String,
    is_in_paste: bool,
    keybindings: KeybindingsManager,
}

impl Input {
    pub fn new() -> Self {
        Self::with_keybindings(KeybindingsManager::with_tui_defaults(BTreeMap::new()))
    }

    pub fn with_keybindings(keybindings: KeybindingsManager) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            on_submit: None,
            on_escape: None,
            focused: false,
            paste_buffer: String::new(),
            is_in_paste: false,
            keybindings,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn get_value(&self) -> &str {
        self.value()
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.cursor.min(self.value.len());
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.value.len());
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn set_on_submit<F>(&mut self, on_submit: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_submit = Some(Box::new(on_submit));
    }

    pub fn clear_on_submit(&mut self) {
        self.on_submit = None;
    }

    pub fn set_on_escape<F>(&mut self, on_escape: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_escape = Some(Box::new(on_escape));
    }

    pub fn clear_on_escape(&mut self) {
        self.on_escape = None;
    }

    pub fn handle_input(&mut self, data: &str) {
        self.process_input(data);
    }

    fn process_input(&mut self, data: &str) {
        let mut current = data.to_owned();

        if current.contains(BRACKETED_PASTE_START) {
            self.is_in_paste = true;
            self.paste_buffer.clear();
            current = current.replace(BRACKETED_PASTE_START, "");
        }

        if self.is_in_paste {
            self.paste_buffer.push_str(&current);
            if let Some(end_index) = self.paste_buffer.find(BRACKETED_PASTE_END) {
                let pasted_text = self.paste_buffer[..end_index].to_owned();
                let remaining =
                    self.paste_buffer[end_index + BRACKETED_PASTE_END.len()..].to_owned();
                self.paste_buffer.clear();
                self.is_in_paste = false;
                self.handle_paste(&pasted_text);
                if !remaining.is_empty() {
                    self.process_input(&remaining);
                }
            }
            return;
        }

        if self.matches_binding(&current, "tui.select.cancel") {
            if let Some(on_escape) = &mut self.on_escape {
                on_escape();
            }
            return;
        }

        if self.matches_binding(&current, "tui.input.submit") || current == "\n" {
            if let Some(on_submit) = &mut self.on_submit {
                on_submit(self.value.clone());
            }
            return;
        }

        if self.matches_binding(&current, "tui.editor.deleteCharBackward") {
            self.handle_backspace();
            return;
        }

        if self.matches_binding(&current, "tui.editor.deleteCharForward") {
            self.handle_forward_delete();
            return;
        }

        if self.matches_binding(&current, "tui.editor.deleteWordBackward") {
            self.delete_word_backward();
            return;
        }

        if self.matches_binding(&current, "tui.editor.deleteWordForward") {
            self.delete_word_forward();
            return;
        }

        if self.matches_binding(&current, "tui.editor.deleteToLineStart") {
            self.delete_to_line_start();
            return;
        }

        if self.matches_binding(&current, "tui.editor.deleteToLineEnd") {
            self.delete_to_line_end();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorLeft") {
            self.move_cursor_left();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorRight") {
            self.move_cursor_right();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorLineStart") {
            self.cursor = 0;
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorLineEnd") {
            self.cursor = self.value.len();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorWordLeft") {
            self.move_word_backward();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorWordRight") {
            self.move_word_forward();
            return;
        }

        if let Some(printable) = decode_kitty_printable(&current) {
            self.insert_text_at_cursor(&printable);
            return;
        }

        if !contains_control_characters(&current) {
            self.insert_text_at_cursor(&current);
        }
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn insert_text_at_cursor(&mut self, text: &str) {
        self.value.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn handle_backspace(&mut self) {
        let Some(previous_start) = previous_grapheme_start(&self.value, self.cursor) else {
            return;
        };
        self.value.replace_range(previous_start..self.cursor, "");
        self.cursor = previous_start;
    }

    fn handle_forward_delete(&mut self) {
        let Some(next_end) = next_grapheme_end(&self.value, self.cursor) else {
            return;
        };
        self.value.replace_range(self.cursor..next_end, "");
    }

    fn move_cursor_left(&mut self) {
        if let Some(previous_start) = previous_grapheme_start(&self.value, self.cursor) {
            self.cursor = previous_start;
        }
    }

    fn move_cursor_right(&mut self) {
        if let Some(next_end) = next_grapheme_end(&self.value, self.cursor) {
            self.cursor = next_end;
        }
    }

    fn move_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let mut graphemes = self.value[..self.cursor]
            .grapheme_indices(true)
            .map(|(index, grapheme)| (index, grapheme.to_owned()))
            .collect::<Vec<_>>();

        while graphemes
            .last()
            .is_some_and(|(_, grapheme)| grapheme_is_whitespace(grapheme))
        {
            if let Some((index, _)) = graphemes.pop() {
                self.cursor = index;
            }
        }

        let Some((_, last_grapheme)) = graphemes.last() else {
            return;
        };
        let consume_punctuation = grapheme_is_punctuation(last_grapheme);

        while let Some((index, grapheme)) = graphemes.last() {
            if grapheme_is_whitespace(grapheme) {
                break;
            }
            if consume_punctuation {
                if !grapheme_is_punctuation(grapheme) {
                    break;
                }
            } else if grapheme_is_punctuation(grapheme) {
                break;
            }
            self.cursor = *index;
            graphemes.pop();
        }
    }

    fn move_word_forward(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }

        let suffix = &self.value[self.cursor..];
        let graphemes = suffix.grapheme_indices(true).collect::<Vec<_>>();
        let mut index = 0;

        while index < graphemes.len() && grapheme_is_whitespace(graphemes[index].1) {
            self.cursor += graphemes[index].1.len();
            index += 1;
        }

        if index >= graphemes.len() {
            return;
        }

        let consume_punctuation = grapheme_is_punctuation(graphemes[index].1);
        while index < graphemes.len() {
            let grapheme = graphemes[index].1;
            if grapheme_is_whitespace(grapheme) {
                break;
            }
            if consume_punctuation {
                if !grapheme_is_punctuation(grapheme) {
                    break;
                }
            } else if grapheme_is_punctuation(grapheme) {
                break;
            }
            self.cursor += grapheme.len();
            index += 1;
        }
    }

    fn delete_word_backward(&mut self) {
        let old_cursor = self.cursor;
        self.move_word_backward();
        if self.cursor == old_cursor {
            return;
        }
        self.value.replace_range(self.cursor..old_cursor, "");
    }

    fn delete_word_forward(&mut self) {
        let start = self.cursor;
        self.move_word_forward();
        let end = self.cursor;
        self.cursor = start;
        if start == end {
            return;
        }
        self.value.replace_range(start..end, "");
    }

    fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.value.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    fn delete_to_line_end(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.value.replace_range(self.cursor.., "");
    }

    fn handle_paste(&mut self, pasted_text: &str) {
        let cleaned = pasted_text
            .replace("\r\n", "")
            .replace('\r', "")
            .replace('\n', "")
            .replace('\t', "    ");
        self.insert_text_at_cursor(&cleaned);
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Input {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let prompt_width = visible_width(PROMPT);
        let available_width = width.saturating_sub(prompt_width);
        if available_width == 0 {
            return vec![slice_by_column(PROMPT, 0, width, true)];
        }

        let total_width = visible_width(&self.value);
        let (visible_text, cursor_display) = if total_width < available_width {
            (self.value.clone(), self.cursor)
        } else {
            let scroll_width = if self.cursor == self.value.len() {
                available_width.saturating_sub(1)
            } else {
                available_width
            };
            let cursor_col = visible_width(&self.value[..self.cursor]);

            if scroll_width == 0 {
                (String::new(), 0)
            } else {
                let half_width = scroll_width / 2;
                let start_col = if cursor_col < half_width {
                    0
                } else if cursor_col > total_width.saturating_sub(half_width) {
                    total_width.saturating_sub(scroll_width)
                } else {
                    cursor_col.saturating_sub(half_width)
                };
                let visible_text = slice_by_column(&self.value, start_col, scroll_width, true);
                let before_cursor = slice_by_column(
                    &self.value,
                    start_col,
                    cursor_col.saturating_sub(start_col),
                    true,
                );
                (visible_text, before_cursor.len())
            }
        };

        let cursor_display = cursor_display.min(visible_text.len());
        let before_cursor = &visible_text[..cursor_display];
        let after_cursor = &visible_text[cursor_display..];
        let (at_cursor, after_cursor) = if let Some(grapheme) = after_cursor.graphemes(true).next()
        {
            (grapheme, &after_cursor[grapheme.len()..])
        } else {
            (" ", "")
        };

        let marker = if self.focused { CURSOR_MARKER } else { "" };
        let cursor_char = format!("{CURSOR_ON}{at_cursor}{CURSOR_OFF}");
        let text_with_cursor = format!("{before_cursor}{marker}{cursor_char}{after_cursor}");
        let visual_length = visible_width(&text_with_cursor);
        let padding = " ".repeat(available_width.saturating_sub(visual_length));

        vec![format!("{PROMPT}{text_with_cursor}{padding}")]
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        self.process_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}

fn previous_grapheme_start(text: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }
    text[..cursor]
        .grapheme_indices(true)
        .last()
        .map(|(index, _)| index)
}

fn next_grapheme_end(text: &str, cursor: usize) -> Option<usize> {
    if cursor >= text.len() {
        return None;
    }
    text[cursor..]
        .grapheme_indices(true)
        .next()
        .map(|(index, grapheme)| cursor + index + grapheme.len())
}

fn contains_control_characters(data: &str) -> bool {
    data.chars().any(|character| {
        let code = character as u32;
        code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
    })
}

fn grapheme_is_whitespace(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(is_whitespace_char)
}

fn grapheme_is_punctuation(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(is_punctuation_char)
}
