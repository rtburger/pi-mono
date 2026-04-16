use crate::{Component, visible_width, wrap_text_with_ansi};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use std::{cell::RefCell, sync::Arc};

type TextStyleFn = dyn Fn(&str) -> String + Send + Sync + 'static;
type HighlightCodeFn = dyn Fn(&str, Option<&str>) -> Vec<String> + Send + Sync + 'static;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderCache {
    text: String,
    width: usize,
    lines: Vec<String>,
}

struct InlineStyleContext<'a> {
    apply_text: &'a dyn Fn(&str) -> String,
    style_prefix: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
enum Inline {
    Text(String),
    Html(String),
    Code(String),
    Break,
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link {
        text: Vec<Inline>,
        url: String,
        link_type: LinkType,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct ListItem {
    blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
struct ListBlock {
    ordered: bool,
    start: u64,
    items: Vec<ListItem>,
}

#[derive(Debug, Clone, PartialEq)]
struct TableBlock {
    header: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
}

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Heading {
        level: HeadingLevel,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    CodeBlock {
        lang: Option<String>,
        text: String,
    },
    List(ListBlock),
    BlockQuote(Vec<Block>),
    Table(TableBlock),
    Hr,
    Html(String),
}

#[derive(Clone)]
pub struct DefaultTextStyle {
    color: Option<Arc<TextStyleFn>>,
    bg_color: Option<Arc<TextStyleFn>>,
    bold: bool,
    italic: bool,
    strikethrough: bool,
    underline: bool,
}

impl DefaultTextStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_color<F>(mut self, color: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.color = Some(Arc::new(color));
        self
    }

    pub fn with_bg_color<F>(mut self, bg_color: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.bg_color = Some(Arc::new(bg_color));
        self
    }

    pub fn with_bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    pub fn with_strikethrough(mut self, strikethrough: bool) -> Self {
        self.strikethrough = strikethrough;
        self
    }

    pub fn with_underline(mut self, underline: bool) -> Self {
        self.underline = underline;
        self
    }

    fn bg_color(&self) -> Option<&TextStyleFn> {
        self.bg_color.as_deref()
    }
}

impl Default for DefaultTextStyle {
    fn default() -> Self {
        Self {
            color: None,
            bg_color: None,
            bold: false,
            italic: false,
            strikethrough: false,
            underline: false,
        }
    }
}

#[derive(Clone)]
pub struct MarkdownTheme {
    heading: Arc<TextStyleFn>,
    link: Arc<TextStyleFn>,
    link_url: Arc<TextStyleFn>,
    code: Arc<TextStyleFn>,
    code_block: Arc<TextStyleFn>,
    code_block_border: Arc<TextStyleFn>,
    quote: Arc<TextStyleFn>,
    quote_border: Arc<TextStyleFn>,
    hr: Arc<TextStyleFn>,
    list_bullet: Arc<TextStyleFn>,
    bold: Arc<TextStyleFn>,
    italic: Arc<TextStyleFn>,
    strikethrough: Arc<TextStyleFn>,
    underline: Arc<TextStyleFn>,
    highlight_code: Option<Arc<HighlightCodeFn>>,
    code_block_indent: String,
}

impl MarkdownTheme {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_heading<F>(mut self, heading: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.heading = Arc::new(heading);
        self
    }

    pub fn with_link<F>(mut self, link: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.link = Arc::new(link);
        self
    }

    pub fn with_link_url<F>(mut self, link_url: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.link_url = Arc::new(link_url);
        self
    }

    pub fn with_code<F>(mut self, code: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.code = Arc::new(code);
        self
    }

    pub fn with_code_block<F>(mut self, code_block: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.code_block = Arc::new(code_block);
        self
    }

    pub fn with_code_block_border<F>(mut self, code_block_border: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.code_block_border = Arc::new(code_block_border);
        self
    }

    pub fn with_quote<F>(mut self, quote: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.quote = Arc::new(quote);
        self
    }

    pub fn with_quote_border<F>(mut self, quote_border: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.quote_border = Arc::new(quote_border);
        self
    }

    pub fn with_hr<F>(mut self, hr: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.hr = Arc::new(hr);
        self
    }

    pub fn with_list_bullet<F>(mut self, list_bullet: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.list_bullet = Arc::new(list_bullet);
        self
    }

    pub fn with_bold<F>(mut self, bold: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.bold = Arc::new(bold);
        self
    }

    pub fn with_italic<F>(mut self, italic: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.italic = Arc::new(italic);
        self
    }

    pub fn with_strikethrough<F>(mut self, strikethrough: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.strikethrough = Arc::new(strikethrough);
        self
    }

    pub fn with_underline<F>(mut self, underline: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.underline = Arc::new(underline);
        self
    }

    pub fn with_highlight_code<F>(mut self, highlight_code: F) -> Self
    where
        F: Fn(&str, Option<&str>) -> Vec<String> + Send + Sync + 'static,
    {
        self.highlight_code = Some(Arc::new(highlight_code));
        self
    }

    pub fn with_code_block_indent(mut self, code_block_indent: impl Into<String>) -> Self {
        self.code_block_indent = code_block_indent.into();
        self
    }

    fn heading(&self, text: &str) -> String {
        (self.heading)(text)
    }

    fn link(&self, text: &str) -> String {
        (self.link)(text)
    }

    fn link_url(&self, text: &str) -> String {
        (self.link_url)(text)
    }

    fn code(&self, text: &str) -> String {
        (self.code)(text)
    }

    fn code_block(&self, text: &str) -> String {
        (self.code_block)(text)
    }

    fn code_block_border(&self, text: &str) -> String {
        (self.code_block_border)(text)
    }

    fn quote(&self, text: &str) -> String {
        (self.quote)(text)
    }

    fn quote_border(&self, text: &str) -> String {
        (self.quote_border)(text)
    }

    fn hr(&self, text: &str) -> String {
        (self.hr)(text)
    }

    fn list_bullet(&self, text: &str) -> String {
        (self.list_bullet)(text)
    }

    fn bold(&self, text: &str) -> String {
        (self.bold)(text)
    }

    fn italic(&self, text: &str) -> String {
        (self.italic)(text)
    }

    fn strikethrough(&self, text: &str) -> String {
        (self.strikethrough)(text)
    }

    fn underline(&self, text: &str) -> String {
        (self.underline)(text)
    }

    fn highlight_code(&self, code: &str, lang: Option<&str>) -> Option<Vec<String>> {
        self.highlight_code
            .as_ref()
            .map(|highlight| highlight(code, lang))
    }

    fn code_block_indent(&self) -> &str {
        &self.code_block_indent
    }
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        Self {
            heading: Arc::new(str::to_owned),
            link: Arc::new(str::to_owned),
            link_url: Arc::new(str::to_owned),
            code: Arc::new(str::to_owned),
            code_block: Arc::new(str::to_owned),
            code_block_border: Arc::new(str::to_owned),
            quote: Arc::new(str::to_owned),
            quote_border: Arc::new(str::to_owned),
            hr: Arc::new(str::to_owned),
            list_bullet: Arc::new(str::to_owned),
            bold: Arc::new(str::to_owned),
            italic: Arc::new(str::to_owned),
            strikethrough: Arc::new(str::to_owned),
            underline: Arc::new(str::to_owned),
            highlight_code: None,
            code_block_indent: "  ".to_owned(),
        }
    }
}

pub struct Markdown {
    text: String,
    padding_x: usize,
    padding_y: usize,
    theme: MarkdownTheme,
    default_text_style: Option<DefaultTextStyle>,
    cache: RefCell<Option<RenderCache>>,
}

impl Markdown {
    pub fn new(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        theme: MarkdownTheme,
    ) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            theme,
            default_text_style: None,
            cache: RefCell::new(None),
        }
    }

    pub fn with_default_text_style(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        theme: MarkdownTheme,
        default_text_style: DefaultTextStyle,
    ) -> Self {
        let mut markdown = Self::new(text, padding_x, padding_y, theme);
        markdown.default_text_style = Some(default_text_style);
        markdown
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.clear_cache();
    }

    fn clear_cache(&self) {
        *self.cache.borrow_mut() = None;
    }

    fn parse_blocks(&self) -> Vec<Block> {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_GFM);

        let parser = Parser::new_ext(&self.text, options);
        let mut iter = parser.peekable();
        parse_blocks_from_events(&mut iter, None)
    }

    fn apply_default_style(&self, text: &str) -> String {
        let Some(default_text_style) = &self.default_text_style else {
            return text.to_owned();
        };

        let mut styled = text.to_owned();
        if let Some(color) = &default_text_style.color {
            styled = color(&styled);
        }
        if default_text_style.bold {
            styled = self.theme.bold(&styled);
        }
        if default_text_style.italic {
            styled = self.theme.italic(&styled);
        }
        if default_text_style.strikethrough {
            styled = self.theme.strikethrough(&styled);
        }
        if default_text_style.underline {
            styled = self.theme.underline(&styled);
        }
        styled
    }

    fn get_style_prefix(&self, style_fn: &dyn Fn(&str) -> String) -> String {
        let sentinel = "\0";
        let styled = style_fn(sentinel);
        styled
            .find(sentinel)
            .map(|index| styled[..index].to_owned())
            .unwrap_or_default()
    }

    fn get_default_style_prefix(&self) -> String {
        let Some(default_text_style) = &self.default_text_style else {
            return String::new();
        };

        let sentinel = "\0";
        let mut styled = sentinel.to_owned();
        if let Some(color) = &default_text_style.color {
            styled = color(&styled);
        }
        if default_text_style.bold {
            styled = self.theme.bold(&styled);
        }
        if default_text_style.italic {
            styled = self.theme.italic(&styled);
        }
        if default_text_style.strikethrough {
            styled = self.theme.strikethrough(&styled);
        }
        if default_text_style.underline {
            styled = self.theme.underline(&styled);
        }

        styled
            .find(sentinel)
            .map(|index| styled[..index].to_owned())
            .unwrap_or_default()
    }

    fn render_inlines(&self, inlines: &[Inline], style_context: &InlineStyleContext<'_>) -> String {
        let mut result = String::new();

        for inline in inlines {
            match inline {
                Inline::Text(text) | Inline::Html(text) => {
                    result.push_str(&apply_text_with_newlines(text, style_context.apply_text));
                }
                Inline::Code(code) => {
                    result.push_str(&self.theme.code(code));
                    result.push_str(style_context.style_prefix);
                }
                Inline::Break => result.push('\n'),
                Inline::Strong(children) => {
                    let inner = self.render_inlines(children, style_context);
                    result.push_str(&self.theme.bold(&inner));
                    result.push_str(style_context.style_prefix);
                }
                Inline::Emphasis(children) => {
                    let inner = self.render_inlines(children, style_context);
                    result.push_str(&self.theme.italic(&inner));
                    result.push_str(style_context.style_prefix);
                }
                Inline::Strikethrough(children) => {
                    let inner = self.render_inlines(children, style_context);
                    result.push_str(&self.theme.strikethrough(&inner));
                    result.push_str(style_context.style_prefix);
                }
                Inline::Link {
                    text,
                    url,
                    link_type,
                } => {
                    let rendered_text = self.render_inlines(text, style_context);
                    let plain_text = inline_plain_text(text);
                    let href_for_comparison =
                        if matches!(link_type, LinkType::Email) || url.starts_with("mailto:") {
                            url.strip_prefix("mailto:").unwrap_or(url)
                        } else {
                            url.as_str()
                        };
                    let styled_link = self.theme.link(&self.theme.underline(&rendered_text));
                    if plain_text == *url || plain_text == href_for_comparison {
                        result.push_str(&styled_link);
                    } else {
                        result.push_str(&styled_link);
                        result.push_str(&self.theme.link_url(&format!(" ({url})")));
                    }
                    result.push_str(style_context.style_prefix);
                }
            }
        }

        while !style_context.style_prefix.is_empty() && result.ends_with(style_context.style_prefix)
        {
            let next_len = result
                .len()
                .saturating_sub(style_context.style_prefix.len());
            result.truncate(next_len);
        }

        result
    }

    fn render_paragraph(&self, inlines: &[Inline], apply_default_text: bool) -> Vec<String> {
        let apply_text = |text: &str| {
            if apply_default_text {
                self.apply_default_style(text)
            } else {
                text.to_owned()
            }
        };
        let style_prefix = if apply_default_text {
            self.get_default_style_prefix()
        } else {
            String::new()
        };
        let style_context = InlineStyleContext {
            apply_text: &apply_text,
            style_prefix: &style_prefix,
        };
        vec![self.render_inlines(inlines, &style_context)]
    }

    fn render_heading(&self, level: HeadingLevel, inlines: &[Inline]) -> Vec<String> {
        let heading_level = level as usize;
        let heading_prefix = format!("{} ", "#".repeat(heading_level));
        let heading_style = |text: &str| {
            if heading_level == 1 {
                self.theme
                    .heading(&self.theme.bold(&self.theme.underline(text)))
            } else {
                self.theme.heading(&self.theme.bold(text))
            }
        };
        let style_prefix = self.get_style_prefix(&heading_style);
        let style_context = InlineStyleContext {
            apply_text: &heading_style,
            style_prefix: &style_prefix,
        };
        let heading_text = self.render_inlines(inlines, &style_context);
        let line = if heading_level >= 3 {
            format!("{}{}", heading_style(&heading_prefix), heading_text)
        } else {
            heading_text
        };
        vec![line]
    }

    fn render_code_block(&self, lang: Option<&str>, text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(
            self.theme
                .code_block_border(&format!("```{}", lang.unwrap_or_default())),
        );
        if let Some(highlighted) = self.theme.highlight_code(text, lang) {
            for line in highlighted {
                lines.push(format!("{}{}", self.theme.code_block_indent(), line));
            }
        } else {
            for line in text.split('\n') {
                lines.push(format!(
                    "{}{}",
                    self.theme.code_block_indent(),
                    self.theme.code_block(line)
                ));
            }
        }
        lines.push(self.theme.code_block_border("```"));
        lines
    }

    fn render_list(
        &self,
        list: &ListBlock,
        width: usize,
        depth: usize,
        apply_default_text: bool,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let indent = "  ".repeat(depth);
        let nested_prefix = "  ".repeat(depth + 1);

        for (index, item) in list.items.iter().enumerate() {
            let bullet = if list.ordered {
                format!("{}. ", list.start + index as u64)
            } else {
                "- ".to_owned()
            };
            let bullet_width = visible_width(&bullet);
            let item_width = width
                .saturating_sub(visible_width(&indent) + bullet_width)
                .max(1);
            let item_lines =
                self.render_list_item_blocks(&item.blocks, item_width, depth, apply_default_text);

            if item_lines.is_empty() {
                lines.push(format!("{indent}{}", self.theme.list_bullet(&bullet)));
                continue;
            }

            lines.push(format!(
                "{indent}{}{}",
                self.theme.list_bullet(&bullet),
                item_lines[0]
            ));

            for line in item_lines.into_iter().skip(1) {
                if line.is_empty() {
                    lines.push(format!("{indent}  "));
                } else if line.starts_with(&nested_prefix) {
                    lines.push(line);
                } else {
                    lines.push(format!("{indent}  {line}"));
                }
            }
        }

        lines
    }

    fn render_list_item_blocks(
        &self,
        blocks: &[Block],
        width: usize,
        depth: usize,
        apply_default_text: bool,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let mut first = true;
        for block in blocks {
            let block_lines = match block {
                Block::List(list) => self.render_list(list, width, depth + 1, apply_default_text),
                _ => self.render_block(block, width, apply_default_text),
            };
            if block_lines.is_empty() {
                continue;
            }
            if !first {
                lines.push(String::new());
            }
            lines.extend(block_lines);
            first = false;
        }
        lines
    }

    fn render_blockquote(&self, blocks: &[Block], width: usize) -> Vec<String> {
        let quote_content_width = width.saturating_sub(2).max(1);
        let mut quote_lines = self.render_blocks(blocks, quote_content_width, false);
        while quote_lines.last().is_some_and(|line| line.is_empty()) {
            quote_lines.pop();
        }

        let quote_style = |text: &str| self.theme.quote(&self.theme.italic(text));
        let quote_style_prefix = self.get_style_prefix(&quote_style);
        let apply_quote_style = |line: &str| {
            if quote_style_prefix.is_empty() {
                quote_style(line)
            } else {
                quote_style(&line.replace("\x1b[0m", &format!("\x1b[0m{quote_style_prefix}")))
            }
        };

        let mut lines = Vec::new();
        for quote_line in quote_lines {
            let styled_line = apply_quote_style(&quote_line);
            for wrapped in wrap_text_with_ansi(&styled_line, quote_content_width) {
                lines.push(format!("{}{}", self.theme.quote_border("│ "), wrapped));
            }
        }
        lines
    }

    fn render_html_block(&self, html: &str, apply_default_text: bool) -> Vec<String> {
        let trimmed = html.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let line = if apply_default_text {
            self.apply_default_style(trimmed)
        } else {
            trimmed.to_owned()
        };
        vec![line]
    }

    fn render_table(
        &self,
        table: &TableBlock,
        available_width: usize,
        apply_default_text: bool,
    ) -> Vec<String> {
        if table.header.is_empty() {
            return Vec::new();
        }

        let render_cell = |cell: &[Inline]| {
            let apply_text = |text: &str| {
                if apply_default_text {
                    self.apply_default_style(text)
                } else {
                    text.to_owned()
                }
            };
            let style_prefix = if apply_default_text {
                self.get_default_style_prefix()
            } else {
                String::new()
            };
            let style_context = InlineStyleContext {
                apply_text: &apply_text,
                style_prefix: &style_prefix,
            };
            self.render_inlines(cell, &style_context)
        };

        let header = table
            .header
            .iter()
            .map(|cell| render_cell(cell))
            .collect::<Vec<_>>();
        let rows = table
            .rows
            .iter()
            .map(|row| row.iter().map(|cell| render_cell(cell)).collect::<Vec<_>>())
            .collect::<Vec<_>>();

        let num_cols = header.len();
        if num_cols == 0 {
            return Vec::new();
        }

        let border_overhead = 3 * num_cols + 1;
        let available_for_cells = available_width.saturating_sub(border_overhead);
        if available_for_cells < num_cols {
            let mut fallback = vec![header.join(" | ")];
            for row in rows {
                fallback.push(row.join(" | "));
            }
            return fallback;
        }

        let max_unbroken_word_width = 30usize;
        let mut natural_widths = vec![0usize; num_cols];
        let mut min_word_widths = vec![1usize; num_cols];

        for (index, text) in header.iter().enumerate() {
            natural_widths[index] = visible_width(text);
            min_word_widths[index] = longest_word_width(text, max_unbroken_word_width).max(1);
        }
        for row in &rows {
            for (index, text) in row.iter().enumerate() {
                natural_widths[index] = natural_widths[index].max(visible_width(text));
                min_word_widths[index] = min_word_widths[index]
                    .max(longest_word_width(text, max_unbroken_word_width).max(1));
            }
        }

        let mut min_column_widths = min_word_widths.clone();
        let mut min_cells_width = min_column_widths.iter().sum::<usize>();
        if min_cells_width > available_for_cells {
            min_column_widths = vec![1usize; num_cols];
            let remaining = available_for_cells.saturating_sub(num_cols);
            if remaining > 0 {
                let total_weight = min_word_widths
                    .iter()
                    .map(|width| width.saturating_sub(1))
                    .sum::<usize>();
                let growth = min_word_widths
                    .iter()
                    .map(|width| {
                        let weight = width.saturating_sub(1);
                        if total_weight == 0 {
                            0
                        } else {
                            (weight * remaining) / total_weight
                        }
                    })
                    .collect::<Vec<_>>();
                for (index, width) in growth.iter().enumerate() {
                    min_column_widths[index] += *width;
                }
                let allocated = growth.iter().sum::<usize>();
                let mut leftover = remaining.saturating_sub(allocated);
                let mut index = 0usize;
                while leftover > 0 && index < num_cols {
                    min_column_widths[index] += 1;
                    leftover -= 1;
                    index += 1;
                }
            }
            min_cells_width = min_column_widths.iter().sum::<usize>();
        }

        let total_natural_width = natural_widths.iter().sum::<usize>() + border_overhead;
        let mut column_widths = if total_natural_width <= available_width {
            natural_widths
                .iter()
                .enumerate()
                .map(|(index, width)| (*width).max(min_column_widths[index]))
                .collect::<Vec<_>>()
        } else {
            let total_grow_potential = natural_widths
                .iter()
                .enumerate()
                .map(|(index, width)| width.saturating_sub(min_column_widths[index]))
                .sum::<usize>();
            let extra_width = available_for_cells.saturating_sub(min_cells_width);
            let mut widths = min_column_widths
                .iter()
                .enumerate()
                .map(|(index, min_width)| {
                    let natural = natural_widths[index];
                    let delta = natural.saturating_sub(*min_width);
                    let grow = if total_grow_potential == 0 {
                        0
                    } else {
                        (delta * extra_width) / total_grow_potential
                    };
                    min_width + grow
                })
                .collect::<Vec<_>>();

            let allocated = widths.iter().sum::<usize>();
            let mut remaining = available_for_cells.saturating_sub(allocated);
            while remaining > 0 {
                let mut grew = false;
                for (index, width) in widths.iter_mut().enumerate() {
                    if *width < natural_widths[index] && remaining > 0 {
                        *width += 1;
                        remaining -= 1;
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            widths
        };

        if column_widths.is_empty() {
            column_widths = vec![1; num_cols];
        }

        let mut lines = Vec::new();
        let top_border_cells = column_widths
            .iter()
            .map(|width| "─".repeat(*width))
            .collect::<Vec<_>>();
        lines.push(format!("┌─{}─┐", top_border_cells.join("─┬─")));

        let header_cell_lines = header
            .iter()
            .enumerate()
            .map(|(index, text)| wrap_text_with_ansi(text, column_widths[index].max(1)))
            .collect::<Vec<_>>();
        let header_line_count = header_cell_lines.iter().map(Vec::len).max().unwrap_or(0);
        for line_index in 0..header_line_count {
            let row_parts = header_cell_lines
                .iter()
                .enumerate()
                .map(|(col_index, cell_lines)| {
                    let text = cell_lines.get(line_index).cloned().unwrap_or_default();
                    let padded = pad_to_width(&text, column_widths[col_index]);
                    self.theme.bold(&padded)
                })
                .collect::<Vec<_>>();
            lines.push(format!("│ {} │", row_parts.join(" │ ")));
        }

        let separator_cells = column_widths
            .iter()
            .map(|width| "─".repeat(*width))
            .collect::<Vec<_>>();
        let separator = format!("├─{}─┤", separator_cells.join("─┼─"));
        lines.push(separator.clone());

        for (row_index, row) in rows.iter().enumerate() {
            let row_cell_lines = row
                .iter()
                .enumerate()
                .map(|(index, text)| wrap_text_with_ansi(text, column_widths[index].max(1)))
                .collect::<Vec<_>>();
            let row_line_count = row_cell_lines.iter().map(Vec::len).max().unwrap_or(0);
            for line_index in 0..row_line_count {
                let row_parts = row_cell_lines
                    .iter()
                    .enumerate()
                    .map(|(col_index, cell_lines)| {
                        let text = cell_lines.get(line_index).cloned().unwrap_or_default();
                        pad_to_width(&text, column_widths[col_index])
                    })
                    .collect::<Vec<_>>();
                lines.push(format!("│ {} │", row_parts.join(" │ ")));
            }
            if row_index + 1 < rows.len() {
                lines.push(separator.clone());
            }
        }

        let bottom_border_cells = column_widths
            .iter()
            .map(|width| "─".repeat(*width))
            .collect::<Vec<_>>();
        lines.push(format!("└─{}─┘", bottom_border_cells.join("─┴─")));
        lines
    }

    fn render_block(&self, block: &Block, width: usize, apply_default_text: bool) -> Vec<String> {
        match block {
            Block::Heading { level, content } => self.render_heading(*level, content),
            Block::Paragraph(content) => self.render_paragraph(content, apply_default_text),
            Block::CodeBlock { lang, text } => self.render_code_block(lang.as_deref(), text),
            Block::List(list) => self.render_list(list, width, 0, apply_default_text),
            Block::BlockQuote(blocks) => self.render_blockquote(blocks, width),
            Block::Table(table) => self.render_table(table, width, apply_default_text),
            Block::Hr => vec![self.theme.hr(&"─".repeat(width.min(80).max(1)))],
            Block::Html(html) => self.render_html_block(html, apply_default_text),
        }
    }

    fn render_blocks(
        &self,
        blocks: &[Block],
        width: usize,
        apply_default_text: bool,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let mut first = true;
        for block in blocks {
            let block_lines = self.render_block(block, width, apply_default_text);
            if block_lines.is_empty() {
                continue;
            }
            if !first {
                lines.push(String::new());
            }
            lines.extend(block_lines);
            first = false;
        }
        lines
    }
}

impl Component for Markdown {
    fn render(&self, width: usize) -> Vec<String> {
        if self.text.trim().is_empty() {
            return Vec::new();
        }

        if let Some(cache) = self.cache.borrow().as_ref()
            && cache.text == self.text
            && cache.width == width
        {
            return cache.lines.clone();
        }

        let content_width = width.saturating_sub(self.padding_x * 2).max(1);
        let blocks = self.parse_blocks();
        let rendered_lines = self.render_blocks(&blocks, content_width, true);

        let mut wrapped_lines = Vec::new();
        for line in rendered_lines {
            wrapped_lines.extend(wrap_text_with_ansi(&line, content_width));
        }

        let left_margin = " ".repeat(self.padding_x);
        let right_margin = " ".repeat(self.padding_x);
        let mut content_lines = Vec::new();
        for line in wrapped_lines {
            let line_with_margins = format!("{left_margin}{line}{right_margin}");
            if let Some(bg_fn) = self
                .default_text_style
                .as_ref()
                .and_then(DefaultTextStyle::bg_color)
            {
                content_lines.push(apply_background_to_line(&line_with_margins, width, bg_fn));
            } else {
                let visible_len = visible_width(&line_with_margins);
                content_lines.push(format!(
                    "{line_with_margins}{}",
                    " ".repeat(width.saturating_sub(visible_len))
                ));
            }
        }

        let empty_line = " ".repeat(width);
        let mut empty_lines = Vec::new();
        for _ in 0..self.padding_y {
            if let Some(bg_fn) = self
                .default_text_style
                .as_ref()
                .and_then(DefaultTextStyle::bg_color)
            {
                empty_lines.push(apply_background_to_line(&empty_line, width, bg_fn));
            } else {
                empty_lines.push(empty_line.clone());
            }
        }

        let mut lines = Vec::new();
        lines.extend(empty_lines.iter().cloned());
        lines.extend(content_lines);
        lines.extend(empty_lines);

        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };

        *self.cache.borrow_mut() = Some(RenderCache {
            text: self.text.clone(),
            width,
            lines: lines.clone(),
        });
        lines
    }

    fn invalidate(&mut self) {
        self.clear_cache();
    }
}

fn apply_text_with_newlines(text: &str, apply_text: &dyn Fn(&str) -> String) -> String {
    text.split('\n')
        .map(apply_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn inline_plain_text(inlines: &[Inline]) -> String {
    let mut result = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Html(text) | Inline::Code(text) => result.push_str(text),
            Inline::Break => result.push('\n'),
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Strikethrough(children) => result.push_str(&inline_plain_text(children)),
            Inline::Link { text, .. } => result.push_str(&inline_plain_text(text)),
        }
    }
    result
}

fn longest_word_width(text: &str, max_width: usize) -> usize {
    text.split_whitespace()
        .map(visible_width)
        .max()
        .unwrap_or(0)
        .min(max_width)
}

fn pad_to_width(text: &str, width: usize) -> String {
    format!(
        "{text}{}",
        " ".repeat(width.saturating_sub(visible_width(text)))
    )
}

fn apply_background_to_line(line: &str, width: usize, bg_fn: &TextStyleFn) -> String {
    let visible_len = visible_width(line);
    let with_padding = format!("{line}{}", " ".repeat(width.saturating_sub(visible_len)));
    bg_fn(&with_padding)
}

fn parse_blocks_from_events<'a, I>(
    events: &mut std::iter::Peekable<I>,
    until: Option<TagEnd>,
) -> Vec<Block>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut blocks = Vec::new();

    while let Some(event) = events.next() {
        match event {
            Event::End(end) => {
                if until.as_ref() == Some(&end) {
                    break;
                }
            }
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    blocks.push(Block::Paragraph(parse_inlines_from_events(
                        events,
                        TagEnd::Paragraph,
                    )));
                }
                Tag::Heading { level, .. } => {
                    blocks.push(Block::Heading {
                        level,
                        content: parse_inlines_from_events(events, TagEnd::Heading(level)),
                    });
                }
                Tag::CodeBlock(kind) => blocks.push(parse_code_block_from_events(events, kind)),
                Tag::List(start) => blocks.push(Block::List(parse_list_from_events(events, start))),
                Tag::BlockQuote(kind) => blocks.push(Block::BlockQuote(parse_blocks_from_events(
                    events,
                    Some(TagEnd::BlockQuote(kind)),
                ))),
                Tag::HtmlBlock => {
                    let html = parse_html_block_from_events(events);
                    if !html.trim().is_empty() {
                        blocks.push(Block::Html(html));
                    }
                }
                Tag::Table(_alignments) => {
                    blocks.push(Block::Table(parse_table_from_events(events)));
                }
                _ => skip_tag(events, tag.to_end()),
            },
            Event::Rule => blocks.push(Block::Hr),
            Event::Text(text) => {
                if !text.trim().is_empty() {
                    blocks.push(Block::Paragraph(vec![Inline::Text(text.into_string())]));
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let trimmed = html.trim();
                if !trimmed.is_empty() {
                    blocks.push(Block::Html(trimmed.to_owned()));
                }
            }
            _ => {}
        }
    }

    blocks
}

