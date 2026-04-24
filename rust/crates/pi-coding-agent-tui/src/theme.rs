use crate::{KeyHintStyler, StartupHeaderStyler};
use parking_lot::Mutex;
use pi_agent::ThinkingLevel;
use pi_coding_agent_core::{ResourceDiagnostic, SourceInfo};
use pi_tui::MarkdownTheme;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

const CONFIG_DIR_NAME: &str = ".pi";
const ANSI_RESET_FG: &str = "\x1b[39m";
const ANSI_RESET_BG: &str = "\x1b[49m";
const ANSI_BOLD_ON: &str = "\x1b[1m";
const ANSI_BOLD_OFF: &str = "\x1b[22m";
const ANSI_ITALIC_ON: &str = "\x1b[3m";
const ANSI_ITALIC_OFF: &str = "\x1b[23m";
const ANSI_UNDERLINE_ON: &str = "\x1b[4m";
const ANSI_UNDERLINE_OFF: &str = "\x1b[24m";
const ANSI_INVERSE_ON: &str = "\x1b[7m";
const ANSI_INVERSE_OFF: &str = "\x1b[27m";
const ANSI_STRIKETHROUGH_ON: &str = "\x1b[9m";
const ANSI_STRIKETHROUGH_OFF: &str = "\x1b[29m";

const BG_COLOR_KEYS: &[&str] = &[
    "selectedBg",
    "userMessageBg",
    "customMessageBg",
    "toolPendingBg",
    "toolSuccessBg",
    "toolErrorBg",
];

