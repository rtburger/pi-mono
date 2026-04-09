use crate::keybindings::KeyId;
use regex::Regex;
use std::sync::{
    LazyLock,
    atomic::{AtomicBool, Ordering},
};

const MODIFIER_SHIFT: u16 = 1;
const MODIFIER_ALT: u16 = 2;
const MODIFIER_CTRL: u16 = 4;
const LOCK_MASK: u16 = 64 + 128;
const SUPPORTED_MODIFIER_MASK: u16 = MODIFIER_SHIFT | MODIFIER_CTRL | MODIFIER_ALT;

const CODEPOINT_ESCAPE: i32 = 27;
const CODEPOINT_TAB: i32 = 9;
const CODEPOINT_ENTER: i32 = 13;
const CODEPOINT_SPACE: i32 = 32;
const CODEPOINT_BACKSPACE: i32 = 127;
const CODEPOINT_KP_ENTER: i32 = 57414;

const CODEPOINT_ARROW_UP: i32 = -1;
const CODEPOINT_ARROW_DOWN: i32 = -2;
const CODEPOINT_ARROW_RIGHT: i32 = -3;
const CODEPOINT_ARROW_LEFT: i32 = -4;

const CODEPOINT_DELETE: i32 = -10;
const CODEPOINT_INSERT: i32 = -11;
const CODEPOINT_PAGE_UP: i32 = -12;
const CODEPOINT_PAGE_DOWN: i32 = -13;
const CODEPOINT_HOME: i32 = -14;
const CODEPOINT_END: i32 = -15;

static KITTY_PROTOCOL_ACTIVE: AtomicBool = AtomicBool::new(false);

static CSI_U_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\x1b\[(\d+)(?::(\d*))?(?::(\d+))?(?:;(\d+))?(?::(\d+))?u$")
        .expect("valid kitty CSI-u regex")
});
static KITTY_ARROW_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\x1b\[1;(\d+)(?::(\d+))?([ABCD])$").expect("valid kitty arrow regex")
});
static KITTY_FUNCTION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\x1b\[(\d+)(?:;(\d+))?(?::(\d+))?~$").expect("valid kitty function regex")
});
static KITTY_HOME_END_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\x1b\[1;(\d+)(?::(\d+))?([HF])$").expect("valid kitty home/end regex")
});
static MODIFY_OTHER_KEYS_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\x1b\[27;(\d+);(\d+)~$").expect("valid modifyOtherKeys regex"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct ParsedKittySequence {
    codepoint: i32,
    base_layout_key: Option<i32>,
    modifier: u16,
}

#[derive(Debug, Clone, Copy)]
struct ParsedModifyOtherKeysSequence {
    codepoint: i32,
    modifier: u16,
}

pub fn set_kitty_protocol_active(active: bool) {
    KITTY_PROTOCOL_ACTIVE.store(active, Ordering::SeqCst);
}

pub fn is_kitty_protocol_active() -> bool {
    KITTY_PROTOCOL_ACTIVE.load(Ordering::SeqCst)
}

pub fn is_key_release(data: &str) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }

    data.contains(":3u")
        || data.contains(":3~")
        || data.contains(":3A")
        || data.contains(":3B")
        || data.contains(":3C")
        || data.contains(":3D")
        || data.contains(":3H")
        || data.contains(":3F")
}

pub fn is_key_repeat(data: &str) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }

    data.contains(":2u")
        || data.contains(":2~")
        || data.contains(":2A")
        || data.contains(":2B")
        || data.contains(":2C")
        || data.contains(":2D")
        || data.contains(":2H")
        || data.contains(":2F")
}

