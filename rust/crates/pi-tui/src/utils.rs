use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const RESET: &str = "\x1b[0m";
const TAB_WIDTH: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsiCode {
    pub code: String,
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceWithWidthResult {
    pub text: String,
    pub width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractSegmentsResult {
    pub before: String,
    pub before_width: usize,
    pub after: String,
    pub after_width: usize,
}

pub fn visible_width(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }

    if is_printable_ascii(text) {
        return text.len();
    }

    let mut width = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some(length) = extract_ansi_length(text, index) {
                index += length;
                continue;
            }
        }

        let character = text[index..]
            .chars()
            .next()
            .expect("text slice should contain a character");
        if character == '\t' {
            width += TAB_WIDTH;
            index += character.len_utf8();
            continue;
        }

        if character == '\x1b' {
            width += grapheme_width("\x1b");
            index += character.len_utf8();
            continue;
        }

        let mut end = index;
        while end < bytes.len() && bytes[end] != 0x1b && bytes[end] != b'\t' {
            end += 1;
        }

        for grapheme in text[index..end].graphemes(true) {
            width += grapheme_width(grapheme);
        }

        index = end;
    }

    width
}

pub fn extract_ansi_code(text: &str, pos: usize) -> Option<AnsiCode> {
    let length = extract_ansi_length(text, pos)?;
    Some(AnsiCode {
        code: text[pos..pos + length].to_owned(),
        length,
    })
}

pub fn slice_by_column(line: &str, start_col: usize, length: usize, strict: bool) -> String {
    slice_with_width(line, start_col, length, strict).text
}

pub fn slice_with_width(
    line: &str,
    start_col: usize,
    length: usize,
    strict: bool,
) -> SliceWithWidthResult {
    if length == 0 {
        return SliceWithWidthResult {
            text: String::new(),
            width: 0,
        };
    }

    let end_col = start_col + length;
    let mut result = String::new();
    let mut result_width = 0usize;
    let mut current_col = 0usize;
    let mut index = 0usize;
    let mut pending_ansi = String::new();
    let bytes = line.as_bytes();

    while index < bytes.len() {
        if let Some(ansi) = extract_ansi_code(line, index) {
            if current_col >= start_col && current_col < end_col {
                result.push_str(&ansi.code);
            } else if current_col < start_col {
                pending_ansi.push_str(&ansi.code);
            }
            index += ansi.length;
            continue;
        }

        let mut text_end = index;
        while text_end < bytes.len() && extract_ansi_code(line, text_end).is_none() {
            text_end += 1;
        }

        for grapheme in line[index..text_end].graphemes(true) {
            let width = grapheme_width(grapheme);
            let in_range = current_col >= start_col && current_col < end_col;
            let fits = !strict || current_col + width <= end_col;
            if in_range && fits {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push_str(grapheme);
                result_width += width;
            }
            current_col += width;
            if current_col >= end_col {
                break;
            }
        }

        index = text_end;
        if current_col >= end_col {
            break;
        }
    }

    SliceWithWidthResult {
        text: result,
        width: result_width,
    }
}

