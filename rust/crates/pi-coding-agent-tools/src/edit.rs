use crate::path_utils::resolve_to_cwd;
use pi_agent::{AgentTool, AgentToolError, AgentToolResult};
use pi_events::{ToolDefinition, UserContent};
use serde_json::{Value, json};
use std::{fs, path::Path, path::PathBuf};

pub fn edit_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "edit".into(),
        description: "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path to the file to edit (relative or absolute)"},
                "edits": {
                    "type": "array",
                    "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": {"type": "string", "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."},
                            "newText": {"type": "string", "description": "Replacement text for this targeted edit."}
                        },
                        "required": ["oldText", "newText"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["path", "edits"],
            "additionalProperties": false
        }),
    }
}

pub fn create_edit_tool(cwd: impl Into<PathBuf>) -> AgentTool {
    let cwd = cwd.into();
    AgentTool::new(
        edit_tool_definition(),
        move |_tool_call_id, args, signal| {
            let cwd = cwd.clone();
            async move { execute_edit(&cwd, args, signal.as_ref()) }
        },
    )
    .with_prepare_arguments(prepare_edit_arguments)
}

fn execute_edit(
    cwd: &Path,
    args: Value,
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<AgentToolResult, AgentToolError> {
    abort_if_requested(signal)?;

    let path = string_arg(&args, &["path", "file_path"])?;
    let edits = parse_edits(&args, &path)?;
    let absolute_path = resolve_to_cwd(&path, cwd);

    let raw_content = match fs::read(&absolute_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AgentToolError::message(format!("File not found: {path}")));
        }
        Err(error) => return Err(io_error(error)),
    };

    abort_if_requested(signal)?;

    let raw_text = String::from_utf8_lossy(&raw_content);
    let (bom, text_without_bom) = strip_bom(&raw_text);
    let original_ending = detect_line_ending(text_without_bom);
    let normalized_content = normalize_to_lf(text_without_bom);
    let applied = apply_edits_to_normalized_content(&normalized_content, &edits, &path)?;
    let final_content = format!(
        "{bom}{}",
        restore_line_endings(&applied.new_content, original_ending)
    );

    abort_if_requested(signal)?;
    fs::write(&absolute_path, final_content.as_bytes()).map_err(io_error)?;
    abort_if_requested(signal)?;

    Ok(AgentToolResult {
        content: vec![UserContent::Text {
            text: format!("Successfully replaced {} block(s) in {path}.", edits.len()),
        }],
        details: json!({ "firstChangedLine": applied.first_changed_line }),
    })
}