pub fn matches_key(data: &str, key_id: impl AsRef<str>) -> bool {
    let Some(parsed_key_id) = parse_key_id(key_id.as_ref()) else {
        return false;
    };

    let mut modifier = 0u16;
    if parsed_key_id.shift {
        modifier |= MODIFIER_SHIFT;
    }
    if parsed_key_id.alt {
        modifier |= MODIFIER_ALT;
    }
    if parsed_key_id.ctrl {
        modifier |= MODIFIER_CTRL;
    }

    match parsed_key_id.key.as_str() {
        "escape" | "esc" => {
            if modifier != 0 {
                return false;
            }
            data == "\x1b"
                || matches_kitty_sequence(data, CODEPOINT_ESCAPE, 0)
                || matches_modify_other_keys(data, CODEPOINT_ESCAPE, 0)
        }
        "space" => {
            if !is_kitty_protocol_active() {
                if parsed_key_id.ctrl
                    && !parsed_key_id.alt
                    && !parsed_key_id.shift
                    && data == "\x00"
                {
                    return true;
                }
                if parsed_key_id.alt
                    && !parsed_key_id.ctrl
                    && !parsed_key_id.shift
                    && data == "\x1b "
                {
                    return true;
                }
            }

            if modifier == 0 {
                data == " "
                    || matches_kitty_sequence(data, CODEPOINT_SPACE, 0)
                    || matches_modify_other_keys(data, CODEPOINT_SPACE, 0)
            } else {
                matches_kitty_sequence(data, CODEPOINT_SPACE, modifier)
                    || matches_modify_other_keys(data, CODEPOINT_SPACE, modifier)
            }
        }
        "tab" => {
            if parsed_key_id.shift && !parsed_key_id.ctrl && !parsed_key_id.alt {
                data == "\x1b[Z"
                    || matches_kitty_sequence(data, CODEPOINT_TAB, MODIFIER_SHIFT)
                    || matches_modify_other_keys(data, CODEPOINT_TAB, MODIFIER_SHIFT)
            } else if modifier == 0 {
                data == "\t" || matches_kitty_sequence(data, CODEPOINT_TAB, 0)
            } else {
                matches_kitty_sequence(data, CODEPOINT_TAB, modifier)
                    || matches_modify_other_keys(data, CODEPOINT_TAB, modifier)
            }
        }
        "enter" | "return" => {
            if parsed_key_id.shift && !parsed_key_id.ctrl && !parsed_key_id.alt {
                if matches_kitty_sequence(data, CODEPOINT_ENTER, MODIFIER_SHIFT)
                    || matches_kitty_sequence(data, CODEPOINT_KP_ENTER, MODIFIER_SHIFT)
                {
                    return true;
                }
                if matches_modify_other_keys(data, CODEPOINT_ENTER, MODIFIER_SHIFT) {
                    return true;
                }
                if is_kitty_protocol_active() {
                    return data == "\x1b\r" || data == "\n";
                }
                false
            } else if parsed_key_id.alt && !parsed_key_id.ctrl && !parsed_key_id.shift {
                if matches_kitty_sequence(data, CODEPOINT_ENTER, MODIFIER_ALT)
                    || matches_kitty_sequence(data, CODEPOINT_KP_ENTER, MODIFIER_ALT)
                {
                    return true;
                }
                if matches_modify_other_keys(data, CODEPOINT_ENTER, MODIFIER_ALT) {
                    return true;
                }
                if !is_kitty_protocol_active() {
                    return data == "\x1b\r";
                }
                false
            } else if modifier == 0 {
                data == "\r"
                    || (!is_kitty_protocol_active() && data == "\n")
                    || data == "\x1bOM"
                    || matches_kitty_sequence(data, CODEPOINT_ENTER, 0)
                    || matches_kitty_sequence(data, CODEPOINT_KP_ENTER, 0)
            } else {
                matches_kitty_sequence(data, CODEPOINT_ENTER, modifier)
                    || matches_kitty_sequence(data, CODEPOINT_KP_ENTER, modifier)
                    || matches_modify_other_keys(data, CODEPOINT_ENTER, modifier)
            }
        }
        "backspace" => {
            if parsed_key_id.alt && !parsed_key_id.ctrl && !parsed_key_id.shift {
                if data == "\x1b\x7f" || data == "\x1b\x08" {
                    return true;
                }
                matches_kitty_sequence(data, CODEPOINT_BACKSPACE, MODIFIER_ALT)
                    || matches_modify_other_keys(data, CODEPOINT_BACKSPACE, MODIFIER_ALT)
            } else if parsed_key_id.ctrl && !parsed_key_id.alt && !parsed_key_id.shift {
                if matches_raw_backspace(data, MODIFIER_CTRL) {
                    return true;
                }
                matches_kitty_sequence(data, CODEPOINT_BACKSPACE, MODIFIER_CTRL)
                    || matches_modify_other_keys(data, CODEPOINT_BACKSPACE, MODIFIER_CTRL)
            } else if modifier == 0 {
                matches_raw_backspace(data, 0)
                    || matches_kitty_sequence(data, CODEPOINT_BACKSPACE, 0)
                    || matches_modify_other_keys(data, CODEPOINT_BACKSPACE, 0)
            } else {
                matches_kitty_sequence(data, CODEPOINT_BACKSPACE, modifier)
                    || matches_modify_other_keys(data, CODEPOINT_BACKSPACE, modifier)
            }
        }
        "insert" => {
            if modifier == 0 {
                matches!(data, "\x1b[2~") || matches_kitty_sequence(data, CODEPOINT_INSERT, 0)
            } else if matches_legacy_modifier_sequence(data, "insert", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_INSERT, modifier)
            }
        }
        "delete" => {
            if modifier == 0 {
                matches!(data, "\x1b[3~") || matches_kitty_sequence(data, CODEPOINT_DELETE, 0)
            } else if matches_legacy_modifier_sequence(data, "delete", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_DELETE, modifier)
            }
        }
        "clear" => {
            if modifier == 0 {
                matches!(data, "\x1b[E" | "\x1bOE")
            } else {
                matches_legacy_modifier_sequence(data, "clear", modifier)
            }
        }
        "home" => {
            if modifier == 0 {
                matches!(data, "\x1b[H" | "\x1bOH" | "\x1b[1~" | "\x1b[7~")
                    || matches_kitty_sequence(data, CODEPOINT_HOME, 0)
            } else if matches_legacy_modifier_sequence(data, "home", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_HOME, modifier)
            }
        }
        "end" => {
            if modifier == 0 {
                matches!(data, "\x1b[F" | "\x1bOF" | "\x1b[4~" | "\x1b[8~")
                    || matches_kitty_sequence(data, CODEPOINT_END, 0)
            } else if matches_legacy_modifier_sequence(data, "end", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_END, modifier)
            }
        }
        "pageup" => {
            if modifier == 0 {
                matches!(data, "\x1b[5~" | "\x1b[[5~")
                    || matches_kitty_sequence(data, CODEPOINT_PAGE_UP, 0)
            } else if matches_legacy_modifier_sequence(data, "pageUp", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_PAGE_UP, modifier)
            }
        }
        "pagedown" => {
            if modifier == 0 {
                matches!(data, "\x1b[6~" | "\x1b[[6~")
                    || matches_kitty_sequence(data, CODEPOINT_PAGE_DOWN, 0)
            } else if matches_legacy_modifier_sequence(data, "pageDown", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_PAGE_DOWN, modifier)
            }
        }
        "up" => {
            if parsed_key_id.alt && !parsed_key_id.ctrl && !parsed_key_id.shift {
                data == "\x1bp" || matches_kitty_sequence(data, CODEPOINT_ARROW_UP, MODIFIER_ALT)
            } else if modifier == 0 {
                matches!(data, "\x1b[A" | "\x1bOA")
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_UP, 0)
            } else if matches_legacy_modifier_sequence(data, "up", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_ARROW_UP, modifier)
            }
        }
        "down" => {
            if parsed_key_id.alt && !parsed_key_id.ctrl && !parsed_key_id.shift {
                data == "\x1bn" || matches_kitty_sequence(data, CODEPOINT_ARROW_DOWN, MODIFIER_ALT)
            } else if modifier == 0 {
                matches!(data, "\x1b[B" | "\x1bOB")
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_DOWN, 0)
            } else if matches_legacy_modifier_sequence(data, "down", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_ARROW_DOWN, modifier)
            }
        }
        "left" => {
            if parsed_key_id.alt && !parsed_key_id.ctrl && !parsed_key_id.shift {
                data == "\x1b[1;3D"
                    || (!is_kitty_protocol_active() && data == "\x1bB")
                    || data == "\x1bb"
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_LEFT, MODIFIER_ALT)
            } else if parsed_key_id.ctrl && !parsed_key_id.alt && !parsed_key_id.shift {
                data == "\x1b[1;5D"
                    || matches_legacy_modifier_sequence(data, "left", MODIFIER_CTRL)
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_LEFT, MODIFIER_CTRL)
            } else if modifier == 0 {
                matches!(data, "\x1b[D" | "\x1bOD")
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_LEFT, 0)
            } else if matches_legacy_modifier_sequence(data, "left", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_ARROW_LEFT, modifier)
            }
        }
        "right" => {
            if parsed_key_id.alt && !parsed_key_id.ctrl && !parsed_key_id.shift {
                data == "\x1b[1;3C"
                    || (!is_kitty_protocol_active() && data == "\x1bF")
                    || data == "\x1bf"
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_RIGHT, MODIFIER_ALT)
            } else if parsed_key_id.ctrl && !parsed_key_id.alt && !parsed_key_id.shift {
                data == "\x1b[1;5C"
                    || matches_legacy_modifier_sequence(data, "right", MODIFIER_CTRL)
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_RIGHT, MODIFIER_CTRL)
            } else if modifier == 0 {
                matches!(data, "\x1b[C" | "\x1bOC")
                    || matches_kitty_sequence(data, CODEPOINT_ARROW_RIGHT, 0)
            } else if matches_legacy_modifier_sequence(data, "right", modifier) {
                true
            } else {
                matches_kitty_sequence(data, CODEPOINT_ARROW_RIGHT, modifier)
            }
        }
        "f1" => modifier == 0 && matches!(data, "\x1bOP" | "\x1b[11~" | "\x1b[[A"),
        "f2" => modifier == 0 && matches!(data, "\x1bOQ" | "\x1b[12~" | "\x1b[[B"),
        "f3" => modifier == 0 && matches!(data, "\x1bOR" | "\x1b[13~" | "\x1b[[C"),
        "f4" => modifier == 0 && matches!(data, "\x1bOS" | "\x1b[14~" | "\x1b[[D"),
        "f5" => modifier == 0 && matches!(data, "\x1b[15~" | "\x1b[[E"),
        "f6" => modifier == 0 && matches!(data, "\x1b[17~"),
        "f7" => modifier == 0 && matches!(data, "\x1b[18~"),
        "f8" => modifier == 0 && matches!(data, "\x1b[19~"),
        "f9" => modifier == 0 && matches!(data, "\x1b[20~"),
        "f10" => modifier == 0 && matches!(data, "\x1b[21~"),
        "f11" => modifier == 0 && matches!(data, "\x1b[23~"),
        "f12" => modifier == 0 && matches!(data, "\x1b[24~"),
        key if is_ascii_letter_key(key) || is_digit_key(key) || is_symbol_key(key) => {
            let codepoint = key.as_bytes()[0] as i32;
            let raw_ctrl = raw_ctrl_char(key);
            let is_letter = is_ascii_letter_key(key);
            let is_digit = is_digit_key(key);

            if parsed_key_id.ctrl
                && parsed_key_id.alt
                && !parsed_key_id.shift
                && !is_kitty_protocol_active()
            {
                if let Some(raw_ctrl) = raw_ctrl {
                    return data.as_bytes() == [0x1b, raw_ctrl as u8];
                }
            }

            if parsed_key_id.alt
                && !parsed_key_id.ctrl
                && !parsed_key_id.shift
                && !is_kitty_protocol_active()
            {
                if (is_letter || is_digit) && data.as_bytes() == [0x1b, key.as_bytes()[0]] {
                    return true;
                }
            }

            if parsed_key_id.ctrl && !parsed_key_id.shift && !parsed_key_id.alt {
                if let Some(raw_ctrl) = raw_ctrl {
                    if data.as_bytes() == [raw_ctrl as u8] {
                        return true;
                    }
                }
                return matches_kitty_sequence(data, codepoint, MODIFIER_CTRL)
                    || matches_printable_modify_other_keys(data, codepoint, MODIFIER_CTRL);
            }

            if parsed_key_id.ctrl && parsed_key_id.shift && !parsed_key_id.alt {
                return matches_kitty_sequence(data, codepoint, MODIFIER_SHIFT | MODIFIER_CTRL)
                    || matches_printable_modify_other_keys(
                        data,
                        codepoint,
                        MODIFIER_SHIFT | MODIFIER_CTRL,
                    );
            }

            if parsed_key_id.shift && !parsed_key_id.ctrl && !parsed_key_id.alt {
                if is_letter && data == key.to_ascii_uppercase().to_string() {
                    return true;
                }
                return matches_kitty_sequence(data, codepoint, MODIFIER_SHIFT)
                    || matches_printable_modify_other_keys(data, codepoint, MODIFIER_SHIFT);
            }

            if modifier != 0 {
                return matches_kitty_sequence(data, codepoint, modifier)
                    || matches_printable_modify_other_keys(data, codepoint, modifier);
            }

            data == key || matches_kitty_sequence(data, codepoint, 0)
        }
        _ => false,
    }
}