const REQUIRED_COLOR_KEYS: &[&str] = &[
    "accent",
    "border",
    "borderAccent",
    "borderMuted",
    "success",
    "error",
    "warning",
    "muted",
    "dim",
    "text",
    "thinkingText",
    "selectedBg",
    "userMessageBg",
    "userMessageText",
    "customMessageBg",
    "customMessageText",
    "customMessageLabel",
    "toolPendingBg",
    "toolSuccessBg",
    "toolErrorBg",
    "toolTitle",
    "toolOutput",
    "mdHeading",
    "mdLink",
    "mdLinkUrl",
    "mdCode",
    "mdCodeBlock",
    "mdCodeBlockBorder",
    "mdQuote",
    "mdQuoteBorder",
    "mdHr",
    "mdListBullet",
    "toolDiffAdded",
    "toolDiffRemoved",
    "toolDiffContext",
    "syntaxComment",
    "syntaxKeyword",
    "syntaxFunction",
    "syntaxVariable",
    "syntaxString",
    "syntaxNumber",
    "syntaxType",
    "syntaxOperator",
    "syntaxPunctuation",
    "thinkingOff",
    "thinkingMinimal",
    "thinkingLow",
    "thinkingMedium",
    "thinkingHigh",
    "thinkingXhigh",
    "bashMode",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Truecolor,
    Ansi256,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadThemesResult {
    pub themes: Vec<Theme>,
    pub diagnostics: Vec<ResourceDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadThemesOptions {
    pub cwd: PathBuf,
    pub agent_dir: Option<PathBuf>,
    pub theme_paths: Vec<String>,
    pub include_defaults: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSelectionResult {
    pub success: bool,
    pub error: Option<String>,
    pub applied_theme_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeInfo {
    pub name: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeExportColors {
    pub page_bg: Option<String>,
    pub card_bg: Option<String>,
    pub info_bg: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Theme {
    inner: Arc<ThemeInner>,
}

#[derive(Debug, Clone)]
struct ThemeInner {
    name: String,
    source_path: Option<String>,
    source_info: Option<SourceInfo>,
    mode: ColorMode,
    resolved_colors: BTreeMap<String, ResolvedColor>,
    export_colors: ThemeExportColors,
    fg_codes: BTreeMap<String, String>,
    bg_codes: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct ThemeRegistry {
    current: Theme,
    current_name: String,
    registered: BTreeMap<String, Theme>,
}

struct ThemeWatcherHandle {
    stop_tx: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThemeJson {
    name: String,
    #[serde(default)]
    vars: BTreeMap<String, ColorValue>,
    colors: BTreeMap<String, ColorValue>,
    #[serde(default)]
    export: ThemeExportJson,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ThemeExportJson {
    page_bg: Option<ColorValue>,
    card_bg: Option<ColorValue>,
    info_bg: Option<ColorValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ColorValue {
    String(String),
    Integer(u16),
}

impl Theme {
    fn new(
        name: String,
        mode: ColorMode,
        source_path: Option<String>,
        source_info: Option<SourceInfo>,
        resolved_colors: BTreeMap<String, ResolvedColor>,
        export_colors: ThemeExportColors,
        fg_codes: BTreeMap<String, String>,
        bg_codes: BTreeMap<String, String>,
    ) -> Self {
        Self {
            inner: Arc::new(ThemeInner {
                name,
                source_path,
                source_info,
                mode,
                resolved_colors,
                export_colors,
                fg_codes,
                bg_codes,
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn source_path(&self) -> Option<&str> {
        self.inner.source_path.as_deref()
    }

    pub fn source_info(&self) -> Option<&SourceInfo> {
        self.inner.source_info.as_ref()
    }

    pub fn with_source_info(&self, source_info: Option<SourceInfo>) -> Self {
        Self::new(
            self.inner.name.clone(),
            self.inner.mode,
            self.inner.source_path.clone(),
            source_info,
            self.inner.resolved_colors.clone(),
            self.inner.export_colors.clone(),
            self.inner.fg_codes.clone(),
            self.inner.bg_codes.clone(),
        )
    }

    pub fn mode(&self) -> ColorMode {
        self.inner.mode
    }

    pub fn get_color_mode(&self) -> ColorMode {
        self.mode()
    }

    pub fn fg(&self, color: &str, text: impl AsRef<str>) -> String {
        let ansi = self.fg_code(color);
        format!("{ansi}{}{}", text.as_ref(), ANSI_RESET_FG)
    }

    pub fn bg(&self, color: &str, text: impl AsRef<str>) -> String {
        let ansi = self.bg_code(color);
        format!("{ansi}{}{}", text.as_ref(), ANSI_RESET_BG)
    }

    pub fn fg_code(&self, color: &str) -> &str {
        self.inner
            .fg_codes
            .get(color)
            .map(String::as_str)
            .unwrap_or(ANSI_RESET_FG)
    }

    pub fn get_fg_ansi(&self, color: &str) -> &str {
        self.fg_code(color)
    }

    pub fn bg_code(&self, color: &str) -> &str {
        self.inner
            .bg_codes
            .get(color)
            .map(String::as_str)
            .unwrap_or(ANSI_RESET_BG)
    }

    pub fn get_bg_ansi(&self, color: &str) -> &str {
        self.bg_code(color)
    }

    pub fn bold(&self, text: impl AsRef<str>) -> String {
        format!("{ANSI_BOLD_ON}{}{ANSI_BOLD_OFF}", text.as_ref())
    }

    pub fn italic(&self, text: impl AsRef<str>) -> String {
        format!("{ANSI_ITALIC_ON}{}{ANSI_ITALIC_OFF}", text.as_ref())
    }

    pub fn underline(&self, text: impl AsRef<str>) -> String {
        format!("{ANSI_UNDERLINE_ON}{}{ANSI_UNDERLINE_OFF}", text.as_ref())
    }

    pub fn inverse(&self, text: impl AsRef<str>) -> String {
        format!("{ANSI_INVERSE_ON}{}{ANSI_INVERSE_OFF}", text.as_ref())
    }

    pub fn strikethrough(&self, text: impl AsRef<str>) -> String {
        format!(
            "{ANSI_STRIKETHROUGH_ON}{}{ANSI_STRIKETHROUGH_OFF}",
            text.as_ref()
        )
    }

    pub fn get_thinking_border_color(
        &self,
        level: ThinkingLevel,
    ) -> impl Fn(&str) -> String + Send + Sync + 'static {
        let theme = self.clone();
        let color = match level {
            ThinkingLevel::Off => "thinkingOff",
            ThinkingLevel::Minimal => "thinkingMinimal",
            ThinkingLevel::Low => "thinkingLow",
            ThinkingLevel::Medium => "thinkingMedium",
            ThinkingLevel::High => "thinkingHigh",
            ThinkingLevel::XHigh => "thinkingXhigh",
        };
        move |line| theme.fg(color, line)
    }

    pub fn get_bash_mode_border_color(&self) -> impl Fn(&str) -> String + Send + Sync + 'static {
        let theme = self.clone();
        move |line| theme.fg("bashMode", line)
    }

    pub fn export_colors(&self) -> &ThemeExportColors {
        &self.inner.export_colors
    }

    fn resolved_colors(&self) -> &BTreeMap<String, ResolvedColor> {
        &self.inner.resolved_colors
    }

    pub fn background_fill(&self, color: &str) -> impl Fn(&str) -> String + Send + Sync + 'static {
        let ansi = self.bg_code(color).to_owned();
        move |line| format!("{ansi}{line}{ANSI_RESET_BG}")
    }
}

impl PartialEq for Theme {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
            && self.source_path() == other.source_path()
            && self.mode() == other.mode()
    }
}

impl Eq for Theme {}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThemedKeyHintStyler;

impl KeyHintStyler for ThemedKeyHintStyler {
    fn dim(&self, text: &str) -> String {
        current_theme().fg("dim", text)
    }

    fn muted(&self, text: &str) -> String {
        current_theme().fg("muted", text)
    }
}

impl StartupHeaderStyler for ThemedKeyHintStyler {
    fn accent_bold(&self, text: &str) -> String {
        let theme = current_theme();
        theme.fg("accent", theme.bold(text))
    }
}

pub fn markdown_theme() -> MarkdownTheme {
    MarkdownTheme::new()
        .with_heading(|text| {
            let theme = current_theme();
            theme.fg("mdHeading", theme.bold(text))
        })
        .with_link(|text| current_theme().fg("mdLink", text))
        .with_link_url(|text| current_theme().fg("mdLinkUrl", text))
        .with_code(|text| current_theme().fg("mdCode", text))
        .with_code_block(|text| current_theme().fg("mdCodeBlock", text))
        .with_code_block_border(|text| current_theme().fg("mdCodeBlockBorder", text))
        .with_quote(|text| current_theme().fg("mdQuote", text))
        .with_quote_border(|text| current_theme().fg("mdQuoteBorder", text))
        .with_hr(|text| current_theme().fg("mdHr", text))
        .with_list_bullet(|text| current_theme().fg("mdListBullet", text))
        .with_bold(|text| current_theme().bold(text))
        .with_italic(|text| current_theme().italic(text))
        .with_underline(|text| current_theme().underline(text))
        .with_strikethrough(|text| {
            let theme = current_theme();
            theme.fg("dim", theme.strikethrough(text))
        })
}

pub fn current_theme() -> Theme {
    theme_registry().lock().current.clone()
}

pub fn current_theme_name() -> String {
    theme_registry().lock().current_name.clone()
}

pub fn set_registered_themes(themes: Vec<Theme>) {
    let mut registry = theme_registry().lock();
    registry.registered = themes
        .into_iter()
        .map(|theme| (theme.name().to_owned(), theme))
        .collect();
}

pub fn get_available_themes() -> Vec<String> {
    let registry = theme_registry().lock();
    let mut themes = BTreeSet::new();
    themes.extend(built_in_themes().keys().cloned());
    themes.extend(registry.registered.keys().cloned());
    themes.into_iter().collect()
}

pub fn get_available_themes_with_paths() -> Vec<ThemeInfo> {
    let registry = theme_registry().lock();
    let mut themes = BTreeMap::<String, ThemeInfo>::new();

    for (name, theme) in built_in_themes() {
        themes.insert(
            name.clone(),
            ThemeInfo {
                name: name.clone(),
                path: theme.source_path().map(ToOwned::to_owned),
            },
        );
    }

    for theme in registry.registered.values() {
        themes.entry(theme.name().to_owned()).or_insert(ThemeInfo {
            name: theme.name().to_owned(),
            path: theme.source_path().map(ToOwned::to_owned),
        });
    }

    themes.into_values().collect()
}

pub fn get_theme_by_name(name: &str) -> Option<Theme> {
    let registry = theme_registry().lock();
    registry
        .registered
        .get(name)
        .cloned()
        .or_else(|| built_in_theme(name))
}

pub fn set_theme_instance(theme: Theme) {
    {
        let mut registry = theme_registry().lock();
        registry.current = theme;
        registry.current_name = String::from("<in-memory>");
    }
    stop_theme_watcher();
}

pub fn init_theme(theme_name: Option<&str>) -> ThemeSelectionResult {
    let requested = theme_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(detect_default_theme_name);
    set_theme(&requested)
}

pub fn set_theme(theme_name: &str) -> ThemeSelectionResult {
    let fallback = built_in_theme("dark").expect("dark theme must exist");
    let selected_theme = {
        let mut registry = theme_registry().lock();

        if let Some(theme) = registry
            .registered
            .get(theme_name)
            .cloned()
            .or_else(|| built_in_theme(theme_name))
        {
            registry.current = theme.clone();
            registry.current_name = theme_name.to_owned();
            Some(theme)
        } else {
            registry.current = fallback;
            registry.current_name = String::from("dark");
            None
        }
    };

    if let Some(theme) = selected_theme {
        restart_theme_watcher(theme_name, &theme);
        return ThemeSelectionResult {
            success: true,
            error: None,
            applied_theme_name: theme_name.to_owned(),
        };
    }

    stop_theme_watcher();
    ThemeSelectionResult {
        success: false,
        error: Some(format!("Theme not found: {theme_name}")),
        applied_theme_name: String::from("dark"),
    }
}

pub fn load_themes(options: LoadThemesOptions) -> LoadThemesResult {
    let LoadThemesOptions {
        cwd,
        agent_dir,
        theme_paths,
        include_defaults,
    } = options;
    let mut themes_by_name = BTreeMap::<String, Theme>::new();
    let mut diagnostics = Vec::new();

    if include_defaults {
        if let Some(agent_dir) = agent_dir.as_deref() {
            let dir = agent_dir.join("themes");
            load_themes_from_dir(
                &dir,
                &mut themes_by_name,
                &mut diagnostics,
                Some(("user", dir.clone())),
            );
        }
        let dir = cwd.join(CONFIG_DIR_NAME).join("themes");
        load_themes_from_dir(
            &dir,
            &mut themes_by_name,
            &mut diagnostics,
            Some(("project", dir.clone())),
        );
    }

    for theme_path in theme_paths {
        let resolved = resolve_from_cwd(&cwd, &theme_path);
        if !resolved.exists() {
            diagnostics.push(ResourceDiagnostic::new(
                "Theme path does not exist",
                Some(resolved.display().to_string()),
            ));
            continue;
        }

        if resolved.is_dir() {
            load_themes_from_dir(&resolved, &mut themes_by_name, &mut diagnostics, None);
            continue;
        }

        if resolved
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("json")
        {
            diagnostics.push(ResourceDiagnostic::new(
                "Theme path is not a json file",
                Some(resolved.display().to_string()),
            ));
            continue;
        }

        match load_theme_file(&resolved, source_info_for_path(&resolved, None)) {
            Ok(theme) => add_theme(&mut themes_by_name, &mut diagnostics, theme),
            Err(error) => diagnostics.push(ResourceDiagnostic::new(
                error,
                Some(resolved.display().to_string()),
            )),
        }
    }

    LoadThemesResult {
        themes: themes_by_name.into_values().collect(),
        diagnostics,
    }
}

pub fn load_theme_from_path(path: impl AsRef<Path>) -> Result<Theme, String> {
    load_theme_from_path_with_mode(path, detect_color_mode())
}

pub fn load_theme_from_path_with_mode(
    path: impl AsRef<Path>,
    mode: ColorMode,
) -> Result<Theme, String> {
    let path = path.as_ref();
    load_theme_file_with_mode(path, source_info_for_path(path, None), mode)
}

fn theme_registry() -> &'static Mutex<ThemeRegistry> {
    static REGISTRY: OnceLock<Mutex<ThemeRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let dark = built_in_theme("dark").expect("dark theme must exist");
        Mutex::new(ThemeRegistry {
            current: dark.clone(),
            current_name: String::from("dark"),
            registered: BTreeMap::new(),
        })
    })
}

fn theme_watcher_handle() -> &'static Mutex<Option<ThemeWatcherHandle>> {
    static WATCHER: OnceLock<Mutex<Option<ThemeWatcherHandle>>> = OnceLock::new();
    WATCHER.get_or_init(|| Mutex::new(None))
}

fn stop_theme_watcher() {
    let watcher = theme_watcher_handle().lock().take();
    if let Some(watcher) = watcher {
        let _ = watcher.stop_tx.send(());
        let _ = watcher.thread.join();
    }
}

fn restart_theme_watcher(theme_name: &str, theme: &Theme) {
    stop_theme_watcher();

    if matches!(theme_name, "dark" | "light") {
        return;
    }

    let Some(source_path) = theme.source_path().map(ToOwned::to_owned) else {
        return;
    };

    let watched_name = theme_name.to_owned();
    let source_info = theme.source_info().cloned();
    let mode = theme.mode();
    let mut last_content = fs::read_to_string(&source_path).ok();
    let (stop_tx, stop_rx) = mpsc::channel();

    let thread = thread::spawn(move || {
        loop {
            match stop_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            let Ok(content) = fs::read_to_string(&source_path) else {
                continue;
            };
            if last_content.as_ref() == Some(&content) {
                continue;
            }
            last_content = Some(content.clone());

            let Ok(reloaded) = parse_theme_content(
                &source_path,
                &content,
                Some(source_path.clone()),
                source_info.clone(),
                mode,
            ) else {
                continue;
            };

            let mut registry = theme_registry().lock();
            if registry.current_name != watched_name {
                continue;
            }
            registry
                .registered
                .insert(watched_name.clone(), reloaded.clone());
            registry.current = reloaded;
        }
    });

    *theme_watcher_handle().lock() = Some(ThemeWatcherHandle { stop_tx, thread });
}

fn built_in_themes() -> &'static BTreeMap<String, Theme> {
    static BUILTINS: OnceLock<BTreeMap<String, Theme>> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        let mut builtins = BTreeMap::new();
        builtins.insert(
            String::from("dark"),
            load_builtin_theme("dark", include_str!("theme/dark.json")),
        );
        builtins.insert(
            String::from("light"),
            load_builtin_theme("light", include_str!("theme/light.json")),
        );
        builtins
    })
}

fn built_in_theme(name: &str) -> Option<Theme> {
    built_in_themes().get(name).cloned()
}

fn load_builtin_theme(name: &str, content: &str) -> Theme {
    parse_theme_content(name, content, None, None, detect_color_mode())
        .expect("builtin theme must be valid")
}

fn load_themes_from_dir(
    dir: &Path,
    themes_by_name: &mut BTreeMap<String, Theme>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    default_scope: Option<(&str, PathBuf)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let is_file = if entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            true
        } else if entry
            .file_type()
            .map(|file_type| file_type.is_symlink())
            .unwrap_or(false)
        {
            fs::metadata(&path)
                .map(|metadata| metadata.is_file())
                .unwrap_or(false)
        } else {
            false
        };
        if !is_file {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }

        let source_info = source_info_for_path(&path, default_scope.clone());
        match load_theme_file(&path, source_info) {
            Ok(theme) => add_theme(themes_by_name, diagnostics, theme),
            Err(error) => diagnostics.push(ResourceDiagnostic::new(
                error,
                Some(path.display().to_string()),
            )),
        }
    }
}

fn add_theme(
    themes_by_name: &mut BTreeMap<String, Theme>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    theme: Theme,
) {
    if let Some(existing) = themes_by_name.get(theme.name()) {
        diagnostics.push(ResourceDiagnostic::new(
            format!(
                "Theme name collision for {} (keeping {})",
                theme.name(),
                existing.source_path().unwrap_or("<builtin>")
            ),
            theme.source_path().map(ToOwned::to_owned),
        ));
        return;
    }

    themes_by_name.insert(theme.name().to_owned(), theme);
}

fn load_theme_file(path: &Path, source_info: SourceInfo) -> Result<Theme, String> {
    load_theme_file_with_mode(path, source_info, detect_color_mode())
}

fn load_theme_file_with_mode(
    path: &Path,
    source_info: SourceInfo,
    mode: ColorMode,
) -> Result<Theme, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_theme_content(
        &path.display().to_string(),
        &content,
        Some(path.display().to_string()),
        Some(source_info),
        mode,
    )
}

