use crate::{KeybindingsManager, current_theme, key_text};
use pi_events::{ToolResultMessage, UserContent};
use pi_tui::{Component, Container, Spacer, Text};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolRenderResultOptions {
    pub expanded: bool,
    pub is_partial: bool,
}

pub struct ToolRenderContext<'a, TState> {
    pub args: &'a Value,
    pub tool_call_id: &'a str,
    pub state: &'a mut TState,
    pub execution_started: bool,
    pub args_complete: bool,
    pub is_partial: bool,
    pub expanded: bool,
    pub show_images: bool,
    pub is_error: bool,
}

pub type ToolRenderCallFn<TState> =
    dyn for<'a> Fn(&'a Value, ToolRenderContext<'a, TState>) -> Box<dyn Component> + Send + Sync;
pub type ToolRenderResultFn<TState> = dyn for<'a> Fn(
        &'a ToolExecutionResult,
        ToolRenderResultOptions,
        ToolRenderContext<'a, TState>,
    ) -> Box<dyn Component>
    + Send
    + Sync;

#[derive(Clone)]
pub struct ToolExecutionRendererDefinition<TState = ()> {
    render_call: Option<Arc<ToolRenderCallFn<TState>>>,
    render_result: Option<Arc<ToolRenderResultFn<TState>>>,
}

impl<TState> Default for ToolExecutionRendererDefinition<TState> {
    fn default() -> Self {
        Self {
            render_call: None,
            render_result: None,
        }
    }
}

impl<TState> ToolExecutionRendererDefinition<TState> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_render_call<F>(mut self, render_call: F) -> Self
    where
        F: for<'a> Fn(&'a Value, ToolRenderContext<'a, TState>) -> Box<dyn Component>
            + Send
            + Sync
            + 'static,
    {
        self.render_call = Some(Arc::new(render_call));
        self
    }

    pub fn with_render_result<F>(mut self, render_result: F) -> Self
    where
        F: for<'a> Fn(
                &'a ToolExecutionResult,
                ToolRenderResultOptions,
                ToolRenderContext<'a, TState>,
            ) -> Box<dyn Component>
            + Send
            + Sync
            + 'static,
    {
        self.render_result = Some(Arc::new(render_result));
        self
    }
}

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
            details: message.details.unwrap_or(Value::Null),
            is_error: message.is_error,
        }
    }
}

pub struct ToolExecutionComponent<TState = ()> {
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
    renderer_definition: Option<ToolExecutionRendererDefinition<TState>>,
    renderer_state: TState,
    container: Container,
}

impl ToolExecutionComponent<()> {
    pub fn new(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        args: Value,
        options: ToolExecutionOptions,
        keybindings: &KeybindingsManager,
    ) -> Self {
        Self::new_with_definition(
            tool_name,
            tool_call_id,
            args,
            options,
            keybindings,
            None,
            (),
        )
    }
}

impl<TState> ToolExecutionComponent<TState> {
    pub fn new_with_definition(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        args: Value,
        options: ToolExecutionOptions,
        keybindings: &KeybindingsManager,
        renderer_definition: Option<ToolExecutionRendererDefinition<TState>>,
        renderer_state: TState,
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
            renderer_definition,
            renderer_state,
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

        if self.uses_renderer_composition() {
            let mut content = Container::new();
            let mut has_content = false;

            if let Some(call_component) = self.build_call_component() {
                content.add_child(call_component);
                has_content = true;
            }
            if let Some(result_component) = self.build_result_component() {
                content.add_child(result_component);
                has_content = true;
            }

            if has_content {
                self.container.add_child(Box::new(content));
            }
        } else {
            self.container
                .add_child(Box::new(Text::new(self.format_tool_execution(), 1, 1)));
        }
    }

    fn uses_renderer_composition(&self) -> bool {
        self.has_built_in_renderers() || self.renderer_definition.is_some()
    }

    fn has_built_in_renderers(&self) -> bool {
        matches!(self.tool_name.as_str(), "read" | "write" | "edit")
    }

    fn build_call_component(&mut self) -> Option<Box<dyn Component>> {
        let custom_renderer = self
            .renderer_definition
            .as_ref()
            .and_then(|definition| definition.render_call.clone());
        if let Some(render_call) = custom_renderer {
            let args = self.args.clone();
            return Some(render_call(&args, self.render_context_with_args(&args)));
        }

        if let Some(component) = self.build_built_in_call_component() {
            return Some(component);
        }

        if self.renderer_definition.is_some() {
            return Some(self.create_call_fallback());
        }

        None
    }

    fn build_result_component(&mut self) -> Option<Box<dyn Component>> {
        let result = self.result.clone()?;

        let custom_renderer = self
            .renderer_definition
            .as_ref()
            .and_then(|definition| definition.render_result.clone());
        if let Some(render_result) = custom_renderer {
            let args = self.args.clone();
            return Some(render_result(
                &result,
                ToolRenderResultOptions {
                    expanded: self.expanded,
                    is_partial: self.is_partial,
                },
                self.render_context_with_args(&args),
            ));
        }

        if let Some(component) = self.build_built_in_result_component(&result) {
            return Some(component);
        }

        if self.renderer_definition.is_some() {
            return self.create_result_fallback();
        }

        None
    }