pub fn parse_key(data: &str) -> Option<KeyId> {
    if let Some(kitty) = parse_kitty_sequence(data) {
        return format_parsed_key(kitty.codepoint, kitty.modifier, kitty.base_layout_key);
    }

    if let Some(modify_other_keys) = parse_modify_other_keys_sequence(data) {
        return format_parsed_key(
            modify_other_keys.codepoint,
            modify_other_keys.modifier,
            None,
        );
    }

    if is_kitty_protocol_active() && (data == "\x1b\r" || data == "\n") {
        return Some(KeyId::from("shift+enter"));
    }

    if let Some(key_id) = legacy_sequence_key_id(data) {
        return Some(KeyId::from(key_id));
    }

    if data == "\x1b" {
        return Some(KeyId::from("escape"));
    }
    if data == "\x1c" {
        return Some(KeyId::from("ctrl+\\"));
    }
    if data == "\x1d" {
        return Some(KeyId::from("ctrl+]"));
    }
    if data == "\x1f" {
        return Some(KeyId::from("ctrl+-"));
    }
    if data == "\x1b\x1b" {
        return Some(KeyId::from("ctrl+alt+["));
    }
    if data == "\x1b\x1c" {
        return Some(KeyId::from("ctrl+alt+\\"));
    }
    if data == "\x1b\x1d" {
        return Some(KeyId::from("ctrl+alt+]"));
    }
    if data == "\x1b\x1f" {
        return Some(KeyId::from("ctrl+alt+-"));
    }
    if data == "\t" {
        return Some(KeyId::from("tab"));
    }
    if data == "\r" || (!is_kitty_protocol_active() && data == "\n") || data == "\x1bOM" {
        return Some(KeyId::from("enter"));
    }
    if data == "\x00" {
        return Some(KeyId::from("ctrl+space"));
    }
    if data == " " {
        return Some(KeyId::from("space"));
    }
    if data == "\x7f" {
        return Some(KeyId::from("backspace"));
    }
    if data == "\x08" {
        if is_windows_terminal_session() {
            return Some(KeyId::from("ctrl+backspace"));
        }
        return Some(KeyId::from("backspace"));
    }
    if data == "\x1b[Z" {
        return Some(KeyId::from("shift+tab"));
    }
    if !is_kitty_protocol_active() && data == "\x1b\r" {
        return Some(KeyId::from("alt+enter"));
    }
    if !is_kitty_protocol_active() && data == "\x1b " {
        return Some(KeyId::from("alt+space"));
    }
    if data == "\x1b\x7f" || data == "\x1b\x08" {
        return Some(KeyId::from("alt+backspace"));
    }
    if !is_kitty_protocol_active() && data == "\x1bB" {
        return Some(KeyId::from("alt+left"));
    }
    if !is_kitty_protocol_active() && data == "\x1bF" {
        return Some(KeyId::from("alt+right"));
    }
    if !is_kitty_protocol_active() && data.as_bytes().len() == 2 && data.as_bytes()[0] == 0x1b {
        let code = data.as_bytes()[1];
        if (1..=26).contains(&code) {
            let letter = (code + 96) as char;
            return Some(KeyId::from(format!("ctrl+alt+{letter}")));
        }
        if code.is_ascii_lowercase() || code.is_ascii_digit() {
            return Some(KeyId::from(format!("alt+{}", code as char)));
        }
    }
    if data == "\x1b[A" {
        return Some(KeyId::from("up"));
    }
    if data == "\x1b[B" {
        return Some(KeyId::from("down"));
    }
    if data == "\x1b[C" {
        return Some(KeyId::from("right"));
    }
    if data == "\x1b[D" {
        return Some(KeyId::from("left"));
    }
    if data == "\x1b[H" || data == "\x1bOH" {
        return Some(KeyId::from("home"));
    }
    if data == "\x1b[F" || data == "\x1bOF" {
        return Some(KeyId::from("end"));
    }
    if data == "\x1b[3~" {
        return Some(KeyId::from("delete"));
    }
    if data == "\x1b[5~" {
        return Some(KeyId::from("pageUp"));
    }
    if data == "\x1b[6~" {
        return Some(KeyId::from("pageDown"));
    }

    if data.chars().count() == 1 {
        let code = data.chars().next()? as u32;
        if (1..=26).contains(&code) {
            let letter = char::from_u32(code + 96)?;
            return Some(KeyId::from(format!("ctrl+{letter}")));
        }
        if (32..=126).contains(&code) {
            return Some(KeyId::from(data.to_string()));
        }
    }

    None
}

