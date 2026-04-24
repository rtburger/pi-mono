use parking_lot::{Mutex, MutexGuard};
use pi_tui::{
    decode_kitty_printable, is_key_release, is_key_repeat, is_kitty_protocol_active, matches_key,
    parse_key, set_kitty_protocol_active,
};
use std::{ffi::OsString, sync::LazyLock};

static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
const GUARDED_ENV_VARS: &[&str] = &["WT_SESSION", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];

struct TestGuard {
    _lock: MutexGuard<'static, ()>,
    previous_kitty_protocol_active: bool,
    previous_env: Vec<(&'static str, Option<OsString>)>,
}

impl TestGuard {
    fn new() -> Self {
        let lock = TEST_LOCK.lock();
        let previous_kitty_protocol_active = is_kitty_protocol_active();
        let previous_env = GUARDED_ENV_VARS
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect();

        set_kitty_protocol_active(false);

        Self {
            _lock: lock,
            previous_kitty_protocol_active,
            previous_env,
        }
    }

    fn set_kitty_protocol_active(&self, active: bool) {
        set_kitty_protocol_active(active);
    }

    fn set_env(&self, name: &str, value: Option<&str>) {
        match value {
            Some(value) => {
                // SAFETY: tests serialize all environment mutation through TEST_LOCK.
                unsafe { std::env::set_var(name, value) }
            }
            None => {
                // SAFETY: tests serialize all environment mutation through TEST_LOCK.
                unsafe { std::env::remove_var(name) }
            }
        }
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        set_kitty_protocol_active(self.previous_kitty_protocol_active);
        for (name, value) in &self.previous_env {
            match value {
                Some(value) => {
                    // SAFETY: tests serialize all environment mutation through TEST_LOCK.
                    unsafe { std::env::set_var(name, value) }
                }
                None => {
                    // SAFETY: tests serialize all environment mutation through TEST_LOCK.
                    unsafe { std::env::remove_var(name) }
                }
            }
        }
    }
}

fn parsed_key(data: &str) -> Option<String> {
    parse_key(data).map(|key| key.to_string())
}

#[test]
fn kitty_alternate_keys_match_base_layout_digits_symbols_and_navigation() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(true);

    assert!(matches_key("\x1b[1089::99;5u", "ctrl+c"));
    assert!(matches_key("\x1b[1074::100;5u", "ctrl+d"));
    assert!(matches_key("\x1b[1103::122;5u", "ctrl+z"));
    assert!(matches_key("\x1b[1079::112;6u", "ctrl+shift+p"));
    assert!(matches_key("\x1b[99;5u", "ctrl+c"));

    assert!(matches_key("\x1b[49u", "1"));
    assert!(matches_key("\x1b[49;5u", "ctrl+1"));
    assert!(!matches_key("\x1b[49;5u", "ctrl+2"));
    assert_eq!(parsed_key("\x1b[49u"), Some("1".to_string()));
    assert_eq!(parsed_key("\x1b[49;5u"), Some("ctrl+1".to_string()));

    assert!(matches_key("\x1b[57400u", "1"));
    assert!(matches_key("\x1b[57410u", "/"));
    assert!(matches_key("\x1b[57417u", "left"));
    assert!(matches_key("\x1b[57426u", "delete"));
    assert_eq!(parsed_key("\x1b[57399u"), Some("0".to_string()));
    assert_eq!(parsed_key("\x1b[57409u"), Some(".".to_string()));
    assert_eq!(parsed_key("\x1b[57413u"), Some("+".to_string()));
    assert_eq!(parsed_key("\x1b[57416u"), Some(",".to_string()));
    assert_eq!(parsed_key("\x1b[57417u"), Some("left".to_string()));
    assert_eq!(parsed_key("\x1b[57418u"), Some("right".to_string()));
    assert_eq!(parsed_key("\x1b[57419u"), Some("up".to_string()));
    assert_eq!(parsed_key("\x1b[57420u"), Some("down".to_string()));
    assert_eq!(parsed_key("\x1b[57421u"), Some("pageUp".to_string()));
    assert_eq!(parsed_key("\x1b[57422u"), Some("pageDown".to_string()));
    assert_eq!(parsed_key("\x1b[57423u"), Some("home".to_string()));
    assert_eq!(parsed_key("\x1b[57424u"), Some("end".to_string()));
    assert_eq!(parsed_key("\x1b[57425u"), Some("insert".to_string()));
    assert_eq!(parsed_key("\x1b[57426u"), Some("delete".to_string()));
}

