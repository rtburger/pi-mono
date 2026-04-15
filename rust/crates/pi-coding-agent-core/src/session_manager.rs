use crate::messages::{
    BashExecutionMessage, BranchSummaryMessage, CompactionSummaryMessage, CustomMessage,
    CustomMessageContent, create_branch_summary_message, create_compaction_summary_message,
    create_custom_message,
};
use pi_agent::AgentMessage;
use pi_events::{AssistantContent, Message, StopReason, Usage, UsageCost, UserContent};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const CURRENT_SESSION_VERSION: u32 = 3;

const CONFIG_DIR_NAME: &str = ".pi";
const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum SessionManagerError {
    #[error("Entry {0} not found")]
    EntryNotFound(String),
    #[error("Cannot fork: source session file is empty or invalid: {0}")]
    InvalidSourceSession(String),
    #[error("Cannot fork: source session has no header: {0}")]
    MissingSourceHeader(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewSessionOptions {
    pub id: Option<String>,
    pub parent_session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHeader {
    pub version: Option<u32>,
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    pub parent_session: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEntry {
    Message {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        message: AgentMessage,
    },
    ThinkingLevelChange {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        thinking_level: String,
    },
    ModelChange {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        provider: String,
        model_id: String,
    },
    Compaction {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        summary: String,
        first_kept_entry_id: String,
        tokens_before: u64,
        details: Option<Value>,
        from_hook: Option<bool>,
    },
    BranchSummary {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        from_id: String,
        summary: String,
        details: Option<Value>,
        from_hook: Option<bool>,
    },
    Custom {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        custom_type: String,
        data: Option<Value>,
    },
    CustomMessage {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        custom_type: String,
        content: CustomMessageContent,
        details: Option<Value>,
        display: bool,
    },
    Label {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        target_id: String,
        label: Option<String>,
    },
    SessionInfo {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        name: Option<String>,
    },
}

impl SessionEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::Message { id, .. }
            | Self::ThinkingLevelChange { id, .. }
            | Self::ModelChange { id, .. }
            | Self::Compaction { id, .. }
            | Self::BranchSummary { id, .. }
            | Self::Custom { id, .. }
            | Self::CustomMessage { id, .. }
            | Self::Label { id, .. }
            | Self::SessionInfo { id, .. } => id,
        }
    }

    pub fn parent_id(&self) -> Option<&str> {
        match self {
            Self::Message { parent_id, .. }
            | Self::ThinkingLevelChange { parent_id, .. }
            | Self::ModelChange { parent_id, .. }
            | Self::Compaction { parent_id, .. }
            | Self::BranchSummary { parent_id, .. }
            | Self::Custom { parent_id, .. }
            | Self::CustomMessage { parent_id, .. }
            | Self::Label { parent_id, .. }
            | Self::SessionInfo { parent_id, .. } => parent_id.as_deref(),
        }
    }

    pub fn timestamp(&self) -> &str {
        match self {
            Self::Message { timestamp, .. }
            | Self::ThinkingLevelChange { timestamp, .. }
            | Self::ModelChange { timestamp, .. }
            | Self::Compaction { timestamp, .. }
            | Self::BranchSummary { timestamp, .. }
            | Self::Custom { timestamp, .. }
            | Self::CustomMessage { timestamp, .. }
            | Self::Label { timestamp, .. }
            | Self::SessionInfo { timestamp, .. } => timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileEntry {
    Session(SessionHeader),
    Entry(SessionEntry),
}

impl FileEntry {
    pub fn is_session(&self) -> bool {
        matches!(self, Self::Session(_))
    }

    pub fn as_session_header(&self) -> Option<&SessionHeader> {
        match self {
            Self::Session(header) => Some(header),
            Self::Entry(_) => None,
        }
    }

    pub fn as_session_entry(&self) -> Option<&SessionEntry> {
        match self {
            Self::Session(_) => None,
            Self::Entry(entry) => Some(entry),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionModelSelection {
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionContext {
    pub messages: Vec<AgentMessage>,
    pub thinking_level: String,
    pub model: Option<SessionModelSelection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionTreeNode {
    pub entry: SessionEntry,
    pub children: Vec<SessionTreeNode>,
    pub label: Option<String>,
    pub label_timestamp: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub path: String,
    pub id: String,
    pub cwd: String,
    pub name: Option<String>,
    pub parent_session_path: Option<String>,
    pub created: SystemTime,
    pub modified: SystemTime,
    pub message_count: usize,
    pub first_message: String,
    pub all_messages_text: String,
}

pub fn build_session_context(entries: &[SessionEntry], leaf_id: Option<&str>) -> SessionContext {
    build_session_context_internal(entries, leaf_id, false)
}

pub fn get_latest_compaction_entry(entries: &[SessionEntry]) -> Option<SessionEntry> {
    entries
        .iter()
        .rev()
        .find(|entry| matches!(entry, SessionEntry::Compaction { .. }))
        .cloned()
}

pub fn parse_session_entries(content: &str) -> Vec<FileEntry> {
    let mut raw_entries = parse_raw_entries(content);
    if raw_entries.is_empty() {
        return Vec::new();
    }

    if raw_entries.first().is_some_and(is_session_header_value) {
        migrate_raw_entries_to_current(&mut raw_entries);
    }

    raw_entries
        .into_iter()
        .filter_map(|entry| raw_file_entry_to_typed(&entry))
        .collect()
}

pub fn load_entries_from_file(path: impl AsRef<Path>) -> Vec<FileEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let entries = parse_session_entries(&content);
    if entries.first().is_some_and(FileEntry::is_session) {
        entries
    } else {
        Vec::new()
    }
}

pub fn find_most_recent_session(session_dir: impl AsRef<Path>) -> Option<String> {
    let Ok(entries) = fs::read_dir(session_dir) else {
        return None;
    };

    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .filter(|path| !load_entries_from_file(path).is_empty())
        .filter_map(|path| {
            let metadata = fs::metadata(&path).ok()?;
            let modified = metadata.modified().ok()?;
            Some((path, modified))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| right.1.cmp(&left.1));
    candidates
        .first()
        .map(|(path, _)| path.to_string_lossy().into_owned())
}

pub fn get_sessions_dir(agent_dir: Option<&str>) -> String {
    Path::new(&default_agent_dir(agent_dir))
        .join("sessions")
        .to_string_lossy()
        .into_owned()
}

pub fn get_default_session_dir(cwd: &str, agent_dir: Option<&str>) -> String {
    let trimmed = cwd.trim_start_matches(['/', '\\']);
    let safe = trimmed
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' => '-',
            _ => character,
        })
        .collect::<String>();

    Path::new(&get_sessions_dir(agent_dir))
        .join(format!("--{safe}--"))
        .to_string_lossy()
        .into_owned()
}

pub struct SessionManager {
    header: SessionHeader,
    cwd: String,
    session_file: Option<String>,
    session_dir: String,
    persist: bool,
    flushed: bool,
    entries: Vec<SessionEntry>,
    by_id: HashMap<String, SessionEntry>,
    labels_by_id: HashMap<String, String>,
    label_timestamps_by_id: HashMap<String, String>,
    leaf_id: Option<String>,
}

impl SessionManager {
    pub fn create(cwd: &str, session_dir: Option<&str>) -> Result<Self, SessionManagerError> {
        let dir = session_dir
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| get_default_session_dir(cwd, None));
        Self::new(cwd.to_owned(), dir, None, true)
    }

    pub fn open(
        path: &str,
        session_dir: Option<&str>,
        cwd_override: Option<&str>,
    ) -> Result<Self, SessionManagerError> {
        let loaded = read_session_file(path);
        let stored_cwd = loaded
            .entries
            .first()
            .and_then(FileEntry::as_session_header)
            .map(|header| header.cwd.clone());
        let cwd = cwd_override
            .map(ToOwned::to_owned)
            .or(stored_cwd)
            .unwrap_or_else(current_working_directory_string);
        let resolved_path = resolve_path_string(path);
        let dir = session_dir.map(ToOwned::to_owned).unwrap_or_else(|| {
            Path::new(&resolved_path)
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_string_lossy()
                .into_owned()
        });

        Self::new(cwd, dir, Some(resolved_path), true)
    }

    pub fn continue_recent(
        cwd: &str,
        session_dir: Option<&str>,
    ) -> Result<Self, SessionManagerError> {
        let dir = session_dir
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| get_default_session_dir(cwd, None));
        if let Some(path) = find_most_recent_session(&dir) {
            return Self::new(cwd.to_owned(), dir, Some(path), true);
        }
        Self::new(cwd.to_owned(), dir, None, true)
    }

    pub fn in_memory(cwd: &str) -> Self {
        Self::new(cwd.to_owned(), String::new(), None, false)
            .expect("in-memory session manager should always initialize")
    }

    pub fn fork_from(
        source_path: &str,
        target_cwd: &str,
        session_dir: Option<&str>,
    ) -> Result<Self, SessionManagerError> {
        let source_entries = load_entries_from_file(source_path);
        if source_entries.is_empty() {
            return Err(SessionManagerError::InvalidSourceSession(
                source_path.to_owned(),
            ));
        }
        let source_header = source_entries
            .first()
            .and_then(FileEntry::as_session_header)
            .ok_or_else(|| SessionManagerError::MissingSourceHeader(source_path.to_owned()))?;

        let dir = session_dir
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| get_default_session_dir(target_cwd, None));
        fs::create_dir_all(&dir)?;

        let timestamp = current_timestamp_iso();
        let session_id = generate_session_id();
        let file_path = Path::new(&dir)
            .join(session_file_name(&timestamp, &session_id))
            .to_string_lossy()
            .into_owned();

        let header = SessionHeader {
            version: Some(CURRENT_SESSION_VERSION),
            id: session_id,
            timestamp,
            cwd: target_cwd.to_owned(),
            parent_session: Some(resolve_path_string(source_path)),
        };

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&file_path)?;
        writeln!(
            file,
            "{}",
            value_to_json_line(&typed_file_entry_to_raw(&FileEntry::Session(
                header.clone()
            )))
        )?;
        for entry in source_entries.iter().skip(1) {
            writeln!(
                file,
                "{}",
                value_to_json_line(&typed_file_entry_to_raw(entry))
            )?;
        }
        drop(file);

        let mut manager = Self::new(target_cwd.to_owned(), dir, Some(file_path), true)?;
        manager.header.parent_session = Some(resolve_path_string(
            source_header
                .parent_session
                .as_deref()
                .unwrap_or(source_path),
        ));
        Ok(manager)
    }

    pub fn list(cwd: &str, session_dir: Option<&str>) -> Vec<SessionInfo> {
        let dir = session_dir
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| get_default_session_dir(cwd, None));
        let mut sessions = list_sessions_from_dir(&dir);
        sessions.sort_by(|left, right| right.modified.cmp(&left.modified));
        sessions
    }

    pub fn list_all(agent_dir: Option<&str>) -> Vec<SessionInfo> {
        let sessions_dir = get_sessions_dir(agent_dir);
        let Ok(entries) = fs::read_dir(sessions_dir) else {
            return Vec::new();
        };

        let mut sessions = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            sessions.extend(list_sessions_from_dir(&path));
        }
        sessions.sort_by(|left, right| right.modified.cmp(&left.modified));
        sessions
    }

    fn new(
        cwd: String,
        session_dir: String,
        session_file: Option<String>,
        persist: bool,
    ) -> Result<Self, SessionManagerError> {
        if persist && !session_dir.is_empty() {
            fs::create_dir_all(&session_dir)?;
        }

        let mut manager = Self {
            header: SessionHeader {
                version: Some(CURRENT_SESSION_VERSION),
                id: String::new(),
                timestamp: current_timestamp_iso(),
                cwd: cwd.clone(),
                parent_session: None,
            },
            cwd,
            session_file: None,
            session_dir,
            persist,
            flushed: false,
            entries: Vec::new(),
            by_id: HashMap::new(),
            labels_by_id: HashMap::new(),
            label_timestamps_by_id: HashMap::new(),
            leaf_id: None,
        };

        if let Some(path) = session_file {
            manager.set_session_file(&path)?;
        } else {
            manager.new_session(NewSessionOptions::default());
        }

        Ok(manager)
    }

    pub fn set_session_file(&mut self, session_file: &str) -> Result<(), SessionManagerError> {
        let resolved = resolve_path_string(session_file);
        self.session_file = Some(resolved.clone());

        if Path::new(&resolved).exists() {
            let loaded = read_session_file(&resolved);
            if loaded.entries.is_empty() {
                let explicit_path = resolved.clone();
                self.new_session(NewSessionOptions::default());
                self.session_file = Some(explicit_path);
                self.rewrite_file()?;
                self.flushed = true;
                return Ok(());
            }

            let Some(header) = loaded
                .entries
                .first()
                .and_then(FileEntry::as_session_header)
                .cloned()
            else {
                let explicit_path = resolved.clone();
                self.new_session(NewSessionOptions::default());
                self.session_file = Some(explicit_path);
                self.rewrite_file()?;
                self.flushed = true;
                return Ok(());
            };

            self.header = header;
            self.entries = loaded
                .entries
                .into_iter()
                .filter_map(|entry| match entry {
                    FileEntry::Session(_) => None,
                    FileEntry::Entry(entry) => Some(entry),
                })
                .collect();
            self.build_index();
            self.flushed = true;

            if loaded.migrated {
                self.rewrite_file()?;
            }
            return Ok(());
        }

        let explicit_path = resolved;
        self.new_session(NewSessionOptions::default());
        self.session_file = Some(explicit_path);
        Ok(())
    }

    pub fn new_session(&mut self, options: NewSessionOptions) -> Option<String> {
        let timestamp = current_timestamp_iso();
        self.header = SessionHeader {
            version: Some(CURRENT_SESSION_VERSION),
            id: options.id.unwrap_or_else(generate_session_id),
            timestamp: timestamp.clone(),
            cwd: self.cwd.clone(),
            parent_session: options.parent_session,
        };
        self.entries.clear();
        self.by_id.clear();
        self.labels_by_id.clear();
        self.label_timestamps_by_id.clear();
        self.leaf_id = None;
        self.flushed = false;

        if self.persist {
            self.session_file = Some(
                Path::new(&self.session_dir)
                    .join(session_file_name(&timestamp, &self.header.id))
                    .to_string_lossy()
                    .into_owned(),
            );
        }

        self.session_file.clone()
    }

    fn build_index(&mut self) {
        self.by_id.clear();
        self.labels_by_id.clear();
        self.label_timestamps_by_id.clear();
        self.leaf_id = None;

        for entry in &self.entries {
            self.by_id.insert(entry.id().to_owned(), entry.clone());
            self.leaf_id = Some(entry.id().to_owned());
            if let SessionEntry::Label {
                target_id,
                label,
                timestamp,
                ..
            } = entry
            {
                if let Some(label) = label.clone() {
                    self.labels_by_id.insert(target_id.clone(), label);
                    self.label_timestamps_by_id
                        .insert(target_id.clone(), timestamp.clone());
                } else {
                    self.labels_by_id.remove(target_id);
                    self.label_timestamps_by_id.remove(target_id);
                }
            }
        }
    }

    fn rewrite_file(&self) -> Result<(), SessionManagerError> {
        if !self.persist {
            return Ok(());
        }
        let Some(session_file) = self.session_file.as_ref() else {
            return Ok(());
        };
        if let Some(parent) = Path::new(session_file).parent() {
            fs::create_dir_all(parent)?;
        }

        let mut content = String::new();
        content.push_str(&value_to_json_line(&typed_file_entry_to_raw(
            &FileEntry::Session(self.header.clone()),
        )));
        content.push('\n');
        for entry in &self.entries {
            content.push_str(&value_to_json_line(&typed_file_entry_to_raw(
                &FileEntry::Entry(entry.clone()),
            )));
            content.push('\n');
        }
        fs::write(session_file, content)?;
        Ok(())
    }

    fn persist_entry(&mut self, entry: &SessionEntry) -> Result<(), SessionManagerError> {
        if !self.persist {
            return Ok(());
        }
        let Some(session_file) = self.session_file.as_ref() else {
            return Ok(());
        };

        if !self
            .entries
            .iter()
            .any(|current| matches!(current, SessionEntry::Message { message, .. } if message.is_assistant()))
        {
            self.flushed = false;
            return Ok(());
        }

        if !self.flushed {
            self.rewrite_file()?;
            self.flushed = true;
            return Ok(());
        }

        if let Some(parent) = Path::new(session_file).parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(session_file)?;
        writeln!(
            file,
            "{}",
            value_to_json_line(&typed_file_entry_to_raw(&FileEntry::Entry(entry.clone())))
        )?;
        Ok(())
    }

    fn append_entry(&mut self, entry: SessionEntry) -> Result<String, SessionManagerError> {
        let id = entry.id().to_owned();
        self.entries.push(entry.clone());
        self.by_id.insert(id.clone(), entry.clone());
        self.leaf_id = Some(id.clone());
        self.persist_entry(&entry)?;
        Ok(id)
    }

    pub fn append_message(
        &mut self,
        message: impl Into<AgentMessage>,
    ) -> Result<String, SessionManagerError> {
        let entry = SessionEntry::Message {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            message: message.into(),
        };
        self.append_entry(entry)
    }

    pub fn append_thinking_level_change(
        &mut self,
        thinking_level: impl Into<String>,
    ) -> Result<String, SessionManagerError> {
        let entry = SessionEntry::ThinkingLevelChange {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            thinking_level: thinking_level.into(),
        };
        self.append_entry(entry)
    }

    pub fn append_model_change(
        &mut self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<String, SessionManagerError> {
        let entry = SessionEntry::ModelChange {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            provider: provider.into(),
            model_id: model_id.into(),
        };
        self.append_entry(entry)
    }

    pub fn append_compaction(
        &mut self,
        summary: impl Into<String>,
        first_kept_entry_id: impl Into<String>,
        tokens_before: u64,
        details: Option<Value>,
        from_hook: Option<bool>,
    ) -> Result<String, SessionManagerError> {
        let entry = SessionEntry::Compaction {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            summary: summary.into(),
            first_kept_entry_id: first_kept_entry_id.into(),
            tokens_before,
            details,
            from_hook,
        };
        self.append_entry(entry)
    }

    pub fn append_custom_entry(
        &mut self,
        custom_type: impl Into<String>,
        data: Option<Value>,
    ) -> Result<String, SessionManagerError> {
        let entry = SessionEntry::Custom {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            custom_type: custom_type.into(),
            data,
        };
        self.append_entry(entry)
    }

    pub fn append_session_info(
        &mut self,
        name: impl Into<String>,
    ) -> Result<String, SessionManagerError> {
        let trimmed = name.into().trim().to_owned();
        let entry = SessionEntry::SessionInfo {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            name: (!trimmed.is_empty()).then_some(trimmed),
        };
        self.append_entry(entry)
    }

    pub fn get_session_name(&self) -> Option<String> {
        self.entries.iter().rev().find_map(|entry| {
            let SessionEntry::SessionInfo { name, .. } = entry else {
                return None;
            };
            name.as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
        })
    }

    pub fn append_custom_message_entry(
        &mut self,
        custom_type: impl Into<String>,
        content: CustomMessageContent,
        display: bool,
        details: Option<Value>,
    ) -> Result<String, SessionManagerError> {
        let entry = SessionEntry::CustomMessage {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            custom_type: custom_type.into(),
            content,
            details,
            display,
        };
        self.append_entry(entry)
    }

    pub fn is_persisted(&self) -> bool {
        self.persist
    }

    pub fn get_cwd(&self) -> &str {
        &self.cwd
    }

    pub fn get_session_dir(&self) -> &str {
        &self.session_dir
    }

    pub fn get_session_id(&self) -> &str {
        &self.header.id
    }

    pub fn get_session_file(&self) -> Option<&str> {
        self.session_file.as_deref()
    }

    pub fn get_leaf_id(&self) -> Option<&str> {
        self.leaf_id.as_deref()
    }

    pub fn get_leaf_entry(&self) -> Option<&SessionEntry> {
        self.leaf_id.as_ref().and_then(|id| self.by_id.get(id))
    }

    pub fn get_entry(&self, id: &str) -> Option<&SessionEntry> {
        self.by_id.get(id)
    }

    pub fn get_children(&self, parent_id: &str) -> Vec<SessionEntry> {
        self.by_id
            .values()
            .filter(|entry| entry.parent_id() == Some(parent_id))
            .cloned()
            .collect()
    }

    pub fn get_label(&self, id: &str) -> Option<&str> {
        self.labels_by_id.get(id).map(String::as_str)
    }

    pub fn append_label_change(
        &mut self,
        target_id: &str,
        label: Option<String>,
    ) -> Result<String, SessionManagerError> {
        if !self.by_id.contains_key(target_id) {
            return Err(SessionManagerError::EntryNotFound(target_id.to_owned()));
        }

        let entry = SessionEntry::Label {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: self.leaf_id.clone(),
            timestamp: current_timestamp_iso(),
            target_id: target_id.to_owned(),
            label,
        };

        if let SessionEntry::Label {
            target_id,
            label,
            timestamp,
            ..
        } = &entry
        {
            if let Some(label) = label.clone() {
                self.labels_by_id.insert(target_id.clone(), label);
                self.label_timestamps_by_id
                    .insert(target_id.clone(), timestamp.clone());
            } else {
                self.labels_by_id.remove(target_id);
                self.label_timestamps_by_id.remove(target_id);
            }
        }

        self.append_entry(entry)
    }

    pub fn get_branch(&self, from_id: Option<&str>) -> Vec<SessionEntry> {
        let mut path = Vec::new();
        let start_id = from_id.or(self.leaf_id.as_deref());
        let mut current = start_id.and_then(|id| self.by_id.get(id));
        while let Some(entry) = current {
            path.push(entry.clone());
            current = entry
                .parent_id()
                .and_then(|parent_id| self.by_id.get(parent_id));
        }
        path.reverse();
        path
    }

    pub fn build_session_context(&self) -> SessionContext {
        build_session_context_internal(&self.entries, self.leaf_id.as_deref(), true)
    }

    pub fn get_header(&self) -> &SessionHeader {
        &self.header
    }

    pub fn get_entries(&self) -> &[SessionEntry] {
        &self.entries
    }

    pub fn get_tree(&self) -> Vec<SessionTreeNode> {
        let mut entries_by_id = HashMap::<String, SessionEntry>::new();
        let mut children_by_parent = HashMap::<Option<String>, Vec<String>>::new();
        let mut roots = Vec::new();

        for entry in &self.entries {
            entries_by_id.insert(entry.id().to_owned(), entry.clone());
        }

        for entry in &self.entries {
            let parent_id = entry.parent_id().map(ToOwned::to_owned);
            let is_root = match parent_id.as_deref() {
                None => true,
                Some(parent_id) if parent_id == entry.id() => true,
                Some(parent_id) => !entries_by_id.contains_key(parent_id),
            };

            if is_root {
                roots.push(entry.id().to_owned());
            } else {
                children_by_parent
                    .entry(parent_id)
                    .or_default()
                    .push(entry.id().to_owned());
            }
        }

        let mut tree = roots
            .into_iter()
            .filter_map(|id| {
                build_tree_node(
                    &id,
                    &entries_by_id,
                    &children_by_parent,
                    &self.labels_by_id,
                    &self.label_timestamps_by_id,
                )
            })
            .collect::<Vec<_>>();
        sort_tree_children(&mut tree);
        tree
    }

    pub fn branch(&mut self, branch_from_id: &str) -> Result<(), SessionManagerError> {
        if !self.by_id.contains_key(branch_from_id) {
            return Err(SessionManagerError::EntryNotFound(
                branch_from_id.to_owned(),
            ));
        }
        self.leaf_id = Some(branch_from_id.to_owned());
        Ok(())
    }

    pub fn reset_leaf(&mut self) {
        self.leaf_id = None;
    }

    pub fn branch_with_summary(
        &mut self,
        branch_from_id: Option<&str>,
        summary: impl Into<String>,
        details: Option<Value>,
        from_hook: Option<bool>,
    ) -> Result<String, SessionManagerError> {
        if let Some(branch_from_id) = branch_from_id
            && !self.by_id.contains_key(branch_from_id)
        {
            return Err(SessionManagerError::EntryNotFound(
                branch_from_id.to_owned(),
            ));
        }

        self.leaf_id = branch_from_id.map(ToOwned::to_owned);
        let entry = SessionEntry::BranchSummary {
            id: generate_entry_id(self.by_id.keys().map(|key| key.as_str())),
            parent_id: branch_from_id.map(ToOwned::to_owned),
            timestamp: current_timestamp_iso(),
            from_id: branch_from_id.unwrap_or("root").to_owned(),
            summary: summary.into(),
            details,
            from_hook,
        };
        self.append_entry(entry)
    }

    pub fn create_branched_session(
        &mut self,
        leaf_id: &str,
    ) -> Result<Option<String>, SessionManagerError> {
        let previous_session_file = self.session_file.clone();
        let path = self.get_branch(Some(leaf_id));
        if path.is_empty() {
            return Err(SessionManagerError::EntryNotFound(leaf_id.to_owned()));
        }

        let path_without_labels = path
            .into_iter()
            .filter(|entry| !matches!(entry, SessionEntry::Label { .. }))
            .collect::<Vec<_>>();
        let session_id = generate_session_id();
        let timestamp = current_timestamp_iso();
        let session_file = self.persist.then(|| {
            Path::new(&self.session_dir)
                .join(session_file_name(&timestamp, &session_id))
                .to_string_lossy()
                .into_owned()
        });

        let header = SessionHeader {
            version: Some(CURRENT_SESSION_VERSION),
            id: session_id,
            timestamp,
            cwd: self.cwd.clone(),
            parent_session: previous_session_file,
        };

        let path_entry_ids = path_without_labels
            .iter()
            .map(|entry| entry.id().to_owned())
            .collect::<HashSet<_>>();
        let mut labels_to_write = Vec::new();
        for (target_id, label) in &self.labels_by_id {
            if path_entry_ids.contains(target_id) {
                labels_to_write.push((
                    target_id.clone(),
                    label.clone(),
                    self.label_timestamps_by_id
                        .get(target_id)
                        .cloned()
                        .unwrap_or_else(current_timestamp_iso),
                ));
            }
        }

        let mut existing_ids = path_without_labels
            .iter()
            .map(|entry| entry.id().to_owned())
            .collect::<HashSet<_>>();
        let mut label_entries = Vec::new();
        let mut parent_id = path_without_labels
            .last()
            .map(|entry| entry.id().to_owned());
        for (target_id, label, timestamp) in labels_to_write {
            let id = generate_entry_id(existing_ids.iter().map(String::as_str));
            existing_ids.insert(id.clone());
            label_entries.push(SessionEntry::Label {
                id,
                parent_id: parent_id.clone(),
                timestamp,
                target_id,
                label: Some(label),
            });
            parent_id = label_entries.last().map(|entry| entry.id().to_owned());
        }

        self.header = header;
        self.entries = path_without_labels;
        self.entries.extend(label_entries);
        self.session_file = session_file.clone();
        self.build_index();

        if self.persist {
            if self
                .entries
                .iter()
                .any(|entry| matches!(entry, SessionEntry::Message { message, .. } if message.is_assistant()))
            {
                self.rewrite_file()?;
                self.flushed = true;
            } else {
                self.flushed = false;
            }
        }

        Ok(session_file)
    }
}

fn build_session_context_internal(
    entries: &[SessionEntry],
    leaf_id: Option<&str>,
    treat_none_as_empty: bool,
) -> SessionContext {
    let by_id = entries
        .iter()
        .map(|entry| (entry.id().to_owned(), entry.clone()))
        .collect::<HashMap<_, _>>();

    if treat_none_as_empty && leaf_id.is_none() {
        return SessionContext {
            messages: Vec::new(),
            thinking_level: String::from("off"),
            model: None,
        };
    }

    let mut leaf = leaf_id.and_then(|id| by_id.get(id)).cloned();
    if leaf.is_none() {
        leaf = entries.last().cloned();
    }
    let Some(leaf) = leaf else {
        return SessionContext {
            messages: Vec::new(),
            thinking_level: String::from("off"),
            model: None,
        };
    };

    let mut path = Vec::new();
    let mut current = Some(leaf);
    while let Some(entry) = current {
        let next = entry
            .parent_id()
            .and_then(|parent_id| by_id.get(parent_id))
            .cloned();
        path.push(entry);
        current = next;
    }
    path.reverse();

    let mut thinking_level = String::from("off");
    let mut model = None;
    let mut latest_compaction = None;

    for entry in &path {
        match entry {
            SessionEntry::ThinkingLevelChange {
                thinking_level: level,
                ..
            } => {
                thinking_level = level.clone();
            }
            SessionEntry::ModelChange {
                provider, model_id, ..
            } => {
                model = Some(SessionModelSelection {
                    provider: provider.clone(),
                    model_id: model_id.clone(),
                });
            }
            SessionEntry::Message { message, .. } => {
                if let AgentMessage::Standard(Message::Assistant {
                    provider,
                    model: model_id,
                    ..
                }) = message
                {
                    model = Some(SessionModelSelection {
                        provider: provider.clone(),
                        model_id: model_id.clone(),
                    });
                }
            }
            SessionEntry::Compaction { .. } => {
                latest_compaction = Some(entry.clone());
            }
            SessionEntry::BranchSummary { .. }
            | SessionEntry::Custom { .. }
            | SessionEntry::CustomMessage { .. }
            | SessionEntry::Label { .. }
            | SessionEntry::SessionInfo { .. } => {}
        }
    }

    let mut messages = Vec::new();

    if let Some(SessionEntry::Compaction {
        id,
        summary,
        first_kept_entry_id,
        tokens_before,
        timestamp,
        ..
    }) = latest_compaction
    {
        messages.push(create_compaction_summary_message(
            summary,
            tokens_before,
            timestamp_to_millis(&timestamp),
        ));

        let compaction_index = path.iter().position(|entry| entry.id() == id).unwrap_or(0);
        let mut found_first_kept = false;
        for entry in path.iter().take(compaction_index) {
            if entry.id() == first_kept_entry_id {
                found_first_kept = true;
            }
            if found_first_kept {
                push_session_context_message(&mut messages, entry);
            }
        }
        for entry in path.iter().skip(compaction_index + 1) {
            push_session_context_message(&mut messages, entry);
        }
    } else {
        for entry in &path {
            push_session_context_message(&mut messages, entry);
        }
    }

    SessionContext {
        messages,
        thinking_level,
        model,
    }
}

fn push_session_context_message(messages: &mut Vec<AgentMessage>, entry: &SessionEntry) {
    if let Some(message) = session_entry_to_agent_message(entry) {
        messages.push(message);
    }
}

fn session_entry_to_agent_message(entry: &SessionEntry) -> Option<AgentMessage> {
    match entry {
        SessionEntry::Message { message, .. } => Some(message.clone()),
        SessionEntry::CustomMessage {
            custom_type,
            content,
            details,
            display,
            timestamp,
            ..
        } => Some(create_custom_message(
            custom_type.clone(),
            content.clone(),
            *display,
            details.clone(),
            timestamp_to_millis(timestamp),
        )),
        SessionEntry::BranchSummary {
            summary,
            from_id,
            timestamp,
            ..
        } => Some(create_branch_summary_message(
            summary.clone(),
            from_id.clone(),
            timestamp_to_millis(timestamp),
        )),
        SessionEntry::Compaction {
            summary,
            tokens_before,
            timestamp,
            ..
        } => Some(create_compaction_summary_message(
            summary.clone(),
            *tokens_before,
            timestamp_to_millis(timestamp),
        )),
        SessionEntry::ThinkingLevelChange { .. }
        | SessionEntry::ModelChange { .. }
        | SessionEntry::Custom { .. }
        | SessionEntry::Label { .. }
        | SessionEntry::SessionInfo { .. } => None,
    }
}

fn list_sessions_from_dir(dir: impl AsRef<Path>) -> Vec<SessionInfo> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut sessions = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "jsonl")
        {
            continue;
        }
        if let Some(info) = build_session_info(&path) {
            sessions.push(info);
        }
    }
    sessions
}