pub fn extract_segments(
    line: &str,
    before_end: usize,
    after_start: usize,
    after_len: usize,
    strict_after: bool,
) -> ExtractSegmentsResult {
    let mut before = String::new();
    let mut before_width = 0usize;
    let mut after = String::new();
    let mut after_width = 0usize;
    let mut current_col = 0usize;
    let mut index = 0usize;
    let mut pending_ansi_before = String::new();
    let mut after_started = false;
    let after_end = after_start + after_len;
    let mut tracker = AnsiCodeTracker::default();
    let bytes = line.as_bytes();

    while index < bytes.len() {
        if let Some(ansi) = extract_ansi_code(line, index) {
            tracker.process(&ansi.code);
            if current_col < before_end {
                pending_ansi_before.push_str(&ansi.code);
            } else if current_col >= after_start && current_col < after_end && after_started {
                after.push_str(&ansi.code);
            }
            index += ansi.length;
            continue;
        }

        let mut text_end = index;
        while text_end < bytes.len() && extract_ansi_code(line, text_end).is_none() {
            text_end += 1;
        }

        for grapheme in line[index..text_end].graphemes(true) {
            let width = grapheme_width(grapheme);

            if current_col < before_end {
                if !pending_ansi_before.is_empty() {
                    before.push_str(&pending_ansi_before);
                    pending_ansi_before.clear();
                }
                before.push_str(grapheme);
                before_width += width;
            } else if current_col >= after_start && current_col < after_end {
                let fits = !strict_after || current_col + width <= after_end;
                if fits {
                    if !after_started {
                        after.push_str(&tracker.active_codes());
                        after_started = true;
                    }
                    after.push_str(grapheme);
                    after_width += width;
                }
            }

            current_col += width;
            if if after_len == 0 {
                current_col >= before_end
            } else {
                current_col >= after_end
            } {
                break;
            }
        }

        index = text_end;
        if if after_len == 0 {
            current_col >= before_end
        } else {
            current_col >= after_end
        } {
            break;
        }
    }

    ExtractSegmentsResult {
        before,
        before_width,
        after,
        after_width,
    }
}

pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut result = Vec::new();
    let mut tracker = AnsiCodeTracker::default();

    for input_line in text.split('\n') {
        let prefix = if result.is_empty() {
            String::new()
        } else {
            tracker.active_codes()
        };

        let mut prefixed_line = prefix;
        prefixed_line.push_str(input_line);
        result.extend(wrap_single_line(&prefixed_line, width));
        update_tracker_from_text(input_line, &mut tracker);
    }

    if result.is_empty() {
        vec![String::new()]
    } else {
        result
    }
}

pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str, pad: bool) -> String {
    if max_width == 0 {
        return String::new();
    }

    if text.is_empty() {
        return if pad {
            " ".repeat(max_width)
        } else {
            String::new()
        };
    }

    let ellipsis_width = visible_width(ellipsis);
    if ellipsis_width >= max_width {
        let (visible_width_so_far, overflowed) = measure_until_overflow(text, max_width);
        if !overflowed {
            return if pad {
                let mut result = text.to_owned();
                result.push_str(&" ".repeat(max_width.saturating_sub(visible_width_so_far)));
                result
            } else {
                text.to_owned()
            };
        }

        let clipped_ellipsis = truncate_fragment_to_width(ellipsis, max_width);
        if clipped_ellipsis.width == 0 {
            return if pad {
                " ".repeat(max_width)
            } else {
                String::new()
            };
        }

        return finalize_truncated_result(
            "",
            0,
            &clipped_ellipsis.text,
            clipped_ellipsis.width,
            max_width,
            pad,
        );
    }

    if is_printable_ascii(text) {
        if text.len() <= max_width {
            return if pad {
                let mut result = text.to_owned();
                result.push_str(&" ".repeat(max_width - text.len()));
                result
            } else {
                text.to_owned()
            };
        }

        let target_width = max_width - ellipsis_width;
        return finalize_truncated_result(
            &text[..target_width],
            target_width,
            ellipsis,
            ellipsis_width,
            max_width,
            pad,
        );
    }

    let target_width = max_width - ellipsis_width;
    let mut result = String::new();
    let mut pending_ansi = String::new();
    let mut visible_so_far = 0usize;
    let mut kept_width = 0usize;
    let mut keep_contiguous_prefix = true;
    let mut overflowed = false;

    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some(length) = extract_ansi_length(text, index) {
                pending_ansi.push_str(&text[index..index + length]);
                index += length;
                continue;
            }
        }

        let character = text[index..]
            .chars()
            .next()
            .expect("text slice should contain a character");
        if character == '\t' {
            if keep_contiguous_prefix && kept_width + TAB_WIDTH <= target_width {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push('\t');
                kept_width += TAB_WIDTH;
            } else {
                keep_contiguous_prefix = false;
                pending_ansi.clear();
            }

            visible_so_far += TAB_WIDTH;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }

            index += character.len_utf8();
            continue;
        }

        if character == '\x1b' {
            let width = grapheme_width("\x1b");
            if keep_contiguous_prefix && kept_width + width <= target_width {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push(character);
                kept_width += width;
            } else {
                keep_contiguous_prefix = false;
                pending_ansi.clear();
            }

            visible_so_far += width;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }

            index += character.len_utf8();
            continue;
        }

        let mut end = index;
        while end < bytes.len() && bytes[end] != 0x1b && bytes[end] != b'\t' {
            end += 1;
        }

        for grapheme in text[index..end].graphemes(true) {
            let width = grapheme_width(grapheme);
            if keep_contiguous_prefix && kept_width + width <= target_width {
                if !pending_ansi.is_empty() {
                    result.push_str(&pending_ansi);
                    pending_ansi.clear();
                }
                result.push_str(grapheme);
                kept_width += width;
            } else {
                keep_contiguous_prefix = false;
                pending_ansi.clear();
            }

            visible_so_far += width;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }
        }

        if overflowed {
            break;
        }

        index = end;
    }

    if !overflowed && index >= text.len() {
        return if pad {
            let mut original = text.to_owned();
            original.push_str(&" ".repeat(max_width.saturating_sub(visible_so_far)));
            original
        } else {
            text.to_owned()
        };
    }

    finalize_truncated_result(
        &result,
        kept_width,
        ellipsis,
        ellipsis_width,
        max_width,
        pad,
    )
}

