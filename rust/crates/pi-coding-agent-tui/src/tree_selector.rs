use crate::selector_common::{
    ActionCallback, CancelCallback, SelectCallback, cycle_index, matches_binding, max_visible,
    render_hint_line, sanitize_display_text, visible_window,
};
use crate::{KeybindingsManager, current_theme};
use pi_agent::AgentMessage;
use pi_coding_agent_core::{SessionEntry, SessionTreeNode};
use pi_events::{AssistantContent, Message, UserContent};
use pi_tui::{Component, Input, fuzzy_filter, truncate_to_width};
use std::{
    borrow::Cow,
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeFilterMode {
    Default,
    NoTools,
    UserOnly,
    LabeledOnly,
    All,
}

#[derive(Debug, Clone)]
struct FlatTreeNode {
    entry: SessionEntry,
    label: Option<String>,
    label_timestamp: Option<String>,
    ancestor_continues: Vec<bool>,
    is_last: bool,
    search_text: String,
}

struct LabelEditState {
    entry_id: String,
    input: Input,
}

pub struct TreeSelectorComponent {
    keybindings: KeybindingsManager,
    search_input: Input,
    flat_nodes: Vec<FlatTreeNode>,
    filtered_nodes: Vec<FlatTreeNode>,
    selected_index: usize,
    filter_mode: TreeFilterMode,
    current_leaf_id: Option<String>,
    active_path_ids: BTreeSet<String>,
    on_select: Option<SelectCallback<String>>,
    on_cancel: Option<CancelCallback>,
    on_label_change: Option<ActionCallback<(String, Option<String>)>>,
    label_edit: Option<LabelEditState>,
    show_label_timestamps: bool,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl TreeSelectorComponent {
    pub fn new(
        keybindings: &KeybindingsManager,
        tree: Vec<SessionTreeNode>,
        current_leaf_id: Option<String>,
        initial_selected_id: Option<String>,
        initial_filter_mode: TreeFilterMode,
    ) -> Self {
        let mut flat_nodes = Vec::new();
        flatten_tree(&tree, current_leaf_id.as_deref(), &mut flat_nodes, &[]);
        let active_path_ids = build_active_path_ids(&flat_nodes, current_leaf_id.as_deref());

        let mut selector = Self {
            keybindings: keybindings.clone(),
            search_input: Input::with_keybindings(keybindings.deref().clone()),
            flat_nodes,
            filtered_nodes: Vec::new(),
            selected_index: 0,
            filter_mode: initial_filter_mode,
            current_leaf_id,
            active_path_ids,
            on_select: None,
            on_cancel: None,
            on_label_change: None,
            label_edit: None,
            show_label_timestamps: false,
            viewport_size: Cell::new(None),
        };
        selector.refresh();
        let initial_selected_id = initial_selected_id.or_else(|| selector.current_leaf_id.clone());
        if let Some(initial_selected_id) = initial_selected_id {
            if let Some(index) = selector
                .filtered_nodes
                .iter()
                .position(|node| node.entry.id() == initial_selected_id)
            {
                selector.selected_index = index;
            }
        }
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

    pub fn set_on_label_change<F>(&mut self, on_label_change: F)
    where
        F: FnMut((String, Option<String>)) + Send + 'static,
    {
        self.on_label_change = Some(Box::new(on_label_change));
    }

    pub fn set_filter_mode(&mut self, filter_mode: TreeFilterMode) {
        self.filter_mode = filter_mode;
        self.refresh();
    }

    pub fn filter_mode(&self) -> TreeFilterMode {
        self.filter_mode
    }

    fn refresh(&mut self) {
        let previous_id = self
            .filtered_nodes
            .get(self.selected_index)
            .map(|node| node.entry.id().to_owned());
        let query = self.search_input.get_value().trim().to_owned();

        let filtered = self
            .flat_nodes
            .iter()
            .filter(|node| passes_filter(node, self.filter_mode, self.current_leaf_id.as_deref()))
            .cloned()
            .collect::<Vec<_>>();
        self.filtered_nodes = if query.is_empty() {
            filtered
        } else {
            fuzzy_filter(&filtered, &query, |node| {
                Cow::Borrowed(node.search_text.as_str())
            })
            .into_iter()
            .cloned()
            .collect()
        };

        if let Some(previous_id) = previous_id
            && let Some(index) = self
                .filtered_nodes
                .iter()
                .position(|node| node.entry.id() == previous_id)
        {
            self.selected_index = index;
            return;
        }

        self.selected_index = self
            .selected_index
            .min(self.filtered_nodes.len().saturating_sub(1));
    }

    fn status_suffix(&self) -> String {
        let mut suffix = match self.filter_mode {
            TreeFilterMode::Default => String::from("[default]"),
            TreeFilterMode::NoTools => String::from("[no-tools]"),
            TreeFilterMode::UserOnly => String::from("[user]"),
            TreeFilterMode::LabeledOnly => String::from("[labeled]"),
            TreeFilterMode::All => String::from("[all]"),
        };
        if self.show_label_timestamps {
            suffix.push_str(" [+label time]");
        }
        suffix
    }

    fn render_tree_lines(&self, width: usize) -> Vec<String> {
        if self.filtered_nodes.is_empty() {
            let message = if self.search_input.get_value().trim().is_empty() {
                "No entries found"
            } else {
                "No matching entries"
            };
            return vec![
                truncate_to_width(message, width, "...", false),
                truncate_to_width(
                    &format!("  (0/0) {}", self.status_suffix()),
                    width,
                    "...",
                    false,
                ),
            ];
        }

        let theme = current_theme();
        let max_visible = max_visible(&self.viewport_size, 8, 12);
        let (start_index, end_index) =
            visible_window(self.selected_index, self.filtered_nodes.len(), max_visible);
        let mut lines = Vec::new();

        for (visible_index, node) in self.filtered_nodes[start_index..end_index]
            .iter()
            .enumerate()
        {
            let actual_index = start_index + visible_index;
            let is_selected = actual_index == self.selected_index;
            let cursor = if is_selected {
                theme.fg("accent", "› ")
            } else {
                String::from("  ")
            };
            let prefix = tree_prefix(node);
            let path_marker = if self.active_path_ids.contains(node.entry.id()) {
                theme.fg("accent", "• ")
            } else {
                String::new()
            };
            let current_marker = if self
                .current_leaf_id
                .as_deref()
                .is_some_and(|id| id == node.entry.id())
            {
                format!(" {}", theme.fg("accent", "current"))
            } else {
                String::new()
            };
            let label = node
                .label
                .as_deref()
                .map(|label| format!("{} ", theme.fg("warning", format!("[{label}]"))))
                .unwrap_or_default();
            let label_timestamp = if self.show_label_timestamps {
                node.label_timestamp
                    .as_deref()
                    .map(|timestamp| {
                        format!("{} ", theme.fg("muted", format_label_timestamp(timestamp)))
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let content = format!(
                "{}{}{}{}{}",
                theme.fg("dim", prefix),
                path_marker,
                label,
                label_timestamp,
                entry_display_text(&node.entry),
            );
            let mut line = format!("{cursor}{content}{current_marker}");
            if is_selected {
                line = theme.bg("selectedBg", line);
            }
            lines.push(truncate_to_width(&line, width, "...", false));
        }

        lines.push(truncate_to_width(
            &format!(
                "  ({}/{}) {}",
                self.selected_index + 1,
                self.filtered_nodes.len(),
                self.status_suffix()
            ),
            width,
            "...",
            false,
        ));
        lines
    }

    fn render_label_editor(&self, width: usize) -> Vec<String> {
        let Some(label_edit) = self.label_edit.as_ref() else {
            return Vec::new();
        };

        let mut lines = Vec::new();
        lines.push(truncate_to_width(
            "Label (empty to remove)",
            width,
            "...",
            false,
        ));
        lines.extend(label_edit.input.render(width));
        lines.push(render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "save"),
                ("tui.select.cancel", "cancel"),
            ],
        ));
        lines
    }

    fn toggle_filter(&mut self, target: TreeFilterMode) {
        self.filter_mode = if self.filter_mode == target {
            TreeFilterMode::Default
        } else {
            target
        };
        self.refresh();
    }

    fn cycle_filter(&mut self, forward: bool) {
        const MODES: [TreeFilterMode; 5] = [
            TreeFilterMode::Default,
            TreeFilterMode::NoTools,
            TreeFilterMode::UserOnly,
            TreeFilterMode::LabeledOnly,
            TreeFilterMode::All,
        ];
        let current_index = MODES
            .iter()
            .position(|mode| *mode == self.filter_mode)
            .unwrap_or(0);
        let next_index = if forward {
            (current_index + 1) % MODES.len()
        } else if current_index == 0 {
            MODES.len() - 1
        } else {
            current_index - 1
        };
        self.filter_mode = MODES[next_index];
        self.refresh();
    }

    fn open_label_editor(&mut self) {
        let Some(node) = self.filtered_nodes.get(self.selected_index) else {
            return;
        };
        let mut input = Input::with_keybindings(self.keybindings.deref().clone());
        if let Some(label) = node.label.as_deref() {
            input.set_value(label);
        }
        self.label_edit = Some(LabelEditState {
            entry_id: node.entry.id().to_owned(),
            input,
        });
    }

    fn close_label_editor(&mut self) {
        self.label_edit = None;
    }

    fn apply_label_change(&mut self, entry_id: &str, label: Option<String>) {
        for node in &mut self.flat_nodes {
            if node.entry.id() == entry_id {
                node.label = label.clone();
                if label.is_none() {
                    node.label_timestamp = None;
                }
                node.search_text = searchable_text(
                    &node.entry,
                    node.label.as_deref(),
                    node.label_timestamp.as_deref(),
                );
                break;
            }
        }
        self.refresh();
        if let Some(on_label_change) = &mut self.on_label_change {
            on_label_change((entry_id.to_owned(), label));
        }
    }
}

impl Component for TreeSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width("Session tree", width, "...", false));
        if self.label_edit.is_some() {
            lines.extend(self.render_label_editor(width));
        } else {
            lines.extend(self.search_input.render(width));
            lines.extend(self.render_tree_lines(width));
            lines.push(render_hint_line(
                &self.keybindings,
                width,
                &[
                    ("tui.select.confirm", "select"),
                    ("tui.select.cancel", "cancel"),
                    ("tui.select.down", "navigate"),
                ],
            ));
        }
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {
        self.search_input.invalidate();
        if let Some(label_edit) = &mut self.label_edit {
            label_edit.input.invalidate();
        }
    }

    fn handle_input(&mut self, data: &str) {
        if self.label_edit.is_some() {
            if matches_binding(&self.keybindings, data, "tui.select.cancel") {
                self.close_label_editor();
                return;
            }

            if matches_binding(&self.keybindings, data, "tui.select.confirm") {
                let (entry_id, value) = {
                    let label_edit = self
                        .label_edit
                        .as_ref()
                        .expect("label editor should be present");
                    (
                        label_edit.entry_id.clone(),
                        label_edit.input.get_value().trim().to_owned(),
                    )
                };
                self.close_label_editor();
                self.apply_label_change(&entry_id, (!value.is_empty()).then_some(value));
                return;
            }

            if let Some(label_edit) = &mut self.label_edit {
                label_edit.input.handle_input(data);
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

        if matches_binding(&self.keybindings, data, "app.tree.filterDefault") {
            self.filter_mode = TreeFilterMode::Default;
            self.refresh();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.filterNoTools") {
            self.toggle_filter(TreeFilterMode::NoTools);
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.filterUserOnly") {
            self.toggle_filter(TreeFilterMode::UserOnly);
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.filterLabeledOnly") {
            self.toggle_filter(TreeFilterMode::LabeledOnly);
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.filterAll") {
            self.toggle_filter(TreeFilterMode::All);
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.filterCycleForward") {
            self.cycle_filter(true);
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.filterCycleBackward") {
            self.cycle_filter(false);
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.editLabel") {
            self.open_label_editor();
            return;
        }

        if matches_binding(&self.keybindings, data, "app.tree.toggleLabelTimestamp") {
            self.show_label_timestamps = !self.show_label_timestamps;
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.up") {
            self.selected_index =
                cycle_index(self.selected_index, self.filtered_nodes.len(), false);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            self.selected_index = cycle_index(self.selected_index, self.filtered_nodes.len(), true);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
            self.selected_index =
                self.selected_index
                    .saturating_sub(max_visible(&self.viewport_size, 8, 12));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
            self.selected_index = (self.selected_index + max_visible(&self.viewport_size, 8, 12))
                .min(self.filtered_nodes.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm") {
            if let Some(node) = self.filtered_nodes.get(self.selected_index)
                && let Some(on_select) = &mut self.on_select
            {
                on_select(node.entry.id().to_owned());
            }
            return;
        }

        self.search_input.handle_input(data);
        self.refresh();
    }

    fn set_focused(&mut self, focused: bool) {
        if let Some(label_edit) = &mut self.label_edit {
            label_edit.input.set_focused(focused);
        } else {
            self.search_input.set_focused(focused);
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
        self.search_input.set_viewport_size(width, 1);
    }
}

fn flatten_tree(
    nodes: &[SessionTreeNode],
    current_leaf_id: Option<&str>,
    target: &mut Vec<FlatTreeNode>,
    ancestors: &[bool],
) {
    let mut ordered = nodes.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|node| !subtree_contains_leaf(node, current_leaf_id));

    for (index, node) in ordered.iter().enumerate() {
        let is_last = index + 1 == ordered.len();
        target.push(FlatTreeNode {
            entry: node.entry.clone(),
            label: node.label.clone(),
            label_timestamp: node.label_timestamp.clone(),
            ancestor_continues: ancestors.to_vec(),
            is_last,
            search_text: searchable_text(
                &node.entry,
                node.label.as_deref(),
                node.label_timestamp.as_deref(),
            ),
        });

        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(!is_last);
        flatten_tree(&node.children, current_leaf_id, target, &child_ancestors);
    }
}

fn subtree_contains_leaf(node: &SessionTreeNode, current_leaf_id: Option<&str>) -> bool {
    let Some(current_leaf_id) = current_leaf_id else {
        return false;
    };
    if node.entry.id() == current_leaf_id {
        return true;
    }
    node.children
        .iter()
        .any(|child| subtree_contains_leaf(child, Some(current_leaf_id)))
}

fn build_active_path_ids(
    nodes: &[FlatTreeNode],
    current_leaf_id: Option<&str>,
) -> BTreeSet<String> {
    let mut parent_map = BTreeMap::<String, Option<String>>::new();
    for node in nodes {
        parent_map.insert(
            node.entry.id().to_owned(),
            node.entry.parent_id().map(str::to_owned),
        );
    }

    let mut active = BTreeSet::new();
    let mut current = current_leaf_id.map(str::to_owned);
    while let Some(id) = current {
        active.insert(id.clone());
        current = parent_map.get(&id).cloned().flatten();
    }
    active
}

fn searchable_text(
    entry: &SessionEntry,
    label: Option<&str>,
    label_timestamp: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(label) = label {
        parts.push(label.to_owned());
    }
    if let Some(label_timestamp) = label_timestamp {
        parts.push(label_timestamp.to_owned());
    }
    parts.push(entry_plain_text(entry));
    parts.join(" ")
}

fn entry_plain_text(entry: &SessionEntry) -> String {
    match entry {
        SessionEntry::Message { message, .. } => match message {
            AgentMessage::Standard(Message::User { content, .. }) => {
                format!("user {}", user_content_text(content))
            }
            AgentMessage::Standard(Message::Assistant {
                content,
                error_message,
                ..
            }) => {
                let mut text = assistant_content_text(content);
                if let Some(error_message) = error_message {
                    text.push(' ');
                    text.push_str(error_message);
                }
                format!("assistant {text}")
            }
            AgentMessage::Standard(Message::ToolResult {
                tool_name, content, ..
            }) => {
                format!("tool {} {}", tool_name, user_content_text(content))
            }
            AgentMessage::Custom(message) => {
                format!("{} {}", message.role, message.payload)
            }
        },
        SessionEntry::ThinkingLevelChange { thinking_level, .. } => {
            format!("thinking {thinking_level}")
        }
        SessionEntry::ModelChange {
            provider, model_id, ..
        } => format!("model {provider}/{model_id}"),
        SessionEntry::Compaction { summary, .. } => format!("compaction {summary}"),
        SessionEntry::BranchSummary { summary, .. } => format!("branch summary {summary}"),
        SessionEntry::Custom {
            custom_type, data, ..
        } => {
            format!("custom {custom_type} {}", data.clone().unwrap_or_default())
        }
        SessionEntry::CustomMessage {
            custom_type,
            content,
            ..
        } => format!("custom message {custom_type} {content:?}"),
        SessionEntry::Label { label, .. } => {
            format!("label {}", label.clone().unwrap_or_default())
        }
        SessionEntry::SessionInfo { name, .. } => {
            format!("session {}", name.clone().unwrap_or_default())
        }
    }
}

fn entry_display_text(entry: &SessionEntry) -> String {
    sanitize_display_text(&entry_plain_text(entry))
}

fn user_content_text(content: &[UserContent]) -> String {
    let mut text = String::new();
    for part in content {
        match part {
            UserContent::Text { text: value } => {
                text.push_str(value);
                text.push(' ');
            }
            UserContent::Image { mime_type, .. } => {
                text.push_str("[image:");
                text.push_str(mime_type);
                text.push_str("] ");
            }
        }
    }
    text.trim().to_owned()
}

fn assistant_content_text(content: &[AssistantContent]) -> String {
    let mut text = String::new();
    for part in content {
        match part {
            AssistantContent::Text { text: value, .. } => {
                text.push_str(value);
                text.push(' ');
            }
            AssistantContent::Thinking { thinking, .. } => {
                text.push_str(thinking);
                text.push(' ');
            }
            AssistantContent::ToolCall { name, .. } => {
                text.push('[');
                text.push_str(name);
                text.push_str("] ");
            }
        }
    }
    text.trim().to_owned()
}

fn passes_filter(
    node: &FlatTreeNode,
    filter_mode: TreeFilterMode,
    current_leaf_id: Option<&str>,
) -> bool {
    let is_current_leaf = current_leaf_id.is_some_and(|id| id == node.entry.id());
    match filter_mode {
        TreeFilterMode::All => true,
        TreeFilterMode::LabeledOnly => node.label.is_some(),
        TreeFilterMode::UserOnly => matches!(
            node.entry,
            SessionEntry::Message {
                message: AgentMessage::Standard(Message::User { .. }),
                ..
            }
        ),
        TreeFilterMode::NoTools => {
            if matches!(
                node.entry,
                SessionEntry::Message {
                    message: AgentMessage::Standard(Message::ToolResult { .. }),
                    ..
                }
            ) {
                return false;
            }
            passes_filter(node, TreeFilterMode::Default, current_leaf_id)
        }
        TreeFilterMode::Default => {
            if is_current_leaf {
                return true;
            }
            !matches!(
                node.entry,
                SessionEntry::ThinkingLevelChange { .. }
                    | SessionEntry::ModelChange { .. }
                    | SessionEntry::Custom { .. }
                    | SessionEntry::Label { .. }
                    | SessionEntry::SessionInfo { .. }
            )
        }
    }
}

fn tree_prefix(node: &FlatTreeNode) -> String {
    if node.ancestor_continues.is_empty() {
        return String::new();
    }

    let mut prefix = String::new();
    for continue_line in node
        .ancestor_continues
        .iter()
        .take(node.ancestor_continues.len().saturating_sub(1))
    {
        if *continue_line {
            prefix.push_str("│  ");
        } else {
            prefix.push_str("   ");
        }
    }
    prefix.push_str(if node.is_last { "└─ " } else { "├─ " });
    prefix
}

fn format_label_timestamp(timestamp: &str) -> String {
    let Ok(parsed) = chrono_like_parse(timestamp) else {
        return sanitize_display_text(timestamp);
    };
    parsed
}

fn chrono_like_parse(timestamp: &str) -> Result<String, ()> {
    let timestamp = timestamp.trim();
    if timestamp.len() >= 16 {
        let time = &timestamp[11..16.min(timestamp.len())];
        if timestamp.len() >= 10 {
            return Ok(format!("{}", time));
        }
    }
    Err(())
}
