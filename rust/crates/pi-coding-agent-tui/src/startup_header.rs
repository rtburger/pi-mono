use crate::{KeyHintStyler, KeybindingsManager, key_hint, key_text, raw_key_hint};
use pi_tui::{Component, wrap_text_with_ansi};

const ONBOARDING_TEXT: &str =
    "Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.";
const CODE_BLOCK_INDENT: &str = "  ";
const HORIZONTAL_RULE_WIDTH: usize = 40;

pub trait StartupHeaderStyler: KeyHintStyler {
    fn accent_bold(&self, text: &str) -> String;

    fn bold(&self, text: &str) -> String {
        text.to_owned()
    }

    fn border(&self, text: &str) -> String {
        text.to_owned()
    }

    fn heading(&self, text: &str) -> String {
        self.bold(text)
    }

    fn link(&self, text: &str) -> String {
        text.to_owned()
    }

    fn link_url(&self, text: &str) -> String {
        text.to_owned()
    }

    fn code(&self, text: &str) -> String {
        text.to_owned()
    }

    fn code_block(&self, text: &str) -> String {
        text.to_owned()
    }

    fn code_block_border(&self, text: &str) -> String {
        text.to_owned()
    }

    fn list_bullet(&self, text: &str) -> String {
        text.to_owned()
    }

    fn strikethrough(&self, text: &str) -> String {
        text.to_owned()
    }
}

impl StartupHeaderStyler for crate::PlainKeyHintStyler {
    fn accent_bold(&self, text: &str) -> String {
        text.to_owned()
    }
}

pub fn build_startup_header_text(
    app_name: &str,
    version: &str,
    keybindings: &KeybindingsManager,
    styler: &impl StartupHeaderStyler,
    quiet: bool,
) -> String {
    if quiet {
        return String::new();
    }

    let logo = styler.accent_bold(app_name) + &styler.dim(&format!(" v{version}"));

    let hint = |keybinding: &str, description: &str| {
        key_hint(keybindings, styler, keybinding, description)
    };

    let instructions = [
        hint("app.interrupt", "to interrupt"),
        hint("app.clear", "to clear"),
        raw_key_hint(
            styler,
            &format!("{} twice", key_text(keybindings, "app.clear")),
            "to exit",
        ),
        hint("app.exit", "to exit (empty)"),
        hint("tui.editor.deleteToLineEnd", "to delete to end"),
        hint("app.thinking.cycle", "to cycle thinking level"),
        raw_key_hint(
            styler,
            &format!(
                "{}/{}",
                key_text(keybindings, "app.model.cycleForward"),
                key_text(keybindings, "app.model.cycleBackward")
            ),
            "to cycle models",
        ),
        hint("app.model.select", "to select model"),
        hint("app.tools.expand", "to toggle tool output"),
        hint("app.thinking.toggle", "to toggle thinking"),
        hint("app.editor.external", "for external editor"),
        raw_key_hint(styler, "/", "for commands"),
        raw_key_hint(styler, "!", "to run bash"),
        raw_key_hint(styler, "!!", "to run bash (no context)"),
        hint("app.message.followUp", "to queue follow-up"),
        hint("app.message.dequeue", "to edit all queued messages"),
        hint("app.clipboard.pasteImage", "to paste image"),
    ]
    .join("\n");

    let onboarding = styler.dim(ONBOARDING_TEXT);

    format!("{logo}\n{instructions}\n\n{onboarding}")
}

pub fn build_condensed_changelog_notice(
    changelog_markdown: &str,
    fallback_version: &str,
    styler: &impl StartupHeaderStyler,
) -> String {
    let latest_version = extract_latest_changelog_version(changelog_markdown)
        .unwrap_or_else(|| fallback_version.to_owned());
    format!(
        "Updated to v{latest_version}. Use {} to view full changelog.",
        styler.bold("/changelog")
    )
}

#[derive(Debug, Clone)]
pub struct StartupHeaderComponent {
    text: String,
}