fn parse_theme_content(
    label: &str,
    content: &str,
    source_path: Option<String>,
    source_info: Option<SourceInfo>,
    mode: ColorMode,
) -> Result<Theme, String> {
    let parsed: ThemeJson = serde_json::from_str(content)
        .map_err(|error| format!("Failed to parse theme {label}: {error}"))?;
    validate_theme_json(label, &parsed)?;
    create_theme(parsed, mode, source_path, source_info)
}

fn create_theme(
    parsed: ThemeJson,
    mode: ColorMode,
    source_path: Option<String>,
    source_info: Option<SourceInfo>,
) -> Result<Theme, String> {
    let ThemeJson {
        name,
        vars,
        colors,
        export,
    } = parsed;
    let mut resolved_colors = BTreeMap::new();
    let mut fg_codes = BTreeMap::new();
    let mut bg_codes = BTreeMap::new();

    for (token, value) in &colors {
        let resolved = resolve_color_value(value, &vars, &mut Vec::new())?;
        let ansi = if BG_COLOR_KEYS.contains(&token.as_str()) {
            bg_ansi(&resolved, mode)?
        } else {
            fg_ansi(&resolved, mode)?
        };

        if BG_COLOR_KEYS.contains(&token.as_str()) {
            bg_codes.insert(token.clone(), ansi);
        } else {
            fg_codes.insert(token.clone(), ansi);
        }
        resolved_colors.insert(token.clone(), resolved);
    }

    let page_bg = resolve_optional_color(export.page_bg.as_ref(), &vars)?;
    let card_bg = resolve_optional_color(export.card_bg.as_ref(), &vars)?;
    let info_bg = resolve_optional_color(export.info_bg.as_ref(), &vars)?;
    let export_colors = ThemeExportColors {
        page_bg: page_bg
            .as_ref()
            .and_then(|color| resolved_color_to_css(color, None)),
        card_bg: card_bg
            .as_ref()
            .and_then(|color| resolved_color_to_css(color, None)),
        info_bg: info_bg
            .as_ref()
            .and_then(|color| resolved_color_to_css(color, None)),
    };

    Ok(Theme::new(
        name,
        mode,
        source_path,
        source_info,
        resolved_colors,
        export_colors,
        fg_codes,
        bg_codes,
    ))
}

