use crate::{KeyHintStyler, StartupHeaderStyler};
use pi_coding_agent_core::{ResourceDiagnostic, SourceInfo};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
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
    fg_codes: BTreeMap<String, String>,
    bg_codes: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct ThemeRegistry {
    current: Theme,
    current_name: String,
    registered: BTreeMap<String, Theme>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThemeJson {
    name: String,
    #[serde(default)]
    vars: BTreeMap<String, ColorValue>,
    colors: BTreeMap<String, ColorValue>,
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
        fg_codes: BTreeMap<String, String>,
        bg_codes: BTreeMap<String, String>,
    ) -> Self {
        Self {
            inner: Arc::new(ThemeInner {
                name,
                source_path,
                source_info,
                mode,
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

    pub fn mode(&self) -> ColorMode {
        self.inner.mode
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

    pub fn bg_code(&self, color: &str) -> &str {
        self.inner
            .bg_codes
            .get(color)
            .map(String::as_str)
            .unwrap_or(ANSI_RESET_BG)
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

    pub fn strikethrough(&self, text: impl AsRef<str>) -> String {
        format!(
            "{ANSI_STRIKETHROUGH_ON}{}{ANSI_STRIKETHROUGH_OFF}",
            text.as_ref()
        )
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

    fn bold(&self, text: &str) -> String {
        current_theme().bold(text)
    }

    fn border(&self, text: &str) -> String {
        current_theme().fg("borderMuted", text)
    }

    fn heading(&self, text: &str) -> String {
        let theme = current_theme();
        theme.fg("mdHeading", theme.bold(text))
    }

    fn link(&self, text: &str) -> String {
        current_theme().fg("mdLink", text)
    }

    fn link_url(&self, text: &str) -> String {
        current_theme().fg("mdLinkUrl", text)
    }

    fn code(&self, text: &str) -> String {
        current_theme().fg("mdCode", text)
    }

    fn code_block(&self, text: &str) -> String {
        current_theme().fg("mdCodeBlock", text)
    }

    fn code_block_border(&self, text: &str) -> String {
        current_theme().fg("mdCodeBlockBorder", text)
    }

    fn list_bullet(&self, text: &str) -> String {
        current_theme().fg("mdListBullet", text)
    }

    fn strikethrough(&self, text: &str) -> String {
        let theme = current_theme();
        theme.fg("dim", theme.strikethrough(text))
    }
}

pub fn current_theme() -> Theme {
    theme_registry()
        .lock()
        .expect("theme registry mutex poisoned")
        .current
        .clone()
}

pub fn current_theme_name() -> String {
    theme_registry()
        .lock()
        .expect("theme registry mutex poisoned")
        .current_name
        .clone()
}

pub fn set_registered_themes(themes: Vec<Theme>) {
    let mut registry = theme_registry()
        .lock()
        .expect("theme registry mutex poisoned");
    registry.registered = themes
        .into_iter()
        .map(|theme| (theme.name().to_owned(), theme))
        .collect();
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
    let mut registry = theme_registry()
        .lock()
        .expect("theme registry mutex poisoned");

    if let Some(theme) = registry
        .registered
        .get(theme_name)
        .cloned()
        .or_else(|| built_in_theme(theme_name))
    {
        registry.current = theme;
        registry.current_name = theme_name.to_owned();
        return ThemeSelectionResult {
            success: true,
            error: None,
            applied_theme_name: theme_name.to_owned(),
        };
    }

    registry.current = fallback;
    registry.current_name = String::from("dark");
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
    let path = path.as_ref();
    load_theme_file(path, source_info_for_path(path, None))
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
    parse_theme_content(name, content, None, None).expect("builtin theme must be valid")
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
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_theme_content(
        &path.display().to_string(),
        &content,
        Some(path.display().to_string()),
        Some(source_info),
    )
}

fn parse_theme_content(
    label: &str,
    content: &str,
    source_path: Option<String>,
    source_info: Option<SourceInfo>,
) -> Result<Theme, String> {
    let parsed: ThemeJson = serde_json::from_str(content)
        .map_err(|error| format!("Failed to parse theme {label}: {error}"))?;
    validate_theme_json(label, &parsed)?;
    create_theme(parsed, detect_color_mode(), source_path, source_info)
}

fn create_theme(
    parsed: ThemeJson,
    mode: ColorMode,
    source_path: Option<String>,
    source_info: Option<SourceInfo>,
) -> Result<Theme, String> {
    let mut fg_codes = BTreeMap::new();
    let mut bg_codes = BTreeMap::new();

    for (name, value) in &parsed.colors {
        let resolved = resolve_color_value(value, &parsed.vars, &mut Vec::new())?;
        let ansi = if BG_COLOR_KEYS.contains(&name.as_str()) {
            bg_ansi(&resolved, mode)?
        } else {
            fg_ansi(&resolved, mode)?
        };

        if BG_COLOR_KEYS.contains(&name.as_str()) {
            bg_codes.insert(name.clone(), ansi);
        } else {
            fg_codes.insert(name.clone(), ansi);
        }
    }

    Ok(Theme::new(
        parsed.name,
        mode,
        source_path,
        source_info,
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
    16 + 36 * r_index + 6 * g_index + b_index
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
