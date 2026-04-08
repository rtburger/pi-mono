use pi_tui::{fuzzy_filter, fuzzy_match};
use std::borrow::Cow;

#[test]
fn empty_query_matches_everything_with_zero_score() {
    let result = fuzzy_match("", "anything");
    assert!(result.matches);
    assert_eq!(result.score, 0.0);
}

#[test]
fn query_longer_than_text_does_not_match() {
    let result = fuzzy_match("longquery", "short");
    assert!(!result.matches);
}

#[test]
fn exact_match_has_good_score() {
    let result = fuzzy_match("test", "test");
    assert!(result.matches);
    assert!(result.score < 0.0);
}

#[test]
fn characters_must_appear_in_order() {
    let match_in_order = fuzzy_match("abc", "aXbXc");
    let match_out_of_order = fuzzy_match("abc", "cba");

    assert!(match_in_order.matches);
    assert!(!match_out_of_order.matches);
}

#[test]
fn matching_is_case_insensitive() {
    assert!(fuzzy_match("ABC", "abc").matches);
    assert!(fuzzy_match("abc", "ABC").matches);
}

#[test]
fn consecutive_matches_score_better_than_scattered_matches() {
    let consecutive = fuzzy_match("foo", "foobar");
    let scattered = fuzzy_match("foo", "f_o_o_bar");

    assert!(consecutive.matches);
    assert!(scattered.matches);
    assert!(consecutive.score < scattered.score);
}

#[test]
fn word_boundary_matches_score_better() {
    let at_boundary = fuzzy_match("fb", "foo-bar");
    let not_at_boundary = fuzzy_match("fb", "afbx");

    assert!(at_boundary.matches);
    assert!(not_at_boundary.matches);
    assert!(at_boundary.score < not_at_boundary.score);
}

#[test]
fn matches_swapped_alpha_numeric_tokens() {
    let result = fuzzy_match("codex52", "gpt-5.2-codex");
    assert!(result.matches);
}

#[test]
fn empty_query_returns_all_items_unchanged() {
    let items = vec![
        String::from("apple"),
        String::from("banana"),
        String::from("cherry"),
    ];

    let result = fuzzy_filter(&items, "", |item| Cow::Borrowed(item.as_str()))
        .into_iter()
        .map(|item| item.as_str())
        .collect::<Vec<_>>();

    assert_eq!(result, vec!["apple", "banana", "cherry"]);
}

#[test]
fn filters_out_non_matching_items() {
    let items = vec![
        String::from("apple"),
        String::from("banana"),
        String::from("cherry"),
    ];

    let result = fuzzy_filter(&items, "an", |item| Cow::Borrowed(item.as_str()))
        .into_iter()
        .map(|item| item.as_str())
        .collect::<Vec<_>>();

    assert!(result.contains(&"banana"));
    assert!(!result.contains(&"apple"));
    assert!(!result.contains(&"cherry"));
}

#[test]
fn sorts_results_by_match_quality() {
    let items = vec![
        String::from("a_p_p"),
        String::from("app"),
        String::from("application"),
    ];

    let result = fuzzy_filter(&items, "app", |item| Cow::Borrowed(item.as_str()))
        .into_iter()
        .map(|item| item.as_str())
        .collect::<Vec<_>>();

    assert_eq!(result[0], "app");
}

#[test]
fn works_with_custom_get_text_function() {
    struct Item {
        name: &'static str,
        id: u32,
    }

    let items = vec![
        Item { name: "foo", id: 1 },
        Item { name: "bar", id: 2 },
        Item {
            name: "foobar",
            id: 3,
        },
    ];

    let result = fuzzy_filter(&items, "foo", |item| Cow::Borrowed(item.name));

    assert_eq!(result.len(), 2);
    assert!(result.iter().any(|item| item.name == "foo" && item.id == 1));
    assert!(
        result
            .iter()
            .any(|item| item.name == "foobar" && item.id == 3)
    );
}
