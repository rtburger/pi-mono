use parking_lot::Mutex;
use pi_tui::{Component, SelectItem, SelectList, SelectListLayoutOptions, SelectListTheme};
use std::sync::Arc;

const KEY_UP: &str = "\x1b[A";
const KEY_DOWN: &str = "\x1b[B";
const KEY_ENTER: &str = "\n";
const KEY_ESCAPE: &str = "\x1b";

fn themed_select_list(items: Vec<SelectItem>, max_visible: usize) -> SelectList {
    SelectList::with_layout(
        items,
        max_visible,
        SelectListTheme::new()
            .with_selected_prefix(|text| format!("<{text}>"))
            .with_selected_text(|text| format!("[{text}]"))
            .with_description(|text| format!("({text})"))
            .with_scroll_info(|text| format!("<{text}>"))
            .with_no_match(|text| format!("!{text}!")),
        SelectListLayoutOptions::default().with_truncate_primary(|context| {
            if context.is_selected {
                format!("{} *", context.text)
            } else {
                context.text.to_owned()
            }
        }),
    )
}

#[test]
fn select_list_wraps_selection_and_invokes_callbacks() {
    let selected = Arc::new(Mutex::new(None::<String>));
    let changed = Arc::new(Mutex::new(Vec::<String>::new()));
    let cancelled = Arc::new(Mutex::new(false));
    let mut list = themed_select_list(
        vec![
            SelectItem {
                value: String::from("one"),
                label: String::from("One"),
                description: Some(String::from("First option")),
            },
            SelectItem {
                value: String::from("two"),
                label: String::from("Two"),
                description: Some(String::from("Second option")),
            },
        ],
        5,
    );
    {
        let selected = Arc::clone(&selected);
        list.set_on_select(move |item| *selected.lock() = Some(item.value));
    }
    {
        let changed = Arc::clone(&changed);
        list.set_on_selection_change(move |item| changed.lock().push(item.value));
    }
    {
        let cancelled = Arc::clone(&cancelled);
        list.set_on_cancel(move || *cancelled.lock() = true);
    }

    list.handle_input(KEY_UP);
    list.handle_input(KEY_ENTER);
    list.handle_input(KEY_ESCAPE);

    assert_eq!(*selected.lock(), Some(String::from("two")));
    assert_eq!(changed.lock().as_slice(), &[String::from("two")]);
    assert!(*cancelled.lock());
}

#[test]
fn select_list_renders_description_columns_and_scroll_info() {
    let mut list = themed_select_list(
        vec![
            SelectItem {
                value: String::from("alpha"),
                label: String::from("Alpha"),
                description: Some(String::from("Alpha description")),
            },
            SelectItem {
                value: String::from("beta"),
                label: String::from("Beta"),
                description: Some(String::from("Beta description")),
            },
            SelectItem {
                value: String::from("gamma"),
                label: String::from("Gamma"),
                description: Some(String::from("Gamma description")),
            },
            SelectItem {
                value: String::from("delta"),
                label: String::from("Delta"),
                description: Some(String::from("Delta description")),
            },
        ],
        2,
    );
    list.set_selected_index(2);

    let lines = list.render(60);

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("Beta") || lines[1].contains("Gamma"));
    assert!(lines.iter().any(|line| line.contains("Gamma description")));
    assert!(
        lines[1].contains("<→ >") || lines[1].contains("<→ >["),
        "lines: {lines:?}"
    );
    assert!(lines[2].contains("(3/4)"), "lines: {lines:?}");
}

#[test]
fn select_list_shows_no_match_message_after_filtering() {
    let mut list = themed_select_list(
        vec![SelectItem {
            value: String::from("alpha"),
            label: String::from("Alpha"),
            description: None,
        }],
        5,
    );

    list.set_filter("zzz");

    assert_eq!(
        list.render(40),
        vec![String::from("!  No matching commands!")]
    );
}

#[test]
fn select_list_filter_matches_item_values_by_prefix() {
    let mut list = themed_select_list(
        vec![
            SelectItem {
                value: String::from("theme:dark"),
                label: String::from("Dark"),
                description: None,
            },
            SelectItem {
                value: String::from("theme:light"),
                label: String::from("Light"),
                description: None,
            },
        ],
        5,
    );

    list.set_filter("theme:l");
    let lines = list.render(40);

    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("Light"), "lines: {lines:?}");
    assert!(!lines[0].contains("Dark"), "lines: {lines:?}");
    list.handle_input(KEY_DOWN);
    assert_eq!(
        list.get_selected_item().expect("selected item").value,
        String::from("theme:light")
    );
}