fn build_session_info(path: impl AsRef<Path>) -> Option<SessionInfo> {
    let path = path.as_ref();
    let entries = load_entries_from_file(path);
    let header = entries.first()?.as_session_header()?.clone();
    let metadata = fs::metadata(path).ok()?;

    let mut message_count = 0usize;
    let mut first_message = None;
    let mut all_messages = Vec::new();
    let mut name = None;
    let mut last_activity: Option<u64> = None;

    for entry in entries
        .iter()
        .skip(1)
        .filter_map(FileEntry::as_session_entry)
    {
        if let SessionEntry::SessionInfo { name: value, .. } = entry {
            name = value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
        }

        let SessionEntry::Message {
            message, timestamp, ..
        } = entry
        else {
            continue;
        };
        message_count += 1;

        if let Some(timestamp) = activity_timestamp(message, timestamp) {
            last_activity = Some(last_activity.map_or(timestamp, |current| current.max(timestamp)));
        }

        let Some(text) = extract_text_content(message) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        if first_message.is_none()
            && matches!(message, AgentMessage::Standard(Message::User { .. }))
        {
            first_message = Some(text.clone());
        }
        all_messages.push(text);
    }

    let created = parse_system_time(&header.timestamp)
        .unwrap_or_else(|| metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
    let modified = last_activity
        .map(system_time_from_millis)
        .or_else(|| parse_system_time(&header.timestamp))
        .unwrap_or_else(|| metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));

    Some(SessionInfo {
        path: path.to_string_lossy().into_owned(),
        id: header.id,
        cwd: header.cwd,
        name,
        parent_session_path: header.parent_session,
        created,
        modified,
        message_count,
        first_message: first_message.unwrap_or_else(|| String::from("(no messages)")),
        all_messages_text: all_messages.join(" "),
    })
}