impl StartupHeaderComponent {
    pub fn new(
        app_name: &str,
        version: &str,
        keybindings: &KeybindingsManager,
        styler: &impl StartupHeaderStyler,
        quiet: bool,
    ) -> Self {
        Self {
            text: build_startup_header_text(app_name, version, keybindings, styler, quiet),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl Component for StartupHeaderComponent {
    fn render(&self, width: usize) -> Vec<String> {
        render_wrapped_text(&self.text, width)
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone)]
enum ChangelogContent {
    Condensed(String),
    Expanded { title: String, lines: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct BuiltInHeaderComponent {
    startup_header: StartupHeaderComponent,
    quiet: bool,
    changelog_content: Option<ChangelogContent>,
}

impl BuiltInHeaderComponent {
    pub fn new(
        app_name: &str,
        version: &str,
        keybindings: &KeybindingsManager,
        styler: &impl StartupHeaderStyler,
        quiet: bool,
        changelog_markdown: Option<&str>,
        show_condensed_changelog: bool,
    ) -> Self {
        let changelog_content = changelog_markdown
            .map(str::trim)
            .filter(|markdown| !markdown.is_empty())
            .map(|markdown| {
                if quiet || show_condensed_changelog {
                    ChangelogContent::Condensed(build_condensed_changelog_notice(
                        markdown, version, styler,
                    ))
                } else {
                    ChangelogContent::Expanded {
                        title: styler.accent_bold("What's New"),
                        lines: render_markdown_lines(markdown, styler),
                    }
                }
            });

        Self {
            startup_header: StartupHeaderComponent::new(
                app_name,
                version,
                keybindings,
                styler,
                quiet,
            ),
            quiet,
            changelog_content,
        }
    }
}

impl Component for BuiltInHeaderComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let render_width = width.max(1);
        let mut lines = Vec::new();

        if !self.quiet {
            lines.push(String::new());
            lines.extend(self.startup_header.render(render_width));
            lines.push(String::new());
        }

        if let Some(changelog_content) = &self.changelog_content {
            match changelog_content {
                ChangelogContent::Condensed(notice) => {
                    if self.quiet {
                        lines.push(String::new());
                        lines.extend(render_wrapped_text(notice, render_width));
                    } else {
                        let border = "─".repeat(render_width);
                        lines.push(border.clone());
                        lines.extend(render_wrapped_text(notice, render_width));
                        lines.push(border);
                    }
                }
                ChangelogContent::Expanded {
                    title,
                    lines: changelog_lines,
                } => {
                    let border = "─".repeat(render_width);
                    lines.push(border.clone());
                    lines.extend(render_wrapped_text(title, render_width));
                    lines.push(String::new());
                    lines.extend(render_wrapped_lines(changelog_lines, render_width));
                    lines.push(String::new());
                    lines.push(border);
                }
            }
        }

        lines
    }

    fn invalidate(&mut self) {}
}

fn render_wrapped_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let render_width = width.max(1);
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(wrap_text_with_ansi(line, render_width));
        }
    }

    lines
}

fn render_wrapped_lines(lines: &[String], width: usize) -> Vec<String> {
    let render_width = width.max(1);
    let mut wrapped = Vec::new();

    for line in lines {
        if line.is_empty() {
            wrapped.push(String::new());
        } else {
            wrapped.extend(wrap_text_with_ansi(line, render_width));
        }
    }

    wrapped
}

fn render_markdown_lines(markdown: &str, styler: &impl StartupHeaderStyler) -> Vec<String> {
    let normalized = markdown.replace("\r\n", "\n");
    let source_lines = normalized.lines().collect::<Vec<_>>();
    let mut rendered = Vec::new();
    let mut index = 0;

    while index < source_lines.len() {
        while index < source_lines.len() && source_lines[index].trim().is_empty() {
            index += 1;
        }
        if index >= source_lines.len() {
            break;
        }

        push_block_spacing(&mut rendered);

        let line = source_lines[index];
        if is_code_fence_line(line) {
            rendered.extend(render_code_block(&source_lines, &mut index, styler));
            continue;
        }

        if let Some((level, heading_text)) = parse_heading_line(line) {
            rendered.push(render_heading_line(level, heading_text, styler));
            index += 1;
            continue;
        }

        if is_horizontal_rule(line) {
            rendered.push(styler.border(&"─".repeat(HORIZONTAL_RULE_WIDTH)));
            index += 1;
            continue;
        }

        if parse_list_item_line(line).is_some() {
            rendered.extend(render_list_block(&source_lines, &mut index, styler));
            continue;
        }

        rendered.push(render_paragraph_block(&source_lines, &mut index, styler));
    }

    rendered
}