    fn render_context_with_args<'a>(
        &'a mut self,
        args: &'a Value,
    ) -> ToolRenderContext<'a, TState> {
        let is_error = self
            .result
            .as_ref()
            .map(|result| result.is_error)
            .unwrap_or(false);
        ToolRenderContext {
            args,
            tool_call_id: &self.tool_call_id,
            state: &mut self.renderer_state,
            execution_started: self.execution_started,
            args_complete: self.args_complete,
            is_partial: self.is_partial,
            expanded: self.expanded,
            show_images: self.show_images,
            is_error,
        }
    }

    fn build_built_in_call_component(&self) -> Option<Box<dyn Component>> {
        let text = match self.tool_name.as_str() {
            "read" => format_read_call(&self.args),
            "write" => format_write_call(&self.args, self.expanded, &self.expand_key_text),
            "edit" => format_edit_call(&self.args),
            _ => return None,
        };
        Some(Box::new(Text::new(
            current_theme().fg("toolTitle", text),
            0,
            0,
        )))
    }

    fn build_built_in_result_component(
        &self,
        result: &ToolExecutionResult,
    ) -> Option<Box<dyn Component>> {
        let text = match self.tool_name.as_str() {
            "read" => format_read_result(self.text_output()),
            "write" => format_write_result(result.is_error, self.text_output()),
            "edit" => format_edit_result(result, self.text_output()),
            _ => return None,
        }?;
        Some(Box::new(Text::new(
            current_theme().fg("toolOutput", text),
            0,
            0,
        )))
    }

    fn create_call_fallback(&self) -> Box<dyn Component> {
        Box::new(Text::new(
            current_theme().fg("toolTitle", &self.tool_name),
            0,
            0,
        ))
    }

    fn create_result_fallback(&self) -> Option<Box<dyn Component>> {
        let output = self.text_output();
        if output.is_empty() {
            return None;
        }
        Some(Box::new(Text::new(
            current_theme().fg("toolOutput", output),
            0,
            0,
        )))
    }

    fn format_tool_execution(&self) -> String {
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

impl<TState> Component for ToolExecutionComponent<TState> {
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

fn format_read_call(args: &Value) -> String {
    let mut text = format!("read {}", path_arg(args).unwrap_or("..."));
    if let Some(range) = read_range_suffix(args) {
        text.push_str(&range);
    }
    text
}

fn format_read_result(output: String) -> Option<String> {
    let output = trim_trailing_empty_lines(&output);
    if output.is_empty() {
        None
    } else {
        Some(format!("\n{output}"))
    }
}

const WRITE_COLLAPSED_PREVIEW_MAX_LINES: usize = 10;

fn format_write_call(args: &Value, expanded: bool, expand_key_text: &str) -> String {
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

    text
}

fn format_write_result(is_error: bool, output: String) -> Option<String> {
    if !is_error {
        return None;
    }

    let output = trim_trailing_empty_lines(&output);
    if output.is_empty() {
        None
    } else {
        Some(format!("\n{output}"))
    }
}

fn format_edit_call(args: &Value) -> String {
    format!("edit {}", path_arg(args).unwrap_or("..."))
}

fn format_edit_result(result: &ToolExecutionResult, output: String) -> Option<String> {
    if result.is_error {
        let output = trim_trailing_empty_lines(&output);
        if output.is_empty() {
            return None;
        }
        return Some(format!("\n{output}"));
    }

    let diff = result.details.get("diff").and_then(Value::as_str)?;
    if diff.is_empty() {
        None
    } else {
        Some(format!("\n{}", render_diff(diff)))
    }
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
    let theme = current_theme();
    let mut rendered = Vec::new();
    let lines = diff_text.split('\n').collect::<Vec<_>>();
    let mut index = 0usize;

    while index < lines.len() {
        let Some(parsed) = parse_diff_line(lines[index]) else {
            rendered.push(colorize_diff_line(
                theme.fg_code("toolDiffContext"),
                lines[index],
            ));
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
                        theme.fg_code("toolDiffRemoved"),
                        &format!("-{} {}", removed_lines[0].line_num, removed_content),
                    ));
                    rendered.push(colorize_diff_line(
                        theme.fg_code("toolDiffAdded"),
                        &format!("+{} {}", added_lines[0].line_num, added_content),
                    ));
                } else {
                    for line in removed_lines {
                        rendered.push(colorize_diff_line(
                            theme.fg_code("toolDiffRemoved"),
                            &format!("-{} {}", line.line_num, replace_tabs(&line.content)),
                        ));
                    }
                    for line in added_lines {
                        rendered.push(colorize_diff_line(
                            theme.fg_code("toolDiffAdded"),
                            &format!("+{} {}", line.line_num, replace_tabs(&line.content)),
                        ));
                    }
                }
            }
            '+' => {
                rendered.push(colorize_diff_line(
                    theme.fg_code("toolDiffAdded"),
                    &format!("+{} {}", parsed.line_num, replace_tabs(&parsed.content)),
                ));
                index += 1;
            }
            _ => {
                rendered.push(colorize_diff_line(
                    theme.fg_code("toolDiffContext"),
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