fn activity_timestamp(message: &AgentMessage, entry_timestamp: &str) -> Option<u64> {
    match message {
        AgentMessage::Standard(Message::User { timestamp, .. })
        | AgentMessage::Standard(Message::Assistant { timestamp, .. }) => Some(*timestamp),
        AgentMessage::Standard(Message::ToolResult { .. }) | AgentMessage::Custom(_) => {
            timestamp_to_millis_checked(entry_timestamp)
        }
    }
}

fn extract_text_content(message: &AgentMessage) -> Option<String> {
    match message {
        AgentMessage::Standard(Message::User { content, .. }) => Some(
            content
                .iter()
                .filter_map(|block| match block {
                    UserContent::Text { text } => Some(text.as_str()),
                    UserContent::Image { .. } => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        AgentMessage::Standard(Message::Assistant { content, .. }) => Some(
            content
                .iter()
                .filter_map(|block| match block {
                    AssistantContent::Text { text, .. } => Some(text.as_str()),
                    AssistantContent::Thinking { .. } | AssistantContent::ToolCall { .. } => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        AgentMessage::Standard(Message::ToolResult { .. }) | AgentMessage::Custom(_) => None,
    }
}

fn build_tree_node(
    id: &str,
    entries_by_id: &HashMap<String, SessionEntry>,
    children_by_parent: &HashMap<Option<String>, Vec<String>>,
    labels_by_id: &HashMap<String, String>,
    label_timestamps_by_id: &HashMap<String, String>,
) -> Option<SessionTreeNode> {
    let entry = entries_by_id.get(id)?.clone();
    let children = children_by_parent
        .get(&Some(id.to_owned()))
        .into_iter()
        .flat_map(|children| children.iter())
        .filter_map(|child_id| {
            build_tree_node(
                child_id,
                entries_by_id,
                children_by_parent,
                labels_by_id,
                label_timestamps_by_id,
            )
        })
        .collect::<Vec<_>>();

    Some(SessionTreeNode {
        entry,
        children,
        label: labels_by_id.get(id).cloned(),
        label_timestamp: label_timestamps_by_id.get(id).cloned(),
    })
}

fn sort_tree_children(nodes: &mut [SessionTreeNode]) {
    let mut stack = nodes.iter_mut().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        node.children
            .sort_by(|left, right| left.entry.timestamp().cmp(right.entry.timestamp()));
        stack.extend(node.children.iter_mut());
    }
}

struct LoadedSessionFile {
    entries: Vec<FileEntry>,
    migrated: bool,
}

fn read_session_file(path: impl AsRef<Path>) -> LoadedSessionFile {
    let Ok(content) = fs::read_to_string(path) else {
        return LoadedSessionFile {
            entries: Vec::new(),
            migrated: false,
        };
    };

    let mut raw_entries = parse_raw_entries(&content);
    if raw_entries.is_empty() || !raw_entries.first().is_some_and(is_session_header_value) {
        return LoadedSessionFile {
            entries: Vec::new(),
            migrated: false,
        };
    }

    let migrated = migrate_raw_entries_to_current(&mut raw_entries);
    let entries = raw_entries
        .into_iter()
        .filter_map(|entry| raw_file_entry_to_typed(&entry))
        .collect::<Vec<_>>();

    if !entries.first().is_some_and(FileEntry::is_session) {
        return LoadedSessionFile {
            entries: Vec::new(),
            migrated: false,
        };
    }

    LoadedSessionFile { entries, migrated }
}

fn parse_raw_entries(content: &str) -> Vec<Value> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                serde_json::from_str::<Value>(trimmed).ok()
            }
        })
        .collect()
}

fn is_session_header_value(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.get("type").and_then(Value::as_str) == Some("session")
            && object.get("id").and_then(Value::as_str).is_some()
    })
}

