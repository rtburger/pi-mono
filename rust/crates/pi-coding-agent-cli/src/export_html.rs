use pi_agent::AgentMessage;
use pi_coding_agent_core::{CustomMessageContent, SessionEntry, SessionManager};
use pi_events::{AssistantContent, Message, StopReason, UserContent};
use serde_json::Value;
use std::{fs, path::Path};

const APP_NAME: &str = "pi";

pub fn default_html_file_name(session_manager: &SessionManager) -> String {
    let stem = session_manager
        .get_session_file()
        .and_then(|path| Path::new(path).file_stem())
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| session_manager.get_session_id().to_owned());
    format!("{APP_NAME}-session-{stem}.html")
}

pub fn export_session_to_html(
    session_manager: &SessionManager,
    output_path: impl AsRef<Path>,
) -> Result<String, String> {
    let output_path = output_path.as_ref();
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }

    let html = render_session_html(session_manager);
    fs::write(output_path, html).map_err(|error| format!("{}: {error}", output_path.display()))?;
    Ok(output_path.to_string_lossy().into_owned())
}

pub fn render_session_html(session_manager: &SessionManager) -> String {
    let branch = session_manager.get_branch(session_manager.get_leaf_id());
    let session_name = session_manager.get_session_name();
    let mut html = String::new();

    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str("  <title>Pi Session Export</title>\n");
    html.push_str("  <style>\n");
    html.push_str(STYLES);
    html.push_str("\n  </style>\n</head>\n<body>\n");
    html.push_str("  <main class=\"page\">\n");
    html.push_str("    <header class=\"hero\">\n");
    html.push_str("      <div class=\"hero-label\">Pi Session Export</div>\n");
    html.push_str("      <h1>");
    html.push_str(&escape_text(
        session_name.as_deref().unwrap_or("Untitled session"),
    ));
    html.push_str("</h1>\n");
    html.push_str("      <p class=\"hero-subtitle\">Current branch snapshot rendered by the Rust migration.</p>\n");
    html.push_str("      <dl class=\"meta\">\n");
    append_meta_item(&mut html, "Session ID", session_manager.get_session_id());
    append_meta_item(&mut html, "Working directory", session_manager.get_cwd());
    append_meta_item(&mut html, "Entries", &branch.len().to_string());
    append_meta_item(
        &mut html,
        "Leaf",
        session_manager.get_leaf_id().unwrap_or("root"),
    );
    html.push_str("      </dl>\n");
    html.push_str("    </header>\n");

    if branch.is_empty() {
        html.push_str("    <section class=\"entry entry-empty\">\n");
        html.push_str("      <p>No entries in this session yet.</p>\n");
        html.push_str("    </section>\n");
    } else {
        html.push_str("    <section class=\"entries\">\n");
        for entry in &branch {
            html.push_str(&render_entry(entry));
        }
        html.push_str("    </section>\n");
    }

    html.push_str("  </main>\n</body>\n</html>\n");
    html
}

fn append_meta_item(html: &mut String, label: &str, value: &str) {
    html.push_str("        <div class=\"meta-item\"><dt>");
    html.push_str(&escape_text(label));
    html.push_str("</dt><dd>");
    html.push_str(&escape_text(value));
    html.push_str("</dd></div>\n");
}