fn validate_theme_json(label: &str, theme: &ThemeJson) -> Result<(), String> {
    let missing = REQUIRED_COLOR_KEYS
        .iter()
        .filter(|key| !theme.colors.contains_key(**key))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    let mut message = format!("Invalid theme \"{label}\":\n\nMissing required color tokens:\n");
    for key in missing {
        message.push_str(&format!("  - {key}\n"));
    }
    message.push_str(
        "\nPlease add these colors to your theme's \"colors\" object. See dark.json and light.json for reference values.",
    );
    Err(message)
}

fn resolve_color_value(
    value: &ColorValue,
    vars: &BTreeMap<String, ColorValue>,
    visited: &mut Vec<String>,
) -> Result<ResolvedColor, String> {
    match value {
        ColorValue::Integer(value) => {
            if *value > 255 {
                return Err(format!("Invalid 256-color value: {value}"));
            }
            Ok(ResolvedColor::Ansi256(*value as u8))
        }
        ColorValue::String(value) if value.is_empty() => Ok(ResolvedColor::Default),
        ColorValue::String(value) if value.starts_with('#') => {
            validate_hex(value)?;
            Ok(ResolvedColor::Hex(value.clone()))
        }
        ColorValue::String(value) => {
            if visited.iter().any(|entry| entry == value) {
                return Err(format!("Circular variable reference detected: {value}"));
            }
            let Some(next) = vars.get(value) else {
                return Err(format!("Variable reference not found: {value}"));
            };
            visited.push(value.clone());
            let resolved = resolve_color_value(next, vars, visited);
            visited.pop();
            resolved
        }
    }
}

