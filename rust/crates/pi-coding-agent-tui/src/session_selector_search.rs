use pi_coding_agent_core::SessionInfo;
use pi_tui::fuzzy_match;
use regex::RegexBuilder;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Threaded,
    Recent,
    Relevance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFilter {
    All,
    Named,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchTokenKind {
    Fuzzy,
    Phrase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchToken {
    pub kind: SearchTokenKind,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchMode {
    Tokens,
    Regex,
}

#[derive(Debug, Clone)]
pub struct ParsedSearchQuery {
    pub mode: SearchMode,
    pub tokens: Vec<SearchToken>,
    pub regex: Option<regex::Regex>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchResult {
    pub matches: bool,
    pub score: f64,
}

pub fn has_session_name(session: &SessionInfo) -> bool {
    session
        .name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
}

pub fn parse_search_query(query: &str) -> ParsedSearchQuery {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return ParsedSearchQuery {
            mode: SearchMode::Tokens,
            tokens: Vec::new(),
            regex: None,
            error: None,
        };
    }

    if let Some(pattern) = trimmed.strip_prefix("re:") {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return ParsedSearchQuery {
                mode: SearchMode::Regex,
                tokens: Vec::new(),
                regex: None,
                error: Some(String::from("Empty regex")),
            };
        }
        return match RegexBuilder::new(pattern).case_insensitive(true).build() {
            Ok(regex) => ParsedSearchQuery {
                mode: SearchMode::Regex,
                tokens: Vec::new(),
                regex: Some(regex),
                error: None,
            },
            Err(error) => ParsedSearchQuery {
                mode: SearchMode::Regex,
                tokens: Vec::new(),
                regex: None,
                error: Some(error.to_string()),
            },
        };
    }

    let mut tokens = Vec::new();
    let mut buffer = String::new();
    let mut in_quote = false;

    let flush = |tokens: &mut Vec<SearchToken>, buffer: &mut String, kind: SearchTokenKind| {
        let value = buffer.trim();
        if value.is_empty() {
            buffer.clear();
            return;
        }
        tokens.push(SearchToken {
            kind,
            value: value.to_owned(),
        });
        buffer.clear();
    };

    for character in trimmed.chars() {
        if character == '"' {
            if in_quote {
                flush(&mut tokens, &mut buffer, SearchTokenKind::Phrase);
                in_quote = false;
            } else {
                flush(&mut tokens, &mut buffer, SearchTokenKind::Fuzzy);
                in_quote = true;
            }
            continue;
        }

        if !in_quote && character.is_whitespace() {
            flush(&mut tokens, &mut buffer, SearchTokenKind::Fuzzy);
            continue;
        }

        buffer.push(character);
    }

    if in_quote {
        tokens = trimmed
            .split_whitespace()
            .filter(|token| !token.is_empty())
            .map(|token| SearchToken {
                kind: SearchTokenKind::Fuzzy,
                value: token.to_owned(),
            })
            .collect();
    } else {
        flush(&mut tokens, &mut buffer, SearchTokenKind::Fuzzy);
    }

    ParsedSearchQuery {
        mode: SearchMode::Tokens,
        tokens,
        regex: None,
        error: None,
    }
}

pub fn match_session(session: &SessionInfo, parsed: &ParsedSearchQuery) -> MatchResult {
    let text = session_search_text(session);

    match parsed.mode {
        SearchMode::Regex => {
            let Some(regex) = parsed.regex.as_ref() else {
                return MatchResult {
                    matches: false,
                    score: 0.0,
                };
            };
            if let Some(found) = regex.find(&text) {
                return MatchResult {
                    matches: true,
                    score: found.start() as f64 * 0.1,
                };
            }
            MatchResult {
                matches: false,
                score: 0.0,
            }
        }
        SearchMode::Tokens => {
            if parsed.tokens.is_empty() {
                return MatchResult {
                    matches: true,
                    score: 0.0,
                };
            }

            let mut total_score = 0.0;
            let mut normalized_text = None::<String>;
            for token in &parsed.tokens {
                match token.kind {
                    SearchTokenKind::Phrase => {
                        let text = normalized_text
                            .get_or_insert_with(|| normalize_whitespace_lower(&text));
                        let phrase = normalize_whitespace_lower(&token.value);
                        if phrase.is_empty() {
                            continue;
                        }
                        let Some(index) = text.find(&phrase) else {
                            return MatchResult {
                                matches: false,
                                score: 0.0,
                            };
                        };
                        total_score += index as f64 * 0.1;
                    }
                    SearchTokenKind::Fuzzy => {
                        let matched = fuzzy_match(&token.value, &text);
                        if !matched.matches {
                            return MatchResult {
                                matches: false,
                                score: 0.0,
                            };
                        }
                        total_score += matched.score;
                    }
                }
            }

            MatchResult {
                matches: true,
                score: total_score,
            }
        }
    }
}

pub fn filter_and_sort_sessions(
    sessions: &[SessionInfo],
    query: &str,
    sort_mode: SortMode,
    name_filter: NameFilter,
) -> Vec<SessionInfo> {
    let name_filtered = sessions
        .iter()
        .filter(|session| match name_filter {
            NameFilter::All => true,
            NameFilter::Named => has_session_name(session),
        })
        .cloned()
        .collect::<Vec<_>>();

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return match sort_mode {
            SortMode::Recent => {
                let mut sessions = name_filtered;
                sessions
                    .sort_by_key(|session| std::cmp::Reverse(system_time_millis(session.modified)));
                sessions
            }
            SortMode::Threaded | SortMode::Relevance => name_filtered,
        };
    }

    let parsed = parse_search_query(query);
    if parsed.error.is_some() {
        return Vec::new();
    }

    if matches!(sort_mode, SortMode::Recent) {
        return name_filtered
            .into_iter()
            .filter(|session| match_session(session, &parsed).matches)
            .collect();
    }

    let mut scored = name_filtered
        .into_iter()
        .filter_map(|session| {
            let matched = match_session(&session, &parsed);
            matched.matches.then_some((session, matched.score))
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                system_time_millis(right.0.modified).cmp(&system_time_millis(left.0.modified))
            })
    });

    scored.into_iter().map(|(session, _)| session).collect()
}

fn session_search_text(session: &SessionInfo) -> String {
    format!(
        "{} {} {} {}",
        session.id,
        session.name.as_deref().unwrap_or_default(),
        session.all_messages_text,
        session.cwd,
    )
}

fn normalize_whitespace_lower(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn system_time_millis(value: SystemTime) -> u128 {
    value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
