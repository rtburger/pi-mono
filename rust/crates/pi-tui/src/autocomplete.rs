use crate::fuzzy_filter;
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_FUZZY_RESULTS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteSuggestions {
    pub items: Vec<AutocompleteItem>,
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionResult {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

pub trait AutocompleteProvider: Send + Sync {
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions>;

    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> CompletionResult;

    fn should_trigger_file_completion(
        &self,
        _lines: &[String],
        _cursor_line: usize,
        _cursor_col: usize,
    ) -> bool {
        true
    }
}

pub type ArgumentCompleter = Arc<dyn Fn(&str) -> Option<Vec<AutocompleteItem>> + Send + Sync>;

#[derive(Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub argument_completions: Option<ArgumentCompleter>,
}

#[derive(Clone)]
pub struct CombinedAutocompleteProvider {
    commands: Vec<SlashCommand>,
    base_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPathPrefix {
    raw_prefix: String,
    is_at_prefix: bool,
    is_quoted_prefix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopedQuery {
    base_dir: PathBuf,
    query: String,
    display_base: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileEntry {
    path: String,
    is_directory: bool,
}

impl CombinedAutocompleteProvider {
    pub fn new(commands: Vec<SlashCommand>, base_path: impl Into<PathBuf>) -> Self {
        Self {
            commands,
            base_path: base_path.into(),
        }
    }

    fn extract_at_prefix(&self, text: &str) -> Option<String> {
        let quoted_prefix = extract_quoted_prefix(text);
        if quoted_prefix
            .as_deref()
            .is_some_and(|value| value.starts_with("@\""))
        {
            return quoted_prefix;
        }

        let token_start = find_last_delimiter(text)
            .map(|index| index + 1)
            .unwrap_or(0);
        if text[token_start..].starts_with('@') {
            return Some(text[token_start..].to_owned());
        }

        None
    }

    fn extract_path_prefix(&self, text: &str, force_extract: bool) -> Option<String> {
        if let Some(prefix) = extract_quoted_prefix(text) {
            return Some(prefix);
        }

        let token_start = find_last_delimiter(text)
            .map(|index| index + 1)
            .unwrap_or(0);
        let path_prefix = &text[token_start..];

        if force_extract {
            return Some(path_prefix.to_owned());
        }

        if path_prefix.contains('/')
            || path_prefix.starts_with('.')
            || path_prefix.starts_with("~/")
        {
            return Some(path_prefix.to_owned());
        }

        if path_prefix.is_empty() && text.ends_with(' ') {
            return Some(String::new());
        }

        None
    }

    fn get_file_suggestions(&self, prefix: &str) -> Vec<AutocompleteItem> {
        let parsed = parse_path_prefix(prefix);
        let raw_prefix = parsed.raw_prefix.as_str();
        let expanded_prefix = expand_home_path(raw_prefix).unwrap_or_else(|| raw_prefix.to_owned());

        let is_root_prefix = raw_prefix.is_empty()
            || matches!(raw_prefix, "./" | "../" | "~" | "~/" | "/")
            || (parsed.is_at_prefix && raw_prefix.is_empty());

        let (search_dir, display_prefix, search_prefix) = if is_root_prefix
            || raw_prefix.ends_with('/')
        {
            let search_dir = if raw_prefix.starts_with('~') || expanded_prefix.starts_with('/') {
                PathBuf::from(&expanded_prefix)
            } else {
                self.base_path.join(&expanded_prefix)
            };
            (search_dir, raw_prefix.to_owned(), String::new())
        } else {
            let search_dir = if raw_prefix.starts_with('~') || expanded_prefix.starts_with('/') {
                PathBuf::from(display_dirname(&expanded_prefix))
            } else {
                self.base_path.join(display_dirname(&expanded_prefix))
            };
            let search_prefix = display_basename(raw_prefix).to_owned();
            (search_dir, raw_prefix.to_owned(), search_prefix)
        };

        let Ok(entries) = fs::read_dir(&search_dir) else {
            return Vec::new();
        };

        let mut suggestions = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name
                .to_lowercase()
                .starts_with(&search_prefix.to_lowercase())
            {
                continue;
            }

            let file_type = entry.file_type().ok();
            let is_symlink = file_type.as_ref().is_some_and(fs::FileType::is_symlink);
            let is_directory = file_type.as_ref().is_some_and(fs::FileType::is_dir)
                || (is_symlink
                    && entry
                        .path()
                        .metadata()
                        .map(|metadata| metadata.is_dir())
                        .unwrap_or(false));

            let mut relative_path = if display_prefix.ends_with('/') {
                format!("{display_prefix}{name}")
            } else if display_prefix.contains('/') || display_prefix.contains('\\') {
                if display_prefix.starts_with("~/") {
                    let home_relative_dir = &display_prefix[2..];
                    let dir = display_dirname(home_relative_dir);
                    if dir == "." {
                        format!("~/{name}")
                    } else {
                        format!("~/{}/{}", to_display_path(&dir), name)
                    }
                } else if display_prefix.starts_with('/') {
                    let dir = display_dirname(&display_prefix);
                    if dir == "/" {
                        format!("/{name}")
                    } else {
                        format!("{}/{}", to_display_path(&dir), name)
                    }
                } else {
                    let mut joined = if display_dirname(&display_prefix) == "." {
                        name.clone()
                    } else {
                        format!(
                            "{}/{}",
                            to_display_path(&display_dirname(&display_prefix)),
                            name
                        )
                    };
                    if display_prefix.starts_with("./") && !joined.starts_with("./") {
                        joined = format!("./{joined}");
                    }
                    joined
                }
            } else if display_prefix.starts_with('~') {
                format!("~/{name}")
            } else {
                name.clone()
            };

            relative_path = to_display_path(&relative_path);
            let completion_path = if is_directory {
                format!("{relative_path}/")
            } else {
                relative_path
            };
            let value = build_completion_value(&completion_path, is_directory, &parsed);
            suggestions.push(AutocompleteItem {
                value,
                label: format!("{name}{}", if is_directory { "/" } else { "" }),
                description: None,
            });
        }

        suggestions.sort_by(|left, right| {
            let left_is_dir = left.value.ends_with('/');
            let right_is_dir = right.value.ends_with('/');
            match (left_is_dir, right_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => left.label.cmp(&right.label),
            }
        });

        suggestions
    }

    fn score_entry(&self, file_path: &str, query: &str, is_directory: bool) -> i32 {
        let file_name = display_basename(file_path).to_lowercase();
        let lower_query = query.to_lowercase();
        let mut score = 0;

        if file_name == lower_query {
            score = 100;
        } else if file_name.starts_with(&lower_query) {
            score = 80;
        } else if file_name.contains(&lower_query) {
            score = 50;
        } else if file_path.to_lowercase().contains(&lower_query) {
            score = 30;
        }

        if is_directory && score > 0 {
            score += 10;
        }

        score
    }

    fn resolve_scoped_fuzzy_query(&self, raw_query: &str) -> Option<ScopedQuery> {
        let normalized_query = to_display_path(raw_query);
        let slash_index = normalized_query.rfind('/')?;
        let display_base = normalized_query[..=slash_index].to_owned();
        let query = normalized_query[slash_index + 1..].to_owned();

        let base_dir = if display_base.starts_with("~/") {
            PathBuf::from(expand_home_path(&display_base)?)
        } else if display_base.starts_with('/') {
            PathBuf::from(&display_base)
        } else {
            self.base_path.join(&display_base)
        };

        if !base_dir.is_dir() {
            return None;
        }

        Some(ScopedQuery {
            base_dir,
            query,
            display_base,
        })
    }

    fn scoped_path_for_display(&self, display_base: &str, relative_path: &str) -> String {
        let normalized_relative_path = to_display_path(relative_path);
        if display_base == "/" {
            return format!("/{normalized_relative_path}");
        }
        format!(
            "{}{}",
            to_display_path(display_base),
            normalized_relative_path
        )
    }

    fn get_fuzzy_file_suggestions(
        &self,
        query: &str,
        is_quoted_prefix: bool,
    ) -> Vec<AutocompleteItem> {
        let scoped_query = self.resolve_scoped_fuzzy_query(query);
        let walk_base_dir = scoped_query
            .as_ref()
            .map(|scoped| scoped.base_dir.as_path())
            .unwrap_or(self.base_path.as_path());
        let walk_query = scoped_query
            .as_ref()
            .map(|scoped| scoped.query.as_str())
            .unwrap_or(query);

        let entries = collect_recursive_entries(walk_base_dir);
        let mut scored = entries
            .into_iter()
            .filter_map(|entry| {
                let score = if walk_query.is_empty() {
                    1
                } else {
                    self.score_entry(&entry.path, walk_query, entry.is_directory)
                };
                (score > 0).then_some((entry, score))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| right.1.cmp(&left.1));

        scored
            .into_iter()
            .take(MAX_FUZZY_RESULTS)
            .map(|(entry, _)| {
                let display_path = if let Some(scoped) = &scoped_query {
                    self.scoped_path_for_display(&scoped.display_base, &entry.path)
                } else {
                    entry.path.clone()
                };
                let entry_name = display_basename(display_path.trim_end_matches('/')).to_owned();
                let completion_path = if entry.is_directory {
                    format!("{display_path}/")
                } else {
                    display_path.clone()
                };
                let parsed = ParsedPathPrefix {
                    raw_prefix: query.to_owned(),
                    is_at_prefix: true,
                    is_quoted_prefix,
                };
                let value = build_completion_value(&completion_path, entry.is_directory, &parsed);
                AutocompleteItem {
                    value,
                    label: format!("{entry_name}{}", if entry.is_directory { "/" } else { "" }),
                    description: Some(display_path),
                }
            })
            .collect()
    }
}

impl AutocompleteProvider for CombinedAutocompleteProvider {
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    ) -> Option<AutocompleteSuggestions> {
        let current_line = lines.get(cursor_line).map(String::as_str).unwrap_or("");
        let cursor_col = cursor_col.min(current_line.len());
        let text_before_cursor = &current_line[..cursor_col];

        if let Some(at_prefix) = self.extract_at_prefix(text_before_cursor) {
            let parsed = parse_path_prefix(&at_prefix);
            let suggestions =
                self.get_fuzzy_file_suggestions(&parsed.raw_prefix, parsed.is_quoted_prefix);
            if suggestions.is_empty() {
                return None;
            }
            return Some(AutocompleteSuggestions {
                items: suggestions,
                prefix: at_prefix,
            });
        }

        if !force && text_before_cursor.starts_with('/') {
            let space_index = text_before_cursor.find(' ');
            if let Some(space_index) = space_index {
                let command_name = &text_before_cursor[1..space_index];
                let argument_text = &text_before_cursor[space_index + 1..];
                let command = self
                    .commands
                    .iter()
                    .find(|command| command.name == command_name)?;
                let completer = command.argument_completions.as_ref()?;
                let items = completer(argument_text)?;
                if items.is_empty() {
                    return None;
                }
                return Some(AutocompleteSuggestions {
                    items,
                    prefix: argument_text.to_owned(),
                });
            }

            let prefix = &text_before_cursor[1..];
            let filtered = fuzzy_filter(&self.commands, prefix, |command| {
                Cow::Borrowed(command.name.as_str())
            });
            if filtered.is_empty() {
                return None;
            }

            return Some(AutocompleteSuggestions {
                items: filtered
                    .into_iter()
                    .map(|command| AutocompleteItem {
                        value: command.name.clone(),
                        label: command.name.clone(),
                        description: command.description.clone(),
                    })
                    .collect(),
                prefix: text_before_cursor.to_owned(),
            });
        }

        let path_prefix = self.extract_path_prefix(text_before_cursor, force)?;
        let suggestions = self.get_file_suggestions(&path_prefix);
        if suggestions.is_empty() {
            return None;
        }

        Some(AutocompleteSuggestions {
            items: suggestions,
            prefix: path_prefix,
        })
    }

    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        item: &AutocompleteItem,
        prefix: &str,
    ) -> CompletionResult {
        apply_completion(lines, cursor_line, cursor_col, item, prefix)
    }

    fn should_trigger_file_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> bool {
        let current_line = lines.get(cursor_line).map(String::as_str).unwrap_or("");
        let cursor_col = cursor_col.min(current_line.len());
        let text_before_cursor = &current_line[..cursor_col];
        !(text_before_cursor.trim_start().starts_with('/')
            && !text_before_cursor.trim_start().contains(' '))
    }
}

pub fn apply_completion(
    lines: &[String],
    cursor_line: usize,
    cursor_col: usize,
    item: &AutocompleteItem,
    prefix: &str,
) -> CompletionResult {
    let current_line = lines.get(cursor_line).cloned().unwrap_or_default();
    let prefix_len = prefix.len().min(cursor_col.min(current_line.len()));
    let before_prefix = current_line[..cursor_col - prefix_len].to_owned();
    let after_cursor = current_line[cursor_col.min(current_line.len())..].to_owned();
    let is_quoted_prefix = prefix.starts_with('"') || prefix.starts_with("@\"");
    let has_leading_quote_after_cursor = after_cursor.starts_with('"');
    let has_trailing_quote_in_item = item.value.ends_with('"');
    let adjusted_after_cursor =
        if is_quoted_prefix && has_trailing_quote_in_item && has_leading_quote_after_cursor {
            after_cursor[1..].to_owned()
        } else {
            after_cursor
        };

    let is_slash_command =
        prefix.starts_with('/') && before_prefix.trim().is_empty() && !prefix[1..].contains('/');
    if is_slash_command {
        let new_line = format!("{before_prefix}/{} {adjusted_after_cursor}", item.value);
        let mut new_lines = lines.to_vec();
        if new_lines.len() <= cursor_line {
            new_lines.resize(cursor_line + 1, String::new());
        }
        new_lines[cursor_line] = new_line;
        return CompletionResult {
            lines: new_lines,
            cursor_line,
            cursor_col: before_prefix.len() + item.value.len() + 2,
        };
    }

    if prefix.starts_with('@') {
        let is_directory = item.label.ends_with('/');
        let suffix = if is_directory { "" } else { " " };
        let new_line = format!(
            "{before_prefix}{}{suffix}{adjusted_after_cursor}",
            item.value
        );
        let mut new_lines = lines.to_vec();
        if new_lines.len() <= cursor_line {
            new_lines.resize(cursor_line + 1, String::new());
        }
        new_lines[cursor_line] = new_line;

        let has_trailing_quote = item.value.ends_with('"');
        let cursor_offset = if is_directory && has_trailing_quote {
            item.value.len().saturating_sub(1)
        } else {
            item.value.len()
        };

        return CompletionResult {
            lines: new_lines,
            cursor_line,
            cursor_col: before_prefix.len() + cursor_offset + suffix.len(),
        };
    }

    let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];
    if text_before_cursor.contains('/') && text_before_cursor.contains(' ') {
        let new_line = format!("{before_prefix}{}{adjusted_after_cursor}", item.value);
        let mut new_lines = lines.to_vec();
        if new_lines.len() <= cursor_line {
            new_lines.resize(cursor_line + 1, String::new());
        }
        new_lines[cursor_line] = new_line;

        let is_directory = item.label.ends_with('/');
        let has_trailing_quote = item.value.ends_with('"');
        let cursor_offset = if is_directory && has_trailing_quote {
            item.value.len().saturating_sub(1)
        } else {
            item.value.len()
        };

        return CompletionResult {
            lines: new_lines,
            cursor_line,
            cursor_col: before_prefix.len() + cursor_offset,
        };
    }

    let new_line = format!("{before_prefix}{}{adjusted_after_cursor}", item.value);
    let mut new_lines = lines.to_vec();
    if new_lines.len() <= cursor_line {
        new_lines.resize(cursor_line + 1, String::new());
    }
    new_lines[cursor_line] = new_line;

    let is_directory = item.label.ends_with('/');
    let has_trailing_quote = item.value.ends_with('"');
    let cursor_offset = if is_directory && has_trailing_quote {
        item.value.len().saturating_sub(1)
    } else {
        item.value.len()
    };

    CompletionResult {
        lines: new_lines,
        cursor_line,
        cursor_col: before_prefix.len() + cursor_offset,
    }
}

fn collect_recursive_entries(base_dir: &Path) -> Vec<FileEntry> {
    let mut results = Vec::new();
    collect_recursive_entries_inner(base_dir, "", &mut results);
    results
}

fn collect_recursive_entries_inner(base_dir: &Path, prefix: &str, results: &mut Vec<FileEntry>) {
    let Ok(entries) = fs::read_dir(base_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }

        let display_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let is_symlink = file_type.is_symlink();
        let is_directory = file_type.is_dir()
            || (is_symlink
                && entry
                    .path()
                    .metadata()
                    .map(|metadata| metadata.is_dir())
                    .unwrap_or(false));

        results.push(FileEntry {
            path: display_path.clone(),
            is_directory,
        });

        if file_type.is_dir() {
            collect_recursive_entries_inner(&entry.path(), &display_path, results);
        }
    }
}

