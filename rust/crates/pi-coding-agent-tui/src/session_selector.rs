use crate::selector_common::{
    ActionCallback, CancelCallback, SelectCallback, matches_binding, max_visible, render_hint_line,
    sanitize_display_text, visible_window,
};
use crate::session_selector_search::{
    NameFilter, SortMode, filter_and_sort_sessions, has_session_name,
};
use crate::{KeybindingsManager, current_theme};
use pi_coding_agent_core::SessionInfo;
use pi_tui::{Component, Input, truncate_to_width, visible_width};
use std::{
    cell::Cell,
    env,
    ops::Deref,
    time::{SystemTime, UNIX_EPOCH},
};

type RenameCallback = ActionCallback<String>;
type DeleteCallback = ActionCallback<String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSelectorScope {
    Current,
    All,
}

pub struct SessionSelectorComponent {
    keybindings: KeybindingsManager,
    search_input: Input,
    current_sessions: Vec<SessionInfo>,
    all_sessions: Vec<SessionInfo>,
    current_session_path: Option<String>,
    scope: SessionSelectorScope,
    sort_mode: SortMode,
    name_filter: NameFilter,
    show_path: bool,
    filtered_sessions: Vec<SessionInfo>,
    selected_index: usize,
    on_select: Option<SelectCallback<String>>,
    on_cancel: Option<CancelCallback>,
    on_rename: Option<RenameCallback>,
    on_delete: Option<DeleteCallback>,
    status_message: Option<String>,
    confirming_delete_path: Option<String>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl SessionSelectorComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        current_sessions: Vec<SessionInfo>,
        all_sessions: Vec<SessionInfo>,
        current_session_path: Option<String>,
    ) -> Self {
        let mut selector = Self {
            keybindings: keybindings.clone(),
            search_input: Input::with_keybindings(keybindings.deref().clone()),
            current_sessions,
            all_sessions,
            current_session_path,
            scope: SessionSelectorScope::Current,
            sort_mode: SortMode::Threaded,
            name_filter: NameFilter::All,
            show_path: false,
            filtered_sessions: Vec::new(),
            selected_index: 0,
            on_select: None,
            on_cancel: None,
            on_rename: None,
            on_delete: None,
            status_message: None,
            confirming_delete_path: None,
            viewport_size: Cell::new(None),
        };
        selector.refresh();
        selector
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

    pub fn set_on_rename<F>(&mut self, on_rename: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_rename = Some(Box::new(on_rename));
    }

    pub fn set_on_delete<F>(&mut self, on_delete: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_delete = Some(Box::new(on_delete));
    }

    pub fn set_status_message(&mut self, message: Option<String>) {
        self.status_message = message;
    }

    pub fn set_current_sessions(&mut self, sessions: Vec<SessionInfo>) {
        self.current_sessions = sessions;
        self.refresh();
    }

    pub fn set_all_sessions(&mut self, sessions: Vec<SessionInfo>) {
        self.all_sessions = sessions;
        self.refresh();
    }

    pub fn selected_session_path(&self) -> Option<&str> {
        self.filtered_sessions
            .get(self.selected_index)
            .map(|session| session.path.as_str())
    }

    pub fn scope(&self) -> SessionSelectorScope {
        self.scope
    }

    pub fn sort_mode(&self) -> SortMode {
        self.sort_mode
    }

    pub fn name_filter(&self) -> NameFilter {
        self.name_filter
    }

    fn active_sessions(&self) -> &[SessionInfo] {
        match self.scope {
            SessionSelectorScope::Current => &self.current_sessions,
            SessionSelectorScope::All => &self.all_sessions,
        }
    }

    fn refresh(&mut self) {
        let previous_path = self.selected_session_path().map(str::to_owned);
        let query = self.search_input.get_value().to_owned();
        self.filtered_sessions = if matches!(self.sort_mode, SortMode::Threaded)
            && query.trim().is_empty()
        {
            let mut sessions = self
                .active_sessions()
                .iter()
                .filter(|session| match self.name_filter {
                    NameFilter::All => true,
                    NameFilter::Named => has_session_name(session),
                })
                .cloned()
                .collect::<Vec<_>>();
            sessions.sort_by_key(|session| std::cmp::Reverse(system_time_millis(session.modified)));
            sessions
        } else {
            filter_and_sort_sessions(
                self.active_sessions(),
                &query,
                self.sort_mode,
                self.name_filter,
            )
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

    fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            SessionSelectorScope::Current => SessionSelectorScope::All,
            SessionSelectorScope::All => SessionSelectorScope::Current,
        };
        self.refresh();
    }

    fn toggle_sort_mode(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Threaded => SortMode::Recent,
            SortMode::Recent => SortMode::Relevance,
            SortMode::Relevance => SortMode::Threaded,
        };
        self.refresh();
    }

    fn toggle_name_filter(&mut self) {
        self.name_filter = match self.name_filter {
            NameFilter::All => NameFilter::Named,
            NameFilter::Named => NameFilter::All,
        };
        self.refresh();
    }

    fn title(&self) -> &'static str {
        match self.scope {
            SessionSelectorScope::Current => "Resume session (current folder)",
            SessionSelectorScope::All => "Resume session (all)",
        }
    }

    fn render_header_line(&self, width: usize) -> String {
        let theme = current_theme();
        let title = theme.bold(self.title());
        let scope_label = match self.scope {
            SessionSelectorScope::Current => "Current",
            SessionSelectorScope::All => "All",
        };
        let name_label = match self.name_filter {
            NameFilter::All => "All",
            NameFilter::Named => "Named",
        };
        let sort_label = match self.sort_mode {
            SortMode::Threaded => "Threaded",
            SortMode::Recent => "Recent",
            SortMode::Relevance => "Fuzzy",
        };
        let right = format!(
            "{}  {}  {}",
            theme.fg("accent", format!("Scope: {scope_label}")),
            theme.fg("muted", format!("Name: {name_label}")),
            theme.fg("muted", format!("Sort: {sort_label}")),
        );
        let available_left = width.saturating_sub(visible_width(&right) + 1);
        let left = truncate_to_width(&title, available_left, "...", false);
        let spacing = width.saturating_sub(visible_width(&left) + visible_width(&right));
        format!("{left}{}{right}", " ".repeat(spacing))
    }

    fn render_state_line(&self, width: usize) -> Option<String> {
        if self.confirming_delete_path.is_some() {
            return Some(render_hint_line(
                &self.keybindings,
                width,
                &[
                    ("tui.select.confirm", "confirm delete"),
                    ("tui.select.cancel", "cancel"),
                ],
            ));
        }

        self.status_message
            .as_deref()
            .map(|message| truncate_to_width(message, width, "...", false))
    }

    fn empty_message(&self) -> &'static str {
        if !self.search_input.get_value().trim().is_empty() {
            return "No matching sessions";
        }

        match self.scope {
            SessionSelectorScope::Current if self.all_sessions.is_empty() => "No sessions found",
            SessionSelectorScope::Current => {
                "No sessions in current folder. Press Tab to view all."
            }
            SessionSelectorScope::All => "No sessions found",
        }
    }

    fn render_session_lines(&self, width: usize) -> Vec<String> {
        if self.filtered_sessions.is_empty() {
            return vec![truncate_to_width(self.empty_message(), width, "...", false)];
        }

        let theme = current_theme();
        let max_visible = max_visible(&self.viewport_size, 8, 10);
        let (start_index, end_index) = visible_window(
            self.selected_index,
            self.filtered_sessions.len(),
            max_visible,
        );
        let mut lines = Vec::new();

        for (visible_index, session) in self.filtered_sessions[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            let is_selected = actual_index == self.selected_index;
            let is_current = self
                .current_session_path
                .as_deref()
                .is_some_and(|path| path == session.path);
            let is_confirming_delete = self
                .confirming_delete_path
                .as_deref()
                .is_some_and(|path| path == session.path);

            let cursor = if is_selected {
                theme.fg("accent", "› ")
            } else {
                String::from("  ")
            };
            let label_source = session
                .name
                .as_deref()
                .unwrap_or(session.first_message.as_str());
            let label = sanitize_display_text(label_source);
            let age = format_session_age(session.modified);
            let mut right = format!("{} msg · {age}", session.message_count);
            if matches!(self.scope, SessionSelectorScope::All) && !session.cwd.is_empty() {
                right = format!("{} · {right}", shorten_home_path(&session.cwd));
            }
            if self.show_path {
                right = format!("{} · {right}", shorten_home_path(&session.path));
            }

            let marker = if is_current {
                format!(" {}", theme.fg("accent", "✓"))
            } else {
                String::new()
            };
            let left = format!("{cursor}{label}{marker}");
            let available_left = width.saturating_sub(visible_width(&right) + 1);
            let left = truncate_to_width(&left, available_left, "...", false);
            let spacing = width
                .saturating_sub(visible_width(&left) + visible_width(&right))
                .max(1);
            let mut line = format!("{left}{}{right}", " ".repeat(spacing));
            if is_confirming_delete {
                line = theme.fg("error", line);
            } else if is_selected {
                line = theme.bg("selectedBg", line);
            }
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

impl Component for SessionSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(self.render_header_line(width));
        lines.extend(self.search_input.render(width));
        if let Some(state_line) = self.render_state_line(width) {
            lines.push(state_line);
        }
        lines.extend(self.render_session_lines(width));
        let hint_line = if self.confirming_delete_path.is_none() {
            render_hint_line(
                &self.keybindings,
                width,
                &[
                    ("tui.select.confirm", "select"),
                    ("tui.select.cancel", "cancel"),
                    ("tui.input.tab", "scope"),
                    ("app.session.toggleSort", "sort"),
                    ("app.session.toggleNamedFilter", "named"),
                    ("app.session.togglePath", "path"),
                    ("app.session.rename", "rename"),
                    ("app.session.delete", "delete"),
                ],
            )
        } else {
            String::new()
        };
        if !hint_line.is_empty() {
            lines.push(hint_line);
        }
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.search_input.invalidate();
    }

    fn handle_input(&mut self, data: &str) {
        if self.confirming_delete_path.is_some() {
            if matches_binding(&self.keybindings, data, "tui.select.cancel") {
                self.confirming_delete_path = None;
                return;
            }
            if matches_binding(&self.keybindings, data, "tui.select.confirm") {
                if let Some(path) = self.confirming_delete_path.take()
                    && let Some(on_delete) = &mut self.on_delete
                {
                    on_delete(path);
                }
                return;
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.cancel") {
            if self.search_input.get_value().is_empty() {
                if let Some(on_cancel) = &mut self.on_cancel {
                    on_cancel();
                }
            } else {
                self.search_input.clear();
                self.refresh();
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.input.tab") {
            self.toggle_scope();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.session.toggleSort") {
            self.toggle_sort_mode();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.session.toggleNamedFilter") {
            self.toggle_name_filter();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.session.togglePath") {
            self.show_path = !self.show_path;
            return;
        }

        if matches_binding(&self.keybindings, data, "app.session.rename") {
            if let Some(session) = self.filtered_sessions.get(self.selected_index)
                && let Some(on_rename) = &mut self.on_rename
            {
                on_rename(session.path.clone());
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "app.session.delete") {
            if let Some(session) = self.filtered_sessions.get(self.selected_index) {
                self.confirming_delete_path = Some(session.path.clone());
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "app.session.deleteNoninvasive") {
            if self.search_input.get_value().is_empty() {
                if let Some(session) = self.filtered_sessions.get(self.selected_index) {
                    self.confirming_delete_path = Some(session.path.clone());
                }
            } else {
                self.search_input.handle_input(data);
                self.refresh();
            }
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.up") {
            if self.filtered_sessions.is_empty() {
                return;
            }
            self.selected_index = self.selected_index.saturating_sub(1);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            if self.filtered_sessions.is_empty() {
                return;
            }
            self.selected_index =
                (self.selected_index + 1).min(self.filtered_sessions.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
            self.selected_index =
                self.selected_index
                    .saturating_sub(max_visible(&self.viewport_size, 8, 10));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + max_visible(&self.viewport_size, 8, 10))
                .min(self.filtered_sessions.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm") {
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

fn system_time_millis(value: SystemTime) -> u128 {
    value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