pub fn is_whitespace_char(character: char) -> bool {
    character.is_whitespace()
}

pub fn is_punctuation_char(character: char) -> bool {
    matches!(
        character,
        '(' | ')'
            | '{'
            | '}'
            | '['
            | ']'
            | '<'
            | '>'
            | '.'
            | ','
            | ';'
            | ':'
            | '\''
            | '"'
            | '!'
            | '?'
            | '+'
            | '-'
            | '='
            | '*'
            | '/'
            | '\\'
            | '|'
            | '&'
            | '%'
            | '^'
            | '$'
            | '#'
            | '@'
            | '~'
            | '`'
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TruncatedFragment {
    text: String,
    width: usize,
}

#[derive(Debug, Default, Clone)]
struct AnsiCodeTracker {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strikethrough: bool,
    fg_color: Option<String>,
    bg_color: Option<String>,
}

impl AnsiCodeTracker {
    fn process(&mut self, ansi_code: &str) {
        if !ansi_code.ends_with('m') {
            return;
        }

        let Some(params) = ansi_code
            .strip_prefix("\x1b[")
            .and_then(|value| value.strip_suffix('m'))
        else {
            return;
        };

        if params.is_empty() || params == "0" {
            self.reset();
            return;
        }

        let parts = params.split(';').collect::<Vec<_>>();
        let mut index = 0usize;
        while index < parts.len() {
            let Some(code) = parts[index].parse::<u16>().ok() else {
                index += 1;
                continue;
            };

            if (code == 38 || code == 48) && parts.get(index + 1) == Some(&"5") {
                if let Some(color) = parts.get(index + 2) {
                    let value = format!("{};{};{}", parts[index], parts[index + 1], color);
                    if code == 38 {
                        self.fg_color = Some(value);
                    } else {
                        self.bg_color = Some(value);
                    }
                    index += 3;
                    continue;
                }
            }

            if (code == 38 || code == 48) && parts.get(index + 1) == Some(&"2") {
                if let (Some(r), Some(g), Some(b)) = (
                    parts.get(index + 2),
                    parts.get(index + 3),
                    parts.get(index + 4),
                ) {
                    let value = format!("{};{};{};{};{}", parts[index], parts[index + 1], r, g, b);
                    if code == 38 {
                        self.fg_color = Some(value);
                    } else {
                        self.bg_color = Some(value);
                    }
                    index += 5;
                    continue;
                }
            }

            match code {
                0 => self.reset(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                5 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strikethrough = true,
                21 => self.bold = false,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strikethrough = false,
                39 => self.fg_color = None,
                49 => self.bg_color = None,
                30..=37 | 90..=97 => self.fg_color = Some(code.to_string()),
                40..=47 | 100..=107 => self.bg_color = Some(code.to_string()),
                _ => {}
            }

            index += 1;
        }
    }

    fn active_codes(&self) -> String {
        let mut codes = Vec::new();
        if self.bold {
            codes.push(Cow::Borrowed("1"));
        }
        if self.dim {
            codes.push(Cow::Borrowed("2"));
        }
        if self.italic {
            codes.push(Cow::Borrowed("3"));
        }
        if self.underline {
            codes.push(Cow::Borrowed("4"));
        }
        if self.blink {
            codes.push(Cow::Borrowed("5"));
        }
        if self.inverse {
            codes.push(Cow::Borrowed("7"));
        }
        if self.hidden {
            codes.push(Cow::Borrowed("8"));
        }
        if self.strikethrough {
            codes.push(Cow::Borrowed("9"));
        }
        if let Some(fg_color) = &self.fg_color {
            codes.push(Cow::Owned(fg_color.clone()));
        }
        if let Some(bg_color) = &self.bg_color {
            codes.push(Cow::Owned(bg_color.clone()));
        }

        if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        }
    }

    fn line_end_reset(&self) -> Option<&'static str> {
        if self.underline {
            Some("\x1b[24m")
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.bold = false;
        self.dim = false;
        self.italic = false;
        self.underline = false;
        self.blink = false;
        self.inverse = false;
        self.hidden = false;
        self.strikethrough = false;
        self.fg_color = None;
        self.bg_color = None;
    }
}

fn is_printable_ascii(text: &str) -> bool {
    text.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

fn extract_ansi_length(text: &str, pos: usize) -> Option<usize> {
    if pos >= text.len() || !text.is_char_boundary(pos) || text.as_bytes()[pos] != 0x1b {
        return None;
    }

    let bytes = text.as_bytes();
    let next = *bytes.get(pos + 1)?;

    match next {
        b'[' => {
            let mut index = pos + 2;
            while index < bytes.len() && !matches!(bytes[index], b'm' | b'G' | b'K' | b'H' | b'J') {
                index += 1;
            }
            (index < bytes.len()).then_some(index + 1 - pos)
        }
        b']' | b'_' => {
            let mut index = pos + 2;
            while index < bytes.len() {
                if bytes[index] == 0x07 {
                    return Some(index + 1 - pos);
                }
                if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                    return Some(index + 2 - pos);
                }
                index += 1;
            }
            None
        }
        _ => None,
    }
}

fn grapheme_width(segment: &str) -> usize {
    if segment.is_empty() {
        return 0;
    }

    let width = UnicodeWidthStr::width(segment);
    if width == 0 {
        return 0;
    }

    let Some(first) = segment.chars().next() else {
        return 0;
    };
    let first = first as u32;

    if is_regional_indicator(first) {
        return 2;
    }

    if is_likely_emoji(segment, first) {
        return 2;
    }

    width
}

fn is_likely_emoji(segment: &str, first: u32) -> bool {
    (0x1f000..=0x1fbff).contains(&first)
        || (0x2600..=0x27bf).contains(&first)
        || (0x2b50..=0x2b55).contains(&first)
        || segment.contains('\u{fe0f}')
        || segment.contains('\u{200d}')
}

fn is_regional_indicator(codepoint: u32) -> bool {
    (0x1f1e6..=0x1f1ff).contains(&codepoint)
}

fn measure_until_overflow(text: &str, limit: usize) -> (usize, bool) {
    let mut width = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some(length) = extract_ansi_length(text, index) {
                index += length;
                continue;
            }
        }

        let character = text[index..]
            .chars()
            .next()
            .expect("text slice should contain a character");
        if character == '\t' {
            width += TAB_WIDTH;
            if width > limit {
                return (width, true);
            }
            index += character.len_utf8();
            continue;
        }

        if character == '\x1b' {
            width += grapheme_width("\x1b");
            if width > limit {
                return (width, true);
            }
            index += character.len_utf8();
            continue;
        }

        let mut end = index;
        while end < bytes.len() && bytes[end] != 0x1b && bytes[end] != b'\t' {
            end += 1;
        }

        for grapheme in text[index..end].graphemes(true) {
            width += grapheme_width(grapheme);
            if width > limit {
                return (width, true);
            }
        }

        index = end;
    }

    (width, false)
}