pub fn decode_kitty_printable(data: &str) -> Option<String> {
    let captures = CSI_U_REGEX.captures(data)?;
    let codepoint = parse_required_capture(&captures, 1)?;
    let shifted_key = parse_optional_capture(&captures, 2);
    let modifier = parse_optional_capture(&captures, 4)
        .and_then(|value| value.checked_sub(1))
        .unwrap_or(0) as u16;

    if (modifier & !KITTY_PRINTABLE_ALLOWED_MODIFIERS) != 0 {
        return None;
    }
    if (modifier & (MODIFIER_ALT | MODIFIER_CTRL)) != 0 {
        return None;
    }

    let mut effective_codepoint = codepoint;
    if (modifier & MODIFIER_SHIFT) != 0 {
        if let Some(shifted_key) = shifted_key {
            effective_codepoint = shifted_key;
        }
    }

    effective_codepoint = normalize_kitty_functional_codepoint(effective_codepoint);
    if effective_codepoint < 32 {
        return None;
    }

    char::from_u32(effective_codepoint as u32).map(|character| character.to_string())
}

const KITTY_PRINTABLE_ALLOWED_MODIFIERS: u16 = MODIFIER_SHIFT | LOCK_MASK;

#[derive(Debug)]
struct ParsedKeyId {
    key: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

fn parse_key_id(key_id: &str) -> Option<ParsedKeyId> {
    let normalized = key_id.to_lowercase();
    let parts = normalized.split('+').collect::<Vec<_>>();
    let key = parts.last()?.to_string();
    if key.is_empty() {
        return None;
    }

    Some(ParsedKeyId {
        key,
        ctrl: parts.contains(&"ctrl"),
        shift: parts.contains(&"shift"),
        alt: parts.contains(&"alt"),
    })
}

fn parse_kitty_sequence(data: &str) -> Option<ParsedKittySequence> {
    if let Some(captures) = CSI_U_REGEX.captures(data) {
        let codepoint = parse_required_capture(&captures, 1)?;
        let _shifted_key = parse_optional_capture(&captures, 2);
        let base_layout_key = parse_optional_capture(&captures, 3);
        let modifier = parse_optional_capture(&captures, 4)
            .and_then(|value| value.checked_sub(1))
            .unwrap_or(0) as u16;
        let _event_type = parse_event_type(captures.get(5).map(|capture| capture.as_str()));

        return Some(ParsedKittySequence {
            codepoint,
            base_layout_key,
            modifier,
        });
    }

    if let Some(captures) = KITTY_ARROW_REGEX.captures(data) {
        let modifier = parse_required_capture(&captures, 1)?;
        let _event_type = parse_event_type(captures.get(2).map(|capture| capture.as_str()));
        let codepoint = match captures.get(3)?.as_str() {
            "A" => CODEPOINT_ARROW_UP,
            "B" => CODEPOINT_ARROW_DOWN,
            "C" => CODEPOINT_ARROW_RIGHT,
            "D" => CODEPOINT_ARROW_LEFT,
            _ => return None,
        };

        return Some(ParsedKittySequence {
            codepoint,
            base_layout_key: None,
            modifier: modifier.saturating_sub(1) as u16,
        });
    }

    if let Some(captures) = KITTY_FUNCTION_REGEX.captures(data) {
        let key_number = parse_required_capture(&captures, 1)?;
        let modifier = parse_optional_capture(&captures, 2)
            .and_then(|value| value.checked_sub(1))
            .unwrap_or(0) as u16;
        let _event_type = parse_event_type(captures.get(3).map(|capture| capture.as_str()));
        let codepoint = match key_number {
            2 => CODEPOINT_INSERT,
            3 => CODEPOINT_DELETE,
            5 => CODEPOINT_PAGE_UP,
            6 => CODEPOINT_PAGE_DOWN,
            7 => CODEPOINT_HOME,
            8 => CODEPOINT_END,
            _ => return None,
        };

        return Some(ParsedKittySequence {
            codepoint,
            base_layout_key: None,
            modifier,
        });
    }

    if let Some(captures) = KITTY_HOME_END_REGEX.captures(data) {
        let modifier = parse_required_capture(&captures, 1)?;
        let _event_type = parse_event_type(captures.get(2).map(|capture| capture.as_str()));
        let codepoint = match captures.get(3)?.as_str() {
            "H" => CODEPOINT_HOME,
            "F" => CODEPOINT_END,
            _ => return None,
        };

        return Some(ParsedKittySequence {
            codepoint,
            base_layout_key: None,
            modifier: modifier.saturating_sub(1) as u16,
        });
    }

    None
}

fn parse_modify_other_keys_sequence(data: &str) -> Option<ParsedModifyOtherKeysSequence> {
    let captures = MODIFY_OTHER_KEYS_REGEX.captures(data)?;
    let modifier = parse_required_capture(&captures, 1)?;
    let codepoint = parse_required_capture(&captures, 2)?;
    Some(ParsedModifyOtherKeysSequence {
        codepoint,
        modifier: modifier.saturating_sub(1) as u16,
    })
}

fn parse_required_capture(captures: &regex::Captures<'_>, index: usize) -> Option<i32> {
    captures.get(index)?.as_str().parse::<i32>().ok()
}

fn parse_optional_capture(captures: &regex::Captures<'_>, index: usize) -> Option<i32> {
    let capture = captures.get(index)?.as_str();
    if capture.is_empty() {
        None
    } else {
        capture.parse::<i32>().ok()
    }
}

fn parse_event_type(value: Option<&str>) -> KeyEventType {
    match value.and_then(|value| value.parse::<i32>().ok()) {
        Some(2) => KeyEventType::Repeat,
        Some(3) => KeyEventType::Release,
        _ => KeyEventType::Press,
    }
}

fn matches_kitty_sequence(data: &str, expected_codepoint: i32, expected_modifier: u16) -> bool {
    let Some(parsed) = parse_kitty_sequence(data) else {
        return false;
    };

    let actual_modifier = parsed.modifier & !LOCK_MASK;
    let expected_modifier = expected_modifier & !LOCK_MASK;
    if actual_modifier != expected_modifier {
        return false;
    }

    let normalized_codepoint = normalize_kitty_functional_codepoint(parsed.codepoint);
    let normalized_expected_codepoint = normalize_kitty_functional_codepoint(expected_codepoint);
    if normalized_codepoint == normalized_expected_codepoint {
        return true;
    }

    if parsed.base_layout_key == Some(expected_codepoint) {
        let is_latin_letter = is_latin_letter_codepoint(normalized_codepoint);
        let is_known_symbol = symbol_from_codepoint(normalized_codepoint)
            .map(is_symbol_char)
            .unwrap_or(false);
        if !is_latin_letter && !is_known_symbol {
            return true;
        }
    }

    false
}

fn matches_modify_other_keys(data: &str, expected_keycode: i32, expected_modifier: u16) -> bool {
    let Some(parsed) = parse_modify_other_keys_sequence(data) else {
        return false;
    };

    parsed.codepoint == expected_keycode && parsed.modifier == expected_modifier
}

fn matches_printable_modify_other_keys(
    data: &str,
    expected_keycode: i32,
    expected_modifier: u16,
) -> bool {
    expected_modifier != 0 && matches_modify_other_keys(data, expected_keycode, expected_modifier)
}

fn matches_legacy_modifier_sequence(data: &str, key: &str, modifier: u16) -> bool {
    match (key, modifier) {
        ("up", MODIFIER_SHIFT) => matches!(data, "\x1b[a"),
        ("down", MODIFIER_SHIFT) => matches!(data, "\x1b[b"),
        ("right", MODIFIER_SHIFT) => matches!(data, "\x1b[c"),
        ("left", MODIFIER_SHIFT) => matches!(data, "\x1b[d"),
        ("clear", MODIFIER_SHIFT) => matches!(data, "\x1b[e"),
        ("insert", MODIFIER_SHIFT) => matches!(data, "\x1b[2$"),
        ("delete", MODIFIER_SHIFT) => matches!(data, "\x1b[3$"),
        ("pageUp", MODIFIER_SHIFT) => matches!(data, "\x1b[5$"),
        ("pageDown", MODIFIER_SHIFT) => matches!(data, "\x1b[6$"),
        ("home", MODIFIER_SHIFT) => matches!(data, "\x1b[7$"),
        ("end", MODIFIER_SHIFT) => matches!(data, "\x1b[8$"),
        ("up", MODIFIER_CTRL) => matches!(data, "\x1bOa"),
        ("down", MODIFIER_CTRL) => matches!(data, "\x1bOb"),
        ("right", MODIFIER_CTRL) => matches!(data, "\x1bOc"),
        ("left", MODIFIER_CTRL) => matches!(data, "\x1bOd"),
        ("clear", MODIFIER_CTRL) => matches!(data, "\x1bOe"),
        ("insert", MODIFIER_CTRL) => matches!(data, "\x1b[2^"),
        ("delete", MODIFIER_CTRL) => matches!(data, "\x1b[3^"),
        ("pageUp", MODIFIER_CTRL) => matches!(data, "\x1b[5^"),
        ("pageDown", MODIFIER_CTRL) => matches!(data, "\x1b[6^"),
        ("home", MODIFIER_CTRL) => matches!(data, "\x1b[7^"),
        ("end", MODIFIER_CTRL) => matches!(data, "\x1b[8^"),
        _ => false,
    }
}

fn legacy_sequence_key_id(data: &str) -> Option<&'static str> {
    match data {
        "\x1bOA" => Some("up"),
        "\x1bOB" => Some("down"),
        "\x1bOC" => Some("right"),
        "\x1bOD" => Some("left"),
        "\x1bOH" => Some("home"),
        "\x1bOF" => Some("end"),
        "\x1b[E" | "\x1bOE" => Some("clear"),
        "\x1bOe" => Some("ctrl+clear"),
        "\x1b[e" => Some("shift+clear"),
        "\x1b[2~" => Some("insert"),
        "\x1b[2$" => Some("shift+insert"),
        "\x1b[2^" => Some("ctrl+insert"),
        "\x1b[3$" => Some("shift+delete"),
        "\x1b[3^" => Some("ctrl+delete"),
        "\x1b[[5~" => Some("pageUp"),
        "\x1b[[6~" => Some("pageDown"),
        "\x1b[a" => Some("shift+up"),
        "\x1b[b" => Some("shift+down"),
        "\x1b[c" => Some("shift+right"),
        "\x1b[d" => Some("shift+left"),
        "\x1bOa" => Some("ctrl+up"),
        "\x1bOb" => Some("ctrl+down"),
        "\x1bOc" => Some("ctrl+right"),
        "\x1bOd" => Some("ctrl+left"),
        "\x1b[5$" => Some("shift+pageUp"),
        "\x1b[6$" => Some("shift+pageDown"),
        "\x1b[7$" => Some("shift+home"),
        "\x1b[8$" => Some("shift+end"),
        "\x1b[5^" => Some("ctrl+pageUp"),
        "\x1b[6^" => Some("ctrl+pageDown"),
        "\x1b[7^" => Some("ctrl+home"),
        "\x1b[8^" => Some("ctrl+end"),
        "\x1bOP" => Some("f1"),
        "\x1bOQ" => Some("f2"),
        "\x1bOR" => Some("f3"),
        "\x1bOS" => Some("f4"),
        "\x1b[11~" => Some("f1"),
        "\x1b[12~" => Some("f2"),
        "\x1b[13~" => Some("f3"),
        "\x1b[14~" => Some("f4"),
        "\x1b[[A" => Some("f1"),
        "\x1b[[B" => Some("f2"),
        "\x1b[[C" => Some("f3"),
        "\x1b[[D" => Some("f4"),
        "\x1b[[E" => Some("f5"),
        "\x1b[15~" => Some("f5"),
        "\x1b[17~" => Some("f6"),
        "\x1b[18~" => Some("f7"),
        "\x1b[19~" => Some("f8"),
        "\x1b[20~" => Some("f9"),
        "\x1b[21~" => Some("f10"),
        "\x1b[23~" => Some("f11"),
        "\x1b[24~" => Some("f12"),
        "\x1bb" => Some("alt+left"),
        "\x1bf" => Some("alt+right"),
        "\x1bp" => Some("alt+up"),
        "\x1bn" => Some("alt+down"),
        _ => None,
    }
}

