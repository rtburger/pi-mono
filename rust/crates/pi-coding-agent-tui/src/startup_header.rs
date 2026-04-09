use crate::{KeyHintStyler, KeybindingsManager, key_hint, key_text, raw_key_hint};
use pi_tui::{Component, wrap_text_with_ansi};

const ONBOARDING_TEXT: &str =
    "Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.";

pub trait StartupHeaderStyler: KeyHintStyler {
    fn accent_bold(&self, text: &str) -> String;

    fn bold(&self, text: &str) -> String {
        text.to_owned()
    }

    fn border(&self, text: &str) -> String {
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
        hint("app.suspend", "to suspend"),
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
        hint("app.tools.expand", "to expand tools"),
        hint("app.thinking.toggle", "to expand thinking"),
        hint("app.editor.external", "for external editor"),
        raw_key_hint(styler, "/", "for commands"),
        raw_key_hint(styler, "!", "to run bash"),
        raw_key_hint(styler, "!!", "to run bash (no context)"),
        hint("app.message.followUp", "to queue follow-up"),
        hint("app.message.dequeue", "to edit all queued messages"),
        hint("app.clipboard.pasteImage", "to paste image"),
        raw_key_hint(styler, "drop files", "to attach"),
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
pub struct BuiltInHeaderComponent {
    startup_header: StartupHeaderComponent,
    quiet: bool,
    condensed_changelog_notice: Option<String>,
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
        let condensed_changelog_notice = changelog_markdown.and_then(|markdown| {
            if quiet || show_condensed_changelog {
                Some(build_condensed_changelog_notice(markdown, version, styler))
            } else {
                None
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
            condensed_changelog_notice,
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

        if let Some(notice) = &self.condensed_changelog_notice {
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

        if lines.is_empty() {
            lines.push(String::new());
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
