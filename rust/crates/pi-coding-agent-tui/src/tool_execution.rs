use crate::{KeybindingsManager, key_text};
use pi_events::{ToolResultMessage, UserContent};
use pi_tui::{Component, Container, Spacer, Text};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionOptions {
    pub show_images: bool,
}

impl Default for ToolExecutionOptions {
    fn default() -> Self {
        Self { show_images: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecutionResult {
    pub content: Vec<UserContent>,
    pub details: Value,
    pub is_error: bool,
}

impl Default for ToolExecutionResult {
    fn default() -> Self {
        Self {
            content: Vec::new(),
            details: Value::Null,
            is_error: false,
        }
    }
}

impl From<ToolResultMessage> for ToolExecutionResult {
    fn from(message: ToolResultMessage) -> Self {
        Self {
            content: message.content,
            details: Value::Null,
            is_error: message.is_error,
        }
    }
}

pub struct ToolExecutionComponent {
    tool_name: String,
    tool_call_id: String,
    args: Value,
    expanded: bool,
    expand_key_text: String,
    show_images: bool,
    execution_started: bool,
    args_complete: bool,
    is_partial: bool,
    result: Option<ToolExecutionResult>,
    container: Container,
}

impl ToolExecutionComponent {
    pub fn new(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        args: Value,
        options: ToolExecutionOptions,
        keybindings: &KeybindingsManager,
    ) -> Self {
        let mut component = Self {
            tool_name: tool_name.into(),
            tool_call_id: tool_call_id.into(),
            args,
            expanded: false,
            expand_key_text: key_text(keybindings, "app.tools.expand"),
            show_images: options.show_images,
            execution_started: false,
            args_complete: false,
            is_partial: true,
            result: None,
            container: Container::new(),
        };
        component.rebuild();
        component
    }

    pub fn update_args(&mut self, args: Value) {
        self.args = args;
        self.rebuild();
    }

    pub fn mark_execution_started(&mut self) {
        self.execution_started = true;
        self.rebuild();
    }

    pub fn set_args_complete(&mut self) {
        self.args_complete = true;
        self.rebuild();
    }

    pub fn update_result(&mut self, result: ToolExecutionResult, is_partial: bool) {
        self.result = Some(result);
        self.is_partial = is_partial;
        self.rebuild();
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.rebuild();
    }

    pub fn set_show_images(&mut self, show_images: bool) {
        self.show_images = show_images;
        self.rebuild();
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn tool_call_id(&self) -> &str {
        &self.tool_call_id
    }

    fn rebuild(&mut self) {
        self.container.clear();
        self.container.add_child(Box::new(Spacer::new(1)));
        self.container
            .add_child(Box::new(Text::new(self.format_tool_execution(), 1, 1)));
    }

    fn format_tool_execution(&self) -> String {
        if let Some(built_in) = self.format_built_in_tool_execution() {
            return built_in;
        }

        let mut text = self.tool_name.clone();
        let formatted_args = format_json_value(&self.args);
        if !formatted_args.is_empty() {
            text.push_str("\n\n");
            text.push_str(&formatted_args);
        }

        let output = self.text_output();
        if !output.is_empty() {
            text.push('\n');
            text.push_str(&output);
        }

        text
    }

    fn format_built_in_tool_execution(&self) -> Option<String> {
        match self.tool_name.as_str() {
            "read" => Some(format_read_tool_execution(&self.args, self.text_output())),
            "write" => Some(format_write_tool_execution(
                &self.args,
                self.result
                    .as_ref()
                    .map(|result| result.is_error)
                    .unwrap_or(false),
                self.text_output(),
                self.expanded,
                &self.expand_key_text,
            )),
            "edit" => Some(format_edit_tool_execution(
                &self.args,
                self.result.as_ref(),
                self.text_output(),
            )),
            _ => None,
        }
    }

    fn text_output(&self) -> String {
        let Some(result) = &self.result else {
            return String::new();
        };

        let mut blocks = Vec::new();
        for block in &result.content {
            match block {
                UserContent::Text { text } => {
                    let normalized = text.replace('\r', "");
                    if !normalized.is_empty() {
                        blocks.push(normalized);
                    }
                }
                UserContent::Image { mime_type, .. } => {
                    if self.show_images {
                        blocks.push(image_fallback(mime_type));
                    } else {
                        blocks.push(image_fallback(mime_type));
                    }
                }
            }
        }

        blocks.join("\n")
    }
}

impl Component for ToolExecutionComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
        self.rebuild();
    }
}

fn format_json_value(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn format_read_tool_execution(args: &Value, output: String) -> String {
    let mut text = format!("read {}", path_arg(args).unwrap_or("..."));
    if let Some(range) = read_range_suffix(args) {
        text.push_str(&range);
    }

    let output = trim_trailing_empty_lines(&output);
    if !output.is_empty() {
        text.push_str("\n\n");
        text.push_str(&output);
    }

    text
}

const WRITE_COLLAPSED_PREVIEW_MAX_LINES: usize = 10;

fn format_write_tool_execution(
    args: &Value,
    is_error: bool,
    output: String,
    expanded: bool,
    expand_key_text: &str,
) -> String {
    let mut text = format!("write {}", path_arg(args).unwrap_or("..."));

    if let Some(content) = string_arg(args, &["content"]) {
        let preview = trim_trailing_empty_lines(&content.replace('\r', ""));
        if !preview.is_empty() {
            let preview_lines = preview.split('\n').collect::<Vec<_>>();
            let total_lines = preview_lines.len();
            let max_lines = if expanded {
                total_lines
            } else {
                total_lines.min(WRITE_COLLAPSED_PREVIEW_MAX_LINES)
            };
            let remaining = total_lines.saturating_sub(max_lines);
            text.push_str("\n\n");
            text.push_str(&preview_lines[..max_lines].join("\n"));
            if remaining > 0 {
                text.push_str(&format!(
                    "\n... ({remaining} more lines, {total_lines} total, {expand_key_text} to expand)"
                ));
            }
        }
    }

    if is_error {
        let output = trim_trailing_empty_lines(&output);
        if !output.is_empty() {
            text.push_str("\n\n");
            text.push_str(&output);
        }
    }

    text
}

fn format_edit_tool_execution(
    args: &Value,
    result: Option<&ToolExecutionResult>,
    output: String,
) -> String {
    let mut text = format!("edit {}", path_arg(args).unwrap_or("..."));

    if let Some(result) = result {
        if result.is_error {
            let output = trim_trailing_empty_lines(&output);
            if !output.is_empty() {
                text.push_str("\n\n");
                text.push_str(&output);
            }
        } else if let Some(diff) = result.details.get("diff").and_then(Value::as_str) {
            if !diff.is_empty() {
                text.push_str("\n\n");
                text.push_str(&render_diff(diff));
            }
        }
    }

    text
}

fn path_arg<'a>(args: &'a Value) -> Option<&'a str> {
    string_arg(args, &["path", "file_path"])
}

fn string_arg<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
}

