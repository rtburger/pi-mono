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
            "edit" => Some(format_edit_tool_execution(&self.args, self.text_output())),
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

fn format_edit_tool_execution(args: &Value, output: String) -> String {
    let mut text = format!("edit {}", path_arg(args).unwrap_or("..."));
    let output = trim_trailing_empty_lines(&output);
    if !output.is_empty() {
        text.push_str("\n\n");
        text.push_str(&output);
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

fn image_fallback(mime_type: &str) -> String {
    format!("[Image: [{mime_type}]]")
}
