use pi_agent::ThinkingLevel;
use pi_ai::Transport;
use std::collections::BTreeMap;

const VALID_THINKING_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh"];
const VALID_TRANSPORTS: &[&str] = &["sse", "websocket", "auto"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Text,
    Json,
    Rpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrintOutputMode {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Interactive,
    Print,
    Json,
    Rpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolName {
    Read,
    Bash,
    Edit,
    Write,
    Grep,
    Find,
    Ls,
}

impl ToolName {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "bash" => Some(Self::Bash),
            "edit" => Some(Self::Edit),
            "write" => Some(Self::Write),
            "grep" => Some(Self::Grep),
            "find" => Some(Self::Find),
            "ls" => Some(Self::Ls),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Bash => "bash",
            Self::Edit => "edit",
            Self::Write => "write",
            Self::Grep => "grep",
            Self::Find => "find",
            Self::Ls => "ls",
        }
    }

    pub fn all() -> &'static [Self] {
        const TOOLS: &[ToolName] = &[
            ToolName::Read,
            ToolName::Bash,
            ToolName::Edit,
            ToolName::Write,
            ToolName::Grep,
            ToolName::Find,
            ToolName::Ls,
        ];
        TOOLS
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListModels {
    All,
    Search(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownFlagValue {
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub thinking: Option<ThinkingLevel>,
    pub transport: Option<Transport>,
    pub continue_session: bool,
    pub resume: bool,
    pub help: bool,
    pub version: bool,
    pub mode: Option<Mode>,
    pub no_session: bool,
    pub session: Option<String>,
    pub fork: Option<String>,
    pub session_dir: Option<String>,
    pub models: Option<Vec<String>>,
    pub tools: Option<Vec<ToolName>>,
    pub no_tools: bool,
    pub extensions: Option<Vec<String>>,
    pub no_extensions: bool,
    pub print: bool,
    pub export: Option<String>,
    pub no_skills: bool,
    pub skills: Option<Vec<String>>,
    pub prompt_templates: Option<Vec<String>>,
    pub no_prompt_templates: bool,
    pub themes: Option<Vec<String>>,
    pub no_themes: bool,
    pub list_models: Option<ListModels>,
    pub offline: bool,
    pub verbose: bool,
    pub messages: Vec<String>,
    pub file_args: Vec<String>,
    pub unknown_flags: BTreeMap<String, UnknownFlagValue>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn is_valid_thinking_level(level: &str) -> bool {
    VALID_THINKING_LEVELS.contains(&level)
}

pub fn parse_thinking_level(level: &str) -> Option<ThinkingLevel> {
    match level {
        "off" => Some(ThinkingLevel::Off),
        "minimal" => Some(ThinkingLevel::Minimal),
        "low" => Some(ThinkingLevel::Low),
        "medium" => Some(ThinkingLevel::Medium),
        "high" => Some(ThinkingLevel::High),
        "xhigh" => Some(ThinkingLevel::XHigh),
        _ => None,
    }
}

fn parse_transport(value: &str) -> Option<Transport> {
    match value {
        "sse" => Some(Transport::Sse),
        "websocket" => Some(Transport::WebSocket),
        "auto" => Some(Transport::Auto),
        _ => None,
    }
}

pub fn parse_args(args: &[String]) -> Args {
    let mut result = Args::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];

        if arg == "--help" || arg == "-h" {
            result.help = true;
        } else if arg == "--version" || arg == "-v" {
            result.version = true;
        } else if arg == "--mode" && index + 1 < args.len() {
            index += 1;
            result.mode = match args[index].as_str() {
                "text" => Some(Mode::Text),
                "json" => Some(Mode::Json),
                "rpc" => Some(Mode::Rpc),
                _ => None,
            };
        } else if arg == "--continue" || arg == "-c" {
            result.continue_session = true;
        } else if arg == "--resume" || arg == "-r" {
            result.resume = true;
        } else if arg == "--provider" && index + 1 < args.len() {
            index += 1;
            result.provider = Some(args[index].clone());
        } else if arg == "--model" && index + 1 < args.len() {
            index += 1;
            result.model = Some(args[index].clone());
        } else if arg == "--api-key" && index + 1 < args.len() {
            index += 1;
            result.api_key = Some(args[index].clone());
        } else if arg == "--system-prompt" && index + 1 < args.len() {
            index += 1;
            result.system_prompt = Some(args[index].clone());
        } else if arg == "--append-system-prompt" && index + 1 < args.len() {
            index += 1;
            result.append_system_prompt = Some(args[index].clone());
        } else if arg == "--no-session" {
            result.no_session = true;
        } else if arg == "--session" && index + 1 < args.len() {
            index += 1;
            result.session = Some(args[index].clone());
        } else if arg == "--fork" && index + 1 < args.len() {
            index += 1;
            result.fork = Some(args[index].clone());
        } else if arg == "--session-dir" && index + 1 < args.len() {
            index += 1;
            result.session_dir = Some(args[index].clone());
        } else if arg == "--models" && index + 1 < args.len() {
            index += 1;
            result.models = Some(
                args[index]
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect(),
            );
        } else if arg == "--no-tools" {
            result.no_tools = true;
        } else if arg == "--tools" && index + 1 < args.len() {
            index += 1;
            let mut valid_tools = Vec::new();
            for name in args[index]
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(tool) = ToolName::parse(name) {
                    valid_tools.push(tool);
                } else {
                    result.diagnostics.push(Diagnostic {
                        kind: DiagnosticKind::Warning,
                        message: format!(
                            "Unknown tool \"{name}\". Valid tools: {}",
                            ToolName::all()
                                .iter()
                                .copied()
                                .map(ToolName::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    });
                }
            }
            result.tools = Some(valid_tools);
        } else if arg == "--thinking" && index + 1 < args.len() {
            index += 1;
            let level = &args[index];
            if let Some(level) = parse_thinking_level(level) {
                result.thinking = Some(level);
            } else {
                result.diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::Warning,
                    message: format!(
                        "Invalid thinking level \"{level}\". Valid values: {}",
                        VALID_THINKING_LEVELS.join(", ")
                    ),
                });
            }
        } else if arg == "--transport" && index + 1 < args.len() {
            index += 1;
            let transport = &args[index];
            if let Some(transport) = parse_transport(transport) {
                result.transport = Some(transport);
            } else {
                result.diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::Warning,
                    message: format!(
                        "Invalid transport \"{transport}\". Valid values: {}",
                        VALID_TRANSPORTS.join(", ")
                    ),
                });
            }
        } else if arg == "--print" || arg == "-p" {
            result.print = true;
        } else if arg == "--export" && index + 1 < args.len() {
            index += 1;
            result.export = Some(args[index].clone());
        } else if arg == "--extension" || arg == "-e" {
            if index + 1 < args.len() {
                index += 1;
            }
            result.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::Error,
                message: String::from("Extensions are not supported in the Rust CLI rewrite"),
            });
        } else if arg == "--no-extensions" || arg == "-ne" {
            result.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::Error,
                message: String::from("Extensions are not supported in the Rust CLI rewrite"),
            });
        } else if arg == "--skill" && index + 1 < args.len() {
            index += 1;
            result
                .skills
                .get_or_insert_with(Vec::new)
                .push(args[index].clone());
        } else if arg == "--prompt-template" && index + 1 < args.len() {
            index += 1;
            result
                .prompt_templates
                .get_or_insert_with(Vec::new)
                .push(args[index].clone());
        } else if arg == "--theme" && index + 1 < args.len() {
            index += 1;
            result
                .themes
                .get_or_insert_with(Vec::new)
                .push(args[index].clone());
        } else if arg == "--no-skills" || arg == "-ns" {
            result.no_skills = true;
        } else if arg == "--no-prompt-templates" || arg == "-np" {
            result.no_prompt_templates = true;
        } else if arg == "--no-themes" {
            result.no_themes = true;
        } else if arg == "--list-models" {
            if index + 1 < args.len()
                && !args[index + 1].starts_with('-')
                && !args[index + 1].starts_with('@')
            {
                index += 1;
                result.list_models = Some(ListModels::Search(args[index].clone()));
            } else {
                result.list_models = Some(ListModels::All);
            }
        } else if arg == "--verbose" {
            result.verbose = true;
        } else if arg == "--offline" {
            result.offline = true;
        } else if let Some(file_arg) = arg.strip_prefix('@') {
            result.file_args.push(file_arg.to_string());
        } else if let Some(flag) = arg.strip_prefix("--") {
            if let Some((name, value)) = flag.split_once('=') {
                result.unknown_flags.insert(
                    name.to_string(),
                    UnknownFlagValue::String(value.to_string()),
                );
            } else {
                let next = args.get(index + 1);
                if let Some(next) = next
                    && !next.starts_with('-')
                    && !next.starts_with('@')
                {
                    index += 1;
                    result.unknown_flags.insert(
                        flag.to_string(),
                        UnknownFlagValue::String(args[index].clone()),
                    );
                } else {
                    result
                        .unknown_flags
                        .insert(flag.to_string(), UnknownFlagValue::Bool(true));
                }
            }
        } else if arg.starts_with('-') {
            result.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::Error,
                message: format!("Unknown option: {arg}"),
            });
        } else {
            result.messages.push(arg.clone());
        }

        index += 1;
    }

    result
}

pub fn resolve_app_mode(parsed: &Args, stdin_is_tty: bool) -> AppMode {
    match parsed.mode {
        Some(Mode::Rpc) => AppMode::Rpc,
        Some(Mode::Json) => AppMode::Json,
        _ if parsed.print || !stdin_is_tty => AppMode::Print,
        _ => AppMode::Interactive,
    }
}

pub fn to_print_output_mode(app_mode: AppMode) -> Option<PrintOutputMode> {
    match app_mode {
        AppMode::Print => Some(PrintOutputMode::Text),
        AppMode::Json => Some(PrintOutputMode::Json),
        AppMode::Interactive | AppMode::Rpc => None,
    }
}
