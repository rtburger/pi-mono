use crate::{Component, visible_width, wrap_text_with_ansi};

pub struct Text {
    text: String,
    padding_x: usize,
    padding_y: usize,
    custom_bg_fn: Option<Box<TextBgFn>>,
}

type TextBgFn = dyn Fn(&str) -> String + Send + Sync + 'static;

impl Text {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            custom_bg_fn: None,
        }
    }

    pub fn with_custom_bg_fn<F>(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        custom_bg_fn: F,
    ) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let mut text_component = Self::new(text, padding_x, padding_y);
        text_component.set_custom_bg_fn(custom_bg_fn);
        text_component
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn set_custom_bg_fn<F>(&mut self, custom_bg_fn: F)
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.custom_bg_fn = Some(Box::new(custom_bg_fn));
    }

    pub fn clear_custom_bg_fn(&mut self) {
        self.custom_bg_fn = None;
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::new("", 1, 1)
    }
}

impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        if self.text.trim().is_empty() {
            return Vec::new();
        }

        let normalized_text = self.text.replace('\t', "   ");
        let content_width = usize::max(1, width.saturating_sub(self.padding_x * 2));
        let wrapped_lines = wrap_text_with_ansi(&normalized_text, content_width);
        let left_margin = " ".repeat(self.padding_x);
        let right_margin = " ".repeat(self.padding_x);
        let mut content_lines = Vec::new();

        for line in wrapped_lines {
            let line_with_margins = format!("{left_margin}{line}{right_margin}");
            if let Some(custom_bg_fn) = &self.custom_bg_fn {
                content_lines.push(apply_background_to_line(
                    &line_with_margins,
                    width,
                    custom_bg_fn,
                ));
            } else {
                let visible_len = visible_width(&line_with_margins);
                let padding_needed = width.saturating_sub(visible_len);
                content_lines.push(line_with_margins + &" ".repeat(padding_needed));
            }
        }

        let empty_line = " ".repeat(width);
        let empty_lines = (0..self.padding_y)
            .map(|_| {
                if let Some(custom_bg_fn) = &self.custom_bg_fn {
                    apply_background_to_line(&empty_line, width, custom_bg_fn)
                } else {
                    empty_line.clone()
                }
            })
            .collect::<Vec<_>>();

        let mut result = Vec::new();
        result.extend(empty_lines.iter().cloned());
        result.extend(content_lines);
        result.extend(empty_lines);

        if result.is_empty() {
            vec![String::new()]
        } else {
            result
        }
    }

    fn invalidate(&mut self) {}
}

fn apply_background_to_line(line: &str, width: usize, bg_fn: &TextBgFn) -> String {
    let visible_len = visible_width(line);
    let padding_needed = width.saturating_sub(visible_len);
    let with_padding = format!("{line}{}", " ".repeat(padding_needed));
    bg_fn(&with_padding)
}