fn truncate_fragment_to_width(text: &str, max_width: usize) -> TruncatedFragment {
    if max_width == 0 || text.is_empty() {
        return TruncatedFragment {
            text: String::new(),
            width: 0,
        };
    }

    if is_printable_ascii(text) {
        let clipped = &text[..text.len().min(max_width)];
        return TruncatedFragment {
            text: clipped.to_owned(),
            width: clipped.len(),
        };
    }

    let mut result = String::new();
    let mut width = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some(length) = extract_ansi_length(text, index) {
                result.push_str(&text[index..index + length]);
                index += length;
                continue;
            }
        }

        let character = text[index..]
            .chars()
            .next()
            .expect("text slice should contain a character");
        if character == '\t' {
            if width + TAB_WIDTH > max_width {
                break;
            }
            result.push('\t');
            width += TAB_WIDTH;
            index += character.len_utf8();
            continue;
        }

        if character == '\x1b' {
            let character_width = grapheme_width("\x1b");
            if width + character_width > max_width {
                break;
            }
            result.push(character);
            width += character_width;
            index += character.len_utf8();
            continue;
        }

        let mut end = index;
        while end < bytes.len() && bytes[end] != 0x1b && bytes[end] != b'\t' {
            end += 1;
        }

        for grapheme in text[index..end].graphemes(true) {
            let grapheme_width = grapheme_width(grapheme);
            if width + grapheme_width > max_width {
                return TruncatedFragment {
                    text: result,
                    width,
                };
            }
            result.push_str(grapheme);
            width += grapheme_width;
        }

        index = end;
    }

    TruncatedFragment {
        text: result,
        width,
    }
}