fn is_windows_terminal_session() -> bool {
    std::env::var_os("WT_SESSION").is_some()
        && std::env::var_os("SSH_CONNECTION").is_none()
        && std::env::var_os("SSH_CLIENT").is_none()
        && std::env::var_os("SSH_TTY").is_none()
}

fn matches_raw_backspace(data: &str, expected_modifier: u16) -> bool {
    if data == "\x7f" {
        return expected_modifier == 0;
    }
    if data != "\x08" {
        return false;
    }
    if is_windows_terminal_session() {
        expected_modifier == MODIFIER_CTRL
    } else {
        expected_modifier == 0
    }
}

fn raw_ctrl_char(key: &str) -> Option<char> {
    let character = key.chars().next()?.to_ascii_lowercase();
    let code = character as u32;
    if (97..=122).contains(&code) || matches!(character, '[' | '\\' | ']' | '_') {
        return char::from_u32(code & 0x1f);
    }
    if character == '-' {
        return char::from_u32(31);
    }
    None
}

fn normalize_kitty_functional_codepoint(codepoint: i32) -> i32 {
    match codepoint {
        57399 => 48,
        57400 => 49,
        57401 => 50,
        57402 => 51,
        57403 => 52,
        57404 => 53,
        57405 => 54,
        57406 => 55,
        57407 => 56,
        57408 => 57,
        57409 => 46,
        57410 => 47,
        57411 => 42,
        57412 => 45,
        57413 => 43,
        57415 => 61,
        57416 => 44,
        57417 => CODEPOINT_ARROW_LEFT,
        57418 => CODEPOINT_ARROW_RIGHT,
        57419 => CODEPOINT_ARROW_UP,
        57420 => CODEPOINT_ARROW_DOWN,
        57421 => CODEPOINT_PAGE_UP,
        57422 => CODEPOINT_PAGE_DOWN,
        57423 => CODEPOINT_HOME,
        57424 => CODEPOINT_END,
        57425 => CODEPOINT_INSERT,
        57426 => CODEPOINT_DELETE,
        _ => codepoint,
    }
}