fn render_entry(entry: &SessionEntry) -> String {
    match entry {
        SessionEntry::Message {
            timestamp, message, ..
        } => render_message_entry(timestamp, message),
        SessionEntry::ThinkingLevelChange {
            timestamp,
            thinking_level,
            ..
        } => render_info_entry(
            "entry-thinking-level",
            "Thinking level",
            timestamp,
            &render_plain_text_block(thinking_level),
        ),
        SessionEntry::ModelChange {
            timestamp,
            provider,
            model_id,
            ..
        } => render_info_entry(
            "entry-model-change",
            "Model change",
            timestamp,
            &render_plain_text_block(&format!("{provider}/{model_id}")),
        ),
        SessionEntry::Compaction {
            timestamp,
            summary,
            first_kept_entry_id,
            tokens_before,
            details,
            ..
        } => {
            let mut body = String::new();
            body.push_str(&render_plain_text_block(summary));
            body.push_str("<div class=\"pill-row\">");
            body.push_str(&render_pill(&format!(
                "First kept entry: {first_kept_entry_id}"
            )));
            body.push_str(&render_pill(&format!("Tokens before: {tokens_before}")));
            body.push_str("</div>");
            if let Some(details) = details {
                body.push_str(&render_json_details("Compaction details", details));
            }
            render_info_entry("entry-compaction", "Compaction", timestamp, &body)
        }
        SessionEntry::BranchSummary {
            timestamp,
            from_id,
            summary,
            details,
            ..
        } => {
            let mut body = String::new();
            body.push_str(&render_plain_text_block(summary));
            body.push_str("<div class=\"pill-row\">");
            body.push_str(&render_pill(&format!("From: {from_id}")));
            body.push_str("</div>");
            if let Some(details) = details {
                body.push_str(&render_json_details("Branch summary details", details));
            }
            render_info_entry("entry-branch-summary", "Branch summary", timestamp, &body)
        }
        SessionEntry::Custom {
            timestamp,
            custom_type,
            data,
            ..
        } => {
            let mut body = String::new();
            if let Some(data) = data {
                body.push_str(&render_json_details(custom_type, data));
            } else {
                body.push_str(&render_plain_text_block("No payload"));
            }
            render_info_entry("entry-custom", custom_type, timestamp, &body)
        }
        SessionEntry::CustomMessage {
            timestamp,
            custom_type,
            content,
            details,
            display,
            ..
        } => {
            if !display {
                return String::new();
            }
            let mut body = render_custom_message_content(content);
            if let Some(details) = details {
                body.push_str(&render_json_details("Details", details));
            }
            render_info_entry("entry-custom-message", custom_type, timestamp, &body)
        }
        SessionEntry::Label {
            timestamp,
            target_id,
            label,
            ..
        } => render_info_entry(
            "entry-label",
            "Label",
            timestamp,
            &render_plain_text_block(&format!(
                "{} → {}",
                target_id,
                label.as_deref().unwrap_or("(removed)")
            )),
        ),
        SessionEntry::SessionInfo {
            timestamp, name, ..
        } => render_info_entry(
            "entry-session-info",
            "Session info",
            timestamp,
            &render_plain_text_block(name.as_deref().unwrap_or("(unnamed)")),
        ),
    }
}

fn render_message_entry(timestamp: &str, message: &AgentMessage) -> String {
    match message {
        AgentMessage::Standard(standard) => render_standard_message(timestamp, standard),
        AgentMessage::Custom(custom) => render_info_entry(
            "entry-custom-message",
            custom.role.as_str(),
            timestamp,
            &render_json_details("Payload", &custom.payload),
        ),
    }
}

fn render_standard_message(timestamp: &str, message: &Message) -> String {
    match message {
        Message::User { content, .. } => render_role_entry(
            "entry-user",
            "User",
            timestamp,
            &render_user_content(content),
            None,
        ),
        Message::Assistant {
            content,
            provider,
            model,
            usage,
            stop_reason,
            error_message,
            ..
        } => {
            let mut body = String::new();
            for block in content {
                body.push_str(&render_assistant_block(block));
            }
            if let Some(error_message) = error_message.as_deref() {
                body.push_str(&format!(
                    "<div class=\"callout error\"><strong>Error</strong><pre>{}</pre></div>",
                    escape_text(error_message)
                ));
            }

            let mut footer_parts = vec![format!("{provider}/{model}")];
            if usage.total_tokens > 0 {
                footer_parts.push(format!("{} total tokens", usage.total_tokens));
            }
            if *stop_reason != StopReason::Stop {
                footer_parts.push(format!("stop: {}", stop_reason_label(stop_reason)));
            }
            render_role_entry(
                "entry-assistant",
                "Assistant",
                timestamp,
                &body,
                Some(&footer_parts.join(" · ")),
            )
        }
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            details,
            is_error,
            ..
        } => {
            let mut body = String::new();
            body.push_str(&render_user_content(content));
            if let Some(details) = details {
                body.push_str(&render_json_details("Tool result details", details));
            }
            body.push_str("<div class=\"pill-row\">");
            body.push_str(&render_pill(&format!("Tool: {tool_name}")));
            body.push_str(&render_pill(&format!("Call ID: {tool_call_id}")));
            if *is_error {
                body.push_str(&render_pill("Result: error"));
            }
            body.push_str("</div>");
            render_role_entry(
                if *is_error {
                    "entry-tool-result entry-tool-result-error"
                } else {
                    "entry-tool-result"
                },
                "Tool result",
                timestamp,
                &body,
                None,
            )
        }
    }
}

