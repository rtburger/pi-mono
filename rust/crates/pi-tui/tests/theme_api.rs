use pi_tui::{
    CombinedAutocompleteProvider, Component, DefaultTextStyle, Editor, EditorTheme, Markdown,
    MarkdownTheme, SelectItem, SelectList, SelectListTheme, SettingItem, SettingsList,
    SettingsListTheme, SlashCommand,
};
use std::sync::Arc;

fn themed_select_items() -> Vec<SelectItem> {
    vec![
        SelectItem {
            value: String::from("alpha"),
            label: String::from("Alpha"),
            description: Some(String::from("First option")),
        },
        SelectItem {
            value: String::from("beta"),
            label: String::from("Beta"),
            description: Some(String::from("Second option")),
        },
    ]
}

fn themed_settings_items() -> Vec<SettingItem> {
    vec![SettingItem {
        id: String::from("theme"),
        label: String::from("Theme"),
        description: Some(String::from("Current color theme")),
        current_value: String::from("dark"),
        values: Some(vec![String::from("dark"), String::from("light")]),
        submenu: None,
    }]
}

#[test]
fn theme_builders_are_cloneable_and_reusable() {
    let markdown_theme = MarkdownTheme::new()
        .with_heading(|text| format!("<heading:{text}>"))
        .with_bold(|text| format!("<bold:{text}>"));
    let default_text_style = DefaultTextStyle::new()
        .with_color(|text| format!("<fg:{text}>"))
        .with_underline(true);

    let first_markdown = Markdown::with_default_text_style(
        "# Title",
        0,
        0,
        markdown_theme.clone(),
        default_text_style.clone(),
    );
    let second_markdown =
        Markdown::with_default_text_style("# Title", 0, 0, markdown_theme, default_text_style);
    assert_eq!(first_markdown.render(40), second_markdown.render(40));

    let select_theme = SelectListTheme::new()
        .with_selected_prefix(|text| format!("<prefix:{text}>"))
        .with_selected_text(|text| format!("<selected:{text}>"))
        .with_description(|text| format!("<desc:{text}>"));
    let first_select = SelectList::new(themed_select_items(), 5, select_theme.clone());
    let second_select = SelectList::new(themed_select_items(), 5, select_theme);
    assert_eq!(first_select.render(60), second_select.render(60));

    let settings_theme = SettingsListTheme::new()
        .with_label(|text, selected| format!("<label:{selected}:{text}>"))
        .with_value(|text, selected| format!("<value:{selected}:{text}>"))
        .with_description(|text| format!("<settings-desc:{text}>"))
        .with_hint(|text| format!("<hint:{text}>"));
    let first_settings = SettingsList::new(themed_settings_items(), 5, settings_theme.clone());
    let second_settings = SettingsList::new(themed_settings_items(), 5, settings_theme);
    assert_eq!(first_settings.render(80), second_settings.render(80));
}

#[test]
fn editor_theme_styles_borders_and_autocomplete_menu() {
    let theme = EditorTheme::new()
        .with_border_color(|text| format!("<border:{text}>"))
        .with_select_list(
            SelectListTheme::new()
                .with_selected_prefix(|text| format!("<prefix:{text}>"))
                .with_selected_text(|text| format!("<selected:{text}>"))
                .with_description(|text| format!("<desc:{text}>")),
        );
    let provider = Arc::new(CombinedAutocompleteProvider::new(
        vec![
            SlashCommand {
                name: String::from("model"),
                description: Some(String::from("Select model")),
                argument_completions: None,
            },
            SlashCommand {
                name: String::from("help"),
                description: Some(String::from("Show help")),
                argument_completions: None,
            },
        ],
        ".",
    ));

    let mut editor = Editor::with_theme(theme);
    editor.set_autocomplete_provider(provider);
    editor.handle_input("/");

    let lines = editor.render(50);

    assert!(lines[0].starts_with("<border:"), "lines: {lines:?}");
    assert!(
        lines.iter().any(|line| line.contains("<prefix:→ >")
            && line.contains("<selected:model")
            && line.contains("Select model")),
        "lines: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("help") && line.contains("<desc:")),
        "lines: {lines:?}"
    );
}