fn format_key_name_with_modifiers(key_name: &str, modifier: u16) -> Option<KeyId> {
    let effective_modifier = modifier & !LOCK_MASK;
    if (effective_modifier & !SUPPORTED_MODIFIER_MASK) != 0 {
        return None;
    }

    let mut parts = Vec::new();
    if (effective_modifier & MODIFIER_SHIFT) != 0 {
        parts.push("shift");
    }
    if (effective_modifier & MODIFIER_CTRL) != 0 {
        parts.push("ctrl");
    }
    if (effective_modifier & MODIFIER_ALT) != 0 {
        parts.push("alt");
    }

    if parts.is_empty() {
        Some(KeyId::from(key_name))
    } else {
        Some(KeyId::from(format!("{}+{key_name}", parts.join("+"))))
    }
}

fn format_parsed_key(codepoint: i32, modifier: u16, base_layout_key: Option<i32>) -> Option<KeyId> {
    let normalized_codepoint = normalize_kitty_functional_codepoint(codepoint);
    let is_latin_letter = is_latin_letter_codepoint(normalized_codepoint);
    let is_digit = is_digit_codepoint(normalized_codepoint);
    let is_known_symbol = symbol_from_codepoint(normalized_codepoint)
        .map(is_symbol_char)
        .unwrap_or(false);

    let effective_codepoint = if is_latin_letter || is_digit || is_known_symbol {
        normalized_codepoint
    } else {
        base_layout_key.unwrap_or(normalized_codepoint)
    };

    let key_name = if effective_codepoint == CODEPOINT_ESCAPE {
        Some("escape".to_string())
    } else if effective_codepoint == CODEPOINT_TAB {
        Some("tab".to_string())
    } else if effective_codepoint == CODEPOINT_ENTER || effective_codepoint == CODEPOINT_KP_ENTER {
        Some("enter".to_string())
    } else if effective_codepoint == CODEPOINT_SPACE {
        Some("space".to_string())
    } else if effective_codepoint == CODEPOINT_BACKSPACE {
        Some("backspace".to_string())
    } else if effective_codepoint == CODEPOINT_DELETE {
        Some("delete".to_string())
    } else if effective_codepoint == CODEPOINT_INSERT {
        Some("insert".to_string())
    } else if effective_codepoint == CODEPOINT_HOME {
        Some("home".to_string())
    } else if effective_codepoint == CODEPOINT_END {
        Some("end".to_string())
    } else if effective_codepoint == CODEPOINT_PAGE_UP {
        Some("pageUp".to_string())
    } else if effective_codepoint == CODEPOINT_PAGE_DOWN {
        Some("pageDown".to_string())
    } else if effective_codepoint == CODEPOINT_ARROW_UP {
        Some("up".to_string())
    } else if effective_codepoint == CODEPOINT_ARROW_DOWN {
        Some("down".to_string())
    } else if effective_codepoint == CODEPOINT_ARROW_LEFT {
        Some("left".to_string())
    } else if effective_codepoint == CODEPOINT_ARROW_RIGHT {
        Some("right".to_string())
    } else if is_digit_codepoint(effective_codepoint)
        || is_latin_letter_codepoint(effective_codepoint)
    {
        char::from_u32(effective_codepoint as u32).map(|character| character.to_string())
    } else {
        symbol_from_codepoint(effective_codepoint).map(|character| character.to_string())
    }?;

    format_key_name_with_modifiers(&key_name, modifier)
}