fn number_arg(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|value| value as u64))
    })
}

fn read_range_suffix(args: &Value) -> Option<String> {
    let start_line = number_arg(args, "offset").unwrap_or(1);
    let limit = number_arg(args, "limit");

    if args.get("offset").is_none() && limit.is_none() {
        return None;
    }

    let end_line = limit.map(|limit| start_line + limit.saturating_sub(1));
    Some(match end_line {
        Some(end_line) => format!(":{start_line}-{end_line}"),
        None => format!(":{start_line}"),
    })
}

fn trim_trailing_empty_lines(text: &str) -> String {
    let mut lines = text.split('\n').collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_INVERSE_ON: &str = "\x1b[7m";
const ANSI_INVERSE_OFF: &str = "\x1b[27m";
const ANSI_DIFF_CONTEXT: &str = "\x1b[90m";
const ANSI_DIFF_REMOVED: &str = "\x1b[31m";
const ANSI_DIFF_ADDED: &str = "\x1b[32m";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedDiffLine {
    prefix: char,
    line_num: String,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WordDiffPart {
    Equal(String),
    Removed(String),
    Added(String),
}

fn render_diff(diff_text: &str) -> String {
    let mut rendered = Vec::new();
    let lines = diff_text.split('\n').collect::<Vec<_>>();
    let mut index = 0usize;

    while index < lines.len() {
        let Some(parsed) = parse_diff_line(lines[index]) else {
            rendered.push(colorize_diff_line(ANSI_DIFF_CONTEXT, lines[index]));
            index += 1;
            continue;
        };

        match parsed.prefix {
            '-' => {
                let mut removed_lines = Vec::new();
                while index < lines.len() {
                    let Some(line) = parse_diff_line(lines[index]) else {
                        break;
                    };
                    if line.prefix != '-' {
                        break;
                    }
                    removed_lines.push(line);
                    index += 1;
                }

                let mut added_lines = Vec::new();
                while index < lines.len() {
                    let Some(line) = parse_diff_line(lines[index]) else {
                        break;
                    };
                    if line.prefix != '+' {
                        break;
                    }
                    added_lines.push(line);
                    index += 1;
                }

                if removed_lines.len() == 1 && added_lines.len() == 1 {
                    let (removed_content, added_content) =
                        render_intra_line_diff(&removed_lines[0].content, &added_lines[0].content);
                    rendered.push(colorize_diff_line(
                        ANSI_DIFF_REMOVED,
                        &format!("-{} {}", removed_lines[0].line_num, removed_content),
                    ));
                    rendered.push(colorize_diff_line(
                        ANSI_DIFF_ADDED,
                        &format!("+{} {}", added_lines[0].line_num, added_content),
                    ));
                } else {
                    for line in removed_lines {
                        rendered.push(colorize_diff_line(
                            ANSI_DIFF_REMOVED,
                            &format!("-{} {}", line.line_num, replace_tabs(&line.content)),
                        ));
                    }
                    for line in added_lines {
                        rendered.push(colorize_diff_line(
                            ANSI_DIFF_ADDED,
                            &format!("+{} {}", line.line_num, replace_tabs(&line.content)),
                        ));
                    }
                }
            }
            '+' => {
                rendered.push(colorize_diff_line(
                    ANSI_DIFF_ADDED,
                    &format!("+{} {}", parsed.line_num, replace_tabs(&parsed.content)),
                ));
                index += 1;
            }
            _ => {
                rendered.push(colorize_diff_line(
                    ANSI_DIFF_CONTEXT,
                    &format!(" {} {}", parsed.line_num, replace_tabs(&parsed.content)),
                ));
                index += 1;
            }
        }
    }

    rendered.join("\n")
}

fn parse_diff_line(line: &str) -> Option<ParsedDiffLine> {
    let mut chars = line.chars();
    let prefix = chars.next()?;
    if !matches!(prefix, '+' | '-' | ' ') {
        return None;
    }

    let remainder = chars.as_str();
    let space_index = remainder.find(' ')?;
    let (line_num, content_with_space) = remainder.split_at(space_index);
    let content = content_with_space.strip_prefix(' ')?;

    if !line_num
        .chars()
        .all(|character| character.is_ascii_whitespace() || character.is_ascii_digit())
    {
        return None;
    }

    Some(ParsedDiffLine {
        prefix,
        line_num: line_num.to_owned(),
        content: content.to_owned(),
    })
}

fn replace_tabs(text: &str) -> String {
    text.replace('\t', "   ")
}

fn colorize_diff_line(color: &str, line: &str) -> String {
    format!("{color}{line}{ANSI_RESET}")
}

fn render_intra_line_diff(old_content: &str, new_content: &str) -> (String, String) {
    let old_tokens = tokenize_words(old_content);
    let new_tokens = tokenize_words(new_content);
    let diff = diff_word_tokens(&old_tokens, &new_tokens);

    let mut removed_line = String::new();
    let mut added_line = String::new();
    let mut is_first_removed = true;
    let mut is_first_added = true;

    for part in diff {
        match part {
            WordDiffPart::Equal(value) => {
                removed_line.push_str(&value);
                added_line.push_str(&value);
            }
            WordDiffPart::Removed(value) => {
                let mut value = value;
                if is_first_removed {
                    let leading_whitespace = value
                        .chars()
                        .take_while(|character| character.is_whitespace())
                        .collect::<String>();
                    removed_line.push_str(&leading_whitespace);
                    value = value[leading_whitespace.len()..].to_owned();
                    is_first_removed = false;
                }
                if !value.is_empty() {
                    removed_line.push_str(ANSI_INVERSE_ON);
                    removed_line.push_str(&value);
                    removed_line.push_str(ANSI_INVERSE_OFF);
                }
            }
            WordDiffPart::Added(value) => {
                let mut value = value;
                if is_first_added {
                    let leading_whitespace = value
                        .chars()
                        .take_while(|character| character.is_whitespace())
                        .collect::<String>();
                    added_line.push_str(&leading_whitespace);
                    value = value[leading_whitespace.len()..].to_owned();
                    is_first_added = false;
                }
                if !value.is_empty() {
                    added_line.push_str(ANSI_INVERSE_ON);
                    added_line.push_str(&value);
                    added_line.push_str(ANSI_INVERSE_OFF);
                }
            }
        }
    }

    (replace_tabs(&removed_line), replace_tabs(&added_line))
}

fn tokenize_words(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let characters = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < characters.len() {
        if characters[index].is_whitespace() {
            let start = index;
            while index < characters.len() && characters[index].is_whitespace() {
                index += 1;
            }
            if tokens.is_empty() {
                tokens.push(characters[start..index].iter().collect());
            } else if let Some(last) = tokens.last_mut() {
                last.push_str(&characters[start..index].iter().collect::<String>());
            }
            continue;
        }

        let start = index;
        while index < characters.len() && !characters[index].is_whitespace() {
            index += 1;
        }
        tokens.push(characters[start..index].iter().collect());
    }

    tokens
}

fn diff_word_tokens(old_tokens: &[String], new_tokens: &[String]) -> Vec<WordDiffPart> {
    let mut lcs = vec![vec![0usize; new_tokens.len() + 1]; old_tokens.len() + 1];
    for old_index in (0..old_tokens.len()).rev() {
        for new_index in (0..new_tokens.len()).rev() {
            lcs[old_index][new_index] = if old_tokens[old_index] == new_tokens[new_index] {
                lcs[old_index + 1][new_index + 1] + 1
            } else {
                lcs[old_index + 1][new_index].max(lcs[old_index][new_index + 1])
            };
        }
    }

    let mut parts = Vec::new();
    let mut old_index = 0usize;
    let mut new_index = 0usize;
    while old_index < old_tokens.len() && new_index < new_tokens.len() {
        if old_tokens[old_index] == new_tokens[new_index] {
            parts.push(WordDiffPart::Equal(old_tokens[old_index].clone()));
            old_index += 1;
            new_index += 1;
        } else if lcs[old_index + 1][new_index] >= lcs[old_index][new_index + 1] {
            parts.push(WordDiffPart::Removed(old_tokens[old_index].clone()));
            old_index += 1;
        } else {
            parts.push(WordDiffPart::Added(new_tokens[new_index].clone()));
            new_index += 1;
        }
    }

    while old_index < old_tokens.len() {
        parts.push(WordDiffPart::Removed(old_tokens[old_index].clone()));
        old_index += 1;
    }
    while new_index < new_tokens.len() {
        parts.push(WordDiffPart::Added(new_tokens[new_index].clone()));
        new_index += 1;
    }

    parts
}

fn image_fallback(mime_type: &str) -> String {
    format!("[Image: [{mime_type}]]")
}