fn parse_path_prefix(prefix: &str) -> ParsedPathPrefix {
    if let Some(raw_prefix) = prefix.strip_prefix("@\"") {
        return ParsedPathPrefix {
            raw_prefix: raw_prefix.to_owned(),
            is_at_prefix: true,
            is_quoted_prefix: true,
        };
    }
    if let Some(raw_prefix) = prefix.strip_prefix('"') {
        return ParsedPathPrefix {
            raw_prefix: raw_prefix.to_owned(),
            is_at_prefix: false,
            is_quoted_prefix: true,
        };
    }
    if let Some(raw_prefix) = prefix.strip_prefix('@') {
        return ParsedPathPrefix {
            raw_prefix: raw_prefix.to_owned(),
            is_at_prefix: true,
            is_quoted_prefix: false,
        };
    }

    ParsedPathPrefix {
        raw_prefix: prefix.to_owned(),
        is_at_prefix: false,
        is_quoted_prefix: false,
    }
}

fn build_completion_value(path: &str, _is_directory: bool, parsed: &ParsedPathPrefix) -> String {
    let needs_quotes = parsed.is_quoted_prefix || path.contains(' ');
    let prefix = if parsed.is_at_prefix { "@" } else { "" };

    if !needs_quotes {
        return format!("{prefix}{path}");
    }

    format!("{prefix}\"{path}\"")
}