#[test]
fn kitty_alternate_keys_handle_shifted_and_event_variants_and_prefer_codepoint_when_needed() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(true);

    assert!(matches_key("\x1b[99:67:99;2u", "shift+c"));
    assert!(matches_key("\x1b[1089::99;5:3u", "ctrl+c"));
    assert!(matches_key("\x1b[1089:1057:99;6:2u", "ctrl+shift+c"));

    let dvorak_ctrl_k = "\x1b[107::118;5u";
    assert!(matches_key(dvorak_ctrl_k, "ctrl+k"));
    assert!(!matches_key(dvorak_ctrl_k, "ctrl+v"));
    assert_eq!(parsed_key(dvorak_ctrl_k), Some("ctrl+k".to_string()));

    let dvorak_ctrl_slash = "\x1b[47::91;5u";
    assert!(matches_key(dvorak_ctrl_slash, "ctrl+/"));
    assert!(!matches_key(dvorak_ctrl_slash, "ctrl+["));
    assert_eq!(parsed_key(dvorak_ctrl_slash), Some("ctrl+/".to_string()));

    let cyrillic_ctrl_c = "\x1b[1089::99;5u";
    assert!(!matches_key(cyrillic_ctrl_c, "ctrl+d"));
    assert!(!matches_key(cyrillic_ctrl_c, "ctrl+shift+c"));
    assert_eq!(parsed_key(cyrillic_ctrl_c), Some("ctrl+c".to_string()));
    assert_eq!(parsed_key("\x1b[99;9u"), None);
}

#[test]
fn modify_other_keys_sequences_match_and_parse_core_variants() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(false);

    assert!(matches_key("\x1b[27;5;99~", "ctrl+c"));
    assert_eq!(parsed_key("\x1b[27;5;99~"), Some("ctrl+c".to_string()));
    assert!(matches_key("\x1b[27;5;100~", "ctrl+d"));
    assert_eq!(parsed_key("\x1b[27;5;100~"), Some("ctrl+d".to_string()));
    assert!(matches_key("\x1b[27;5;122~", "ctrl+z"));
    assert_eq!(parsed_key("\x1b[27;5;122~"), Some("ctrl+z".to_string()));

    assert!(matches_key("\x1b[27;5;13~", "ctrl+enter"));
    assert!(matches_key("\x1b[27;2;13~", "shift+enter"));
    assert!(matches_key("\x1b[27;3;13~", "alt+enter"));
    assert_eq!(parsed_key("\x1b[27;5;13~"), Some("ctrl+enter".to_string()));
    assert_eq!(parsed_key("\x1b[27;2;13~"), Some("shift+enter".to_string()));
    assert_eq!(parsed_key("\x1b[27;3;13~"), Some("alt+enter".to_string()));

    assert!(matches_key("\x1b[27;2;9~", "shift+tab"));
    assert!(matches_key("\x1b[27;5;9~", "ctrl+tab"));
    assert!(matches_key("\x1b[27;3;9~", "alt+tab"));
    assert_eq!(parsed_key("\x1b[27;2;9~"), Some("shift+tab".to_string()));
    assert_eq!(parsed_key("\x1b[27;5;9~"), Some("ctrl+tab".to_string()));
    assert_eq!(parsed_key("\x1b[27;3;9~"), Some("alt+tab".to_string()));

    assert!(matches_key("\x1b[27;1;127~", "backspace"));
    assert!(matches_key("\x1b[27;5;127~", "ctrl+backspace"));
    assert!(matches_key("\x1b[27;3;127~", "alt+backspace"));
    assert_eq!(parsed_key("\x1b[27;1;127~"), Some("backspace".to_string()));
    assert_eq!(
        parsed_key("\x1b[27;5;127~"),
        Some("ctrl+backspace".to_string())
    );
    assert_eq!(
        parsed_key("\x1b[27;3;127~"),
        Some("alt+backspace".to_string())
    );

    assert!(matches_key("\x1b[27;1;27~", "escape"));
    assert_eq!(parsed_key("\x1b[27;1;27~"), Some("escape".to_string()));
    assert!(matches_key("\x1b[27;1;32~", "space"));
    assert!(matches_key("\x1b[27;5;32~", "ctrl+space"));
    assert_eq!(parsed_key("\x1b[27;1;32~"), Some("space".to_string()));
    assert_eq!(parsed_key("\x1b[27;5;32~"), Some("ctrl+space".to_string()));

    assert!(matches_key("\x1b[27;5;47~", "ctrl+/"));
    assert_eq!(parsed_key("\x1b[27;5;47~"), Some("ctrl+/".to_string()));
    assert!(matches_key("\x1b[27;5;49~", "ctrl+1"));
    assert!(matches_key("\x1b[27;2;49~", "shift+1"));
    assert_eq!(parsed_key("\x1b[27;5;49~"), Some("ctrl+1".to_string()));
    assert_eq!(parsed_key("\x1b[27;2;49~"), Some("shift+1".to_string()));
}