fn parse_inlines_from_events<'a, I>(
    events: &mut std::iter::Peekable<I>,
    until: TagEnd,
) -> Vec<Inline>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut inlines = Vec::new();

    while let Some(event) = events.next() {
        match event {
            Event::End(end) if end == until => break,
            Event::Text(text) => push_inline_text(&mut inlines, Inline::Text(text.into_string())),
            Event::Code(text) => inlines.push(Inline::Code(text.into_string())),
            Event::SoftBreak | Event::HardBreak => inlines.push(Inline::Break),
            Event::Html(html) | Event::InlineHtml(html) => {
                push_inline_text(&mut inlines, Inline::Html(html.into_string()));
            }
            Event::Start(tag) => match tag {
                Tag::Emphasis => inlines.push(Inline::Emphasis(parse_inlines_from_events(
                    events,
                    TagEnd::Emphasis,
                ))),
                Tag::Strong => inlines.push(Inline::Strong(parse_inlines_from_events(
                    events,
                    TagEnd::Strong,
                ))),
                Tag::Strikethrough => inlines.push(Inline::Strikethrough(
                    parse_inlines_from_events(events, TagEnd::Strikethrough),
                )),
                Tag::Link {
                    dest_url,
                    link_type,
                    ..
                } => inlines.push(Inline::Link {
                    text: parse_inlines_from_events(events, TagEnd::Link),
                    url: dest_url.into_string(),
                    link_type,
                }),
                Tag::Image { dest_url, .. } => inlines.push(Inline::Link {
                    text: parse_inlines_from_events(events, TagEnd::Image),
                    url: dest_url.into_string(),
                    link_type: LinkType::Inline,
                }),
                other => {
                    let nested = parse_inlines_from_events(events, other.to_end());
                    inlines.extend(nested);
                }
            },
            _ => {}
        }
    }

    inlines
}