fn finalize_truncated_result(
    prefix: &str,
    prefix_width: usize,
    ellipsis: &str,
    ellipsis_width: usize,
    max_width: usize,
    pad: bool,
) -> String {
    let visible_width = prefix_width + ellipsis_width;
    let mut result = String::new();
    result.push_str(prefix);
    result.push_str(RESET);
    if !ellipsis.is_empty() {
        result.push_str(ellipsis);
        result.push_str(RESET);
    }

    if pad {
        result.push_str(&" ".repeat(max_width.saturating_sub(visible_width)));
    }

    result
}

fn update_tracker_from_text(text: &str, tracker: &mut AnsiCodeTracker) {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some(length) = extract_ansi_length(text, index) {
                tracker.process(&text[index..index + length]);
                index += length;
                continue;
            }
        }

        index += text[index..]
            .chars()
            .next()
            .expect("text slice should contain a character")
            .len_utf8();
    }
}

fn split_into_tokens_with_ansi(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut pending_ansi = String::new();
    let mut in_whitespace = false;
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some(length) = extract_ansi_length(text, index) {
                pending_ansi.push_str(&text[index..index + length]);
                index += length;
                continue;
            }
        }

        let character = text[index..]
            .chars()
            .next()
            .expect("text slice should contain a character");
        let character_is_space = character == ' ';

        if character_is_space != in_whitespace && !current.is_empty() {
            tokens.push(current);
            current = String::new();
        }

        if !pending_ansi.is_empty() {
            current.push_str(&pending_ansi);
            pending_ansi.clear();
        }

        in_whitespace = character_is_space;
        current.push(character);
        index += character.len_utf8();
    }

    if !pending_ansi.is_empty() {
        current.push_str(&pending_ansi);
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }

    if visible_width(line) <= width {
        return vec![line.to_owned()];
    }

    let mut wrapped = Vec::new();
    let mut tracker = AnsiCodeTracker::default();
    let tokens = split_into_tokens_with_ansi(line);

    let mut current_line = String::new();
    let mut current_visible_length = 0usize;

    for token in tokens {
        let token_visible_length = visible_width(&token);
        let is_whitespace = token.trim().is_empty();

        if token_visible_length > width && !is_whitespace {
            if !current_line.is_empty() {
                let mut line_to_wrap = current_line;
                if let Some(reset) = tracker.line_end_reset() {
                    line_to_wrap.push_str(reset);
                }
                wrapped.push(line_to_wrap);
                current_line = String::new();
                current_visible_length = 0;
            }

            let broken = break_long_word(&token, width, &mut tracker);
            let last_index = broken.len().saturating_sub(1);
            for (index, line) in broken.into_iter().enumerate() {
                if index == last_index {
                    current_visible_length = visible_width(&line);
                    current_line = line;
                } else {
                    wrapped.push(line);
                }
            }
            continue;
        }

        let total_needed = current_visible_length + token_visible_length;
        if total_needed > width && current_visible_length > 0 {
            let mut line_to_wrap = current_line.trim_end().to_owned();
            if let Some(reset) = tracker.line_end_reset() {
                line_to_wrap.push_str(reset);
            }
            wrapped.push(line_to_wrap);
            if is_whitespace {
                current_line = tracker.active_codes();
                current_visible_length = 0;
            } else {
                current_line = tracker.active_codes();
                current_line.push_str(&token);
                current_visible_length = token_visible_length;
            }
        } else {
            current_line.push_str(&token);
            current_visible_length += token_visible_length;
        }

        update_tracker_from_text(&token, &mut tracker);
    }

    if !current_line.is_empty() {
        wrapped.push(current_line);
    }

    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect()
    }
}

