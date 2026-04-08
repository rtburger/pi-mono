use pi_agent::ThinkingLevel;
use pi_coding_agent_cli::{
    AppMode, DiagnosticKind, ListModels, Mode, PrintOutputMode, ToolName, UnknownFlagValue,
    build_initial_message, parse_args, parse_thinking_level, resolve_app_mode,
    to_print_output_mode,
};
use pi_events::UserContent;

fn parse(input: &[&str]) -> pi_coding_agent_cli::Args {
    parse_args(
        &input
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn parses_common_print_mode_flags() {
    let args = parse(&[
        "--provider",
        "openai",
        "--model",
        "gpt-4o",
        "--api-key",
        "sk-test",
        "--system-prompt",
        "system",
        "--append-system-prompt",
        "suffix",
        "--thinking",
        "high",
        "--mode",
        "json",
        "--print",
        "@README.md",
        "summarize",
    ]);

    assert_eq!(args.provider.as_deref(), Some("openai"));
    assert_eq!(args.model.as_deref(), Some("gpt-4o"));
    assert_eq!(args.api_key.as_deref(), Some("sk-test"));
    assert_eq!(args.system_prompt.as_deref(), Some("system"));
    assert_eq!(args.append_system_prompt.as_deref(), Some("suffix"));
    assert_eq!(args.thinking, Some(ThinkingLevel::High));
    assert_eq!(args.mode, Some(Mode::Json));
    assert!(args.print);
    assert_eq!(args.file_args, vec![String::from("README.md")]);
    assert_eq!(args.messages, vec![String::from("summarize")]);
    assert!(args.diagnostics.is_empty());
}

#[test]
fn warns_for_invalid_thinking_level() {
    let args = parse(&["--thinking", "maximum"]);

    assert_eq!(args.thinking, None);
    assert_eq!(args.diagnostics.len(), 1);
    assert_eq!(args.diagnostics[0].kind, DiagnosticKind::Warning);
    assert!(
        args.diagnostics[0]
            .message
            .contains("Invalid thinking level \"maximum\"")
    );
}

#[test]
fn parses_tool_list_and_warns_for_unknown_tools() {
    let args = parse(&["--tools", "read,bash,unknown,ls"]);

    assert_eq!(
        args.tools,
        Some(vec![ToolName::Read, ToolName::Bash, ToolName::Ls])
    );
    assert_eq!(args.diagnostics.len(), 1);
    assert_eq!(args.diagnostics[0].kind, DiagnosticKind::Warning);
    assert!(
        args.diagnostics[0]
            .message
            .contains("Unknown tool \"unknown\"")
    );
}

#[test]
fn captures_unknown_long_flags_like_typescript_cli() {
    let args = parse(&["--plan", "fast", "--custom-flag=value", "--bool-flag"]);

    assert_eq!(
        args.unknown_flags.get("plan"),
        Some(&UnknownFlagValue::String(String::from("fast")))
    );
    assert_eq!(
        args.unknown_flags.get("custom-flag"),
        Some(&UnknownFlagValue::String(String::from("value")))
    );
    assert_eq!(
        args.unknown_flags.get("bool-flag"),
        Some(&UnknownFlagValue::Bool(true))
    );
    assert!(args.messages.is_empty());
}

#[test]
fn parses_list_models_with_optional_search_pattern() {
    let all = parse(&["--list-models"]);
    let search = parse(&["--list-models", "sonnet"]);

    assert_eq!(all.list_models, Some(ListModels::All));
    assert_eq!(
        search.list_models,
        Some(ListModels::Search(String::from("sonnet")))
    );
}

#[test]
fn resolves_app_modes_like_typescript_main() {
    let interactive = parse(&[]);
    let print = parse(&["--print"]);
    let json = parse(&["--mode", "json"]);
    let rpc = parse(&["--mode", "rpc"]);

    assert_eq!(resolve_app_mode(&interactive, true), AppMode::Interactive);
    assert_eq!(resolve_app_mode(&print, true), AppMode::Print);
    assert_eq!(resolve_app_mode(&json, true), AppMode::Json);
    assert_eq!(resolve_app_mode(&rpc, true), AppMode::Rpc);
    assert_eq!(resolve_app_mode(&interactive, false), AppMode::Print);
    assert_eq!(
        to_print_output_mode(AppMode::Print),
        Some(PrintOutputMode::Text)
    );
    assert_eq!(
        to_print_output_mode(AppMode::Json),
        Some(PrintOutputMode::Json)
    );
    assert_eq!(to_print_output_mode(AppMode::Interactive), None);
}

#[test]
fn parses_thinking_levels() {
    assert_eq!(parse_thinking_level("off"), Some(ThinkingLevel::Off));
    assert_eq!(
        parse_thinking_level("minimal"),
        Some(ThinkingLevel::Minimal)
    );
    assert_eq!(parse_thinking_level("low"), Some(ThinkingLevel::Low));
    assert_eq!(parse_thinking_level("medium"), Some(ThinkingLevel::Medium));
    assert_eq!(parse_thinking_level("high"), Some(ThinkingLevel::High));
    assert_eq!(parse_thinking_level("xhigh"), Some(ThinkingLevel::XHigh));
    assert_eq!(parse_thinking_level("invalid"), None);
}

#[test]
fn build_initial_message_merges_stdin_file_text_and_first_message() {
    let mut messages = vec![String::from("Explain it"), String::from("Second message")];
    let result = build_initial_message(
        &mut messages,
        Some(String::from("file\n")),
        Vec::new(),
        Some(String::from("stdin\n")),
    );

    assert_eq!(
        result.initial_message.as_deref(),
        Some("stdin\nfile\nExplain it")
    );
    assert_eq!(messages, vec![String::from("Second message")]);
    assert_eq!(result.initial_images, None);
}

#[test]
fn build_initial_message_returns_images_when_present() {
    let mut messages = Vec::new();
    let result = build_initial_message(
        &mut messages,
        None,
        vec![UserContent::Image {
            data: String::from("abc"),
            mime_type: String::from("image/png"),
        }],
        Some(String::from("stdin")),
    );

    assert_eq!(result.initial_message.as_deref(), Some("stdin"));
    assert_eq!(result.initial_images.as_ref().map(Vec::len), Some(1));
}
