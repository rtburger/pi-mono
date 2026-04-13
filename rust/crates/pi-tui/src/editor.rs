use crate::{
    keybindings::KeybindingsManager,
    keys::{decode_kitty_printable, matches_key},
    kill_ring::KillRing,
    tui::{CURSOR_MARKER, Component},
    undo_stack::UndoStack,
    utils::{is_punctuation_char, is_whitespace_char, truncate_to_width, visible_width},
};
use std::{cell::Cell, collections::BTreeMap};
use unicode_segmentation::UnicodeSegmentation;

const BRACKETED_PASTE_START: &str = "\x1b[200~";
const BRACKETED_PASTE_END: &str = "\x1b[201~";
const CURSOR_ON: &str = "\x1b[7m";
const CURSOR_OFF: &str = "\x1b[0m";
const DEFAULT_VIEWPORT_HEIGHT: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    pub text: String,
    pub start_index: usize,
    pub end_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorCursor {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditorOptions {
    pub padding_x: usize,
}

type SubmitCallback = dyn FnMut(String) + Send + 'static;
type ChangeCallback = dyn FnMut(String) + Send + 'static;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorAction {
    Kill,
    Yank,
    TypeWord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JumpMode {
    Forward,
    Backward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSnapshot {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisualLine {
    logical_line: usize,
    start_col: usize,
    length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayoutLine {
    text: String,
    has_cursor: bool,
    cursor_pos: usize,
}

pub struct Editor {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    focused: bool,
    keybindings: KeybindingsManager,
    on_submit: Option<Box<SubmitCallback>>,
    on_change: Option<Box<ChangeCallback>>,
    padding_x: usize,
    last_width: Cell<usize>,
    viewport_size: Cell<Option<(usize, usize)>>,
    scroll_offset: Cell<usize>,
    paste_buffer: String,
    is_in_paste: bool,
    jump_mode: Option<JumpMode>,
    history: Vec<String>,
    history_index: Option<usize>,
    preferred_visual_col: Option<usize>,
    kill_ring: KillRing,
    undo_stack: UndoStack<EditorSnapshot>,
    last_action: Option<EditorAction>,
}

impl Editor {
    pub fn new() -> Self {
        Self::with_keybindings_and_options(
            KeybindingsManager::with_tui_defaults(BTreeMap::new()),
            EditorOptions::default(),
        )
    }

    pub fn with_options(options: EditorOptions) -> Self {
        Self::with_keybindings_and_options(
            KeybindingsManager::with_tui_defaults(BTreeMap::new()),
            options,
        )
    }

    pub fn with_keybindings(keybindings: KeybindingsManager) -> Self {
        Self::with_keybindings_and_options(keybindings, EditorOptions::default())
    }

    pub fn with_keybindings_and_options(
        keybindings: KeybindingsManager,
        options: EditorOptions,
    ) -> Self {
        Self {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            focused: false,
            keybindings,
            on_submit: None,
            on_change: None,
            padding_x: options.padding_x,
            last_width: Cell::new(80),
            viewport_size: Cell::new(None),
            scroll_offset: Cell::new(0),
            paste_buffer: String::new(),
            is_in_paste: false,
            jump_mode: None,
            history: Vec::new(),
            history_index: None,
            preferred_visual_col: None,
            kill_ring: KillRing::default(),
            undo_stack: UndoStack::default(),
            last_action: None,
        }
    }

    pub fn get_text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn get_expanded_text(&self) -> String {
        self.get_text()
    }

    pub fn set_text(&mut self, text: impl AsRef<str>) {
        self.history_index = None;
        self.preferred_visual_col = None;
        self.last_action = None;

        let normalized = normalize_text(text.as_ref());
        if self.get_text() != normalized {
            self.push_undo_snapshot();
        }

        self.set_text_internal(&normalized);
    }

    pub fn get_lines(&self) -> Vec<String> {
        self.lines.clone()
    }

    pub fn get_cursor(&self) -> EditorCursor {
        EditorCursor {
            line: self.cursor_line,
            col: self.cursor_col,
        }
    }

    pub fn insert_text_at_cursor(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }

        self.history_index = None;
        self.preferred_visual_col = None;
        self.last_action = None;
        self.push_undo_snapshot();
        self.insert_text_at_cursor_internal(text);
    }

    pub fn set_on_submit<F>(&mut self, callback: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_submit = Some(Box::new(callback));
    }

    pub fn clear_on_submit(&mut self) {
        self.on_submit = None;
    }

    pub fn set_on_change<F>(&mut self, callback: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_change = Some(Box::new(callback));
    }

    pub fn clear_on_change(&mut self) {
        self.on_change = None;
    }

    pub fn add_to_history(&mut self, text: impl AsRef<str>) {
        let trimmed = text.as_ref().trim();
        if trimmed.is_empty() {
            return;
        }
        if self
            .history
            .first()
            .is_some_and(|existing| existing == trimmed)
        {
            return;
        }
        self.history.insert(0, trimmed.to_owned());
        if self.history.len() > 100 {
            self.history.truncate(100);
        }
    }

    pub fn padding_x(&self) -> usize {
        self.padding_x
    }

    pub fn set_padding_x(&mut self, padding_x: usize) {
        self.padding_x = padding_x;
    }

    pub fn is_showing_autocomplete(&self) -> bool {
        false
    }

    pub fn handle_input(&mut self, data: &str) {
        self.process_input(data);
    }

    fn process_input(&mut self, data: &str) {
        let mut current = data.to_owned();

        if let Some(jump_mode) = self.jump_mode.take() {
            if self.matches_binding(&current, "tui.editor.jumpForward")
                || self.matches_binding(&current, "tui.editor.jumpBackward")
            {
                return;
            }

            if !contains_control_characters(&current) {
                self.jump_to_char(&current, jump_mode);
                return;
            }
        }

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

        if self.matches_binding(&current, "tui.editor.undo") {
            self.undo();
            return;
        }

        if self.matches_binding(&current, "tui.input.newLine") || current == "\n" {
            self.add_new_line();
            return;
        }

        if self.matches_binding(&current, "tui.input.submit") || current == "\r" {
            if self.current_line_before_cursor().ends_with('\\') {
                self.handle_backspace();
                self.add_new_line();
                return;
            }
            self.submit_value();
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

        if self.matches_binding(&current, "tui.editor.yank") {
            self.yank();
            return;
        }

        if self.matches_binding(&current, "tui.editor.yankPop") {
            self.yank_pop();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorLineStart") {
            self.history_index = None;
            self.move_to_line_start();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorLineEnd") {
            self.history_index = None;
            self.move_to_line_end();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorWordLeft") {
            self.history_index = None;
            self.move_word_backward();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorWordRight") {
            self.history_index = None;
            self.move_word_forward();
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorLeft") {
            self.history_index = None;
            self.move_cursor_horizontal(-1);
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorRight") {
            self.history_index = None;
            self.move_cursor_horizontal(1);
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorUp") {
            self.last_action = None;
            if self.is_editor_empty() {
                self.navigate_history(-1);
            } else if self.history_index.is_some() && self.is_on_first_visual_line() {
                self.navigate_history(-1);
            } else if self.is_on_first_visual_line() {
                self.move_to_line_start();
            } else {
                self.move_cursor_vertical(-1);
            }
            return;
        }

        if self.matches_binding(&current, "tui.editor.cursorDown") {
            self.last_action = None;
            if self.history_index.is_some() && self.is_on_last_visual_line() {
                self.navigate_history(1);
            } else if self.is_on_last_visual_line() {
                self.move_to_line_end();
            } else {
                self.move_cursor_vertical(1);
            }
            return;
        }

        if self.matches_binding(&current, "tui.editor.jumpForward") {
            self.jump_mode = Some(JumpMode::Forward);
            return;
        }

        if self.matches_binding(&current, "tui.editor.jumpBackward") {
            self.jump_mode = Some(JumpMode::Backward);
            return;
        }

        if let Some(printable) = decode_kitty_printable(&current) {
            self.insert_character(&printable);
            return;
        }

        if !contains_control_characters(&current) {
            self.insert_character(&current);
        }
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn is_editor_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines.first().is_some_and(String::is_empty)
    }

    fn current_line(&self) -> &str {
        self.lines
            .get(self.cursor_line)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn current_line_before_cursor(&self) -> &str {
        &self.current_line()[..self.cursor_col]
    }

    fn set_text_internal(&mut self, text: &str) {
        let normalized = normalize_text(text);
        let mut lines = normalized
            .split('\n')
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push(String::new());
        }
        self.lines = lines;
        self.cursor_line = self.lines.len().saturating_sub(1);
        self.cursor_col = self
            .lines
            .get(self.cursor_line)
            .map(String::len)
            .unwrap_or(0);
        self.scroll_offset.set(0);
        self.preferred_visual_col = None;
        self.emit_change();
    }

    fn insert_text_at_cursor_internal(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let normalized = normalize_text(text);
        let inserted_lines = normalized.split('\n').collect::<Vec<_>>();
        let current_line = self.current_line().to_owned();
        let before_cursor = current_line[..self.cursor_col].to_owned();
        let after_cursor = current_line[self.cursor_col..].to_owned();

        if inserted_lines.len() == 1 {
            self.lines[self.cursor_line] = format!("{before_cursor}{normalized}{after_cursor}");
            self.cursor_col += normalized.len();
        } else {
            let mut replacement = Vec::with_capacity(inserted_lines.len());
            replacement.push(format!("{before_cursor}{}", inserted_lines[0]));
            for line in inserted_lines.iter().skip(1).take(inserted_lines.len() - 2) {
                replacement.push((*line).to_owned());
            }
            replacement.push(format!(
                "{}{after_cursor}",
                inserted_lines[inserted_lines.len() - 1]
            ));
            self.lines
                .splice(self.cursor_line..=self.cursor_line, replacement);
            self.cursor_line += inserted_lines.len() - 1;
            self.cursor_col = inserted_lines[inserted_lines.len() - 1].len();
        }

        self.preferred_visual_col = None;
        self.emit_change();
    }

    fn insert_character(&mut self, character: &str) {
        self.history_index = None;

        if character.chars().next().is_some_and(is_whitespace_char)
            || !matches!(self.last_action, Some(EditorAction::TypeWord))
        {
            self.push_undo_snapshot();
        }

        self.last_action = Some(EditorAction::TypeWord);
        self.insert_text_at_cursor_internal(character);
    }

    fn handle_paste(&mut self, pasted_text: &str) {
        let filtered = normalize_text(pasted_text)
            .chars()
            .filter(|character| *character == '\n' || !character.is_control())
            .collect::<String>();
        self.history_index = None;
        self.last_action = None;
        self.push_undo_snapshot();
        self.insert_text_at_cursor_internal(&filtered);
    }

    fn add_new_line(&mut self) {
        self.push_undo_snapshot();

        let current_line = self.current_line().to_owned();
        let before = current_line[..self.cursor_col].to_owned();
        let after = current_line[self.cursor_col..].to_owned();
        self.lines[self.cursor_line] = before;
        self.lines.insert(self.cursor_line + 1, after);
        self.cursor_line += 1;
        self.cursor_col = 0;
        self.history_index = None;
        self.preferred_visual_col = None;
        self.last_action = None;
        self.emit_change();
    }

    fn submit_value(&mut self) {
        let result = self.get_expanded_text().trim().to_owned();
        self.lines = vec![String::new()];
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.history_index = None;
        self.preferred_visual_col = None;
        self.last_action = None;
        self.scroll_offset.set(0);
        self.undo_stack.clear();
        self.emit_change();
        if let Some(on_submit) = &mut self.on_submit {
            on_submit(result);
        }
    }

    fn handle_backspace(&mut self) {
        self.history_index = None;
        self.last_action = None;

        if self.cursor_col > 0 {
            self.push_undo_snapshot();
            let line = self.current_line().to_owned();
            let previous_start = previous_grapheme_start(&line, self.cursor_col).unwrap_or(0);
            self.lines[self.cursor_line].replace_range(previous_start..self.cursor_col, "");
            self.cursor_col = previous_start;
        } else if self.cursor_line > 0 {
            self.push_undo_snapshot();
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            let previous_len = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&current);
            self.cursor_col = previous_len;
        } else {
            return;
        }

        self.preferred_visual_col = None;
        self.emit_change();
    }

    fn handle_forward_delete(&mut self) {
        self.history_index = None;
        self.last_action = None;

        let current_len = self.current_line().len();
        if self.cursor_col < current_len {
            self.push_undo_snapshot();
            let line = self.current_line().to_owned();
            let next_end = next_grapheme_end(&line, self.cursor_col).unwrap_or(self.cursor_col);
            self.lines[self.cursor_line].replace_range(self.cursor_col..next_end, "");
        } else if self.cursor_line + 1 < self.lines.len() {
            self.push_undo_snapshot();
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
        } else {
            return;
        }

        self.preferred_visual_col = None;
        self.emit_change();
    }

    fn move_to_line_start(&mut self) {
        self.cursor_col = 0;
        self.preferred_visual_col = None;
        self.last_action = None;
    }

    fn move_to_line_end(&mut self) {
        self.cursor_col = self.current_line().len();
        self.preferred_visual_col = None;
        self.last_action = None;
    }

    fn move_cursor_horizontal(&mut self, delta: isize) {
        self.preferred_visual_col = None;
        self.last_action = None;
        if delta < 0 {
            if self.cursor_col > 0 {
                self.cursor_col =
                    previous_grapheme_start(self.current_line(), self.cursor_col).unwrap_or(0);
            } else if self.cursor_line > 0 {
                self.cursor_line -= 1;
                self.cursor_col = self.current_line().len();
            }
            return;
        }

        if self.cursor_col < self.current_line().len() {
            self.cursor_col = next_grapheme_end(self.current_line(), self.cursor_col)
                .unwrap_or(self.current_line().len());
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    fn jump_to_char(&mut self, character: &str, direction: JumpMode) {
        self.last_action = None;

        match direction {
            JumpMode::Forward => {
                for line_index in self.cursor_line..self.lines.len() {
                    let line = &self.lines[line_index];
                    let search_start = if line_index == self.cursor_line {
                        next_grapheme_end(line, self.cursor_col).unwrap_or(line.len())
                    } else {
                        0
                    };

                    if search_start > line.len() {
                        continue;
                    }

                    if let Some(offset) = line[search_start..].find(character) {
                        self.cursor_line = line_index;
                        self.cursor_col = search_start + offset;
                        self.preferred_visual_col = None;
                        return;
                    }
                }
            }
            JumpMode::Backward => {
                for line_index in (0..=self.cursor_line).rev() {
                    let line = &self.lines[line_index];
                    let search_end = if line_index == self.cursor_line {
                        self.cursor_col.min(line.len())
                    } else {
                        line.len()
                    };

                    if search_end == 0 {
                        continue;
                    }

                    if let Some(index) = line[..search_end].rfind(character) {
                        self.cursor_line = line_index;
                        self.cursor_col = index;
                        self.preferred_visual_col = None;
                        return;
                    }
                }
            }
        }
    }

    fn move_word_backward(&mut self) {
        self.last_action = None;
        if self.cursor_col == 0 {
            if self.cursor_line > 0 {
                self.cursor_line -= 1;
                self.cursor_col = self.current_line().len();
            }
            self.preferred_visual_col = None;
            return;
        }

        let mut graphemes = self
            .current_line_before_cursor()
            .grapheme_indices(true)
            .map(|(index, grapheme)| (index, grapheme.to_owned()))
            .collect::<Vec<_>>();

        while graphemes
            .last()
            .is_some_and(|(_, grapheme)| grapheme_is_whitespace(grapheme))
        {
            if let Some((index, _)) = graphemes.pop() {
                self.cursor_col = index;
            }
        }

        let Some((_, last_grapheme)) = graphemes.last() else {
            self.preferred_visual_col = None;
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
            self.cursor_col = *index;
            graphemes.pop();
        }

        self.preferred_visual_col = None;
    }

    fn move_word_forward(&mut self) {
        self.last_action = None;
        if self.cursor_col >= self.current_line().len() {
            if self.cursor_line + 1 < self.lines.len() {
                self.cursor_line += 1;
                self.cursor_col = 0;
            }
            self.preferred_visual_col = None;
            return;
        }

        let current_line = self.current_line().to_owned();
        let suffix = &current_line[self.cursor_col..];
        let graphemes = suffix.grapheme_indices(true).collect::<Vec<_>>();
        let mut index = 0;
        let mut new_cursor_col = self.cursor_col;

        while index < graphemes.len() && grapheme_is_whitespace(graphemes[index].1) {
            new_cursor_col += graphemes[index].1.len();
            index += 1;
        }

        if index >= graphemes.len() {
            self.cursor_col = new_cursor_col;
            self.preferred_visual_col = None;
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
            new_cursor_col += grapheme.len();
            index += 1;
        }

        self.cursor_col = new_cursor_col;
        self.preferred_visual_col = None;
    }

    fn delete_word_backward(&mut self) {
        self.history_index = None;

        if self.cursor_col == 0 {
            if self.cursor_line > 0 {
                self.push_undo_snapshot();
                let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
                self.kill_ring.push("\n", true, accumulate);
                let current = self.lines.remove(self.cursor_line);
                self.cursor_line -= 1;
                let previous_len = self.lines[self.cursor_line].len();
                self.lines[self.cursor_line].push_str(&current);
                self.cursor_col = previous_len;
                self.preferred_visual_col = None;
                self.last_action = Some(EditorAction::Kill);
                self.emit_change();
            }
            return;
        }

        let current_line = self.current_line().to_owned();
        let old_cursor = self.cursor_col;
        let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
        self.push_undo_snapshot();
        self.move_word_backward();
        if self.cursor_col == old_cursor {
            return;
        }
        let delete_from = self.cursor_col;
        let deleted_text = current_line[delete_from..old_cursor].to_owned();
        self.kill_ring.push(deleted_text, true, accumulate);
        self.lines[self.cursor_line].replace_range(delete_from..old_cursor, "");
        self.preferred_visual_col = None;
        self.last_action = Some(EditorAction::Kill);
        self.emit_change();
    }

    fn delete_word_forward(&mut self) {
        self.history_index = None;

        let start_line = self.cursor_line;
        let start_col = self.cursor_col;
        let current_len = self.current_line().len();
        if start_col >= current_len {
            if start_line + 1 < self.lines.len() {
                self.push_undo_snapshot();
                let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
                self.kill_ring.push("\n", false, accumulate);
                let next = self.lines.remove(self.cursor_line + 1);
                self.lines[self.cursor_line].push_str(&next);
                self.preferred_visual_col = None;
                self.last_action = Some(EditorAction::Kill);
                self.emit_change();
            }
            return;
        }

        let current_line = self.current_line().to_owned();
        let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
        self.push_undo_snapshot();
        self.move_word_forward();
        let end = self.cursor_col;
        self.cursor_line = start_line;
        self.cursor_col = start_col;
        if start_col == end {
            return;
        }
        let deleted_text = current_line[start_col..end].to_owned();
        self.kill_ring.push(deleted_text, false, accumulate);
        self.lines[self.cursor_line].replace_range(start_col..end, "");
        self.preferred_visual_col = None;
        self.last_action = Some(EditorAction::Kill);
        self.emit_change();
    }

    fn delete_to_line_start(&mut self) {
        self.history_index = None;

        if self.cursor_col > 0 {
            self.push_undo_snapshot();
            let deleted_text = self.current_line()[..self.cursor_col].to_owned();
            let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
            self.kill_ring.push(deleted_text, true, accumulate);
            self.lines[self.cursor_line].replace_range(..self.cursor_col, "");
            self.cursor_col = 0;
            self.preferred_visual_col = None;
            self.last_action = Some(EditorAction::Kill);
            self.emit_change();
            return;
        }
        if self.cursor_line > 0 {
            self.push_undo_snapshot();
            let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
            self.kill_ring.push("\n", true, accumulate);
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            let previous_len = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&current);
            self.cursor_col = previous_len;
            self.preferred_visual_col = None;
            self.last_action = Some(EditorAction::Kill);
            self.emit_change();
        }
    }

    fn delete_to_line_end(&mut self) {
        self.history_index = None;

        if self.cursor_col < self.current_line().len() {
            self.push_undo_snapshot();
            let deleted_text = self.current_line()[self.cursor_col..].to_owned();
            let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
            self.kill_ring.push(deleted_text, false, accumulate);
            self.lines[self.cursor_line].replace_range(self.cursor_col.., "");
            self.preferred_visual_col = None;
            self.last_action = Some(EditorAction::Kill);
            self.emit_change();
            return;
        }
        if self.cursor_line + 1 < self.lines.len() {
            self.push_undo_snapshot();
            let accumulate = matches!(self.last_action, Some(EditorAction::Kill));
            self.kill_ring.push("\n", false, accumulate);
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.preferred_visual_col = None;
            self.last_action = Some(EditorAction::Kill);
            self.emit_change();
        }
    }

    fn navigate_history(&mut self, direction: isize) {
        self.last_action = None;
        if self.history.is_empty() {
            return;
        }

        let next_index = match (self.history_index, direction) {
            (None, -1) => Some(0),
            (None, 1) => None,
            (Some(index), -1) => Some((index + 1).min(self.history.len() - 1)),
            (Some(0), 1) => None,
            (Some(index), 1) => Some(index - 1),
            (state, _) => state,
        };

        if next_index == self.history_index {
            return;
        }

        if self.history_index.is_none() && next_index.is_some() {
            self.push_undo_snapshot();
        }

        self.history_index = next_index;
        match next_index {
            Some(index) => {
                let text = self.history[index].clone();
                self.set_text_internal(&text);
            }
            None => self.set_text_internal(""),
        }
    }

    fn is_on_first_visual_line(&self) -> bool {
        self.find_current_visual_line(&self.build_visual_line_map()) == 0
    }

    fn is_on_last_visual_line(&self) -> bool {
        let visual_lines = self.build_visual_line_map();
        self.find_current_visual_line(&visual_lines) + 1 == visual_lines.len()
    }

    fn move_cursor_vertical(&mut self, delta: isize) {
        self.last_action = None;
        let visual_lines = self.build_visual_line_map();
        let current_visual_line = self.find_current_visual_line(&visual_lines);
        let target_visual_line = current_visual_line as isize + delta;
        if target_visual_line < 0 || target_visual_line as usize >= visual_lines.len() {
            return;
        }

        let current = visual_lines[current_visual_line];
        let desired_visual_col = self
            .preferred_visual_col
            .unwrap_or(self.cursor_col.saturating_sub(current.start_col));
        let target = visual_lines[target_visual_line as usize];
        self.cursor_line = target.logical_line;
        let logical_line_len = self.lines[target.logical_line].len();
        self.cursor_col =
            (target.start_col + desired_visual_col.min(target.length)).min(logical_line_len);
        self.preferred_visual_col = Some(desired_visual_col);
    }

    fn yank(&mut self) {
        let Some(text) = self.kill_ring.peek().map(str::to_owned) else {
            return;
        };

        self.push_undo_snapshot();
        self.insert_yanked_text(&text);
        self.last_action = Some(EditorAction::Yank);
    }

    fn yank_pop(&mut self) {
        if !matches!(self.last_action, Some(EditorAction::Yank)) || self.kill_ring.len() <= 1 {
            return;
        }

        self.push_undo_snapshot();
        self.delete_yanked_text();
        self.kill_ring.rotate();
        if let Some(text) = self.kill_ring.peek().map(str::to_owned) {
            self.insert_yanked_text(&text);
            self.last_action = Some(EditorAction::Yank);
        }
    }

    fn insert_yanked_text(&mut self, text: &str) {
        self.history_index = None;
        self.insert_text_at_cursor_internal(text);
    }

    fn delete_yanked_text(&mut self) {
        let Some(yanked_text) = self.kill_ring.peek() else {
            return;
        };

        let yank_lines = yanked_text.split('\n').collect::<Vec<_>>();
        if yank_lines.len() == 1 {
            let delete_len = yanked_text.len();
            let current_line = self.current_line().to_owned();
            let before = current_line[..self.cursor_col.saturating_sub(delete_len)].to_owned();
            let after = current_line[self.cursor_col..].to_owned();
            self.lines[self.cursor_line] = format!("{before}{after}");
            self.cursor_col = self.cursor_col.saturating_sub(delete_len);
            self.preferred_visual_col = None;
            self.emit_change();
            return;
        }

        let start_line = self
            .cursor_line
            .saturating_sub(yank_lines.len().saturating_sub(1));
        let Some(start_line_text) = self.lines.get(start_line).cloned() else {
            return;
        };
        let start_col = start_line_text.len().saturating_sub(yank_lines[0].len());
        let after_cursor = self.current_line()[self.cursor_col..].to_owned();
        let before_yank = start_line_text[..start_col].to_owned();
        self.lines.splice(
            start_line..=self.cursor_line,
            [format!("{before_yank}{after_cursor}")],
        );
        self.cursor_line = start_line;
        self.cursor_col = start_col;
        self.preferred_visual_col = None;
        self.emit_change();
    }

    fn build_visual_line_map(&self) -> Vec<VisualLine> {
        let width = self.last_width.get().max(1);
        let mut visual_lines = Vec::new();

        for (index, line) in self.lines.iter().enumerate() {
            if line.is_empty() {
                visual_lines.push(VisualLine {
                    logical_line: index,
                    start_col: 0,
                    length: 0,
                });
                continue;
            }

            if visible_width(line) <= width {
                visual_lines.push(VisualLine {
                    logical_line: index,
                    start_col: 0,
                    length: line.len(),
                });
                continue;
            }

            for chunk in word_wrap_line(line, width) {
                visual_lines.push(VisualLine {
                    logical_line: index,
                    start_col: chunk.start_index,
                    length: chunk.end_index.saturating_sub(chunk.start_index),
                });
            }
        }

        if visual_lines.is_empty() {
            visual_lines.push(VisualLine {
                logical_line: 0,
                start_col: 0,
                length: 0,
            });
        }

        visual_lines
    }

    fn find_current_visual_line(&self, visual_lines: &[VisualLine]) -> usize {
        for (index, visual_line) in visual_lines.iter().enumerate() {
            if visual_line.logical_line != self.cursor_line {
                continue;
            }
            let col_in_segment = self.cursor_col.saturating_sub(visual_line.start_col);
            let is_last_segment_of_line = index + 1 == visual_lines.len()
                || visual_lines[index + 1].logical_line != visual_line.logical_line;
            if col_in_segment < visual_line.length
                || (is_last_segment_of_line && col_in_segment <= visual_line.length)
            {
                return index;
            }
        }

        visual_lines.len().saturating_sub(1)
    }

    fn layout_text(&self, content_width: usize) -> Vec<LayoutLine> {
        let mut layout_lines = Vec::new();

        if self.lines.is_empty() || (self.lines.len() == 1 && self.lines[0].is_empty()) {
            layout_lines.push(LayoutLine {
                text: String::new(),
                has_cursor: true,
                cursor_pos: 0,
            });
            return layout_lines;
        }

        for (line_index, line) in self.lines.iter().enumerate() {
            let is_current_line = line_index == self.cursor_line;
            if line.is_empty() {
                layout_lines.push(LayoutLine {
                    text: String::new(),
                    has_cursor: is_current_line,
                    cursor_pos: 0,
                });
                continue;
            }

            if visible_width(line) <= content_width {
                layout_lines.push(LayoutLine {
                    text: line.clone(),
                    has_cursor: is_current_line,
                    cursor_pos: if is_current_line { self.cursor_col } else { 0 },
                });
                continue;
            }

            let chunks = word_wrap_line(line, content_width);
            for (chunk_index, chunk) in chunks.iter().enumerate() {
                let is_last_chunk = chunk_index + 1 == chunks.len();
                let mut has_cursor = false;
                let mut cursor_pos = 0;

                if is_current_line {
                    if is_last_chunk {
                        has_cursor = self.cursor_col >= chunk.start_index;
                        cursor_pos = self.cursor_col.saturating_sub(chunk.start_index);
                    } else if self.cursor_col >= chunk.start_index
                        && self.cursor_col < chunk.end_index
                    {
                        has_cursor = true;
                        cursor_pos = self.cursor_col - chunk.start_index;
                    }
                }

                layout_lines.push(LayoutLine {
                    text: chunk.text.clone(),
                    has_cursor,
                    cursor_pos: cursor_pos.min(chunk.text.len()),
                });
            }
        }

        if layout_lines.is_empty() {
            layout_lines.push(LayoutLine {
                text: String::new(),
                has_cursor: true,
                cursor_pos: 0,
            });
        }

        layout_lines
    }

    fn max_visible_lines(&self) -> usize {
        let height = self
            .viewport_size
            .get()
            .map(|(_, height)| height)
            .unwrap_or(DEFAULT_VIEWPORT_HEIGHT);
        height.saturating_mul(3).checked_div(10).unwrap_or(0).max(5)
    }

    fn push_undo_snapshot(&mut self) {
        self.undo_stack.push(&EditorSnapshot {
            lines: self.lines.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
        });
    }

    fn undo(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            return;
        };

        self.lines = snapshot.lines;
        self.cursor_line = snapshot.cursor_line;
        self.cursor_col = snapshot.cursor_col;
        self.history_index = None;
        self.last_action = None;
        self.preferred_visual_col = None;
        self.emit_change();
    }

    fn emit_change(&mut self) {
        let text = self.get_text();
        if let Some(on_change) = &mut self.on_change {
            on_change(text);
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Editor {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let max_padding = (width.saturating_sub(1)) / 2;
        let padding_x = self.padding_x.min(max_padding);
        let content_width = width.saturating_sub(padding_x * 2).max(1);
        let layout_width = if padding_x == 0 {
            content_width.saturating_sub(1).max(1)
        } else {
            content_width
        };

        self.last_width.set(layout_width);

        let layout_lines = self.layout_text(layout_width);
        let cursor_line_index = layout_lines
            .iter()
            .position(|line| line.has_cursor)
            .unwrap_or(0);
        let max_visible_lines = self.max_visible_lines();
        let mut scroll_offset = self.scroll_offset.get();
        if cursor_line_index < scroll_offset {
            scroll_offset = cursor_line_index;
        } else if cursor_line_index >= scroll_offset + max_visible_lines {
            scroll_offset = cursor_line_index + 1 - max_visible_lines;
        }
        let max_scroll_offset = layout_lines.len().saturating_sub(max_visible_lines);
        scroll_offset = scroll_offset.min(max_scroll_offset);
        self.scroll_offset.set(scroll_offset);

        let visible_lines = layout_lines
            .iter()
            .skip(scroll_offset)
            .take(max_visible_lines)
            .collect::<Vec<_>>();

        let mut result = Vec::new();
        result.push("─".repeat(width));

        for layout_line in visible_lines {
            let mut display_text = layout_line.text.clone();
            if layout_line.has_cursor {
                let cursor_pos = layout_line.cursor_pos.min(display_text.len());
                let before_cursor = &display_text[..cursor_pos];
                let after_cursor = &display_text[cursor_pos..];
                let marker = if self.focused { CURSOR_MARKER } else { "" };
                if let Some(grapheme) = after_cursor.graphemes(true).next() {
                    let rest = &after_cursor[grapheme.len()..];
                    display_text =
                        format!("{before_cursor}{marker}{CURSOR_ON}{grapheme}{CURSOR_OFF}{rest}");
                } else {
                    display_text = format!("{before_cursor}{marker}{CURSOR_ON} {CURSOR_OFF}");
                }
            }

            let mut line = format!("{}{}", " ".repeat(padding_x), display_text);
            let line_width = visible_width(&line);
            if line_width < width {
                line.push_str(&" ".repeat(width - line_width));
            } else if line_width > width {
                line = truncate_to_width(&line, width, "", false);
                let truncated_width = visible_width(&line);
                if truncated_width < width {
                    line.push_str(&" ".repeat(width - truncated_width));
                }
            }
            result.push(line);
        }

        result.push("─".repeat(width));
        result
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        self.process_input(data);
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}

pub fn word_wrap_line(line: &str, max_width: usize) -> Vec<TextChunk> {
    if line.is_empty() || max_width == 0 {
        return vec![TextChunk {
            text: String::new(),
            start_index: 0,
            end_index: 0,
        }];
    }

    if visible_width(line) <= max_width {
        return vec![TextChunk {
            text: line.to_owned(),
            start_index: 0,
            end_index: line.len(),
        }];
    }

    let segments = line
        .grapheme_indices(true)
        .map(|(index, grapheme)| (index, grapheme.to_owned()))
        .collect::<Vec<_>>();

    let mut chunks = Vec::new();
    let mut current_width = 0usize;
    let mut chunk_start = 0usize;
    let mut wrap_opp_index = None::<usize>;
    let mut wrap_opp_width = 0usize;

    for index in 0..segments.len() {
        let (char_index, grapheme) = &segments[index];
        let grapheme_width = visible_width(grapheme);
        let is_whitespace = grapheme_is_whitespace(grapheme);

        if current_width + grapheme_width > max_width {
            if let Some(wrap_index) = wrap_opp_index {
                if current_width.saturating_sub(wrap_opp_width) + grapheme_width <= max_width {
                    chunks.push(TextChunk {
                        text: line[chunk_start..wrap_index].to_owned(),
                        start_index: chunk_start,
                        end_index: wrap_index,
                    });
                    chunk_start = wrap_index;
                    current_width = current_width.saturating_sub(wrap_opp_width);
                } else if chunk_start < *char_index {
                    chunks.push(TextChunk {
                        text: line[chunk_start..*char_index].to_owned(),
                        start_index: chunk_start,
                        end_index: *char_index,
                    });
                    chunk_start = *char_index;
                    current_width = 0;
                }
            } else if chunk_start < *char_index {
                chunks.push(TextChunk {
                    text: line[chunk_start..*char_index].to_owned(),
                    start_index: chunk_start,
                    end_index: *char_index,
                });
                chunk_start = *char_index;
                current_width = 0;
            }
            wrap_opp_index = None;
        }

        current_width += grapheme_width;

        if is_whitespace
            && segments
                .get(index + 1)
                .is_some_and(|(_, next)| !grapheme_is_whitespace(next))
        {
            wrap_opp_index = segments.get(index + 1).map(|(next_index, _)| *next_index);
            wrap_opp_width = current_width;
        }
    }

    chunks.push(TextChunk {
        text: line[chunk_start..].to_owned(),
        start_index: chunk_start,
        end_index: line.len(),
    });

    chunks
}

fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
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