fn validate_hex(value: &str) -> Result<(), String> {
    let hex = value.trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(format!("Invalid hex color: {value}"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedColor {
    Default,
    Hex(String),
    Ansi256(u8),
}

fn fg_ansi(color: &ResolvedColor, mode: ColorMode) -> Result<String, String> {
    match color {
        ResolvedColor::Default => Ok(String::from(ANSI_RESET_FG)),
        ResolvedColor::Ansi256(index) => Ok(format!("\x1b[38;5;{index}m")),
        ResolvedColor::Hex(hex) => match mode {
            ColorMode::Truecolor => {
                let (r, g, b) = hex_to_rgb(hex)?;
                Ok(format!("\x1b[38;2;{r};{g};{b}m"))
            }
            ColorMode::Ansi256 => {
                let (r, g, b) = hex_to_rgb(hex)?;
                Ok(format!("\x1b[38;5;{}m", rgb_to_256(r, g, b)))
            }
        },
    }
}

fn bg_ansi(color: &ResolvedColor, mode: ColorMode) -> Result<String, String> {
    match color {
        ResolvedColor::Default => Ok(String::from(ANSI_RESET_BG)),
        ResolvedColor::Ansi256(index) => Ok(format!("\x1b[48;5;{index}m")),
        ResolvedColor::Hex(hex) => match mode {
            ColorMode::Truecolor => {
                let (r, g, b) = hex_to_rgb(hex)?;
                Ok(format!("\x1b[48;2;{r};{g};{b}m"))
            }
            ColorMode::Ansi256 => {
                let (r, g, b) = hex_to_rgb(hex)?;
                Ok(format!("\x1b[48;5;{}m", rgb_to_256(r, g, b)))
            }
        },
    }
}

fn detect_color_mode() -> ColorMode {
    match env::var("COLORTERM").ok().as_deref() {
        Some("truecolor") | Some("24bit") => return ColorMode::Truecolor,
        _ => {}
    }
    if env::var_os("WT_SESSION").is_some() {
        return ColorMode::Truecolor;
    }
    if matches!(
        env::var("TERM_PROGRAM").ok().as_deref(),
        Some("Apple_Terminal")
    ) {
        return ColorMode::Ansi256;
    }
    let term = env::var("TERM").unwrap_or_default();
    if term.is_empty() || term == "dumb" || term == "linux" {
        return ColorMode::Ansi256;
    }
    if term == "screen" || term.starts_with("screen-") || term.starts_with("screen.") {
        return ColorMode::Ansi256;
    }
    ColorMode::Truecolor
}

fn detect_default_theme_name() -> String {
    let colorfgbg = env::var("COLORFGBG").unwrap_or_default();
    if !colorfgbg.is_empty() {
        let parts = colorfgbg.split(';').collect::<Vec<_>>();
        if parts.len() >= 2
            && let Ok(background) = parts[1].parse::<u16>()
        {
            return if background < 8 {
                String::from("dark")
            } else {
                String::from("light")
            };
        }
    }
    String::from("dark")
}

fn resolve_theme_for_lookup(theme_name: Option<&str>) -> Result<(Theme, String), String> {
    match theme_name {
        Some(name) => get_theme_by_name(name)
            .map(|theme| (theme, name.to_owned()))
            .ok_or_else(|| format!("Theme not found: {name}")),
        None => Ok((current_theme(), current_theme_name())),
    }
}

pub fn get_resolved_theme_colors(
    theme_name: Option<&str>,
) -> Result<BTreeMap<String, String>, String> {
    let (theme, effective_name) = resolve_theme_for_lookup(theme_name)?;
    let default_text = if effective_name == "light" {
        "#000000"
    } else {
        "#e5e5e7"
    };

    Ok(theme
        .resolved_colors()
        .iter()
        .filter_map(|(key, color)| {
            resolved_color_to_css(color, Some(default_text)).map(|value| (key.clone(), value))
        })
        .collect())
}

pub fn is_light_theme(theme_name: Option<&str>) -> bool {
    theme_name == Some("light")
}

pub fn get_theme_export_colors(theme_name: Option<&str>) -> Result<ThemeExportColors, String> {
    let (theme, _) = resolve_theme_for_lookup(theme_name)?;
    Ok(theme.export_colors().clone())
}

fn resolve_optional_color(
    value: Option<&ColorValue>,
    vars: &BTreeMap<String, ColorValue>,
) -> Result<Option<ResolvedColor>, String> {
    value
        .map(|color| resolve_color_value(color, vars, &mut Vec::new()))
        .transpose()
}

fn resolved_color_to_css(color: &ResolvedColor, default_text: Option<&str>) -> Option<String> {
    match color {
        ResolvedColor::Default => default_text.map(ToOwned::to_owned),
        ResolvedColor::Hex(hex) => Some(hex.clone()),
        ResolvedColor::Ansi256(index) => Some(ansi_256_to_hex(*index)),
    }
}

fn ansi_256_to_hex(index: u8) -> String {
    const BASIC_COLORS: [&str; 16] = [
        "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
        "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    ];

    if index < 16 {
        return BASIC_COLORS[index as usize].to_owned();
    }

    if index < 232 {
        let cube_index = index - 16;
        let r = cube_index / 36;
        let g = (cube_index % 36) / 6;
        let b = cube_index % 6;
        let to_hex = |value: u8| {
            let channel = if value == 0 { 0 } else { 55 + value * 40 };
            format!("{channel:02x}")
        };
        return format!("#{}{}{}", to_hex(r), to_hex(g), to_hex(b));
    }

    let gray = 8 + (index - 232) * 10;
    format!("#{gray:02x}{gray:02x}{gray:02x}")
}

fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), String> {
    validate_hex(hex)?;
    let cleaned = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&cleaned[0..2], 16).map_err(|error| error.to_string())?;
    let g = u8::from_str_radix(&cleaned[2..4], 16).map_err(|error| error.to_string())?;
    let b = u8::from_str_radix(&cleaned[4..6], 16).map_err(|error| error.to_string())?;
    Ok((r, g, b))
}

const CUBE_VALUES: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let r_index = closest_cube_index(r);
    let g_index = closest_cube_index(g);
    let b_index = closest_cube_index(b);
    let cube_r = CUBE_VALUES[r_index as usize];
    let cube_g = CUBE_VALUES[g_index as usize];
    let cube_b = CUBE_VALUES[b_index as usize];
    let cube_index = 16 + 36 * r_index + 6 * g_index + b_index;
    let cube_distance = color_distance(r, g, b, cube_r, cube_g, cube_b);

    let gray = ((299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) + 500) / 1000) as u8;
    let gray_index = closest_gray_index(gray);
    let gray_value = 8 + gray_index * 10;
    let gray_distance = color_distance(r, g, b, gray_value, gray_value, gray_value);

    let spread = r.max(g).max(b) - r.min(g).min(b);
    if spread < 10 && gray_distance < cube_distance {
        return 232 + gray_index;
    }

    cube_index
}