fn render_role_entry(
    class_name: &str,
    title: &str,
    timestamp: &str,
    body: &str,
    footer: Option<&str>,
) -> String {
    let mut html = String::new();
    html.push_str("      <article class=\"entry ");
    html.push_str(class_name);
    html.push_str("\">\n");
    html.push_str("        <header class=\"entry-header\">\n");
    html.push_str("          <div class=\"entry-title\">");
    html.push_str(&escape_text(title));
    html.push_str("</div>\n");
    html.push_str("          <time class=\"entry-time\">");
    html.push_str(&escape_text(timestamp));
    html.push_str("</time>\n");
    html.push_str("        </header>\n");
    html.push_str("        <div class=\"entry-body\">\n");
    html.push_str(body);
    html.push_str("        </div>\n");
    if let Some(footer) = footer {
        html.push_str("        <footer class=\"entry-footer\">");
        html.push_str(&escape_text(footer));
        html.push_str("</footer>\n");
    }
    html.push_str("      </article>\n");
    html
}

fn render_info_entry(class_name: &str, title: &str, timestamp: &str, body: &str) -> String {
    render_role_entry(class_name, title, timestamp, body, None)
}

fn render_user_content(content: &[UserContent]) -> String {
    let mut html = String::new();
    for block in content {
        match block {
            UserContent::Text { text } => html.push_str(&render_plain_text_block(text)),
            UserContent::Image { data, mime_type } => {
                html.push_str(&render_image_block(data, mime_type))
            }
        }
    }
    html
}

fn render_custom_message_content(content: &CustomMessageContent) -> String {
    match content {
        CustomMessageContent::Text(text) => render_plain_text_block(text),
        CustomMessageContent::Blocks(blocks) => render_user_content(blocks),
    }
}

fn render_assistant_block(block: &AssistantContent) -> String {
    match block {
        AssistantContent::Text { text, .. } => render_plain_text_block(text),
        AssistantContent::Thinking {
            thinking, redacted, ..
        } => {
            let summary = if *redacted {
                "Thinking (redacted)"
            } else {
                "Thinking"
            };
            format!(
                "<details class=\"details\"><summary>{}</summary><pre>{}</pre></details>",
                escape_text(summary),
                escape_text(thinking)
            )
        }
        AssistantContent::ToolCall {
            id,
            name,
            arguments,
            ..
        } => {
            let arguments =
                serde_json::to_string_pretty(arguments).unwrap_or_else(|_| String::from("{}"));
            let summary = format!("Tool call: {name} ({id})");
            format!(
                "<details class=\"details\"><summary>{}</summary><pre>{}</pre></details>",
                escape_text(&summary),
                escape_text(&arguments)
            )
        }
    }
}

fn render_plain_text_block(text: &str) -> String {
    format!("<pre>{}</pre>", escape_text(text))
}

fn render_json_details(label: &str, value: &Value) -> String {
    let json = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    format!(
        "<details class=\"details\"><summary>{}</summary><pre>{}</pre></details>",
        escape_text(label),
        escape_text(&json)
    )
}

fn render_pill(text: &str) -> String {
    format!("<span class=\"pill\">{}</span>", escape_text(text))
}

fn render_image_block(data: &str, mime_type: &str) -> String {
    format!(
        "<figure class=\"image-block\"><img src=\"data:{};base64,{}\" alt=\"Embedded image\"></figure>",
        escape_attribute(mime_type),
        escape_attribute(data)
    )
}

fn stop_reason_label(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::Stop => "stop",
        StopReason::Length => "length",
        StopReason::ToolUse => "tool_use",
        StopReason::Error => "error",
        StopReason::Aborted => "aborted",
    }
}