fn prepare_edit_arguments(input: Value) -> Value {
    let Value::Object(mut object) = input else {
        return input;
    };

    let old_text = object.remove("oldText");
    let new_text = object.remove("newText");
    let edits = object
        .entry(String::from("edits"))
        .or_insert_with(|| Value::Array(Vec::new()));

    if let (Some(Value::String(old_text)), Some(Value::String(new_text))) = (old_text, new_text) {
        if let Value::Array(edits) = edits {
            edits.push(json!({ "oldText": old_text, "newText": new_text }));
        }
    }

    Value::Object(object)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditBlock {
    old_text: String,
    new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedEditsResult {
    base_content: String,
    new_content: String,
    first_changed_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchResult {
    found: bool,
    index: usize,
    match_length: usize,
    used_fuzzy_match: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchedEdit {
    edit_index: usize,
    match_index: usize,
    match_length: usize,
    new_text: String,
}

fn parse_edits(args: &Value, path: &str) -> Result<Vec<EditBlock>, AgentToolError> {
    let edits = args.get("edits").and_then(Value::as_array).ok_or_else(|| {
        AgentToolError::message(
            "Edit tool input is invalid. edits must contain at least one replacement.",
        )
    })?;

    if edits.is_empty() {
        return Err(AgentToolError::message(
            "Edit tool input is invalid. edits must contain at least one replacement.",
        ));
    }

    edits
        .iter()
        .enumerate()
        .map(|(index, edit)| parse_edit_block(edit, index, edits.len(), path))
        .collect()
}

fn parse_edit_block(
    value: &Value,
    index: usize,
    total_edits: usize,
    path: &str,
) -> Result<EditBlock, AgentToolError> {
    let Some(object) = value.as_object() else {
        return Err(AgentToolError::message(format!(
            "edits[{index}] is invalid in {path}."
        )));
    };

    let old_text = object
        .get("oldText")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| empty_old_text_error(path, index, total_edits))?;
    let new_text = object
        .get("newText")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            AgentToolError::message(format!(
                "edits[{index}].newText must be a string in {path}."
            ))
        })?;

    Ok(EditBlock { old_text, new_text })
}

fn apply_edits_to_normalized_content(
    normalized_content: &str,
    edits: &[EditBlock],
    path: &str,
) -> Result<AppliedEditsResult, AgentToolError> {
    let normalized_edits = edits
        .iter()
        .map(|edit| EditBlock {
            old_text: normalize_to_lf(&edit.old_text),
            new_text: normalize_to_lf(&edit.new_text),
        })
        .collect::<Vec<_>>();

    for (index, edit) in normalized_edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return Err(empty_old_text_error(path, index, normalized_edits.len()));
        }
    }

    let initial_matches = normalized_edits
        .iter()
        .map(|edit| fuzzy_find_text(normalized_content, &edit.old_text))
        .collect::<Vec<_>>();
    let base_content = if initial_matches.iter().any(|result| result.used_fuzzy_match) {
        normalize_for_fuzzy_match(normalized_content)
    } else {
        normalized_content.to_string()
    };

    let mut matched_edits = Vec::new();
    for (index, edit) in normalized_edits.iter().enumerate() {
        let match_result = fuzzy_find_text(&base_content, &edit.old_text);
        if !match_result.found {
            return Err(not_found_error(path, index, normalized_edits.len()));
        }

        let occurrences = count_occurrences(&base_content, &edit.old_text);
        if occurrences > 1 {
            return Err(duplicate_error(
                path,
                index,
                normalized_edits.len(),
                occurrences,
            ));
        }

        matched_edits.push(MatchedEdit {
            edit_index: index,
            match_index: match_result.index,
            match_length: match_result.match_length,
            new_text: edit.new_text.clone(),
        });
    }

    matched_edits.sort_by_key(|edit| edit.match_index);
    for window in matched_edits.windows(2) {
        let previous = &window[0];
        let current = &window[1];
        if previous.match_index + previous.match_length > current.match_index {
            return Err(AgentToolError::message(format!(
                "edits[{}] and edits[{}] overlap in {path}. Merge them into one edit or target disjoint regions.",
                previous.edit_index, current.edit_index
            )));
        }
    }

    let first_changed_line = line_number_for_index(
        &base_content,
        matched_edits
            .first()
            .map(|edit| edit.match_index)
            .unwrap_or(0),
    );

    let mut new_content = base_content.clone();
    for edit in matched_edits.iter().rev() {
        new_content.replace_range(
            edit.match_index..edit.match_index + edit.match_length,
            &edit.new_text,
        );
    }

    if new_content == base_content {
        return Err(no_change_error(path, normalized_edits.len()));
    }

    Ok(AppliedEditsResult {
        base_content,
        new_content,
        first_changed_line,
    })
}

fn fuzzy_find_text(content: &str, old_text: &str) -> MatchResult {
    if let Some(index) = content.find(old_text) {
        return MatchResult {
            found: true,
            index,
            match_length: old_text.len(),
            used_fuzzy_match: false,
        };
    }

    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old_text = normalize_for_fuzzy_match(old_text);
    let index = fuzzy_content
        .find(&fuzzy_old_text)
        .unwrap_or_else(|| usize::MAX);
    if index == usize::MAX {
        return MatchResult {
            found: false,
            index: 0,
            match_length: 0,
            used_fuzzy_match: false,
        };
    }

    MatchResult {
        found: true,
        index,
        match_length: fuzzy_old_text.len(),
        used_fuzzy_match: true,
    }
}

