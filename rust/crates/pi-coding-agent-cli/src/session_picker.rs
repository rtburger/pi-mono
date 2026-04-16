use pi_coding_agent_core::SessionInfo;
use pi_coding_agent_tui::{KeybindingsManager, ThemedKeyHintStyler, key_hint};
use pi_tui::{Component, Input, fuzzy_filter, matches_key, truncate_to_width};
use std::{borrow::Cow, cell::Cell, env, ops::Deref, time::SystemTime};

type SelectCallback = Box<dyn FnMut(String) + Send + 'static>;
type CancelCallback = Box<dyn FnMut() + Send + 'static>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionScope {
    Current,
    All,
}

pub struct SessionPickerComponent {
    keybindings: KeybindingsManager,
    search_input: Input,
    current_sessions: Vec<SessionInfo>,
    all_sessions: Vec<SessionInfo>,
    scope: SessionScope,
    filtered_sessions: Vec<SessionInfo>,
    selected_index: usize,
    on_select: Option<SelectCallback>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl SessionPickerComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        current_sessions: Vec<SessionInfo>,
        all_sessions: Vec<SessionInfo>,
    ) -> Self {
        let mut picker = Self {
            keybindings: keybindings.clone(),
            search_input: Input::with_keybindings(keybindings.deref().clone()),
            current_sessions,
            all_sessions,
            scope: SessionScope::Current,
            filtered_sessions: Vec::new(),
            selected_index: 0,
            on_select: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        };
        picker.refresh();
        picker
    }

    pub fn set_on_select<F>(&mut self, on_select: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_select = Some(Box::new(on_select));
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    fn refresh(&mut self) {
        let previous_path = self
            .filtered_sessions
            .get(self.selected_index)
            .map(|session| session.path.clone());
        let query = self.search_input.get_value().trim().to_owned();
        let sessions = self.active_sessions();
        self.filtered_sessions = if query.is_empty() {
            sessions.to_vec()
        } else {
            fuzzy_filter(sessions, &query, |session| {
                Cow::Owned(searchable_text(session))
            })
            .into_iter()
            .cloned()
            .collect()
        };

        if let Some(previous_path) = previous_path
            && let Some(index) = self
                .filtered_sessions
                .iter()
                .position(|session| session.path == previous_path)
        {
            self.selected_index = index;
            return;
        }

        self.selected_index = self
            .selected_index
            .min(self.filtered_sessions.len().saturating_sub(1));
    }

    fn active_sessions(&self) -> &[SessionInfo] {
        match self.scope {
            SessionScope::Current => &self.current_sessions,
            SessionScope::All => &self.all_sessions,
        }
    }

    fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            SessionScope::Current => SessionScope::All,
            SessionScope::All => SessionScope::Current,
        };
        self.refresh();
    }

    fn matches_binding(&self, data: &str, keybinding: &str) -> bool {
        self.keybindings
            .get_keys(keybinding)
            .iter()
            .any(|key| matches_key(data, key.as_str()))
    }

    fn max_visible(&self) -> usize {
        self.viewport_size
            .get()
            .map(|(_, height)| height.saturating_sub(5).max(1))
            .unwrap_or(10)
    }

    fn title(&self) -> &'static str {
        match self.scope {
            SessionScope::Current => "Resume session (current folder)",
            SessionScope::All => "Resume session (all projects)",
        }
    }

    fn empty_message(&self) -> &'static str {
        if !self.search_input.get_value().trim().is_empty() {
            return "No matching sessions";
        }

        match self.scope {
            SessionScope::Current if self.all_sessions.is_empty() => "No sessions found",
            SessionScope::Current => "No sessions in current folder. Press Tab to view all.",
            SessionScope::All => "No sessions found",
        }
    }

    fn render_hint_line(&self, width: usize) -> String {
        let styler = ThemedKeyHintStyler;
        let hint = [
            key_hint(&self.keybindings, &styler, "tui.select.confirm", "select"),
            key_hint(&self.keybindings, &styler, "tui.select.cancel", "cancel"),
            key_hint(&self.keybindings, &styler, "tui.input.tab", "scope"),
            key_hint(&self.keybindings, &styler, "tui.select.down", "navigate"),
        ]
        .into_iter()
        .filter(|hint| !hint.is_empty())
        .collect::<Vec<_>>()
        .join("  ");
        truncate_to_width(&hint, width, "...", false)
    }

    fn render_session_lines(&self, width: usize) -> Vec<String> {
        if self.filtered_sessions.is_empty() {
            return vec![truncate_to_width(self.empty_message(), width, "...", false)];
        }

        let max_visible = self.max_visible();
        let start_index = self
            .selected_index
            .saturating_sub(max_visible / 2)
            .min(self.filtered_sessions.len().saturating_sub(max_visible));
        let end_index = (start_index + max_visible).min(self.filtered_sessions.len());
        let mut lines = Vec::new();

        for (visible_index, session) in self.filtered_sessions[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let label = sanitize_display_text(session_label(session));
            let mut metadata = format!(
                "{} msg · {}",
                session.message_count,
                format_session_age(session.modified)
            );
            if self.scope == SessionScope::All {
                metadata = format!("{} · {metadata}", shorten_home_path(&session.cwd));
            }
            let line = format!("{prefix}{label} — {metadata}");
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        if start_index > 0 || end_index < self.filtered_sessions.len() {
            lines.push(truncate_to_width(
                &format!(
                    "  ({}/{})",
                    self.selected_index + 1,
                    self.filtered_sessions.len()
                ),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for SessionPickerComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(self.title(), width, "...", false));
        lines.extend(self.search_input.render(width));
        lines.extend(self.render_session_lines(width));
        lines.push(self.render_hint_line(width));
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.search_input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if self.matches_binding(data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        if self.matches_binding(data, "tui.input.tab") {
            self.toggle_scope();
            return;
        }

        if self.matches_binding(data, "tui.select.up") {
            if self.filtered_sessions.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index == 0 {
                self.filtered_sessions.len() - 1
            } else {
                self.selected_index - 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.down") {
            if self.filtered_sessions.is_empty() {
                return;
            }
            self.selected_index = if self.selected_index + 1 >= self.filtered_sessions.len() {
                0
            } else {
                self.selected_index + 1
            };
            return;
        }

        if self.matches_binding(data, "tui.select.pageUp") {
            self.selected_index = self.selected_index.saturating_sub(self.max_visible());
            return;
        }

        if self.matches_binding(data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + self.max_visible())
                .min(self.filtered_sessions.len().saturating_sub(1));
            return;
        }

        if self.matches_binding(data, "tui.select.confirm") {
            if let Some(session) = self.filtered_sessions.get(self.selected_index)
                && let Some(on_select) = &mut self.on_select
            {
                on_select(session.path.clone());
            }
            return;
        }

        self.search_input.handle_input(data);
        self.refresh();
    }

    fn set_focused(&mut self, focused: bool) {
        self.search_input.set_focused(focused);
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}

fn searchable_text(session: &SessionInfo) -> String {
    format!(
        "{} {} {} {} {} {}",
        session.id,
        session.name.as_deref().unwrap_or_default(),
        session.first_message,
        session.all_messages_text,
        session.cwd,
        session.path,
    )
}

fn session_label(session: &SessionInfo) -> &str {
    session
        .name
        .as_deref()
        .unwrap_or(session.first_message.as_str())
}

fn sanitize_display_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() && character != '\n' && character != '\t' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn shorten_home_path(path: &str) -> String {
    let Some(home) = env::var_os("HOME") else {
        return path.to_owned();
    };
    let home = home.to_string_lossy();
    if path.starts_with(home.as_ref()) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_owned()
    }
}

fn format_session_age(modified: SystemTime) -> String {
    let Ok(diff) = SystemTime::now().duration_since(modified) else {
        return String::from("now");
    };
    let minutes = diff.as_secs() / 60;
    let hours = diff.as_secs() / 3_600;
    let days = diff.as_secs() / 86_400;

    if minutes == 0 {
        return String::from("now");
    }
    if minutes < 60 {
        return format!("{minutes}m");
    }
    if hours < 24 {
        return format!("{hours}h");
    }
    if days < 7 {
        return format!("{days}d");
    }
    if days < 30 {
        return format!("{}w", days / 7);
    }
    if days < 365 {
        return format!("{}mo", days / 30);
    }
    format!("{}y", days / 365)
}