#[test]
fn legacy_matching_and_parsing_cover_ctrl_enter_space_and_symbols() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(false);

    assert!(matches_key("\x03", "ctrl+c"));
    assert!(matches_key("\x04", "ctrl+d"));
    assert_eq!(parsed_key("\x03"), Some("ctrl+c".to_string()));
    assert_eq!(parsed_key("\x04"), Some("ctrl+d".to_string()));

    assert!(matches_key("\x1b", "escape"));
    assert_eq!(parsed_key("\x1b"), Some("escape".to_string()));

    assert!(matches_key("\n", "enter"));
    assert_eq!(parsed_key("\n"), Some("enter".to_string()));
    assert!(matches_key("\x00", "ctrl+space"));
    assert_eq!(parsed_key("\x00"), Some("ctrl+space".to_string()));

    assert!(matches_key("\x1c", "ctrl+\\"));
    assert_eq!(parsed_key("\x1c"), Some("ctrl+\\".to_string()));
    assert!(matches_key("\x1d", "ctrl+]"));
    assert_eq!(parsed_key("\x1d"), Some("ctrl+]".to_string()));
    assert!(matches_key("\x1f", "ctrl+_"));
    assert!(matches_key("\x1f", "ctrl+-"));
    assert_eq!(parsed_key("\x1f"), Some("ctrl+-".to_string()));

    assert!(matches_key("\x1b\x1b", "ctrl+alt+["));
    assert_eq!(parsed_key("\x1b\x1b"), Some("ctrl+alt+[".to_string()));
    assert!(matches_key("\x1b\x1c", "ctrl+alt+\\"));
    assert_eq!(parsed_key("\x1b\x1c"), Some("ctrl+alt+\\".to_string()));
    assert!(matches_key("\x1b\x1d", "ctrl+alt+]"));
    assert_eq!(parsed_key("\x1b\x1d"), Some("ctrl+alt+]".to_string()));
    assert!(matches_key("\x1b\x1f", "ctrl+alt+_"));
    assert!(matches_key("\x1b\x1f", "ctrl+alt+-"));
    assert_eq!(parsed_key("\x1b\x1f"), Some("ctrl+alt+-".to_string()));
}