fn push_block_spacing(lines: &mut Vec<String>) {
    if lines.last().is_some_and(|line| !line.is_empty()) {
        lines.push(String::new());
    }
}

fn render_code_block(
    source_lines: &[&str],
    index: &mut usize,
    styler: &impl StartupHeaderStyler,
) -> Vec<String> {
    let mut rendered = Vec::new();
    let opening_fence = source_lines[*index].trim_start();
    rendered.push(styler.code_block_border(opening_fence));
    *index += 1;

    while *index < source_lines.len() {
        let line = source_lines[*index];
        if is_code_fence_line(line) {
            rendered.push(styler.code_block_border(line.trim_start()));
            *index += 1;
            break;
        }

        rendered.push(format!("{CODE_BLOCK_INDENT}{}", styler.code_block(line)));
        *index += 1;
    }

    rendered
}

fn render_heading_line(level: usize, text: &str, styler: &impl StartupHeaderStyler) -> String {
    let content = render_inline_markdown(text.trim(), styler);
    if level >= 3 {
        styler.heading(&format!("{} {content}", "#".repeat(level)))
    } else {
        styler.heading(&content)
    }
}

fn render_list_block(
    source_lines: &[&str],
    index: &mut usize,
    styler: &impl StartupHeaderStyler,
) -> Vec<String> {
    let mut rendered = Vec::new();

    while *index < source_lines.len() {
        let line = source_lines[*index];
        if line.trim().is_empty() || starts_new_non_list_block(line) {
            break;
        }

        let Some((indent, marker, content)) = parse_list_item_line(line) else {
            break;
        };
        let mut content = content.to_owned();
        *index += 1;

        while *index < source_lines.len() {
            let continuation = source_lines[*index];
            if continuation.trim().is_empty() || starts_new_block(continuation) {
                break;
            }
            if parse_list_item_line(continuation).is_some() {
                break;
            }

            if !content.is_empty() {
                content.push(' ');
            }
            content.push_str(continuation.trim());
            *index += 1;
        }

        rendered.push(format!(
            "{indent}{} {}",
            styler.list_bullet(&marker),
            render_inline_markdown(content.trim(), styler)
        ));
    }

    rendered
}

fn render_paragraph_block(
    source_lines: &[&str],
    index: &mut usize,
    styler: &impl StartupHeaderStyler,
) -> String {
    let mut paragraph = String::new();

    while *index < source_lines.len() {
        let line = source_lines[*index];
        if line.trim().is_empty() || starts_new_block(line) {
            break;
        }

        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(line.trim());
        *index += 1;
    }

    render_inline_markdown(&paragraph, styler)
}

fn starts_new_block(line: &str) -> bool {
    is_code_fence_line(line)
        || parse_heading_line(line).is_some()
        || is_horizontal_rule(line)
        || parse_list_item_line(line).is_some()
}

fn starts_new_non_list_block(line: &str) -> bool {
    is_code_fence_line(line) || parse_heading_line(line).is_some() || is_horizontal_rule(line)
}

fn is_code_fence_line(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

fn is_horizontal_rule(line: &str) -> bool {
    matches!(line.trim(), "---" | "***" | "___")
}

fn parse_heading_line(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b'#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }

    let text = trimmed[level..].strip_prefix(' ')?;
    Some((level, text))
}

fn parse_list_item_line(line: &str) -> Option<(&str, String, &str)> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len().saturating_sub(trimmed.len())];

    if let Some(content) = trimmed.strip_prefix("- ") {
        return Some((indent, "-".to_owned(), content));
    }

    if let Some(content) = trimmed.strip_prefix("* ") {
        return Some((indent, "*".to_owned(), content));
    }

    let marker_len = trimmed
        .as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if marker_len == 0 {
        return None;
    }

    let suffix = trimmed.get(marker_len..)?;
    let content = suffix.strip_prefix(". ")?;
    Some((indent, trimmed[..marker_len + 1].to_owned(), content))
}