fn escape_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_attribute(value: &str) -> String {
    escape_text(value)
}

const STYLES: &str = r#"
:root {
  color-scheme: dark;
  --bg: #0d1117;
  --panel: #161b22;
  --panel-border: #30363d;
  --panel-muted: #21262d;
  --text: #e6edf3;
  --muted: #8b949e;
  --accent: #58a6ff;
  --success: #3fb950;
  --warning: #d29922;
  --error: #f85149;
  --user: #1f6feb;
  --assistant: #238636;
  --tool: #8957e5;
  --shadow: rgba(0, 0, 0, 0.25);
}

* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  background: linear-gradient(180deg, #0b0f14 0%, var(--bg) 100%);
  color: var(--text);
  font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
}

.page {
  width: min(1100px, calc(100vw - 32px));
  margin: 0 auto;
  padding: 32px 0 48px;
}

.hero {
  background: rgba(22, 27, 34, 0.94);
  border: 1px solid var(--panel-border);
  border-radius: 18px;
  padding: 24px;
  box-shadow: 0 16px 40px var(--shadow);
  margin-bottom: 24px;
}

.hero-label {
  color: var(--accent);
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  margin-bottom: 8px;
}

.hero h1 {
  margin: 0;
  font-size: 30px;
  line-height: 1.2;
}

.hero-subtitle {
  margin: 10px 0 0;
  color: var(--muted);
}

.meta {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 12px;
  margin: 24px 0 0;
}

.meta-item {
  background: var(--panel-muted);
  border: 1px solid var(--panel-border);
  border-radius: 12px;
  padding: 12px 14px;
}

.meta-item dt {
  color: var(--muted);
  font-size: 12px;
  margin-bottom: 6px;
}

.meta-item dd {
  margin: 0;
  word-break: break-word;
}

.entries {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.entry {
  background: rgba(22, 27, 34, 0.94);
  border: 1px solid var(--panel-border);
  border-radius: 16px;
  padding: 18px;
  box-shadow: 0 12px 28px var(--shadow);
}

.entry-empty {
  text-align: center;
  color: var(--muted);
}

.entry-user { border-left: 4px solid var(--user); }
.entry-assistant { border-left: 4px solid var(--assistant); }
.entry-tool-result { border-left: 4px solid var(--tool); }
.entry-tool-result-error { border-left-color: var(--error); }
.entry-compaction { border-left: 4px solid var(--warning); }
.entry-branch-summary { border-left: 4px solid #a371f7; }
.entry-model-change,
.entry-thinking-level,
.entry-session-info,
.entry-label,
.entry-custom,
.entry-custom-message { border-left: 4px solid var(--accent); }

.entry-header {
  display: flex;
  gap: 12px;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 14px;
}

.entry-title {
  font-size: 13px;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--accent);
}

.entry-time {
  color: var(--muted);
  font-size: 12px;
}

.entry-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.entry-footer {
  margin-top: 16px;
  padding-top: 12px;
  border-top: 1px solid var(--panel-border);
  color: var(--muted);
  font-size: 12px;
}

pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  background: #0b0f14;
  border: 1px solid var(--panel-border);
  border-radius: 12px;
  padding: 12px 14px;
  overflow-x: auto;
}

.details {
  border: 1px solid var(--panel-border);
  border-radius: 12px;
  background: #0b0f14;
  overflow: hidden;
}

.details summary {
  cursor: pointer;
  padding: 12px 14px;
  color: var(--muted);
  list-style: none;
}

.details summary::-webkit-details-marker {
  display: none;
}

.details[open] summary {
  border-bottom: 1px solid var(--panel-border);
}

.details pre {
  border: 0;
  border-radius: 0;
}

.pill-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-radius: 999px;
  background: var(--panel-muted);
  border: 1px solid var(--panel-border);
  color: var(--muted);
  font-size: 12px;
}

.image-block {
  margin: 0;
  padding: 0;
}

.image-block img {
  display: block;
  max-width: min(100%, 720px);
  max-height: 480px;
  border-radius: 12px;
  border: 1px solid var(--panel-border);
  background: #0b0f14;
}

