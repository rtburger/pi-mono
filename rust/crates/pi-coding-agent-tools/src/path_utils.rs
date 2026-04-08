use std::{
    env,
    path::{Path, PathBuf},
};

const NARROW_NO_BREAK_SPACE: char = '\u{202F}';

pub fn resolve_to_cwd(file_path: &str, cwd: &Path) -> PathBuf {
    let expanded = expand_path(file_path);
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

pub fn resolve_read_path(file_path: &str, cwd: &Path) -> PathBuf {
    let resolved = resolve_to_cwd(file_path, cwd);
    if resolved.exists() {
        return resolved;
    }

    let am_pm_variant = try_macos_screenshot_path(&resolved);
    if am_pm_variant.exists() {
        return am_pm_variant;
    }

    let curly_quote_variant = try_curly_quote_variant(&resolved);
    if curly_quote_variant.exists() {
        return curly_quote_variant;
    }

    resolved
}

fn expand_path(file_path: &str) -> String {
    let normalized = normalize_unicode_spaces(normalize_at_prefix(file_path));
    match normalized.as_str() {
        "~" => home_dir().unwrap_or(normalized),
        _ if normalized.starts_with("~/") => match home_dir() {
            Some(home) => format!("{home}{}", &normalized[1..]),
            None => normalized,
        },
        _ => normalized,
    }
}

fn normalize_at_prefix(file_path: &str) -> &str {
    file_path.strip_prefix('@').unwrap_or(file_path)
}

fn normalize_unicode_spaces(file_path: &str) -> String {
    file_path
        .chars()
        .map(|ch| {
            if is_unicode_space_variant(ch) {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn is_unicode_space_variant(ch: char) -> bool {
    matches!(
        ch,
        '\u{00A0}'
            | '\u{2000}'
            | '\u{2001}'
            | '\u{2002}'
            | '\u{2003}'
            | '\u{2004}'
            | '\u{2005}'
            | '\u{2006}'
            | '\u{2007}'
            | '\u{2008}'
            | '\u{2009}'
            | '\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

fn try_macos_screenshot_path(path: &Path) -> PathBuf {
    PathBuf::from(
        path.to_string_lossy()
            .replace(" AM.", &format!("{NARROW_NO_BREAK_SPACE}AM."))
            .replace(" PM.", &format!("{NARROW_NO_BREAK_SPACE}PM.")),
    )
}

fn try_curly_quote_variant(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('\'', "\u{2019}"))
}

fn home_dir() -> Option<String> {
    env::var("HOME").ok()
}
