use crate::current_theme;
use parking_lot::Mutex;
use pi_coding_agent_core::{BranchChangeSubscription, FooterDataProvider, FooterDataSnapshot};
use pi_events::Model;
use pi_tui::{Component, RenderHandle, truncate_to_width, visible_width};
use std::{collections::BTreeMap, env, sync::Arc};

#[derive(Debug, Clone, PartialEq)]
pub struct FooterState {
    pub cwd: String,
    pub git_branch: Option<String>,
    pub session_name: Option<String>,
    pub model: Option<Model>,
    pub thinking_level: String,
    pub usage_input: u64,
    pub usage_output: u64,
    pub usage_cache_read: u64,
    pub usage_cache_write: u64,
    pub total_cost: f64,
    pub using_subscription: bool,
    pub context_window: u64,
    pub context_percent: Option<f64>,
    pub auto_compact_enabled: bool,
    pub available_provider_count: usize,
    pub extension_statuses: BTreeMap<String, String>,
}

impl FooterState {
    pub fn apply_data_snapshot(&mut self, snapshot: &FooterDataSnapshot) {
        self.cwd = snapshot.cwd.clone();
        self.git_branch = snapshot.git_branch.clone();
        self.available_provider_count = snapshot.available_provider_count;
        self.extension_statuses = snapshot.extension_statuses.clone();
    }

    pub fn with_data_snapshot(mut self, snapshot: &FooterDataSnapshot) -> Self {
        self.apply_data_snapshot(snapshot);
        self
    }
}

impl Default for FooterState {
    fn default() -> Self {
        Self {
            cwd: String::new(),
            git_branch: None,
            session_name: None,
            model: None,
            thinking_level: "off".to_owned(),
            usage_input: 0,
            usage_output: 0,
            usage_cache_read: 0,
            usage_cache_write: 0,
            total_cost: 0.0,
            using_subscription: false,
            context_window: 0,
            context_percent: None,
            auto_compact_enabled: true,
            available_provider_count: 0,
            extension_statuses: BTreeMap::new(),
        }
    }
}

#[derive(Clone)]
pub struct FooterStateHandle {
    state: Arc<Mutex<FooterState>>,
    render_handle: Option<RenderHandle>,
}

impl FooterStateHandle {
    fn new(state: Arc<Mutex<FooterState>>, render_handle: Option<RenderHandle>) -> Self {
        Self {
            state,
            render_handle,
        }
    }

    pub fn set_state(&self, state: FooterState) {
        *self.state.lock() = state;
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }

    pub fn update(&self, updater: impl FnOnce(&mut FooterState)) {
        updater(&mut self.state.lock());
        if let Some(render_handle) = &self.render_handle {
            render_handle.request_render();
        }
    }
}

pub struct FooterComponent {
    state: Arc<Mutex<FooterState>>,
    pending_snapshot: Arc<Mutex<Option<FooterDataSnapshot>>>,
    data_subscription: Mutex<Option<BranchChangeSubscription>>,
}

impl FooterComponent {
    pub fn new(state: FooterState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
            pending_snapshot: Arc::new(Mutex::new(None)),
            data_subscription: Mutex::new(None),
        }
    }

    pub fn state_handle(&self) -> FooterStateHandle {
        FooterStateHandle::new(Arc::clone(&self.state), None)
    }

    pub fn state_handle_with_render_handle(
        &self,
        render_handle: RenderHandle,
    ) -> FooterStateHandle {
        FooterStateHandle::new(Arc::clone(&self.state), Some(render_handle))
    }

    pub fn state(&self) -> FooterState {
        self.sync_pending_snapshot();
        self.state.lock().clone()
    }

    pub fn set_state(&self, state: FooterState) {
        *self.state.lock() = state;
    }

    pub fn clear_state(&self) {
        *self.state.lock() = FooterState::default();
    }

    pub fn apply_data_snapshot(&self, snapshot: &FooterDataSnapshot) {
        self.state.lock().apply_data_snapshot(snapshot);
    }

    pub fn bind_data_provider(&self, provider: &FooterDataProvider) {
        self.bind_data_provider_internal(provider, None);
    }

    pub fn bind_data_provider_with_render_handle(
        &self,
        provider: &FooterDataProvider,
        render_handle: RenderHandle,
    ) {
        self.bind_data_provider_internal(provider, Some(render_handle));
    }

    pub fn unbind_data_provider(&self) {
        self.data_subscription.lock().take();
        self.pending_snapshot.lock().take();
    }

    fn sync_pending_snapshot(&self) {
        let snapshot = self.pending_snapshot.lock().take();
        if let Some(snapshot) = snapshot {
            self.apply_data_snapshot(&snapshot);
        }
    }

    fn bind_data_provider_internal(
        &self,
        provider: &FooterDataProvider,
        render_handle: Option<RenderHandle>,
    ) {
        self.apply_data_snapshot(&provider.snapshot());
        if let Some(render_handle) = &render_handle {
            render_handle.request_render();
        }
        let pending_snapshot = Arc::clone(&self.pending_snapshot);
        let subscription = provider.on_snapshot_change(move |snapshot| {
            *pending_snapshot.lock() = Some(snapshot);
            if let Some(render_handle) = &render_handle {
                render_handle.request_render();
            }
        });
        *self.data_subscription.lock() = Some(subscription);
    }
}

impl Default for FooterComponent {
    fn default() -> Self {
        Self::new(FooterState::default())
    }
}