#[test]
fn legacy_backspace_heuristic_changes_in_windows_terminal() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(false);
    guard.set_env("WT_SESSION", None);
    guard.set_env("SSH_CONNECTION", None);
    guard.set_env("SSH_CLIENT", None);
    guard.set_env("SSH_TTY", None);

    assert!(matches_key("\x7f", "backspace"));
    assert!(!matches_key("\x7f", "ctrl+backspace"));
    assert_eq!(parsed_key("\x7f"), Some("backspace".to_string()));
    assert!(matches_key("\x08", "backspace"));
    assert!(!matches_key("\x08", "ctrl+backspace"));
    assert_eq!(parsed_key("\x08"), Some("backspace".to_string()));
    assert!(matches_key("\x08", "ctrl+h"));

    guard.set_env("WT_SESSION", Some("test-session"));
    guard.set_env("SSH_CONNECTION", None);
    guard.set_env("SSH_CLIENT", None);
    guard.set_env("SSH_TTY", None);

    assert!(matches_key("\x08", "ctrl+backspace"));
    assert!(!matches_key("\x08", "backspace"));
    assert_eq!(parsed_key("\x08"), Some("ctrl+backspace".to_string()));
    assert!(matches_key("\x08", "ctrl+h"));
}

#[test]
fn legacy_alt_prefixed_sequences_are_mode_aware() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(false);

    assert!(matches_key("\x1b ", "alt+space"));
    assert_eq!(parsed_key("\x1b "), Some("alt+space".to_string()));
    assert!(matches_key("\x1b\x08", "alt+backspace"));
    assert_eq!(parsed_key("\x1b\x08"), Some("alt+backspace".to_string()));
    assert!(matches_key("\x1b\x03", "ctrl+alt+c"));
    assert_eq!(parsed_key("\x1b\x03"), Some("ctrl+alt+c".to_string()));
    assert!(matches_key("\x1bB", "alt+left"));
    assert_eq!(parsed_key("\x1bB"), Some("alt+left".to_string()));
    assert!(matches_key("\x1bF", "alt+right"));
    assert_eq!(parsed_key("\x1bF"), Some("alt+right".to_string()));
    assert!(matches_key("\x1ba", "alt+a"));
    assert_eq!(parsed_key("\x1ba"), Some("alt+a".to_string()));
    assert!(matches_key("\x1b1", "alt+1"));
    assert_eq!(parsed_key("\x1b1"), Some("alt+1".to_string()));
    assert!(matches_key("\x1by", "alt+y"));
    assert_eq!(parsed_key("\x1by"), Some("alt+y".to_string()));
    assert!(matches_key("\x1bz", "alt+z"));
    assert_eq!(parsed_key("\x1bz"), Some("alt+z".to_string()));

    guard.set_kitty_protocol_active(true);

    assert!(!matches_key("\x1b ", "alt+space"));
    assert_eq!(parsed_key("\x1b "), None);
    assert!(matches_key("\x1b\x08", "alt+backspace"));
    assert_eq!(parsed_key("\x1b\x08"), Some("alt+backspace".to_string()));
    assert!(!matches_key("\x1b\x03", "ctrl+alt+c"));
    assert_eq!(parsed_key("\x1b\x03"), None);
    assert!(!matches_key("\x1bB", "alt+left"));
    assert_eq!(parsed_key("\x1bB"), None);
    assert!(!matches_key("\x1bF", "alt+right"));
    assert_eq!(parsed_key("\x1bF"), None);
    assert!(!matches_key("\x1ba", "alt+a"));
    assert_eq!(parsed_key("\x1ba"), None);
    assert!(!matches_key("\x1b1", "alt+1"));
    assert_eq!(parsed_key("\x1b1"), None);
    assert!(!matches_key("\x1by", "alt+y"));
    assert_eq!(parsed_key("\x1by"), None);
}