fn parse_code_block_from_events<'a, I>(
    events: &mut std::iter::Peekable<I>,
    kind: CodeBlockKind<'a>,
) -> Block
where
    I: Iterator<Item = Event<'a>>,
{
    let mut text = String::new();
    while let Some(event) = events.next() {
        match event {
            Event::End(TagEnd::CodeBlock) => break,
            Event::Text(value) | Event::Html(value) | Event::InlineHtml(value) => {
                text.push_str(&value);
            }
            Event::Code(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            _ => {}
        }
    }

    let lang = match kind {
        CodeBlockKind::Fenced(lang) => {
            let trimmed = lang.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CodeBlockKind::Indented => None,
    };

    Block::CodeBlock {
        lang,
        text: text.trim_end_matches('\n').to_owned(),
    }
}

fn parse_list_from_events<'a, I>(
    events: &mut std::iter::Peekable<I>,
    start: Option<u64>,
) -> ListBlock
where
    I: Iterator<Item = Event<'a>>,
{
    let mut items = Vec::new();
    let ordered = start.is_some();

    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::Item) => items.push(ListItem {
                blocks: parse_blocks_from_events(events, Some(TagEnd::Item)),
            }),
            Event::End(TagEnd::List(_)) => break,
            _ => {}
        }
    }

    ListBlock {
        ordered,
        start: start.unwrap_or(1),
        items,
    }
}