fn count_occurrences(content: &str, old_text: &str) -> usize {
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old_text = normalize_for_fuzzy_match(old_text);
    if fuzzy_old_text.is_empty() {
        return 0;
    }
    fuzzy_content.match_indices(&fuzzy_old_text).count()
}

fn normalize_for_fuzzy_match(text: &str) -> String {
    text.split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .map(normalize_fuzzy_char)
        .collect()
}

fn normalize_fuzzy_char(ch: char) -> char {
    match ch {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => '-',
        '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
        | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
        | '\u{3000}' => ' ',
        _ => ch,
    }
}

fn strip_bom(content: &str) -> (&str, &str) {
    if let Some(stripped) = content.strip_prefix('\u{FEFF}') {
        ("\u{FEFF}", stripped)
    } else {
        ("", content)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

fn detect_line_ending(content: &str) -> LineEnding {
    let crlf_index = content.find("\r\n");
    let lf_index = content.find('\n');
    match (crlf_index, lf_index) {
        (Some(crlf_index), Some(lf_index)) if crlf_index < lf_index => LineEnding::Crlf,
        _ => LineEnding::Lf,
    }
}

fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn restore_line_endings(text: &str, line_ending: LineEnding) -> String {
    match line_ending {
        LineEnding::Lf => text.to_string(),
        LineEnding::Crlf => text.replace('\n', "\r\n"),
    }
}

fn line_number_for_index(content: &str, index: usize) -> usize {
    content[..index.min(content.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn string_arg(args: &Value, keys: &[&str]) -> Result<String, AgentToolError> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            AgentToolError::message(format!(
                "Missing required string argument {}",
                keys.join(" or ")
            ))
        })
}

fn abort_if_requested(
    signal: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<(), AgentToolError> {
    if signal.map(|signal| *signal.borrow()).unwrap_or(false) {
        return Err(AgentToolError::message("Operation aborted"));
    }
    Ok(())
}

fn not_found_error(path: &str, edit_index: usize, total_edits: usize) -> AgentToolError {
    if total_edits == 1 {
        AgentToolError::message(format!(
            "Could not find the exact text in {path}. The old text must match exactly including all whitespace and newlines."
        ))
    } else {
        AgentToolError::message(format!(
            "Could not find edits[{edit_index}] in {path}. The oldText must match exactly including all whitespace and newlines."
        ))
    }
}

fn duplicate_error(
    path: &str,
    edit_index: usize,
    total_edits: usize,
    occurrences: usize,
) -> AgentToolError {
    if total_edits == 1 {
        AgentToolError::message(format!(
            "Found {occurrences} occurrences of the text in {path}. The text must be unique. Please provide more context to make it unique."
        ))
    } else {
        AgentToolError::message(format!(
            "Found {occurrences} occurrences of edits[{edit_index}] in {path}. Each oldText must be unique. Please provide more context to make it unique."
        ))
    }
}

fn empty_old_text_error(path: &str, edit_index: usize, total_edits: usize) -> AgentToolError {
    if total_edits == 1 {
        AgentToolError::message(format!("oldText must not be empty in {path}."))
    } else {
        AgentToolError::message(format!(
            "edits[{edit_index}].oldText must not be empty in {path}."
        ))
    }
}

fn no_change_error(path: &str, total_edits: usize) -> AgentToolError {
    if total_edits == 1 {
        AgentToolError::message(format!(
            "No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected."
        ))
    } else {
        AgentToolError::message(format!(
            "No changes made to {path}. The replacements produced identical content."
        ))
    }
}

fn io_error(error: std::io::Error) -> AgentToolError {
    AgentToolError::message(error.to_string())
}
