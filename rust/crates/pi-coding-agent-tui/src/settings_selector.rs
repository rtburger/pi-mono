use crate::KeybindingsManager;
use crate::selector_common::{
    ActionCallback, CancelCallback, matches_binding, max_visible, render_hint_line, visible_window,
};
use crate::tree_selector::TreeFilterMode;
use pi_agent::{ThinkingLevel, Transport};
use pi_tui::{Component, truncate_to_width};
use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    All,
    OneAtATime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleEscapeAction {
    Fork,
    Tree,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsConfig {
    pub supports_images: bool,
    pub auto_compact: bool,
    pub show_images: bool,
    pub auto_resize_images: bool,
    pub block_images: bool,
    pub enable_skill_commands: bool,
    pub steering_mode: DeliveryMode,
    pub follow_up_mode: DeliveryMode,
    pub transport: Transport,
    pub thinking_level: ThinkingLevel,
    pub available_thinking_levels: Vec<ThinkingLevel>,
    pub current_theme: String,
    pub available_themes: Vec<String>,
    pub hide_thinking_block: bool,
    pub collapse_changelog: bool,
    pub double_escape_action: DoubleEscapeAction,
    pub tree_filter_mode: TreeFilterMode,
    pub show_hardware_cursor: bool,
    pub editor_padding_x: usize,
    pub autocomplete_max_visible: usize,
    pub quiet_startup: bool,
    pub clear_on_shrink: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsChange {
    AutoCompact(bool),
    ShowImages(bool),
    AutoResizeImages(bool),
    BlockImages(bool),
    EnableSkillCommands(bool),
    SteeringMode(DeliveryMode),
    FollowUpMode(DeliveryMode),
    Transport(Transport),
    ThinkingLevel(ThinkingLevel),
    Theme(String),
    HideThinkingBlock(bool),
    CollapseChangelog(bool),
    DoubleEscapeAction(DoubleEscapeAction),
    TreeFilterMode(TreeFilterMode),
    ShowHardwareCursor(bool),
    EditorPaddingX(usize),
    AutocompleteMaxVisible(usize),
    QuietStartup(bool),
    ClearOnShrink(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingId {
    AutoCompact,
    ShowImages,
    AutoResizeImages,
    BlockImages,
    SkillCommands,
    SteeringMode,
    FollowUpMode,
    Transport,
    Thinking,
    Theme,
    HideThinking,
    CollapseChangelog,
    QuietStartup,
    DoubleEscapeAction,
    TreeFilterMode,
    ShowHardwareCursor,
    EditorPadding,
    AutocompleteMaxVisible,
    ClearOnShrink,
}

#[derive(Debug, Clone)]
struct SettingRow {
    id: SettingId,
    label: &'static str,
    description: &'static str,
    value: String,
}

#[derive(Debug, Clone)]
struct SubmenuState {
    setting_id: SettingId,
    title: &'static str,
    description: &'static str,
    options: Vec<String>,
    selected_index: usize,
    original_theme: Option<String>,
}

pub struct SettingsSelectorComponent {
    keybindings: KeybindingsManager,
    config: SettingsConfig,
    selected_index: usize,
    submenu: Option<SubmenuState>,
    on_change: Option<ActionCallback<SettingsChange>>,
    on_theme_preview: Option<ActionCallback<String>>,
    on_cancel: Option<CancelCallback>,
    viewport_size: Cell<Option<(usize, usize)>>,
}

impl SettingsSelectorComponent {
    pub fn new(keybindings: &KeybindingsManager, config: SettingsConfig) -> Self {
        Self {
            keybindings: keybindings.clone(),
            config,
            selected_index: 0,
            submenu: None,
            on_change: None,
            on_theme_preview: None,
            on_cancel: None,
            viewport_size: Cell::new(None),
        }
    }

    pub fn set_on_change<F>(&mut self, on_change: F)
    where
        F: FnMut(SettingsChange) + Send + 'static,
    {
        self.on_change = Some(Box::new(on_change));
    }

    pub fn set_on_theme_preview<F>(&mut self, on_preview: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_theme_preview = Some(Box::new(on_preview));
    }

    pub fn set_on_cancel<F>(&mut self, on_cancel: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_cancel = Some(Box::new(on_cancel));
    }

    fn rows(&self) -> Vec<SettingRow> {
        let mut rows = vec![SettingRow {
            id: SettingId::AutoCompact,
            label: "Auto-compact",
            description: "Automatically compact context when it gets too large",
            value: bool_label(self.config.auto_compact).to_owned(),
        }];

        if self.config.supports_images {
            rows.push(SettingRow {
                id: SettingId::ShowImages,
                label: "Show images",
                description: "Render images inline in the terminal",
                value: bool_label(self.config.show_images).to_owned(),
            });
        }

        rows.extend([
            SettingRow {
                id: SettingId::AutoResizeImages,
                label: "Auto-resize images",
                description: "Resize large images before sending them to providers",
                value: bool_label(self.config.auto_resize_images).to_owned(),
            },
            SettingRow {
                id: SettingId::BlockImages,
                label: "Block images",
                description: "Prevent images from being sent to providers",
                value: bool_label(self.config.block_images).to_owned(),
            },
            SettingRow {
                id: SettingId::SkillCommands,
                label: "Skill commands",
                description: "Register skills as slash commands",
                value: bool_label(self.config.enable_skill_commands).to_owned(),
            },
            SettingRow {
                id: SettingId::SteeringMode,
                label: "Steering mode",
                description: "How queued steering messages are delivered while streaming",
                value: delivery_mode_label(self.config.steering_mode).to_owned(),
            },
            SettingRow {
                id: SettingId::FollowUpMode,
                label: "Follow-up mode",
                description: "How queued follow-up messages are delivered",
                value: delivery_mode_label(self.config.follow_up_mode).to_owned(),
            },
            SettingRow {
                id: SettingId::Transport,
                label: "Transport",
                description: "Preferred provider transport when multiple transports are supported",
                value: transport_label(self.config.transport).to_owned(),
            },
            SettingRow {
                id: SettingId::Thinking,
                label: "Thinking level",
                description: "Reasoning depth for thinking-capable models",
                value: thinking_level_label(self.config.thinking_level).to_owned(),
            },
            SettingRow {
                id: SettingId::Theme,
                label: "Theme",
                description: "Color theme for the interface",
                value: self.config.current_theme.clone(),
            },
            SettingRow {
                id: SettingId::HideThinking,
                label: "Hide thinking",
                description: "Hide thinking blocks in assistant responses",
                value: bool_label(self.config.hide_thinking_block).to_owned(),
            },
            SettingRow {
                id: SettingId::CollapseChangelog,
                label: "Collapse changelog",
                description: "Show condensed changelog after updates",
                value: bool_label(self.config.collapse_changelog).to_owned(),
            },
            SettingRow {
                id: SettingId::QuietStartup,
                label: "Quiet startup",
                description: "Disable verbose printing during startup",
                value: bool_label(self.config.quiet_startup).to_owned(),
            },
            SettingRow {
                id: SettingId::DoubleEscapeAction,
                label: "Double-escape action",
                description: "Action when pressing Escape twice with an empty editor",
                value: double_escape_action_label(self.config.double_escape_action).to_owned(),
            },
            SettingRow {
                id: SettingId::TreeFilterMode,
                label: "Tree filter mode",
                description: "Default filter when opening the session tree",
                value: tree_filter_mode_label(self.config.tree_filter_mode).to_owned(),
            },
            SettingRow {
                id: SettingId::ShowHardwareCursor,
                label: "Show hardware cursor",
                description: "Show the terminal cursor while still positioning it for IME support",
                value: bool_label(self.config.show_hardware_cursor).to_owned(),
            },
            SettingRow {
                id: SettingId::EditorPadding,
                label: "Editor padding",
                description: "Horizontal padding for the input editor",
                value: self.config.editor_padding_x.to_string(),
            },
            SettingRow {
                id: SettingId::AutocompleteMaxVisible,
                label: "Autocomplete max items",
                description: "Maximum visible items in autocomplete lists",
                value: self.config.autocomplete_max_visible.to_string(),
            },
            SettingRow {
                id: SettingId::ClearOnShrink,
                label: "Clear on shrink",
                description: "Clear empty rows when content shrinks",
                value: bool_label(self.config.clear_on_shrink).to_owned(),
            },
        ]);

        rows
    }

    fn emit_change(&mut self, change: SettingsChange) {
        if let Some(on_change) = &mut self.on_change {
            on_change(change);
        }
    }

    fn preview_theme(&mut self, theme: &str) {
        if let Some(on_theme_preview) = &mut self.on_theme_preview {
            on_theme_preview(theme.to_owned());
        }
    }

    fn open_submenu(&mut self, setting_id: SettingId) {
        let submenu = match setting_id {
            SettingId::SteeringMode => SubmenuState {
                setting_id,
                title: "Steering mode",
                description: "Select how queued steering messages are delivered",
                options: vec![String::from("one-at-a-time"), String::from("all")],
                selected_index: usize::from(!matches!(
                    self.config.steering_mode,
                    DeliveryMode::OneAtATime
                )),
                original_theme: None,
            },
            SettingId::FollowUpMode => SubmenuState {
                setting_id,
                title: "Follow-up mode",
                description: "Select how queued follow-up messages are delivered",
                options: vec![String::from("one-at-a-time"), String::from("all")],
                selected_index: usize::from(!matches!(
                    self.config.follow_up_mode,
                    DeliveryMode::OneAtATime
                )),
                original_theme: None,
            },
            SettingId::Transport => SubmenuState {
                setting_id,
                title: "Transport",
                description: "Select the preferred provider transport",
                options: vec![
                    String::from("sse"),
                    String::from("websocket"),
                    String::from("auto"),
                ],
                selected_index: match self.config.transport {
                    Transport::Sse => 0,
                    Transport::WebSocket => 1,
                    Transport::Auto => 2,
                },
                original_theme: None,
            },
            SettingId::Thinking => SubmenuState {
                setting_id,
                title: "Thinking level",
                description: "Select reasoning depth for thinking-capable models",
                options: self
                    .config
                    .available_thinking_levels
                    .iter()
                    .map(|level| thinking_level_label(*level).to_owned())
                    .collect(),
                selected_index: self
                    .config
                    .available_thinking_levels
                    .iter()
                    .position(|level| *level == self.config.thinking_level)
                    .unwrap_or(0),
                original_theme: None,
            },
            SettingId::Theme => SubmenuState {
                setting_id,
                title: "Theme",
                description: "Select the active color theme",
                options: self.config.available_themes.clone(),
                selected_index: self
                    .config
                    .available_themes
                    .iter()
                    .position(|theme| theme == &self.config.current_theme)
                    .unwrap_or(0),
                original_theme: Some(self.config.current_theme.clone()),
            },
            SettingId::DoubleEscapeAction => SubmenuState {
                setting_id,
                title: "Double-escape action",
                description: "Action when pressing Escape twice with an empty editor",
                options: vec![
                    String::from("tree"),
                    String::from("fork"),
                    String::from("none"),
                ],
                selected_index: match self.config.double_escape_action {
                    DoubleEscapeAction::Tree => 0,
                    DoubleEscapeAction::Fork => 1,
                    DoubleEscapeAction::None => 2,
                },
                original_theme: None,
            },
            SettingId::TreeFilterMode => SubmenuState {
                setting_id,
                title: "Tree filter mode",
                description: "Default filter when opening the session tree",
                options: vec![
                    String::from("default"),
                    String::from("no-tools"),
                    String::from("user-only"),
                    String::from("labeled-only"),
                    String::from("all"),
                ],
                selected_index: match self.config.tree_filter_mode {
                    TreeFilterMode::Default => 0,
                    TreeFilterMode::NoTools => 1,
                    TreeFilterMode::UserOnly => 2,
                    TreeFilterMode::LabeledOnly => 3,
                    TreeFilterMode::All => 4,
                },
                original_theme: None,
            },
            SettingId::EditorPadding => SubmenuState {
                setting_id,
                title: "Editor padding",
                description: "Horizontal padding for the input editor",
                options: vec![
                    String::from("0"),
                    String::from("1"),
                    String::from("2"),
                    String::from("3"),
                ],
                selected_index: [0_usize, 1, 2, 3]
                    .iter()
                    .position(|value| *value == self.config.editor_padding_x)
                    .unwrap_or(0),
                original_theme: None,
            },
            SettingId::AutocompleteMaxVisible => SubmenuState {
                setting_id,
                title: "Autocomplete max items",
                description: "Maximum visible items in autocomplete lists",
                options: vec![
                    String::from("3"),
                    String::from("5"),
                    String::from("7"),
                    String::from("10"),
                    String::from("15"),
                    String::from("20"),
                ],
                selected_index: [3_usize, 5, 7, 10, 15, 20]
                    .iter()
                    .position(|value| *value == self.config.autocomplete_max_visible)
                    .unwrap_or(0),
                original_theme: None,
            },
            _ => return,
        };
        self.submenu = Some(submenu);
    }

    fn apply_submenu_option(&mut self, option: &str) {
        let Some(submenu) = self.submenu.as_ref() else {
            return;
        };

        match submenu.setting_id {
            SettingId::SteeringMode => {
                let mode = parse_delivery_mode(option);
                self.config.steering_mode = mode;
                self.emit_change(SettingsChange::SteeringMode(mode));
            }
            SettingId::FollowUpMode => {
                let mode = parse_delivery_mode(option);
                self.config.follow_up_mode = mode;
                self.emit_change(SettingsChange::FollowUpMode(mode));
            }
            SettingId::Transport => {
                let transport = parse_transport(option);
                self.config.transport = transport;
                self.emit_change(SettingsChange::Transport(transport));
            }
            SettingId::Thinking => {
                let level = parse_thinking_level(option);
                self.config.thinking_level = level;
                self.emit_change(SettingsChange::ThinkingLevel(level));
            }
            SettingId::Theme => {
                self.config.current_theme = option.to_owned();
                self.emit_change(SettingsChange::Theme(option.to_owned()));
            }
            SettingId::DoubleEscapeAction => {
                let action = parse_double_escape_action(option);
                self.config.double_escape_action = action;
                self.emit_change(SettingsChange::DoubleEscapeAction(action));
            }
            SettingId::TreeFilterMode => {
                let mode = parse_tree_filter_mode(option);
                self.config.tree_filter_mode = mode;
                self.emit_change(SettingsChange::TreeFilterMode(mode));
            }
            SettingId::EditorPadding => {
                let value = option
                    .parse::<usize>()
                    .unwrap_or(self.config.editor_padding_x);
                self.config.editor_padding_x = value;
                self.emit_change(SettingsChange::EditorPaddingX(value));
            }
            SettingId::AutocompleteMaxVisible => {
                let value = option
                    .parse::<usize>()
                    .unwrap_or(self.config.autocomplete_max_visible);
                self.config.autocomplete_max_visible = value;
                self.emit_change(SettingsChange::AutocompleteMaxVisible(value));
            }
            _ => {}
        }
    }

    fn toggle_selected_setting(&mut self) {
        let rows = self.rows();
        let Some(row) = rows.get(self.selected_index) else {
            return;
        };

        match row.id {
            SettingId::AutoCompact => {
                self.config.auto_compact = !self.config.auto_compact;
                self.emit_change(SettingsChange::AutoCompact(self.config.auto_compact));
            }
            SettingId::ShowImages => {
                self.config.show_images = !self.config.show_images;
                self.emit_change(SettingsChange::ShowImages(self.config.show_images));
            }
            SettingId::AutoResizeImages => {
                self.config.auto_resize_images = !self.config.auto_resize_images;
                self.emit_change(SettingsChange::AutoResizeImages(
                    self.config.auto_resize_images,
                ));
            }
            SettingId::BlockImages => {
                self.config.block_images = !self.config.block_images;
                self.emit_change(SettingsChange::BlockImages(self.config.block_images));
            }
            SettingId::SkillCommands => {
                self.config.enable_skill_commands = !self.config.enable_skill_commands;
                self.emit_change(SettingsChange::EnableSkillCommands(
                    self.config.enable_skill_commands,
                ));
            }
            SettingId::HideThinking => {
                self.config.hide_thinking_block = !self.config.hide_thinking_block;
                self.emit_change(SettingsChange::HideThinkingBlock(
                    self.config.hide_thinking_block,
                ));
            }
            SettingId::CollapseChangelog => {
                self.config.collapse_changelog = !self.config.collapse_changelog;
                self.emit_change(SettingsChange::CollapseChangelog(
                    self.config.collapse_changelog,
                ));
            }
            SettingId::QuietStartup => {
                self.config.quiet_startup = !self.config.quiet_startup;
                self.emit_change(SettingsChange::QuietStartup(self.config.quiet_startup));
            }
            SettingId::ShowHardwareCursor => {
                self.config.show_hardware_cursor = !self.config.show_hardware_cursor;
                self.emit_change(SettingsChange::ShowHardwareCursor(
                    self.config.show_hardware_cursor,
                ));
            }
            SettingId::ClearOnShrink => {
                self.config.clear_on_shrink = !self.config.clear_on_shrink;
                self.emit_change(SettingsChange::ClearOnShrink(self.config.clear_on_shrink));
            }
            _ => self.open_submenu(row.id),
        }
    }

    fn render_main_list(&self, width: usize) -> Vec<String> {
        let rows = self.rows();
        let max_visible = max_visible(&self.viewport_size, 8, 12);
        let (start_index, end_index) = visible_window(self.selected_index, rows.len(), max_visible);
        let mut lines = Vec::new();

        for (visible_index, row) in rows[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == self.selected_index {
                "→ "
            } else {
                "  "
            };
            let label = format!("{prefix}{}", row.label);
            let spacing = width.saturating_sub(label.len() + row.value.len()).max(1);
            lines.push(truncate_to_width(
                &format!("{label}{}{value}", " ".repeat(spacing), value = row.value),
                width,
                "...",
                false,
            ));
        }

        if let Some(selected) = rows.get(self.selected_index) {
            lines.push(String::new());
            lines.push(truncate_to_width(selected.description, width, "...", false));
        }

        lines
    }

    fn render_submenu(&self, width: usize) -> Vec<String> {
        let Some(submenu) = self.submenu.as_ref() else {
            return Vec::new();
        };
        let max_visible = max_visible(&self.viewport_size, 7, 10);
        let (start_index, end_index) =
            visible_window(submenu.selected_index, submenu.options.len(), max_visible);
        let mut lines = Vec::new();
        lines.push(truncate_to_width(submenu.description, width, "...", false));

        for (visible_index, option) in submenu.options[start_index..end_index].iter().enumerate() {
            let actual_index = start_index + visible_index;
            let prefix = if actual_index == submenu.selected_index {
                "→ "
            } else {
                "  "
            };
            lines.push(truncate_to_width(
                &format!("{prefix}{option}"),
                width,
                "...",
                false,
            ));
        }

        lines
    }
}

impl Component for SettingsSelectorComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let title = self
            .submenu
            .as_ref()
            .map_or("Settings", |submenu| submenu.title);
        let body = if self.submenu.is_some() {
            self.render_submenu(width)
        } else {
            self.render_main_list(width)
        };
        let hint_line = render_hint_line(
            &self.keybindings,
            width,
            &[
                ("tui.select.confirm", "select"),
                ("tui.select.cancel", "back"),
                ("tui.select.down", "navigate"),
            ],
        );
        let mut lines = Vec::new();
        lines.push("─".repeat(width));
        lines.push(truncate_to_width(title, width, "...", false));
        lines.extend(body);
        lines.push(hint_line);
        lines.push("─".repeat(width));
        lines
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        if self.submenu.is_some() {
            if matches_binding(&self.keybindings, data, "tui.select.cancel") {
                let original_theme = self
                    .submenu
                    .as_ref()
                    .and_then(|submenu| submenu.original_theme.clone());
                if let Some(original_theme) = original_theme {
                    self.preview_theme(&original_theme);
                }
                self.submenu = None;
                return;
            }

            if matches_binding(&self.keybindings, data, "tui.select.up") {
                let preview = if let Some(submenu) = &mut self.submenu {
                    submenu.selected_index = submenu.selected_index.saturating_sub(1);
                    if matches!(submenu.setting_id, SettingId::Theme) {
                        submenu.options.get(submenu.selected_index).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(preview) = preview {
                    self.preview_theme(&preview);
                }
                return;
            }

            if matches_binding(&self.keybindings, data, "tui.select.down") {
                let preview = if let Some(submenu) = &mut self.submenu {
                    submenu.selected_index =
                        (submenu.selected_index + 1).min(submenu.options.len().saturating_sub(1));
                    if matches!(submenu.setting_id, SettingId::Theme) {
                        submenu.options.get(submenu.selected_index).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(preview) = preview {
                    self.preview_theme(&preview);
                }
                return;
            }

            if matches_binding(&self.keybindings, data, "tui.select.pageUp") {
                let preview =
                    if let Some(submenu) = &mut self.submenu {
                        submenu.selected_index = submenu
                            .selected_index
                            .saturating_sub(max_visible(&self.viewport_size, 7, 10));
                        if matches!(submenu.setting_id, SettingId::Theme) {
                            submenu.options.get(submenu.selected_index).cloned()
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                if let Some(preview) = preview {
                    self.preview_theme(&preview);
                }
                return;
            }

            if matches_binding(&self.keybindings, data, "tui.select.pageDown") {
                let preview = if let Some(submenu) = &mut self.submenu {
                    submenu.selected_index = (submenu.selected_index
                        + max_visible(&self.viewport_size, 7, 10))
                    .min(submenu.options.len().saturating_sub(1));
                    if matches!(submenu.setting_id, SettingId::Theme) {
                        submenu.options.get(submenu.selected_index).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(preview) = preview {
                    self.preview_theme(&preview);
                }
                return;
            }

            if matches_binding(&self.keybindings, data, "tui.select.confirm") {
                let option = self
                    .submenu
                    .as_ref()
                    .and_then(|submenu| submenu.options.get(submenu.selected_index).cloned());
                if let Some(option) = option {
                    self.apply_submenu_option(&option);
                }
                self.submenu = None;
                return;
            }

            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.cancel") {
            if let Some(on_cancel) = &mut self.on_cancel {
                on_cancel();
            }
            return;
        }

        let rows = self.rows();
        if matches_binding(&self.keybindings, data, "tui.select.up") {
            self.selected_index = self.selected_index.saturating_sub(1);
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.down") {
            self.selected_index = (self.selected_index + 1).min(rows.len().saturating_sub(1));
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
                .min(rows.len().saturating_sub(1));
            return;
        }

        if matches_binding(&self.keybindings, data, "tui.select.confirm") {
            self.toggle_selected_setting();
        }
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.viewport_size.set(Some((width, height)));
    }
}

fn bool_label(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn delivery_mode_label(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::All => "all",
        DeliveryMode::OneAtATime => "one-at-a-time",
    }
}

fn parse_delivery_mode(value: &str) -> DeliveryMode {
    match value {
        "all" => DeliveryMode::All,
        _ => DeliveryMode::OneAtATime,
    }
}

fn transport_label(transport: Transport) -> &'static str {
    match transport {
        Transport::Sse => "sse",
        Transport::WebSocket => "websocket",
        Transport::Auto => "auto",
    }
}

fn parse_transport(value: &str) -> Transport {
    match value {
        "websocket" => Transport::WebSocket,
        "auto" => Transport::Auto,
        _ => Transport::Sse,
    }
}

fn thinking_level_label(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
    }
}

fn parse_thinking_level(value: &str) -> ThinkingLevel {
    match value {
        "minimal" => ThinkingLevel::Minimal,
        "low" => ThinkingLevel::Low,
        "medium" => ThinkingLevel::Medium,
        "high" => ThinkingLevel::High,
        "xhigh" => ThinkingLevel::XHigh,
        _ => ThinkingLevel::Off,
    }
}

fn double_escape_action_label(action: DoubleEscapeAction) -> &'static str {
    match action {
        DoubleEscapeAction::Fork => "fork",
        DoubleEscapeAction::Tree => "tree",
        DoubleEscapeAction::None => "none",
    }
}

fn parse_double_escape_action(value: &str) -> DoubleEscapeAction {
    match value {
        "fork" => DoubleEscapeAction::Fork,
        "tree" => DoubleEscapeAction::Tree,
        _ => DoubleEscapeAction::None,
    }
}

fn tree_filter_mode_label(mode: TreeFilterMode) -> &'static str {
    match mode {
        TreeFilterMode::Default => "default",
        TreeFilterMode::NoTools => "no-tools",
        TreeFilterMode::UserOnly => "user-only",
        TreeFilterMode::LabeledOnly => "labeled-only",
        TreeFilterMode::All => "all",
    }
}

fn parse_tree_filter_mode(value: &str) -> TreeFilterMode {
    match value {
        "no-tools" => TreeFilterMode::NoTools,
        "user-only" => TreeFilterMode::UserOnly,
        "labeled-only" => TreeFilterMode::LabeledOnly,
        "all" => TreeFilterMode::All,
        _ => TreeFilterMode::Default,
    }
}