fn find_last_delimiter(text: &str) -> Option<usize> {
    text.char_indices()
        .rev()
        .find(|(_, character)| is_path_delimiter(*character))
        .map(|(index, _)| index)
}

fn find_unclosed_quote_start(text: &str) -> Option<usize> {
    let mut in_quotes = false;
    let mut quote_start = None;

    for (index, character) in text.char_indices() {
        if character == '"' {
            in_quotes = !in_quotes;
            if in_quotes {
                quote_start = Some(index);
            }
        }
    }

    if in_quotes { quote_start } else { None }
}

fn extract_quoted_prefix(text: &str) -> Option<String> {
    let quote_start = find_unclosed_quote_start(text)?;

    if quote_start > 0 && text[..quote_start].ends_with('@') {
        let at_index = quote_start - 1;
        if !is_token_start(text, at_index) {
            return None;
        }
        return Some(text[at_index..].to_owned());
    }

    if !is_token_start(text, quote_start) {
        return None;
    }

    Some(text[quote_start..].to_owned())
}

fn is_token_start(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }

    text[..index]
        .chars()
        .next_back()
        .is_some_and(is_path_delimiter)
}

fn is_path_delimiter(character: char) -> bool {
    matches!(character, ' ' | '\t' | '"' | '\'' | '=')
}

fn expand_home_path(path: &str) -> Option<String> {
    let home = home_dir()?;

    if path == "~" {
        return Some(home);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        let mut expanded = format!("{home}/{rest}");
        if path.ends_with('/') && !expanded.ends_with('/') {
            expanded.push('/');
        }
        return Some(expanded);
    }

    Some(path.to_owned())
}

fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .map(|path| to_display_path(&path))
}

fn to_display_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn display_dirname(path: &str) -> String {
    let normalized = to_display_path(path);
    if normalized == "/" {
        return "/".to_owned();
    }
    match normalized.rfind('/') {
        Some(0) => "/".to_owned(),
        Some(index) => normalized[..index].to_owned(),
        None => String::from("."),
    }
}

fn display_basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}