fn symbol_from_codepoint(codepoint: i32) -> Option<char> {
    let character = char::from_u32(codepoint as u32)?;
    if is_symbol_char(character) {
        Some(character)
    } else {
        None
    }
}

fn is_latin_letter_codepoint(codepoint: i32) -> bool {
    (97..=122).contains(&codepoint)
}

fn is_digit_codepoint(codepoint: i32) -> bool {
    (48..=57).contains(&codepoint)
}

fn is_ascii_letter_key(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_lowercase()
}

fn is_digit_key(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_digit()
}

fn is_symbol_key(key: &str) -> bool {
    matches!(
        key,
        "`" | "-"
            | "="
            | "["
            | "]"
            | "\\"
            | ";"
            | "'"
            | ","
            | "."
            | "/"
            | "!"
            | "@"
            | "#"
            | "$"
            | "%"
            | "^"
            | "&"
            | "*"
            | "("
            | ")"
            | "_"
            | "+"
            | "|"
            | "~"
            | "{"
            | "}"
            | ":"
            | "<"
            | ">"
            | "?"
    )
}

fn is_symbol_char(character: char) -> bool {
    matches!(
        character,
        '`' | '-'
            | '='
            | '['
            | ']'
            | '\\'
            | ';'
            | '\''
            | ','
            | '.'
            | '/'
            | '!'
            | '@'
            | '#'
            | '$'
            | '%'
            | '^'
            | '&'
            | '*'
            | '('
            | ')'
            | '_'
            | '+'
            | '|'
            | '~'
            | '{'
            | '}'
            | ':'
            | '<'
            | '>'
            | '?'
    )
}
