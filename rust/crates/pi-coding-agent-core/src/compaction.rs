use crate::{
    convert_to_llm, create_branch_summary_message, create_compaction_summary_message,
    create_custom_message,
    session_manager::{
        SessionEntry, SessionManager, build_session_context, get_latest_compaction_entry,
    },
};
use pi_agent::AgentMessage;
use pi_ai::{SimpleStreamOptions, ThinkingLevel as AiThinkingLevel, complete_simple};
use pi_events::{
    AssistantContent, AssistantMessage, Context, Message, Model, StopReason, Usage, UserContent,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SUMMARIZATION_SYSTEM_PROMPT: &str = "You are a context summarization assistant. Your task is to read a conversation between a user and an AI coding assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary.";
const TOOL_RESULT_MAX_CHARS: usize = 2_000;
const DEFAULT_SUMMARY_MAX_TOKENS: u64 = 2_048;

const SUMMARIZATION_PROMPT: &str = "The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.\n\nUse this EXACT format:\n\n## Goal\n[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]\n\n## Constraints & Preferences\n- [Any constraints, preferences, or requirements mentioned by user]\n- [Or \"(none)\" if none were mentioned]\n\n## Progress\n### Done\n- [x] [Completed tasks/changes]\n\n### In Progress\n- [ ] [Current work]\n\n### Blocked\n- [Issues preventing progress, if any]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale]\n\n## Next Steps\n1. [Ordered list of what should happen next]\n\n## Critical Context\n- [Any data, examples, or references needed to continue]\n- [Or \"(none)\" if not applicable]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

const UPDATE_SUMMARIZATION_PROMPT: &str = "The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.\n\nUpdate the existing structured summary with new information. RULES:\n- PRESERVE all existing information from the previous summary\n- ADD new progress, decisions, and context from the new messages\n- UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed\n- UPDATE \"Next Steps\" based on what was accomplished\n- PRESERVE exact file paths, function names, and error messages\n- If something is no longer relevant, you may remove it\n\nUse this EXACT format:\n\n## Goal\n[Preserve existing goals, add new ones if the task expanded]\n\n## Constraints & Preferences\n- [Preserve existing, add new ones discovered]\n\n## Progress\n### Done\n- [x] [Include previously done items AND newly completed items]\n\n### In Progress\n- [ ] [Current work - update based on progress]\n\n### Blocked\n- [Current blockers - remove if resolved]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale] (preserve all previous, add new)\n\n## Next Steps\n1. [Update based on current state]\n\n## Critical Context\n- [Preserve important context, add new if needed]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

const TURN_PREFIX_SUMMARIZATION_PROMPT: &str = "This is the PREFIX of a turn that was too large to keep. The SUFFIX (recent work) is retained.\n\nSummarize the prefix to provide context for the retained suffix:\n\n## Original Request\n[What did the user ask for in this turn?]\n\n## Early Progress\n- [Key decisions and work done in the prefix]\n\n## Context for Suffix\n- [Information needed to understand the retained recent work]\n\nBe concise. Focus on what's needed to understand the kept suffix.";

const BRANCH_SUMMARY_PREAMBLE: &str = "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n";
const BRANCH_SUMMARY_PROMPT: &str = "Create a structured summary of this conversation branch for context when returning later.\n\nUse this EXACT format:\n\n## Goal\n[What was the user trying to accomplish in this branch?]\n\n## Constraints & Preferences\n- [Any constraints, preferences, or requirements mentioned]\n- [Or \"(none)\" if none were mentioned]\n\n## Progress\n### Done\n- [x] [Completed tasks/changes]\n\n### In Progress\n- [ ] [Work that was started but not finished]\n\n### Blocked\n- [Issues preventing progress, if any]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale]\n\n## Next Steps\n1. [What should happen next to continue this work]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            reserve_tokens: 16_384,
            keep_recent_tokens: 20_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompactionResult {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompactionPreparation {
    pub first_kept_entry_id: String,
    pub messages_to_summarize: Vec<AgentMessage>,
    pub turn_prefix_messages: Vec<AgentMessage>,
    pub is_split_turn: bool,
    pub tokens_before: u64,
    pub previous_summary: Option<String>,
    pub settings: CompactionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextUsageEstimate {
    pub tokens: u64,
    pub usage_tokens: u64,
    pub trailing_tokens: u64,
    pub last_usage_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectEntriesResult {
    pub entries: Vec<SessionEntry>,
    pub common_ancestor_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSummaryOptions {
    pub reserve_tokens: u64,
    pub custom_instructions: Option<String>,
    pub replace_instructions: bool,
}

impl Default for BranchSummaryOptions {
    fn default() -> Self {
        Self {
            reserve_tokens: 16_384,
            custom_instructions: None,
            replace_instructions: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CutPointResult {
    first_kept_entry_index: usize,
    turn_start_index: Option<usize>,
    is_split_turn: bool,
}

pub fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input + usage.output + usage.cache_read + usage.cache_write
    }
}

pub fn estimate_context_tokens(messages: &[AgentMessage]) -> ContextUsageEstimate {
    let usage_info = messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| assistant_usage(message).map(|usage| (index, usage)));

    let Some((index, usage)) = usage_info else {
        let trailing_tokens = messages.iter().map(estimate_tokens).sum();
        return ContextUsageEstimate {
            tokens: trailing_tokens,
            usage_tokens: 0,
            trailing_tokens,
            last_usage_index: None,
        };
    };

    let usage_tokens = calculate_context_tokens(usage);
    let trailing_tokens = messages.iter().skip(index + 1).map(estimate_tokens).sum();

    ContextUsageEstimate {
        tokens: usage_tokens + trailing_tokens,
        usage_tokens,
        trailing_tokens,
        last_usage_index: Some(index),
    }
}

pub fn should_compact(
    context_tokens: u64,
    context_window: u64,
    settings: &CompactionSettings,
) -> bool {
    settings.enabled && context_tokens > context_window.saturating_sub(settings.reserve_tokens)
}

pub fn estimate_tokens(message: &AgentMessage) -> u64 {
    convert_to_llm(vec![message.clone()])
        .iter()
        .map(estimate_llm_message_tokens)
        .sum()
}

pub fn prepare_compaction(
    path_entries: &[SessionEntry],
    settings: CompactionSettings,
) -> Option<CompactionPreparation> {
    if matches!(path_entries.last(), Some(SessionEntry::Compaction { .. })) {
        return None;
    }

    let previous_compaction = path_entries
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, entry)| match entry {
            SessionEntry::Compaction {
                summary,
                first_kept_entry_id,
                ..
            } => Some((index, summary.clone(), first_kept_entry_id.clone())),
            _ => None,
        });

    let (boundary_start, previous_summary) =
        if let Some((index, summary, first_kept_entry_id)) = previous_compaction {
            let first_kept_index = path_entries
                .iter()
                .position(|entry| entry.id() == first_kept_entry_id)
                .unwrap_or(index + 1);
            (first_kept_index, Some(summary))
        } else {
            (0, None)
        };

    if boundary_start >= path_entries.len() {
        return None;
    }

    let tokens_before =
        estimate_context_tokens(&build_session_context(path_entries, None).messages).tokens;
    let cut_point = find_cut_point(
        path_entries,
        boundary_start,
        path_entries.len(),
        settings.keep_recent_tokens,
    );

    let first_kept_entry_id = path_entries
        .get(cut_point.first_kept_entry_index)?
        .id()
        .to_owned();
    let history_end = if cut_point.is_split_turn {
        cut_point
            .turn_start_index
            .unwrap_or(cut_point.first_kept_entry_index)
    } else {
        cut_point.first_kept_entry_index
    };

    let messages_to_summarize = path_entries[boundary_start..history_end]
        .iter()
        .filter_map(compaction_message_from_entry)
        .collect::<Vec<_>>();

    let turn_prefix_messages = if cut_point.is_split_turn {
        let start = cut_point
            .turn_start_index
            .unwrap_or(cut_point.first_kept_entry_index);
        path_entries[start..cut_point.first_kept_entry_index]
            .iter()
            .filter_map(compaction_message_from_entry)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if messages_to_summarize.is_empty() && turn_prefix_messages.is_empty() {
        return None;
    }

    Some(CompactionPreparation {
        first_kept_entry_id,
        messages_to_summarize,
        turn_prefix_messages,
        is_split_turn: cut_point.is_split_turn,
        tokens_before,
        previous_summary,
        settings,
    })
}

pub async fn compact(
    preparation: &CompactionPreparation,
    model: &Model,
    api_key: &str,
    headers: Option<BTreeMap<String, String>>,
    custom_instructions: Option<&str>,
) -> Result<CompactionResult, String> {
    let summary = if preparation.is_split_turn && !preparation.turn_prefix_messages.is_empty() {
        let history_summary = if preparation.messages_to_summarize.is_empty() {
            String::from("No prior history.")
        } else {
            generate_summary(
                &preparation.messages_to_summarize,
                model,
                preparation.settings.reserve_tokens,
                api_key,
                headers.clone(),
                custom_instructions,
                preparation.previous_summary.as_deref(),
            )
            .await?
        };
        let prefix_summary = generate_turn_prefix_summary(
            &preparation.turn_prefix_messages,
            model,
            preparation.settings.reserve_tokens,
            api_key,
            headers,
        )
        .await?;
        format!("{history_summary}\n\n---\n\n**Turn Context (split turn):**\n\n{prefix_summary}")
    } else {
        generate_summary(
            &preparation.messages_to_summarize,
            model,
            preparation.settings.reserve_tokens,
            api_key,
            headers,
            custom_instructions,
            preparation.previous_summary.as_deref(),
        )
        .await?
    };

    Ok(CompactionResult {
        summary,
        first_kept_entry_id: preparation.first_kept_entry_id.clone(),
        tokens_before: preparation.tokens_before,
        details: None,
    })
}

pub fn collect_entries_for_branch_summary(
    session: &SessionManager,
    old_leaf_id: Option<&str>,
    target_id: &str,
) -> CollectEntriesResult {
    let Some(old_leaf_id) = old_leaf_id else {
        return CollectEntriesResult {
            entries: Vec::new(),
            common_ancestor_id: None,
        };
    };

    let old_path = session
        .get_branch(Some(old_leaf_id))
        .into_iter()
        .map(|entry| entry.id().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    let target_path = session.get_branch(Some(target_id));

    let common_ancestor_id = target_path
        .iter()
        .rev()
        .find_map(|entry| old_path.contains(entry.id()).then(|| entry.id().to_owned()));

    let mut entries = Vec::new();
    let mut current = Some(old_leaf_id.to_owned());
    while let Some(current_id) = current {
        if common_ancestor_id.as_deref() == Some(current_id.as_str()) {
            break;
        }
        let Some(entry) = session.get_entry(&current_id).cloned() else {
            break;
        };
        current = entry.parent_id().map(ToOwned::to_owned);
        entries.push(entry);
    }
    entries.reverse();

    CollectEntriesResult {
        entries,
        common_ancestor_id,
    }
}

pub async fn generate_branch_summary(
    entries: &[SessionEntry],
    model: &Model,
    api_key: &str,
    headers: Option<BTreeMap<String, String>>,
    options: BranchSummaryOptions,
) -> Result<String, String> {
    let token_budget = model.context_window.saturating_sub(options.reserve_tokens);
    let messages = prepare_branch_messages(entries, token_budget);
    if messages.is_empty() {
        return Ok(String::from("No content to summarize"));
    }

    let conversation_text = serialize_conversation(&convert_to_llm(messages));
    let instructions = match (
        options.replace_instructions,
        options.custom_instructions.as_deref(),
    ) {
        (true, Some(custom_instructions)) => custom_instructions.to_owned(),
        (_, Some(custom_instructions)) => {
            format!("{BRANCH_SUMMARY_PROMPT}\n\nAdditional focus: {custom_instructions}")
        }
        (_, None) => BRANCH_SUMMARY_PROMPT.to_owned(),
    };
    let prompt_text =
        format!("<conversation>\n{conversation_text}\n</conversation>\n\n{instructions}");

    let response = request_summary(
        model,
        api_key,
        headers,
        prompt_text,
        DEFAULT_SUMMARY_MAX_TOKENS.min(model.max_tokens.max(1)),
        None,
    )
    .await?;

    Ok(format!(
        "{BRANCH_SUMMARY_PREAMBLE}{}",
        assistant_text(&response)
    ))
}

pub fn latest_compaction_timestamp(entries: &[SessionEntry]) -> Option<u64> {
    let entry = get_latest_compaction_entry(entries)?;
    timestamp_to_millis(entry.timestamp())
}

fn assistant_usage(message: &AgentMessage) -> Option<&Usage> {
    let Message::Assistant {
        usage, stop_reason, ..
    } = message.as_standard_message()?
    else {
        return None;
    };
    (*stop_reason != StopReason::Aborted && *stop_reason != StopReason::Error).then_some(usage)
}

fn estimate_llm_message_tokens(message: &Message) -> u64 {
    let chars: u64 = match message {
        Message::User { content, .. } | Message::ToolResult { content, .. } => content
            .iter()
            .map(|block| match block {
                UserContent::Text { text } => text.len() as u64,
                UserContent::Image { .. } => 4_800,
            })
            .sum(),
        Message::Assistant { content, .. } => content
            .iter()
            .map(|block| match block {
                AssistantContent::Text { text, .. } => text.len() as u64,
                AssistantContent::Thinking { thinking, .. } => thinking.len() as u64,
                AssistantContent::ToolCall {
                    name, arguments, ..
                } => {
                    name.len() as u64
                        + serde_json::to_string(arguments).unwrap_or_default().len() as u64
                }
            })
            .sum(),
    };

    chars.div_ceil(4)
}

fn find_cut_point(
    entries: &[SessionEntry],
    start_index: usize,
    end_index: usize,
    keep_recent_tokens: u64,
) -> CutPointResult {
    let cut_points = find_valid_cut_points(entries, start_index, end_index);
    if cut_points.is_empty() {
        return CutPointResult {
            first_kept_entry_index: start_index,
            turn_start_index: None,
            is_split_turn: false,
        };
    }

    let mut accumulated_tokens = 0;
    let mut cut_index = cut_points[0];
    for index in (start_index..end_index).rev() {
        let Some(message) = summary_message_from_entry(&entries[index]) else {
            continue;
        };
        accumulated_tokens += estimate_tokens(&message);
        if accumulated_tokens >= keep_recent_tokens {
            if let Some(found_cut_index) = cut_points.iter().copied().find(|cut| *cut >= index) {
                cut_index = found_cut_index;
            }
            break;
        }
    }

    while cut_index > start_index {
        let previous = &entries[cut_index - 1];
        if matches!(previous, SessionEntry::Compaction { .. }) {
            break;
        }
        if summary_message_from_entry(previous).is_some() {
            break;
        }
        cut_index -= 1;
    }

    let is_turn_start = entry_starts_turn(&entries[cut_index]);
    let turn_start_index = if is_turn_start {
        None
    } else {
        find_turn_start_index(entries, cut_index, start_index)
    };

    CutPointResult {
        first_kept_entry_index: cut_index,
        turn_start_index,
        is_split_turn: !is_turn_start && turn_start_index.is_some(),
    }
}

fn find_valid_cut_points(
    entries: &[SessionEntry],
    start_index: usize,
    end_index: usize,
) -> Vec<usize> {
    let mut cut_points = Vec::new();
    for (index, entry) in entries.iter().enumerate().take(end_index).skip(start_index) {
        match entry {
            SessionEntry::Message { message, .. } => match message.role() {
                "toolResult" => {}
                "user" | "assistant" | "bashExecution" | "custom" | "branchSummary"
                | "compactionSummary" => cut_points.push(index),
                _ => {}
            },
            SessionEntry::BranchSummary { .. } | SessionEntry::CustomMessage { .. } => {
                cut_points.push(index)
            }
            _ => {}
        }
    }
    cut_points
}

fn find_turn_start_index(
    entries: &[SessionEntry],
    entry_index: usize,
    start_index: usize,
) -> Option<usize> {
    (start_index..=entry_index)
        .rev()
        .find(|index| entry_starts_turn(&entries[*index]))
}

fn entry_starts_turn(entry: &SessionEntry) -> bool {
    match entry {
        SessionEntry::BranchSummary { .. } | SessionEntry::CustomMessage { .. } => true,
        SessionEntry::Message { message, .. } => {
            matches!(
                message.role(),
                "user" | "bashExecution" | "custom" | "branchSummary" | "compactionSummary"
            )
        }
        _ => false,
    }
}

fn summary_message_from_entry(entry: &SessionEntry) -> Option<AgentMessage> {
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
            timestamp_to_millis_from_str(timestamp),
        )),
        SessionEntry::BranchSummary {
            summary,
            from_id,
            timestamp,
            ..
        } => Some(create_branch_summary_message(
            summary.clone(),
            from_id.clone(),
            timestamp_to_millis_from_str(timestamp),
        )),
        SessionEntry::Compaction {
            summary,
            tokens_before,
            timestamp,
            ..
        } => Some(create_compaction_summary_message(
            summary.clone(),
            *tokens_before,
            timestamp_to_millis_from_str(timestamp),
        )),
        SessionEntry::ThinkingLevelChange { .. }
        | SessionEntry::ModelChange { .. }
        | SessionEntry::Custom { .. }
        | SessionEntry::Label { .. }
        | SessionEntry::SessionInfo { .. } => None,
    }
}

fn compaction_message_from_entry(entry: &SessionEntry) -> Option<AgentMessage> {
    match entry {
        SessionEntry::Compaction { .. } => None,
        _ => summary_message_from_entry(entry),
    }
}

fn prepare_branch_messages(entries: &[SessionEntry], token_budget: u64) -> Vec<AgentMessage> {
    let mut messages = Vec::new();
    let mut total_tokens = 0;

    for entry in entries.iter().rev() {
        let Some(message) = summary_message_from_entry(entry) else {
            continue;
        };
        let tokens = estimate_tokens(&message);
        if token_budget > 0 && total_tokens + tokens > token_budget {
            if matches!(
                entry,
                SessionEntry::Compaction { .. } | SessionEntry::BranchSummary { .. }
            ) && total_tokens < token_budget.saturating_mul(9) / 10
            {
                messages.insert(0, message);
            }
            break;
        }
        total_tokens += tokens;
        messages.insert(0, message);
    }

    messages
}

async fn generate_summary(
    current_messages: &[AgentMessage],
    model: &Model,
    reserve_tokens: u64,
    api_key: &str,
    headers: Option<BTreeMap<String, String>>,
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
) -> Result<String, String> {
    let base_prompt = if previous_summary.is_some() {
        UPDATE_SUMMARIZATION_PROMPT
    } else {
        SUMMARIZATION_PROMPT
    };
    let base_prompt = if let Some(custom_instructions) = custom_instructions {
        format!("{base_prompt}\n\nAdditional focus: {custom_instructions}")
    } else {
        base_prompt.to_owned()
    };

    let conversation_text = serialize_conversation(&convert_to_llm(current_messages.to_vec()));
    let mut prompt_text = format!("<conversation>\n{conversation_text}\n</conversation>\n\n");
    if let Some(previous_summary) = previous_summary {
        prompt_text.push_str(&format!(
            "<previous-summary>\n{previous_summary}\n</previous-summary>\n\n"
        ));
    }
    prompt_text.push_str(&base_prompt);

    let max_tokens = model
        .max_tokens
        .min((reserve_tokens.saturating_mul(8) / 10).max(256));
    let response = request_summary(
        model,
        api_key,
        headers,
        prompt_text,
        max_tokens,
        model.reasoning.then_some(AiThinkingLevel::High),
    )
    .await?;

    Ok(assistant_text(&response))
}

async fn generate_turn_prefix_summary(
    messages: &[AgentMessage],
    model: &Model,
    reserve_tokens: u64,
    api_key: &str,
    headers: Option<BTreeMap<String, String>>,
) -> Result<String, String> {
    let conversation_text = serialize_conversation(&convert_to_llm(messages.to_vec()));
    let prompt_text = format!(
        "<conversation>\n{conversation_text}\n</conversation>\n\n{TURN_PREFIX_SUMMARIZATION_PROMPT}"
    );
    let max_tokens = model.max_tokens.min((reserve_tokens / 2).max(256));
    let response = request_summary(model, api_key, headers, prompt_text, max_tokens, None).await?;
    Ok(assistant_text(&response))
}

async fn request_summary(
    model: &Model,
    api_key: &str,
    headers: Option<BTreeMap<String, String>>,
    prompt_text: String,
    max_tokens: u64,
    reasoning: Option<AiThinkingLevel>,
) -> Result<AssistantMessage, String> {
    let assistant = complete_simple(
        model.clone(),
        Context {
            system_prompt: Some(SUMMARIZATION_SYSTEM_PROMPT.to_owned()),
            messages: vec![Message::User {
                content: vec![UserContent::Text { text: prompt_text }],
                timestamp: now_ms(),
            }],
            tools: Vec::new(),
        },
        SimpleStreamOptions {
            api_key: Some(api_key.to_owned()),
            headers: headers.unwrap_or_default(),
            max_tokens: Some(max_tokens.max(1)),
            reasoning,
            ..SimpleStreamOptions::default()
        },
    )
    .await
    .map_err(|error| error.to_string())?;

    match assistant.stop_reason {
        StopReason::Error => Err(assistant
            .error_message
            .unwrap_or_else(|| String::from("Summarization failed"))),
        StopReason::Aborted => Err(String::from("Summarization cancelled")),
        StopReason::Stop | StopReason::Length | StopReason::ToolUse => Ok(assistant),
    }
}

fn assistant_text(message: &AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text { text, .. } => Some(text.as_str()),
            AssistantContent::Thinking { .. } | AssistantContent::ToolCall { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn serialize_conversation(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        match message {
            Message::User { content, .. } => {
                let content = content
                    .iter()
                    .filter_map(|content| match content {
                        UserContent::Text { text } => Some(text.as_str()),
                        UserContent::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if !content.is_empty() {
                    parts.push(format!("[User]: {content}"));
                }
            }
            Message::Assistant { content, .. } => {
                let mut text_parts = Vec::new();
                let mut thinking_parts = Vec::new();
                let mut tool_calls = Vec::new();
                for block in content {
                    match block {
                        AssistantContent::Text { text, .. } => text_parts.push(text.clone()),
                        AssistantContent::Thinking { thinking, .. } => {
                            thinking_parts.push(thinking.clone())
                        }
                        AssistantContent::ToolCall {
                            name, arguments, ..
                        } => {
                            let args = arguments
                                .iter()
                                .map(|(key, value)| {
                                    format!(
                                        "{key}={}",
                                        serde_json::to_string(value).unwrap_or_default()
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            tool_calls.push(format!("{name}({args})"));
                        }
                    }
                }
                if !thinking_parts.is_empty() {
                    parts.push(format!(
                        "[Assistant thinking]: {}",
                        thinking_parts.join("\n")
                    ));
                }
                if !text_parts.is_empty() {
                    parts.push(format!("[Assistant]: {}", text_parts.join("\n")));
                }
                if !tool_calls.is_empty() {
                    parts.push(format!("[Assistant tool calls]: {}", tool_calls.join("; ")));
                }
            }
            Message::ToolResult { content, .. } => {
                let content = content
                    .iter()
                    .filter_map(|content| match content {
                        UserContent::Text { text } => Some(text.as_str()),
                        UserContent::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if !content.is_empty() {
                    parts.push(format!(
                        "[Tool result]: {}",
                        truncate_for_summary(&content, TOOL_RESULT_MAX_CHARS)
                    ));
                }
            }
        }
    }
    parts.join("\n\n")
}

fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }

    let truncated = text.chars().take(max_chars).collect::<String>();
    let truncated_chars = text.chars().count().saturating_sub(max_chars);
    format!("{truncated}\n\n[... {truncated_chars} more characters truncated]")
}

fn timestamp_to_millis(value: &str) -> Option<u64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .and_then(|timestamp| u64::try_from(timestamp.unix_timestamp_nanos() / 1_000_000).ok())
}

fn timestamp_to_millis_from_str(value: &str) -> u64 {
    timestamp_to_millis(value).unwrap_or_default()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
