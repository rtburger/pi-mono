use pi_tui::{
    AutocompleteProvider, CombinedAutocompleteProvider, Component, DefaultTextStyle, Editor,
    EditorTheme, EditorThemeSpec, Markdown, MarkdownTheme, MarkdownThemeSpec, SelectItem,
    SelectList, SelectListTheme, SelectListThemeSpec, SettingItem, SettingsList, SettingsListTheme,
    SettingsListThemeSpec, SlashCommand,
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

fn themed_slash_provider() -> Arc<dyn AutocompleteProvider> {
    Arc::new(CombinedAutocompleteProvider::new(
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
    ))
}

fn highlight_code(code: &str, lang: Option<&str>) -> Vec<String> {
    vec![format!("<highlight:{}:{}>", lang.unwrap_or("plain"), code)]
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
fn theme_specs_convert_to_equivalent_concrete_themes() {
    let markdown_spec = MarkdownThemeSpec {
        heading: Arc::new(|text| format!("<heading:{text}>")),
        link: Arc::new(|text| format!("<link:{text}>")),
        link_url: Arc::new(|text| format!("<url:{text}>")),
        code: Arc::new(|text| format!("<code:{text}>")),
        code_block: Arc::new(|text| format!("<block:{text}>")),
        code_block_border: Arc::new(|text| format!("<block-border:{text}>")),
        quote: Arc::new(|text| format!("<quote:{text}>")),
        quote_border: Arc::new(|text| format!("<quote-border:{text}>")),
        hr: Arc::new(|text| format!("<hr:{text}>")),
        list_bullet: Arc::new(|text| format!("<bullet:{text}>")),
        bold: Arc::new(|text| format!("<bold:{text}>")),
        italic: Arc::new(|text| format!("<italic:{text}>")),
        strikethrough: Arc::new(|text| format!("<strike:{text}>")),
        underline: Arc::new(|text| format!("<underline:{text}>")),
        highlight_code: Some(Arc::new(highlight_code)),
        code_block_indent: Some(String::from(":: ")),
    };
    let markdown_builder = MarkdownTheme::new()
        .with_heading(|text| format!("<heading:{text}>"))
        .with_link(|text| format!("<link:{text}>"))
        .with_link_url(|text| format!("<url:{text}>"))
        .with_code(|text| format!("<code:{text}>"))
        .with_code_block(|text| format!("<block:{text}>"))
        .with_code_block_border(|text| format!("<block-border:{text}>"))
        .with_quote(|text| format!("<quote:{text}>"))
        .with_quote_border(|text| format!("<quote-border:{text}>"))
        .with_hr(|text| format!("<hr:{text}>"))
        .with_list_bullet(|text| format!("<bullet:{text}>"))
        .with_bold(|text| format!("<bold:{text}>"))
        .with_italic(|text| format!("<italic:{text}>"))
        .with_strikethrough(|text| format!("<strike:{text}>"))
        .with_underline(|text| format!("<underline:{text}>"))
        .with_highlight_code(highlight_code)
        .with_code_block_indent(":: ");
    let default_text_style = DefaultTextStyle::new()
        .with_color(|text| format!("<fg:{text}>"))
        .with_underline(true);
    let markdown_text = "# Title\n\n[site](https://example.com)\n\n```rust\nfn main() {}\n```";
    let first_markdown = Markdown::with_default_text_style(
        markdown_text,
        0,
        0,
        MarkdownTheme::from(&markdown_spec),
        default_text_style.clone(),
    );
    let second_markdown = Markdown::with_default_text_style(
        markdown_text,
        0,
        0,
        markdown_builder,
        default_text_style,
    );
    assert_eq!(first_markdown.render(60), second_markdown.render(60));

    let select_spec = SelectListThemeSpec {
        selected_prefix: Arc::new(|text| format!("<prefix:{text}>")),
        selected_text: Arc::new(|text| format!("<selected:{text}>")),
        description: Arc::new(|text| format!("<desc:{text}>")),
        scroll_info: Arc::new(|text| format!("<scroll:{text}>")),
        no_match: Arc::new(|text| format!("<no-match:{text}>")),
    };
    let select_builder = SelectListTheme::new()
        .with_selected_prefix(|text| format!("<prefix:{text}>"))
        .with_selected_text(|text| format!("<selected:{text}>"))
        .with_description(|text| format!("<desc:{text}>"))
        .with_scroll_info(|text| format!("<scroll:{text}>"))
        .with_no_match(|text| format!("<no-match:{text}>"));
    let first_select = SelectList::new(
        themed_select_items(),
        5,
        SelectListTheme::from(&select_spec),
    );
    let second_select = SelectList::new(themed_select_items(), 5, select_builder.clone());
    assert_eq!(first_select.render(60), second_select.render(60));

    let settings_spec = SettingsListThemeSpec {
        label: Arc::new(|text, selected| format!("<label:{selected}:{text}>")),
        value: Arc::new(|text, selected| format!("<value:{selected}:{text}>")),
        description: Arc::new(|text| format!("<settings-desc:{text}>")),
        cursor: String::from("> "),
        hint: Arc::new(|text| format!("<hint:{text}>")),
    };
    let settings_builder = SettingsListTheme::new()
        .with_label(|text, selected| format!("<label:{selected}:{text}>"))
        .with_value(|text, selected| format!("<value:{selected}:{text}>"))
        .with_description(|text| format!("<settings-desc:{text}>"))
        .with_cursor("> ")
        .with_hint(|text| format!("<hint:{text}>"));
    let first_settings = SettingsList::new(
        themed_settings_items(),
        5,
        SettingsListTheme::from(&settings_spec),
    );
    let second_settings = SettingsList::new(themed_settings_items(), 5, settings_builder);
    assert_eq!(first_settings.render(80), second_settings.render(80));

    let editor_spec = EditorThemeSpec {
        border_color: Arc::new(|text| format!("<border:{text}>")),
        select_list: select_spec,
    };
    let editor_builder = EditorTheme::new()
        .with_border_color(|text| format!("<border:{text}>"))
        .with_select_list(select_builder);
    let provider = themed_slash_provider();

    let mut first_editor = Editor::with_theme(EditorTheme::from(&editor_spec));
    first_editor.set_autocomplete_provider(Arc::clone(&provider));
    first_editor.handle_input("/");

    let mut second_editor = Editor::with_theme(editor_builder);
    second_editor.set_autocomplete_provider(provider);
    second_editor.handle_input("/");

    assert_eq!(first_editor.render(50), second_editor.render(50));
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