fn migrate_raw_entries_to_current(entries: &mut [Value]) -> bool {
    let version = entries
        .first()
        .and_then(Value::as_object)
        .and_then(|object| object.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(1);

    if version >= CURRENT_SESSION_VERSION as u64 {
        return false;
    }

    if version < 2 {
        migrate_v1_to_v2(entries);
    }
    if version < 3 {
        migrate_v2_to_v3(entries);
    }

    true
}

fn migrate_v1_to_v2(entries: &mut [Value]) {
    let mut existing_ids = HashSet::new();
    let mut previous_id: Option<String> = None;

    for entry in entries.iter_mut() {
        let Some(object) = entry.as_object_mut() else {
            continue;
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "session" {
            object.insert(String::from("version"), json!(2));
            continue;
        }

        let id = generate_entry_id(existing_ids.iter().map(String::as_str));
        existing_ids.insert(id.clone());
        object.insert(String::from("id"), Value::String(id.clone()));
        object.insert(
            String::from("parentId"),
            previous_id
                .as_ref()
                .map(|id| Value::String(id.clone()))
                .unwrap_or(Value::Null),
        );
        previous_id = Some(id);
    }

    let ids_by_index = entries
        .iter()
        .map(|entry| {
            entry.as_object().and_then(|object| {
                object
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
        })
        .collect::<Vec<_>>();

    for entry in entries.iter_mut() {
        let Some(object) = entry.as_object_mut() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("compaction") {
            continue;
        }
        let Some(index) = object.get("firstKeptEntryIndex").and_then(Value::as_u64) else {
            continue;
        };
        if let Some(Some(target_id)) = ids_by_index.get(index as usize) {
            object.insert(
                String::from("firstKeptEntryId"),
                Value::String(target_id.clone()),
            );
        }
        object.remove("firstKeptEntryIndex");
    }
}

fn migrate_v2_to_v3(entries: &mut [Value]) {
    for entry in entries.iter_mut() {
        let Some(object) = entry.as_object_mut() else {
            continue;
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "session" {
            object.insert(String::from("version"), json!(3));
            continue;
        }
        if kind != "message" {
            continue;
        }
        let Some(message) = object.get_mut("message").and_then(Value::as_object_mut) else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) == Some("hookMessage") {
            message.insert(String::from("role"), Value::String(String::from("custom")));
        }
    }
}

fn raw_file_entry_to_typed(value: &Value) -> Option<FileEntry> {
    let object = value.as_object()?;
    let kind = object.get("type")?.as_str()?;

    match kind {
        "session" => Some(FileEntry::Session(SessionHeader {
            version: object
                .get("version")
                .and_then(Value::as_u64)
                .map(|value| value as u32),
            id: object.get("id")?.as_str()?.to_owned(),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            cwd: object.get("cwd")?.as_str()?.to_owned(),
            parent_session: object
                .get("parentSession")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })),
        "message" => Some(FileEntry::Entry(SessionEntry::Message {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            message: raw_agent_message_to_typed(object.get("message")?)?,
        })),
        "thinking_level_change" => Some(FileEntry::Entry(SessionEntry::ThinkingLevelChange {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            thinking_level: object.get("thinkingLevel")?.as_str()?.to_owned(),
        })),
        "model_change" => Some(FileEntry::Entry(SessionEntry::ModelChange {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            provider: object.get("provider")?.as_str()?.to_owned(),
            model_id: object.get("modelId")?.as_str()?.to_owned(),
        })),
        "compaction" => Some(FileEntry::Entry(SessionEntry::Compaction {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            summary: object.get("summary")?.as_str()?.to_owned(),
            first_kept_entry_id: object.get("firstKeptEntryId")?.as_str()?.to_owned(),
            tokens_before: object.get("tokensBefore")?.as_u64()?,
            details: object.get("details").cloned(),
            from_hook: object.get("fromHook").and_then(Value::as_bool),
        })),
        "branch_summary" => Some(FileEntry::Entry(SessionEntry::BranchSummary {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            from_id: object.get("fromId")?.as_str()?.to_owned(),
            summary: object.get("summary")?.as_str()?.to_owned(),
            details: object.get("details").cloned(),
            from_hook: object.get("fromHook").and_then(Value::as_bool),
        })),
        "custom" => Some(FileEntry::Entry(SessionEntry::Custom {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            custom_type: object.get("customType")?.as_str()?.to_owned(),
            data: object.get("data").cloned(),
        })),
        "custom_message" => Some(FileEntry::Entry(SessionEntry::CustomMessage {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            custom_type: object.get("customType")?.as_str()?.to_owned(),
            content: serde_json::from_value(object.get("content")?.clone()).ok()?,
            details: object.get("details").cloned(),
            display: object.get("display")?.as_bool()?,
        })),
        "label" => Some(FileEntry::Entry(SessionEntry::Label {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            target_id: object.get("targetId")?.as_str()?.to_owned(),
            label: optional_string(object.get("label")),
        })),
        "session_info" => Some(FileEntry::Entry(SessionEntry::SessionInfo {
            id: object.get("id")?.as_str()?.to_owned(),
            parent_id: optional_string(object.get("parentId")),
            timestamp: object.get("timestamp")?.as_str()?.to_owned(),
            name: optional_string(object.get("name")),
        })),
        _ => None,
    }
}

fn typed_file_entry_to_raw(entry: &FileEntry) -> Value {
    match entry {
        FileEntry::Session(header) => {
            let mut object = Map::new();
            object.insert(String::from("type"), Value::String(String::from("session")));
            object.insert(String::from("id"), Value::String(header.id.clone()));
            if let Some(version) = header.version {
                object.insert(String::from("version"), json!(version));
            }
            object.insert(
                String::from("timestamp"),
                Value::String(header.timestamp.clone()),
            );
            object.insert(String::from("cwd"), Value::String(header.cwd.clone()));
            if let Some(parent_session) = &header.parent_session {
                object.insert(
                    String::from("parentSession"),
                    Value::String(parent_session.clone()),
                );
            }
            Value::Object(object)
        }
        FileEntry::Entry(entry) => match entry {
            SessionEntry::Message {
                id,
                parent_id,
                timestamp,
                message,
            } => json!({
                "type": "message",
                "id": id,
                "parentId": parent_id,
                "timestamp": timestamp,
                "message": typed_agent_message_to_raw(message),
            }),
            SessionEntry::ThinkingLevelChange {
                id,
                parent_id,
                timestamp,
                thinking_level,
            } => json!({
                "type": "thinking_level_change",
                "id": id,
                "parentId": parent_id,
                "timestamp": timestamp,
                "thinkingLevel": thinking_level,
            }),
            SessionEntry::ModelChange {
                id,
                parent_id,
                timestamp,
                provider,
                model_id,
            } => json!({
                "type": "model_change",
                "id": id,
                "parentId": parent_id,
                "timestamp": timestamp,
                "provider": provider,
                "modelId": model_id,
            }),
            SessionEntry::Compaction {
                id,
                parent_id,
                timestamp,
                summary,
                first_kept_entry_id,
                tokens_before,
                details,
                from_hook,
            } => {
                let mut object = Map::new();
                object.insert(
                    String::from("type"),
                    Value::String(String::from("compaction")),
                );
                object.insert(String::from("id"), Value::String(id.clone()));
                object.insert(
                    String::from("parentId"),
                    parent_id.clone().map(Value::String).unwrap_or(Value::Null),
                );
                object.insert(String::from("timestamp"), Value::String(timestamp.clone()));
                object.insert(String::from("summary"), Value::String(summary.clone()));
                object.insert(
                    String::from("firstKeptEntryId"),
                    Value::String(first_kept_entry_id.clone()),
                );
                object.insert(String::from("tokensBefore"), json!(tokens_before));
                if let Some(details) = details {
                    object.insert(String::from("details"), details.clone());
                }
                if let Some(from_hook) = from_hook {
                    object.insert(String::from("fromHook"), Value::Bool(*from_hook));
                }
                Value::Object(object)
            }
            SessionEntry::BranchSummary {
                id,
                parent_id,
                timestamp,
                from_id,
                summary,
                details,
                from_hook,
            } => {
                let mut object = Map::new();
                object.insert(
                    String::from("type"),
                    Value::String(String::from("branch_summary")),
                );
                object.insert(String::from("id"), Value::String(id.clone()));
                object.insert(
                    String::from("parentId"),
                    parent_id.clone().map(Value::String).unwrap_or(Value::Null),
                );
                object.insert(String::from("timestamp"), Value::String(timestamp.clone()));
                object.insert(String::from("fromId"), Value::String(from_id.clone()));
                object.insert(String::from("summary"), Value::String(summary.clone()));
                if let Some(details) = details {
                    object.insert(String::from("details"), details.clone());
                }
                if let Some(from_hook) = from_hook {
                    object.insert(String::from("fromHook"), Value::Bool(*from_hook));
                }
                Value::Object(object)
            }
            SessionEntry::Custom {
                id,
                parent_id,
                timestamp,
                custom_type,
                data,
            } => {
                let mut object = Map::new();
                object.insert(String::from("type"), Value::String(String::from("custom")));
                object.insert(String::from("id"), Value::String(id.clone()));
                object.insert(
                    String::from("parentId"),
                    parent_id.clone().map(Value::String).unwrap_or(Value::Null),
                );
                object.insert(String::from("timestamp"), Value::String(timestamp.clone()));
                object.insert(
                    String::from("customType"),
                    Value::String(custom_type.clone()),
                );
                if let Some(data) = data {
                    object.insert(String::from("data"), data.clone());
                }
                Value::Object(object)
            }
            SessionEntry::CustomMessage {
                id,
                parent_id,
                timestamp,
                custom_type,
                content,
                details,
                display,
            } => {
                let mut object = Map::new();
                object.insert(
                    String::from("type"),
                    Value::String(String::from("custom_message")),
                );
                object.insert(String::from("id"), Value::String(id.clone()));
                object.insert(
                    String::from("parentId"),
                    parent_id.clone().map(Value::String).unwrap_or(Value::Null),
                );
                object.insert(String::from("timestamp"), Value::String(timestamp.clone()));
                object.insert(
                    String::from("customType"),
                    Value::String(custom_type.clone()),
                );
                object.insert(
                    String::from("content"),
                    serde_json::to_value(content).expect("custom message content should serialize"),
                );
                object.insert(String::from("display"), Value::Bool(*display));
                if let Some(details) = details {
                    object.insert(String::from("details"), details.clone());
                }
                Value::Object(object)
            }
            SessionEntry::Label {
                id,
                parent_id,
                timestamp,
                target_id,
                label,
            } => json!({
                "type": "label",
                "id": id,
                "parentId": parent_id,
                "timestamp": timestamp,
                "targetId": target_id,
                "label": label,
            }),
            SessionEntry::SessionInfo {
                id,
                parent_id,
                timestamp,
                name,
            } => json!({
                "type": "session_info",
                "id": id,
                "parentId": parent_id,
                "timestamp": timestamp,
                "name": name,
            }),
        },
    }
}

fn raw_agent_message_to_typed(value: &Value) -> Option<AgentMessage> {
    let object = value.as_object()?;
    let role = object.get("role")?.as_str()?;
    let timestamp = object.get("timestamp")?.as_u64()?;

    match role {
        "user" => {
            let content = match object.get("content")? {
                Value::String(text) => vec![UserContent::Text { text: text.clone() }],
                other => serde_json::from_value::<Vec<UserContent>>(other.clone()).ok()?,
            };
            Some(Message::User { content, timestamp }.into())
        }
        "assistant" => Some(
            Message::Assistant {
                content: serde_json::from_value(object.get("content")?.clone()).ok()?,
                api: object.get("api")?.as_str()?.to_owned(),
                provider: object.get("provider")?.as_str()?.to_owned(),
                model: object.get("model")?.as_str()?.to_owned(),
                response_id: optional_string(object.get("responseId")),
                usage: parse_usage(object.get("usage")?),
                stop_reason: parse_stop_reason(object.get("stopReason")?)?,
                error_message: optional_string(object.get("errorMessage")),
                timestamp,
            }
            .into(),
        ),
        "toolResult" => Some(
            Message::ToolResult {
                tool_call_id: object.get("toolCallId")?.as_str()?.to_owned(),
                tool_name: object.get("toolName")?.as_str()?.to_owned(),
                content: serde_json::from_value(object.get("content")?.clone()).ok()?,
                details: object.get("details").cloned(),
                is_error: object.get("isError")?.as_bool()?,
                timestamp,
            }
            .into(),
        ),
        other => {
            let mut payload = object.clone();
            payload.remove("role");
            payload.remove("timestamp");
            let payload = payload
                .remove("payload")
                .unwrap_or_else(|| Value::Object(payload));
            Some(AgentMessage::custom(other, payload, timestamp))
        }
    }
}

fn typed_agent_message_to_raw(message: &AgentMessage) -> Value {
    match message {
        AgentMessage::Standard(Message::User { content, timestamp }) => {
            let content = match content.as_slice() {
                [UserContent::Text { text }] => Value::String(text.clone()),
                _ => serde_json::to_value(content).expect("user content should serialize"),
            };
            json!({
                "role": "user",
                "content": content,
                "timestamp": timestamp,
            })
        }
        AgentMessage::Standard(Message::Assistant {
            content,
            api,
            provider,
            model,
            response_id,
            usage,
            stop_reason,
            error_message,
            timestamp,
        }) => {
            let mut object = Map::new();
            object.insert(
                String::from("role"),
                Value::String(String::from("assistant")),
            );
            object.insert(
                String::from("content"),
                serde_json::to_value(content).expect("assistant content should serialize"),
            );
            object.insert(String::from("api"), Value::String(api.clone()));
            object.insert(String::from("provider"), Value::String(provider.clone()));
            object.insert(String::from("model"), Value::String(model.clone()));
            if let Some(response_id) = response_id {
                object.insert(
                    String::from("responseId"),
                    Value::String(response_id.clone()),
                );
            }
            object.insert(String::from("usage"), usage_to_value(usage));
            object.insert(
                String::from("stopReason"),
                stop_reason_to_value(stop_reason),
            );
            if let Some(error_message) = error_message {
                object.insert(
                    String::from("errorMessage"),
                    Value::String(error_message.clone()),
                );
            }
            object.insert(String::from("timestamp"), json!(timestamp));
            Value::Object(object)
        }
        AgentMessage::Standard(Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            details,
            is_error,
            timestamp,
        }) => {
            let mut object = Map::new();
            object.insert(
                String::from("role"),
                Value::String(String::from("toolResult")),
            );
            object.insert(
                String::from("toolCallId"),
                Value::String(tool_call_id.clone()),
            );
            object.insert(String::from("toolName"), Value::String(tool_name.clone()));
            object.insert(
                String::from("content"),
                serde_json::to_value(content).expect("tool result content should serialize"),
            );
            if let Some(details) = details {
                object.insert(String::from("details"), details.clone());
            }
            object.insert(String::from("isError"), Value::Bool(*is_error));
            object.insert(String::from("timestamp"), json!(timestamp));
            Value::Object(object)
        }
        AgentMessage::Custom(message) => custom_agent_message_to_raw(message),
    }
}

fn custom_agent_message_to_raw(message: &pi_agent::CustomAgentMessage) -> Value {
    match message.role.as_str() {
        "bashExecution" => {
            if let Ok(payload) =
                serde_json::from_value::<BashExecutionMessage>(message.payload.clone())
            {
                let mut object = serde_json::to_value(payload)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                object.insert(String::from("role"), Value::String(message.role.clone()));
                object.insert(String::from("timestamp"), json!(message.timestamp));
                return Value::Object(object);
            }
        }
        "custom" => {
            if let Ok(payload) = serde_json::from_value::<CustomMessage>(message.payload.clone()) {
                let mut object = serde_json::to_value(payload)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                object.insert(String::from("role"), Value::String(message.role.clone()));
                object.insert(String::from("timestamp"), json!(message.timestamp));
                return Value::Object(object);
            }
        }
        "branchSummary" => {
            if let Ok(payload) =
                serde_json::from_value::<BranchSummaryMessage>(message.payload.clone())
            {
                let mut object = serde_json::to_value(payload)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                object.insert(String::from("role"), Value::String(message.role.clone()));
                object.insert(String::from("timestamp"), json!(message.timestamp));
                return Value::Object(object);
            }
        }
        "compactionSummary" => {
            if let Ok(payload) =
                serde_json::from_value::<CompactionSummaryMessage>(message.payload.clone())
            {
                let mut object = serde_json::to_value(payload)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                object.insert(String::from("role"), Value::String(message.role.clone()));
                object.insert(String::from("timestamp"), json!(message.timestamp));
                return Value::Object(object);
            }
        }
        _ => {}
    }

    json!({
        "role": message.role,
        "payload": message.payload,
        "timestamp": message.timestamp,
    })
}

fn parse_usage(value: &Value) -> Usage {
    let Some(object) = value.as_object() else {
        return Usage::default();
    };
    let cost = object.get("cost").and_then(Value::as_object);
    Usage {
        input: object.get("input").and_then(Value::as_u64).unwrap_or(0),
        output: object.get("output").and_then(Value::as_u64).unwrap_or(0),
        cache_read: object
            .get("cacheRead")
            .or_else(|| object.get("cache_read"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_write: object
            .get("cacheWrite")
            .or_else(|| object.get("cache_write"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_tokens: object
            .get("totalTokens")
            .or_else(|| object.get("total_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cost: UsageCost {
            input: cost
                .and_then(|cost| cost.get("input"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            output: cost
                .and_then(|cost| cost.get("output"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            cache_read: cost
                .and_then(|cost| cost.get("cacheRead").or_else(|| cost.get("cache_read")))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            cache_write: cost
                .and_then(|cost| cost.get("cacheWrite").or_else(|| cost.get("cache_write")))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            total: cost
                .and_then(|cost| cost.get("total"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
        },
    }
}

fn usage_to_value(usage: &Usage) -> Value {
    json!({
        "input": usage.input,
        "output": usage.output,
        "cacheRead": usage.cache_read,
        "cacheWrite": usage.cache_write,
        "totalTokens": usage.total_tokens,
        "cost": {
            "input": usage.cost.input,
            "output": usage.cost.output,
            "cacheRead": usage.cost.cache_read,
            "cacheWrite": usage.cost.cache_write,
            "total": usage.cost.total,
        }
    })
}

fn parse_stop_reason(value: &Value) -> Option<StopReason> {
    match value.as_str()? {
        "stop" => Some(StopReason::Stop),
        "length" => Some(StopReason::Length),
        "toolUse" => Some(StopReason::ToolUse),
        "error" => Some(StopReason::Error),
        "aborted" => Some(StopReason::Aborted),
        _ => None,
    }
}

fn stop_reason_to_value(reason: &StopReason) -> Value {
    let value = match reason {
        StopReason::Stop => "stop",
        StopReason::Length => "length",
        StopReason::ToolUse => "toolUse",
        StopReason::Error => "error",
        StopReason::Aborted => "aborted",
    };
    Value::String(value.to_owned())
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(ToOwned::to_owned)
}

fn timestamp_to_millis(timestamp: &str) -> u64 {
    timestamp_to_millis_checked(timestamp).unwrap_or(0)
}

fn timestamp_to_millis_checked(timestamp: &str) -> Option<u64> {
    OffsetDateTime::parse(timestamp, &Rfc3339)
        .ok()
        .and_then(|value| u64::try_from(value.unix_timestamp_nanos() / 1_000_000).ok())
}

fn parse_system_time(timestamp: &str) -> Option<SystemTime> {
    timestamp_to_millis_checked(timestamp).map(system_time_from_millis)
}

fn system_time_from_millis(timestamp: u64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(timestamp)
}

fn value_to_json_line(value: &Value) -> String {
    serde_json::to_string(value).expect("session entry should always serialize")
}

fn current_timestamp_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC3339 formatting should always succeed")
}

fn current_working_directory_string() -> String {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .into_owned()
}

fn resolve_path_string(path: &str) -> String {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }

    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
        .to_string_lossy()
        .into_owned()
}

fn default_agent_dir(agent_dir: Option<&str>) -> String {
    if let Some(agent_dir) = agent_dir {
        return expand_home_path(agent_dir);
    }
    if let Some(agent_dir) = env::var_os(ENV_AGENT_DIR) {
        return expand_home_path(&agent_dir.to_string_lossy());
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIR_NAME)
        .join("agent")
        .to_string_lossy()
        .into_owned()
}

fn expand_home_path(path: &str) -> String {
    if path == "~" {
        return env::var("HOME").unwrap_or_else(|_| path.to_owned());
    }
    if let Some(suffix) = path.strip_prefix("~/")
        && let Ok(home) = env::var("HOME")
    {
        return Path::new(&home).join(suffix).to_string_lossy().into_owned();
    }
    path.to_owned()
}

fn session_file_name(timestamp: &str, session_id: &str) -> String {
    let file_timestamp = timestamp.replace([':', '.'], "-");
    format!("{file_timestamp}_{session_id}.jsonl")
}

fn generate_session_id() -> String {
    let hex = unique_hex_32();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn generate_entry_id<'a>(existing_ids: impl IntoIterator<Item = &'a str>) -> String {
    let existing = existing_ids.into_iter().collect::<HashSet<_>>();
    for _ in 0..100 {
        let hex = unique_hex_32();
        let candidate = hex[0..8].to_owned();
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unique_hex_32()
}

fn unique_hex_32() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = u128::from(ID_COUNTER.fetch_add(1, Ordering::Relaxed));
    let process = u128::from(std::process::id());
    let mixed = now ^ (counter << 32) ^ process;
    format!("{mixed:032x}")
}