fn render_inline_markdown(text: &str, styler: &impl StartupHeaderStyler) -> String {
    let mut rendered = String::new();
    let mut index = 0;

    while index < text.len() {
        let remaining = &text[index..];

        if let Some((fragment, consumed)) = try_render_link(remaining, styler) {
            rendered.push_str(&fragment);
            index += consumed;
            continue;
        }

        if let Some((fragment, consumed)) = try_render_code_span(remaining, styler) {
            rendered.push_str(&fragment);
            index += consumed;
            continue;
        }

        if let Some((fragment, consumed)) = try_render_bold(remaining, "**", styler) {
            rendered.push_str(&fragment);
            index += consumed;
            continue;
        }

        if let Some((fragment, consumed)) = try_render_bold(remaining, "__", styler) {
            rendered.push_str(&fragment);
            index += consumed;
            continue;
        }

        if let Some((fragment, consumed)) = try_render_strikethrough(remaining, styler) {
            rendered.push_str(&fragment);
            index += consumed;
            continue;
        }

        let mut chars = remaining.chars();
        let Some(ch) = chars.next() else {
            break;
        };
        rendered.push(ch);
        index += ch.len_utf8();
    }

    rendered
}

fn try_render_link(text: &str, styler: &impl StartupHeaderStyler) -> Option<(String, usize)> {
    if !text.starts_with('[') {
        return None;
    }

    let close_bracket = text.find("](")?;
    let url_start = close_bracket + 2;
    let close_paren = text[url_start..].find(')')?;
    let link_text = text.get(1..close_bracket)?;
    let url = text.get(url_start..url_start + close_paren)?;
    if link_text.is_empty() || url.is_empty() {
        return None;
    }

    let rendered_text = styler.link(&render_inline_markdown(link_text, styler));
    let url_for_comparison = url.strip_prefix("mailto:").unwrap_or(url);
    let rendered = if link_text == url || link_text == url_for_comparison {
        rendered_text
    } else {
        rendered_text + &styler.link_url(&format!(" ({url})"))
    };

    Some((rendered, url_start + close_paren + 1))
}

fn try_render_code_span(text: &str, styler: &impl StartupHeaderStyler) -> Option<(String, usize)> {
    if !text.starts_with('`') {
        return None;
    }

    let end = text[1..].find('`')?;
    let content = text.get(1..1 + end)?;
    Some((styler.code(content), end + 2))
}

fn try_render_bold(
    text: &str,
    marker: &str,
    styler: &impl StartupHeaderStyler,
) -> Option<(String, usize)> {
    if !text.starts_with(marker) {
        return None;
    }

    let content_start = marker.len();
    let end = text[content_start..].find(marker)?;
    if end == 0 {
        return None;
    }

    let content = text.get(content_start..content_start + end)?;
    Some((
        styler.bold(&render_inline_markdown(content, styler)),
        content_start + end + marker.len(),
    ))
}

fn try_render_strikethrough(
    text: &str,
    styler: &impl StartupHeaderStyler,
) -> Option<(String, usize)> {
    if !text.starts_with("~~") {
        return None;
    }

    let end = text[2..].find("~~")?;
    if end == 0 {
        return None;
    }

    let content = text.get(2..2 + end)?;
    Some((
        styler.strikethrough(&render_inline_markdown(content, styler)),
        end + 4,
    ))
}

fn extract_latest_changelog_version(changelog_markdown: &str) -> Option<String> {
    for line in changelog_markdown.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("##")
            && let Some(version) = extract_semver(rest.trim())
        {
            return Some(version);
        }
    }

    None
}

fn extract_semver(text: &str) -> Option<String> {
    text.split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .find(|token| looks_like_semver(token))
        .map(ToOwned::to_owned)
}

fn looks_like_semver(token: &str) -> bool {
    let mut parts = token.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };

    parts.next().is_none()
        && !major.is_empty()
        && !minor.is_empty()
        && !patch.is_empty()
        && major.chars().all(|ch| ch.is_ascii_digit())
        && minor.chars().all(|ch| ch.is_ascii_digit())
        && patch.chars().all(|ch| ch.is_ascii_digit())
}