fn break_long_word(word: &str, width: usize, tracker: &mut AnsiCodeTracker) -> Vec<String> {
    #[derive(Debug)]
    enum SegmentPart {
        Ansi(String),
        Grapheme(String),
    }

    let mut segments = Vec::new();
    let bytes = word.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some(length) = extract_ansi_length(word, index) {
                segments.push(SegmentPart::Ansi(word[index..index + length].to_owned()));
                index += length;
                continue;
            }
        }

        let mut end = index;
        while end < bytes.len() {
            if bytes[end] == 0x1b && extract_ansi_length(word, end).is_some() {
                break;
            }
            end += 1;
        }

        for grapheme in word[index..end].graphemes(true) {
            segments.push(SegmentPart::Grapheme(grapheme.to_owned()));
        }
        index = end;
    }

    let mut lines = Vec::new();
    let mut current_line = tracker.active_codes();
    let mut current_width = 0usize;

    for segment in segments {
        match segment {
            SegmentPart::Ansi(code) => {
                current_line.push_str(&code);
                tracker.process(&code);
            }
            SegmentPart::Grapheme(grapheme) => {
                if grapheme.is_empty() {
                    continue;
                }

                let grapheme_width = visible_width(&grapheme);
                if current_width + grapheme_width > width {
                    if let Some(reset) = tracker.line_end_reset() {
                        current_line.push_str(reset);
                    }
                    lines.push(current_line);
                    current_line = tracker.active_codes();
                    current_width = 0;
                }

                current_line.push_str(&grapheme);
                current_width += grapheme_width;
            }
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}