fn closest_cube_index(value: u8) -> u8 {
    let mut min_distance = u16::MAX;
    let mut min_index = 0u8;
    for (index, cube_value) in CUBE_VALUES.iter().copied().enumerate() {
        let distance = value.abs_diff(cube_value) as u16;
        if distance < min_distance {
            min_distance = distance;
            min_index = index as u8;
        }
    }
    min_index
}

fn closest_gray_index(value: u8) -> u8 {
    let mut min_distance = u16::MAX;
    let mut min_index = 0u8;
    for index in 0..24u8 {
        let gray = 8 + index * 10;
        let distance = value.abs_diff(gray) as u16;
        if distance < min_distance {
            min_distance = distance;
            min_index = index;
        }
    }
    min_index
}

fn color_distance(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> u32 {
    let dr = i32::from(r1) - i32::from(r2);
    let dg = i32::from(g1) - i32::from(g2);
    let db = i32::from(b1) - i32::from(b2);
    (dr * dr) as u32 * 299 + (dg * dg) as u32 * 587 + (db * db) as u32 * 114
}

fn normalize_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed == "~" {
        return home_dir();
    }
    if let Some(path) = trimmed.strip_prefix("~/") {
        return Path::new(&home_dir()).join(path).display().to_string();
    }
    trimmed.to_owned()
}

fn home_dir() -> String {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| String::from("~"))
}

fn resolve_from_cwd(cwd: &Path, input: &str) -> PathBuf {
    let normalized = normalize_path(input);
    let path = Path::new(&normalized);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn source_info_for_path(path: &Path, default_scope: Option<(&str, PathBuf)>) -> SourceInfo {
    if let Some((scope, base_dir)) = default_scope {
        return SourceInfo::local(
            path.display().to_string(),
            scope,
            base_dir.display().to_string(),
        );
    }

    let base_dir = path
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_else(|| String::from("."));
    SourceInfo::temporary(path.display().to_string(), base_dir)
}