impl Component for FooterComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.sync_pending_snapshot();
        let state = self.state.lock().clone();
        let mut lines = Vec::new();

        if has_pwd_line(&state) {
            let pwd_line = current_theme().fg("muted", format_pwd(&state));
            lines.push(truncate_to_width(&pwd_line, width, "...", false));
        }

        if has_stats_line(&state) {
            lines.push(render_stats_line(&state, width));
        }

        if !state.extension_statuses.is_empty() {
            let status_line = state
                .extension_statuses
                .values()
                .map(|text| sanitize_status_text(text))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(truncate_to_width(&status_line, width, "...", false));
        }

        lines
    }

    fn invalidate(&mut self) {}
}

fn has_pwd_line(state: &FooterState) -> bool {
    !state.cwd.is_empty() || state.git_branch.is_some() || state.session_name.is_some()
}

fn has_stats_line(state: &FooterState) -> bool {
    state.model.is_some()
        || state.usage_input > 0
        || state.usage_output > 0
        || state.usage_cache_read > 0
        || state.usage_cache_write > 0
        || state.total_cost > 0.0
        || state.using_subscription
        || state.context_window > 0
        || state.context_percent.is_some()
}

fn format_pwd(state: &FooterState) -> String {
    let mut pwd = state.cwd.clone();
    if let Some(home) = env::var("HOME")
        .ok()
        .or_else(|| env::var("USERPROFILE").ok())
        && pwd.starts_with(&home)
    {
        pwd = format!("~{}", &pwd[home.len()..]);
    }

    if let Some(branch) = &state.git_branch {
        if pwd.is_empty() {
            pwd = format!("({branch})");
        } else {
            pwd = format!("{pwd} ({branch})");
        }
    }

    if let Some(session_name) = &state.session_name {
        if pwd.is_empty() {
            pwd = session_name.clone();
        } else {
            pwd = format!("{pwd} • {session_name}");
        }
    }

    pwd
}

fn render_stats_line(state: &FooterState, width: usize) -> String {
    let theme = current_theme();
    let mut stats_parts = Vec::new();
    if state.usage_input > 0 {
        stats_parts.push(format!("↑{}", format_tokens(state.usage_input)));
    }
    if state.usage_output > 0 {
        stats_parts.push(format!("↓{}", format_tokens(state.usage_output)));
    }
    if state.usage_cache_read > 0 {
        stats_parts.push(format!("R{}", format_tokens(state.usage_cache_read)));
    }
    if state.usage_cache_write > 0 {
        stats_parts.push(format!("W{}", format_tokens(state.usage_cache_write)));
    }
    if state.total_cost > 0.0 || state.using_subscription {
        let suffix = if state.using_subscription {
            " (sub)"
        } else {
            ""
        };
        stats_parts.push(format!("${:.3}{suffix}", state.total_cost));
    }

    let auto_indicator = if state.auto_compact_enabled {
        " (auto)"
    } else {
        ""
    };
    let context_window = if state.context_window > 0 {
        state.context_window
    } else {
        state
            .model
            .as_ref()
            .map(|model| model.context_window)
            .unwrap_or(0)
    };
    let context_display = match state.context_percent {
        Some(percent) => format!(
            "{percent:.1}%/{}{auto_indicator}",
            format_tokens(context_window)
        ),
        None => format!("?/{}{auto_indicator}", format_tokens(context_window)),
    };
    stats_parts.push(context_display);

    let mut stats_left = stats_parts.join(" ");
    let mut stats_left_width = visible_width(&stats_left);
    if stats_left_width > width {
        stats_left = truncate_to_width(&stats_left, width, "...", false);
        stats_left_width = visible_width(&stats_left);
    }

    let model_name = state
        .model
        .as_ref()
        .map(|model| model.id.as_str())
        .unwrap_or("no-model");

    let mut right_side_without_provider = model_name.to_owned();
    if state.model.as_ref().is_some_and(|model| model.reasoning) {
        let thinking_level = if state.thinking_level.is_empty() {
            "off"
        } else {
            &state.thinking_level
        };
        right_side_without_provider = if thinking_level == "off" {
            format!("{model_name} • thinking off")
        } else {
            format!("{model_name} • {thinking_level}")
        };
    }

    let mut right_side = right_side_without_provider.clone();
    if state.available_provider_count > 1
        && let Some(model) = &state.model
    {
        let candidate = format!("({}) {right_side_without_provider}", model.provider);
        if stats_left_width + 2 + visible_width(&candidate) <= width {
            right_side = candidate;
        }
    }

    let right_side_width = visible_width(&right_side);
    let total_needed = stats_left_width + 2 + right_side_width;
    let line = if total_needed <= width {
        let padding_width = width.saturating_sub(stats_left_width + right_side_width);
        format!("{stats_left}{}{right_side}", " ".repeat(padding_width))
    } else {
        let available_for_right = width.saturating_sub(stats_left_width + 2);
        if available_for_right == 0 {
            stats_left
        } else {
            let truncated_right = truncate_to_width(&right_side, available_for_right, "", false);
            let truncated_right_width = visible_width(&truncated_right);
            let padding_width = width.saturating_sub(stats_left_width + truncated_right_width);
            format!("{stats_left}{}{truncated_right}", " ".repeat(padding_width))
        }
    };

    theme.fg("dim", line)
}

fn sanitize_status_text(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_tokens(count: u64) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 10_000 {
        return format!("{:.1}k", count as f64 / 1_000.0);
    }
    if count < 1_000_000 {
        return format!("{}k", (count as f64 / 1_000.0).round() as u64);
    }
    if count < 10_000_000 {
        return format!("{:.1}M", count as f64 / 1_000_000.0);
    }
    format!("{}M", (count as f64 / 1_000_000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::sanitize_status_text;

    #[test]
    fn sanitize_status_text_collapses_newlines_tabs_and_spaces() {
        assert_eq!(
            sanitize_status_text("foo\n\tbar  baz\rqux"),
            "foo bar baz qux"
        );
    }
}
