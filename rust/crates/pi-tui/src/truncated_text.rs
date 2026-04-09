use crate::{Component, truncate_to_width, visible_width};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncatedText {
    text: String,
    padding_x: usize,
    padding_y: usize,
}

impl TruncatedText {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

impl Default for TruncatedText {
    fn default() -> Self {
        Self::new("", 0, 0)
    }
}

impl Component for TruncatedText {
    fn render(&self, width: usize) -> Vec<String> {
        let mut result = Vec::new();
        let empty_line = " ".repeat(width);

        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }

        let available_width = usize::max(1, width.saturating_sub(self.padding_x * 2));
        let single_line_text = self.text.split('\n').next().unwrap_or("");
        let display_text = truncate_to_width(single_line_text, available_width, "...", false);

        let left_padding = " ".repeat(self.padding_x);
        let right_padding = " ".repeat(self.padding_x);
        let mut line = format!("{left_padding}{display_text}{right_padding}");
        let padding_needed = width.saturating_sub(visible_width(&line));
        line.push_str(&" ".repeat(padding_needed));
        result.push(line);

        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }

        result
    }

    fn invalidate(&mut self) {}
}
