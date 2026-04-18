use crate::{
    compaction::collect_entries_for_branch_summary,
    messages::CustomMessageContent,
    session_manager::{SessionEntry, SessionManager, SessionManagerError},
};
use pi_events::{Message, UserContent};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct TreeNavigationPreparation {
    pub old_leaf_id: Option<String>,
    pub target_id: Option<String>,
    pub common_ancestor_id: Option<String>,
    pub entries_to_summarize: Vec<SessionEntry>,
    pub new_leaf_id: Option<String>,
    pub editor_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeNavigationSummary {
    pub summary: String,
    pub details: Option<Value>,
    pub from_hook: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNavigationResult {
    pub old_leaf_id: Option<String>,
    pub new_leaf_id: Option<String>,
    pub editor_text: Option<String>,
    pub summary_entry_id: Option<String>,
}

pub fn prepare_tree_navigation(
    session_manager: &SessionManager,
    target_id: Option<&str>,
) -> Result<TreeNavigationPreparation, SessionManagerError> {
    let old_leaf_id = session_manager.get_leaf_id().map(ToOwned::to_owned);

    let (entries_to_summarize, common_ancestor_id, new_leaf_id, editor_text) =
        if let Some(target_id) = target_id {
            let target_entry = session_manager
                .get_entry(target_id)
                .cloned()
                .ok_or_else(|| SessionManagerError::EntryNotFound(target_id.to_owned()))?;
            let collected = collect_entries_for_branch_summary(
                session_manager,
                old_leaf_id.as_deref(),
                target_id,
            );
            let (new_leaf_id, editor_text) = navigation_target_from_entry(&target_entry);
            (
                collected.entries,
                collected.common_ancestor_id,
                new_leaf_id,
                editor_text,
            )
        } else {
            (
                session_manager.get_branch(old_leaf_id.as_deref()),
                None,
                None,
                None,
            )
        };

    Ok(TreeNavigationPreparation {
        old_leaf_id,
        target_id: target_id.map(ToOwned::to_owned),
        common_ancestor_id,
        entries_to_summarize,
        new_leaf_id,
        editor_text,
    })
}

pub fn apply_tree_navigation(
    session_manager: &mut SessionManager,
    preparation: &TreeNavigationPreparation,
    summary: Option<TreeNavigationSummary>,
    label: Option<&str>,
) -> Result<TreeNavigationResult, SessionManagerError> {
    let label = label.map(str::trim).filter(|label| !label.is_empty());
    let summary_entry_id = if let Some(summary) = summary {
        let summary_id = session_manager.branch_with_summary(
            preparation.new_leaf_id.as_deref(),
            summary.summary,
            summary.details,
            summary.from_hook,
        )?;
        if let Some(label) = label {
            session_manager.append_label_change(&summary_id, Some(label.to_owned()))?;
        }
        Some(summary_id)
    } else {
        if let Some(new_leaf_id) = preparation.new_leaf_id.as_deref() {
            session_manager.branch(new_leaf_id)?;
        } else {
            session_manager.reset_leaf();
        }

        if let Some(label) = label
            && let Some(target_id) = preparation.target_id.as_deref()
        {
            session_manager.append_label_change(target_id, Some(label.to_owned()))?;
        }
        None
    };

    Ok(TreeNavigationResult {
        old_leaf_id: preparation.old_leaf_id.clone(),
        new_leaf_id: session_manager.get_leaf_id().map(ToOwned::to_owned),
        editor_text: preparation.editor_text.clone(),
        summary_entry_id,
    })
}

fn navigation_target_from_entry(entry: &SessionEntry) -> (Option<String>, Option<String>) {
    match entry {
        SessionEntry::Message {
            parent_id, message, ..
        } => match message.as_standard_message() {
            Some(Message::User { content, .. }) => {
                let editor_text = extract_user_text(content);
                (
                    parent_id.clone(),
                    (!editor_text.is_empty()).then_some(editor_text),
                )
            }
            Some(Message::Assistant { .. }) | Some(Message::ToolResult { .. }) | None => {
                (Some(entry.id().to_owned()), None)
            }
        },
        SessionEntry::CustomMessage {
            parent_id, content, ..
        } => {
            let editor_text = extract_custom_message_text(content);
            (
                parent_id.clone(),
                (!editor_text.is_empty()).then_some(editor_text),
            )
        }
        SessionEntry::ThinkingLevelChange { .. }
        | SessionEntry::ModelChange { .. }
        | SessionEntry::Compaction { .. }
        | SessionEntry::BranchSummary { .. }
        | SessionEntry::Custom { .. }
        | SessionEntry::Label { .. }
        | SessionEntry::SessionInfo { .. } => (Some(entry.id().to_owned()), None),
    }
}

fn extract_user_text(content: &[UserContent]) -> String {
    content
        .iter()
        .filter_map(|content| match content {
            UserContent::Text { text } => Some(text.as_str()),
            UserContent::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_owned()
}

fn extract_custom_message_text(content: &CustomMessageContent) -> String {
    match content {
        CustomMessageContent::Text(text) => text.trim().to_owned(),
        CustomMessageContent::Blocks(blocks) => extract_user_text(blocks),
    }
}