#[test]
fn legacy_arrows_function_keys_and_rxvt_modifier_sequences_match() {
    let _guard = TestGuard::new();

    assert!(matches_key("\x1b[A", "up"));
    assert!(matches_key("\x1b[B", "down"));
    assert!(matches_key("\x1b[C", "right"));
    assert!(matches_key("\x1b[D", "left"));
    assert_eq!(parsed_key("\x1b[A"), Some("up".to_string()));
    assert_eq!(parsed_key("\x1b[B"), Some("down".to_string()));
    assert_eq!(parsed_key("\x1b[C"), Some("right".to_string()));
    assert_eq!(parsed_key("\x1b[D"), Some("left".to_string()));

    assert!(matches_key("\x1bOA", "up"));
    assert!(matches_key("\x1bOB", "down"));
    assert!(matches_key("\x1bOC", "right"));
    assert!(matches_key("\x1bOD", "left"));
    assert!(matches_key("\x1bOH", "home"));
    assert!(matches_key("\x1bOF", "end"));
    assert_eq!(parsed_key("\x1bOA"), Some("up".to_string()));
    assert_eq!(parsed_key("\x1bOB"), Some("down".to_string()));
    assert_eq!(parsed_key("\x1bOC"), Some("right".to_string()));
    assert_eq!(parsed_key("\x1bOD"), Some("left".to_string()));
    assert_eq!(parsed_key("\x1bOH"), Some("home".to_string()));
    assert_eq!(parsed_key("\x1bOF"), Some("end".to_string()));

    assert!(matches_key("\x1bOP", "f1"));
    assert!(matches_key("\x1b[24~", "f12"));
    assert!(matches_key("\x1b[E", "clear"));
    assert_eq!(parsed_key("\x1bOP"), Some("f1".to_string()));
    assert_eq!(parsed_key("\x1b[24~"), Some("f12".to_string()));
    assert_eq!(parsed_key("\x1b[E"), Some("clear".to_string()));

    assert!(matches_key("\x1bp", "alt+up"));
    assert!(!matches_key("\x1bp", "up"));
    assert_eq!(parsed_key("\x1bp"), Some("alt+up".to_string()));

    assert!(matches_key("\x1b[a", "shift+up"));
    assert!(matches_key("\x1bOa", "ctrl+up"));
    assert!(matches_key("\x1b[2$", "shift+insert"));
    assert!(matches_key("\x1b[2^", "ctrl+insert"));
    assert!(matches_key("\x1b[7$", "shift+home"));
    assert_eq!(parsed_key("\x1b[2^"), Some("ctrl+insert".to_string()));
    assert_eq!(parsed_key("\x1b[[5~"), Some("pageUp".to_string()));
}

#[test]
fn kitty_printable_decoder_handles_keypad_symbols() {
    let _guard = TestGuard::new();

    assert_eq!(decode_kitty_printable("\x1b[57399u"), Some("0".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57400u"), Some("1".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57409u"), Some(".".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57410u"), Some("/".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57411u"), Some("*".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57412u"), Some("-".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57413u"), Some("+".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57415u"), Some("=".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57416u"), Some(",".to_string()));
    assert_eq!(decode_kitty_printable("\x1b[57417u"), None);
}

#[test]
fn kitty_release_and_repeat_detection_ignore_bracketed_paste_false_positives() {
    let _guard = TestGuard::new();

    assert!(is_key_release("\x1b[99;5:3u"));
    assert!(is_key_repeat("\x1b[99;5:2u"));
    assert!(!is_key_release("\x1b[200~90:62:3F:A5\x1b[201~"));
    assert!(!is_key_repeat("\x1b[200~90:62:2F:A5\x1b[201~"));
}

#[test]
fn kitty_mode_changes_linefeed_interpretation() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(true);

    assert!(matches_key("\n", "shift+enter"));
    assert!(!matches_key("\n", "enter"));
    assert_eq!(parsed_key("\n"), Some("shift+enter".to_string()));
}

#[test]
fn legacy_special_keys_and_plain_digits_parse() {
    let guard = TestGuard::new();
    guard.set_kitty_protocol_active(false);

    assert_eq!(parsed_key("\t"), Some("tab".to_string()));
    assert_eq!(parsed_key("\r"), Some("enter".to_string()));
    assert_eq!(parsed_key(" "), Some("space".to_string()));
    assert_eq!(parsed_key("1"), Some("1".to_string()));
    assert!(matches_key("1", "1"));
}