fn parse_html_block_from_events<'a, I>(events: &mut std::iter::Peekable<I>) -> String
where
    I: Iterator<Item = Event<'a>>,
{
    let mut html = String::new();
    while let Some(event) = events.next() {
        match event {
            Event::End(TagEnd::HtmlBlock) => break,
            Event::Text(value) | Event::Html(value) | Event::InlineHtml(value) => {
                html.push_str(&value);
            }
            Event::SoftBreak | Event::HardBreak => html.push('\n'),
            _ => {}
        }
    }
    html
}

fn parse_table_from_events<'a, I>(events: &mut std::iter::Peekable<I>) -> TableBlock
where
    I: Iterator<Item = Event<'a>>,
{
    let mut header = Vec::new();
    let mut rows = Vec::new();
    let mut in_head = false;

    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableCell) if in_head => {
                header.push(parse_inlines_from_events(events, TagEnd::TableCell));
            }
            Event::Start(Tag::TableRow) => rows.push(parse_table_row_from_events(events)),
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }

    TableBlock { header, rows }
}

fn parse_table_row_from_events<'a, I>(events: &mut std::iter::Peekable<I>) -> Vec<Vec<Inline>>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut cells = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableCell) => {
                cells.push(parse_inlines_from_events(events, TagEnd::TableCell));
            }
            Event::End(TagEnd::TableRow) => break,
            _ => {}
        }
    }
    cells
}

fn push_inline_text(inlines: &mut Vec<Inline>, inline: Inline) {
    match (inlines.last_mut(), inline) {
        (Some(Inline::Text(existing)), Inline::Text(next)) => existing.push_str(&next),
        (Some(Inline::Html(existing)), Inline::Html(next)) => existing.push_str(&next),
        (_, next) => inlines.push(next),
    }
}

fn skip_tag<'a, I>(events: &mut std::iter::Peekable<I>, until: TagEnd)
where
    I: Iterator<Item = Event<'a>>,
{
    let mut depth = 1usize;
    while let Some(event) = events.next() {
        match event {
            Event::Start(tag) if tag.to_end() == until => depth += 1,
            Event::End(end) if end == until => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
}