.callout.error {
  border: 1px solid color-mix(in srgb, var(--error) 45%, var(--panel-border));
  background: color-mix(in srgb, var(--error) 10%, #0b0f14);
  border-radius: 12px;
  padding: 12px;
}

.callout.error strong {
  display: block;
  color: var(--error);
  margin-bottom: 8px;
}

@media (max-width: 720px) {
  .page {
    width: min(100vw - 20px, 1100px);
    padding-top: 20px;
  }

  .hero,
  .entry {
    padding: 14px;
    border-radius: 14px;
  }

  .entry-header {
    flex-direction: column;
    align-items: flex-start;
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::{default_html_file_name, export_session_to_html, render_session_html};
    use pi_coding_agent_core::SessionManager;
    use pi_events::{AssistantContent, Message, Usage, UserContent};
    use serde_json::json;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pi-export-html-{prefix}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn default_html_file_name_falls_back_to_session_id_for_in_memory_sessions() {
        let manager = SessionManager::in_memory("/tmp/pi-export-html-name");
        let file_name = default_html_file_name(&manager);
        assert!(
            file_name.starts_with("pi-session-"),
            "file_name: {file_name}"
        );
        assert!(file_name.ends_with(".html"), "file_name: {file_name}");
        assert!(
            file_name.contains(manager.get_session_id()),
            "file_name: {file_name}"
        );
    }

    #[test]
    fn render_session_html_includes_messages_and_embedded_images() {
        let mut manager = SessionManager::in_memory("/tmp/pi-export-html-render");
        manager.append_session_info("demo session").unwrap();
        manager
            .append_message(Message::User {
                content: vec![
                    UserContent::Text {
                        text: String::from("hello from user"),
                    },
                    UserContent::Image {
                        data: String::from("aGVsbG8="),
                        mime_type: String::from("image/png"),
                    },
                ],
                timestamp: 1,
            })
            .unwrap();
        manager
            .append_message(Message::Assistant {
                content: vec![
                    AssistantContent::Text {
                        text: String::from("hello from assistant"),
                        text_signature: None,
                    },
                    AssistantContent::Thinking {
                        thinking: String::from("private reasoning"),
                        thinking_signature: None,
                        redacted: false,
                    },
                    AssistantContent::ToolCall {
                        id: String::from("tool-1"),
                        name: String::from("read"),
                        arguments: serde_json::from_value(json!({"path": "README.md"})).unwrap(),
                        thought_signature: None,
                    },
                ],
                api: String::from("faux:test"),
                provider: String::from("faux"),
                model: String::from("model-1"),
                response_id: None,
                usage: Usage {
                    total_tokens: 42,
                    ..Usage::default()
                },
                stop_reason: pi_events::StopReason::Stop,
                error_message: None,
                timestamp: 2,
            })
            .unwrap();

        let html = render_session_html(&manager);
        assert!(html.contains("Pi Session Export"), "html: {html}");
        assert!(html.contains("demo session"), "html: {html}");
        assert!(html.contains("hello from user"), "html: {html}");
        assert!(html.contains("hello from assistant"), "html: {html}");
        assert!(html.contains("private reasoning"), "html: {html}");
        assert!(html.contains("Tool call: read (tool-1)"), "html: {html}");
        assert!(
            html.contains("data:image/png;base64,aGVsbG8="),
            "html: {html}"
        );
    }

    #[test]
    fn export_session_to_html_writes_file() {
        let cwd = unique_temp_dir("write");
        let mut manager = SessionManager::in_memory(cwd.to_str().unwrap());
        manager
            .append_message(Message::User {
                content: vec![UserContent::Text {
                    text: String::from("export this"),
                }],
                timestamp: 1,
            })
            .unwrap();
        let output_path = cwd.join("session.html");

        let exported_path = export_session_to_html(&manager, &output_path).unwrap();
        let html = fs::read_to_string(&output_path).unwrap();

        assert_eq!(exported_path, output_path.to_string_lossy());
        assert!(html.contains("export this"), "html: {html}");
        assert!(html.contains("Current branch snapshot"), "html: {html}");
    }
}
