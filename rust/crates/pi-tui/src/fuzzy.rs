use std::{borrow::Cow, cmp::Ordering};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();

    let primary_match = match_query(&query_lower, &text_lower);
    if primary_match.matches {
        return primary_match;
    }

    let Some(swapped_query) = swapped_query(&query_lower) else {
        return primary_match;
    };

    let swapped_match = match_query(&swapped_query, &text_lower);
    if !swapped_match.matches {
        return primary_match;
    }

    FuzzyMatch {
        matches: true,
        score: swapped_match.score + 5.0,
    }
}

pub fn fuzzy_filter<'a, T, F>(items: &'a [T], query: &str, get_text: F) -> Vec<&'a T>
where
    F: for<'b> Fn(&'b T) -> Cow<'b, str>,
{
    if query.trim().is_empty() {
        return items.iter().collect();
    }

    let tokens = query
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return items.iter().collect();
    }

    let mut results = Vec::new();

    for item in items {
        let text = get_text(item);
        let text = text.as_ref();
        let mut total_score = 0.0;
        let mut all_match = true;

        for token in &tokens {
            let matched = fuzzy_match(token, text);
            if matched.matches {
                total_score += matched.score;
            } else {
                all_match = false;
                break;
            }
        }

        if all_match {
            results.push((item, total_score));
        }
    }

    results.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(Ordering::Equal));

    results.into_iter().map(|(item, _)| item).collect()
}

fn match_query(normalized_query: &str, text_lower: &str) -> FuzzyMatch {
    if normalized_query.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }

    let query_chars = normalized_query.chars().collect::<Vec<_>>();
    let text_chars = text_lower.chars().collect::<Vec<_>>();

    if query_chars.len() > text_chars.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    let mut query_index = 0usize;
    let mut score = 0.0;
    let mut last_match_index = -1isize;
    let mut consecutive_matches = 0usize;

    for (index, character) in text_chars.iter().enumerate() {
        if query_index >= query_chars.len() {
            break;
        }

        if *character != query_chars[query_index] {
            continue;
        }

        let is_word_boundary = index == 0 || is_word_boundary_char(text_chars[index - 1]);

        if last_match_index == index as isize - 1 {
            consecutive_matches += 1;
            score -= consecutive_matches as f64 * 5.0;
        } else {
            consecutive_matches = 0;
            if last_match_index >= 0 {
                score += ((index as isize - last_match_index - 1) * 2) as f64;
            }
        }

        if is_word_boundary {
            score -= 10.0;
        }

        score += index as f64 * 0.1;
        last_match_index = index as isize;
        query_index += 1;
    }

    if query_index < query_chars.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    FuzzyMatch {
        matches: true,
        score,
    }
}

fn is_word_boundary_char(character: char) -> bool {
    character.is_whitespace() || matches!(character, '-' | '_' | '.' | '/' | ':')
}

fn swapped_query(query: &str) -> Option<String> {
    swap_query_if_pattern(
        query,
        |character| character.is_ascii_lowercase(),
        |character| character.is_ascii_digit(),
    )
    .or_else(|| {
        swap_query_if_pattern(
            query,
            |character| character.is_ascii_digit(),
            |character| character.is_ascii_lowercase(),
        )
    })
}

fn swap_query_if_pattern<F, S>(query: &str, first: F, second: S) -> Option<String>
where
    F: Fn(char) -> bool,
    S: Fn(char) -> bool,
{
    let split_index = query
        .char_indices()
        .find(|(_, character)| second(*character))
        .map(|(index, _)| index)?;

    if split_index == 0 {
        return None;
    }

    let (left, right) = query.split_at(split_index);
    if right.is_empty() {
        return None;
    }

    if left.chars().all(first) && right.chars().all(second) {
        Some(format!("{right}{left}"))
    } else {
        None
    }
}
