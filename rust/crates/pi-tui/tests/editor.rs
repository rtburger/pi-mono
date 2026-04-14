use pi_tui::{
    AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, CURSOR_MARKER,
    CombinedAutocompleteProvider, Component, Editor, EditorCursor, EditorOptions, SlashCommand,
    apply_completion, visible_width, word_wrap_line,
};
use regex::Regex;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

static PASTE_MARKER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[paste #\d+ (\+\d+ lines|\d+ chars)\]").expect("valid paste marker regex")
});

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn undo(editor: &mut Editor) {
    editor.handle_input("\x1b[45;5u");
}

fn move_right(editor: &mut Editor, count: usize) {
    for _ in 0..count {
        editor.handle_input("\x1b[C");
    }
}

fn type_text(editor: &mut Editor, text: &str) {
    for character in text.chars() {
        let mut buffer = [0; 4];
        editor.handle_input(character.encode_utf8(&mut buffer));
    }
}

fn paste_with_marker(editor: &mut Editor, text: &str) {
    editor.handle_input(&format!("\x1b[200~{text}\x1b[201~"));
}

fn first_marker(text: &str) -> String {
    PASTE_MARKER_REGEX
        .find(text)
        .expect("expected paste marker")
        .as_str()
        .to_owned()
}

fn all_markers(text: &str) -> Vec<String> {
    PASTE_MARKER_REGEX
        .find_iter(text)
        .map(|marker| marker.as_str().to_owned())
        .collect()
}

fn slash_command_provider(commands: Vec<SlashCommand>) -> Arc<CombinedAutocompleteProvider> {
    Arc::new(CombinedAutocompleteProvider::new(
        commands,
        std::env::temp_dir(),
    ))
}

fn attachment_provider(base_path: &Path) -> Arc<CombinedAutocompleteProvider> {
    Arc::new(CombinedAutocompleteProvider::new(Vec::new(), base_path))
}

fn write_file(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(path, content).expect("failed to write file");
}

#[test]
fn backslash_enter_inserts_newline_instead_of_submitting() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_clone = Arc::clone(&submitted);

    let mut editor = Editor::new();
    editor.set_on_submit(move |value| {
        *submitted_clone.lock().expect("submitted mutex poisoned") = Some(value);
    });

    editor.handle_input("\\");
    editor.handle_input("\r");

    assert_eq!(editor.get_text(), "\n");
    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        None
    );
}

#[test]
fn submit_resets_editor_and_emits_trimmed_text() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_clone = Arc::clone(&submitted);

    let mut editor = Editor::new();
    editor.set_on_submit(move |value| {
        *submitted_clone.lock().expect("submitted mutex poisoned") = Some(value);
    });

    editor.handle_input(" ");
    editor.handle_input("h");
    editor.handle_input("i");
    editor.handle_input("\n");
    editor.handle_input("t");
    editor.handle_input("h");
    editor.handle_input("e");
    editor.handle_input("r");
    editor.handle_input("e");
    editor.handle_input(" ");
    editor.handle_input("\r");

    assert_eq!(editor.get_text(), "");
    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("hi\nthere")
    );
}

#[test]
fn backspace_at_start_of_line_merges_with_previous_line() {
    let mut editor = Editor::new();
    editor.set_text("foo\nbar");

    editor.handle_input("\x1b[D");
    editor.handle_input("\x1b[D");
    editor.handle_input("\x1b[D");
    editor.handle_input("\x7f");

    assert_eq!(editor.get_text(), "foobar");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 3 });
}

#[test]
fn history_navigation_supports_multiline_entries() {
    let mut editor = Editor::new();
    editor.add_to_history("older entry");
    editor.add_to_history("line1\nline2\nline3");

    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_text(), "line1\nline2\nline3");

    editor.handle_input("\x1b[A");
    editor.handle_input("\x1b[A");
    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_text(), "older entry");

    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_text(), "line1\nline2\nline3");

    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_text(), "");
}

#[test]
fn public_set_cursor_clamps_to_existing_line_and_column() {
    let mut editor = Editor::new();
    editor.set_text("alpha\nbeta");

    editor.set_cursor(EditorCursor { line: 99, col: 99 });
    assert_eq!(editor.get_cursor(), EditorCursor { line: 1, col: 4 });

    editor.set_cursor(EditorCursor { line: 0, col: 2 });
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 2 });
}

#[test]
fn word_wrap_line_preserves_boundary_whitespace_rules() {
    let chunks = word_wrap_line("hello world test", 11);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].text, "hello ");
    assert_eq!(chunks[1].text, "world test");

    let chunks = word_wrap_line("hello world test", 12);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].text, "hello world ");
    assert_eq!(chunks[1].text, "test");
}

#[test]
fn bracketed_paste_preserves_newlines_and_expands_tabs() {
    let mut editor = Editor::new();

    editor.handle_input("\x1b[200~foo\tbar\nbaz\x1b[201~");

    assert_eq!(editor.get_text(), "foo    bar\nbaz");
}

#[test]
fn render_wraps_wide_text_without_overflow_and_emits_cursor_marker() {
    let mut editor = Editor::with_options(EditorOptions { padding_x: 1 });
    editor.set_text("日本語テスト");
    editor.set_focused(true);
    editor.set_viewport_size(11, 24);

    let lines = editor.render(11);

    assert!(lines.iter().any(|line| line.contains(CURSOR_MARKER)));
    for line in &lines {
        assert!(
            visible_width(line) <= 11,
            "rendered line overflowed width: {line:?}"
        );
    }
}

#[test]
fn ctrl_w_kills_previous_word_and_ctrl_y_yanks_it() {
    let mut editor = Editor::new();
    editor.set_text("foo bar baz");

    editor.handle_input("\x17");
    assert_eq!(editor.get_text(), "foo bar ");

    editor.handle_input("\x01");
    editor.handle_input("\x19");
    assert_eq!(editor.get_text(), "bazfoo bar ");
}

#[test]
fn ctrl_k_kills_to_end_of_line_and_ctrl_y_restores_it() {
    let mut editor = Editor::new();
    editor.set_text("hello world");

    editor.handle_input("\x01");
    for _ in 0..6 {
        editor.handle_input("\x1b[C");
    }
    editor.handle_input("\x0b");
    assert_eq!(editor.get_text(), "hello ");

    editor.handle_input("\x19");
    assert_eq!(editor.get_text(), "hello world");
}

#[test]
fn alt_y_cycles_through_kill_ring_after_yank() {
    let mut editor = Editor::new();

    editor.set_text("first");
    editor.handle_input("\x17");
    editor.set_text("second");
    editor.handle_input("\x17");
    editor.set_text("third");
    editor.handle_input("\x17");

    editor.handle_input("\x19");
    assert_eq!(editor.get_text(), "third");

    editor.handle_input("\x1by");
    assert_eq!(editor.get_text(), "second");

    editor.handle_input("\x1by");
    assert_eq!(editor.get_text(), "first");

    editor.handle_input("\x1by");
    assert_eq!(editor.get_text(), "third");
}

#[test]
fn consecutive_ctrl_w_accumulates_multiline_kills_into_one_ring_entry() {
    let mut editor = Editor::new();
    editor.set_text("1\n2\n3");

    for _ in 0..5 {
        editor.handle_input("\x17");
    }

    assert_eq!(editor.get_text(), "");

    editor.handle_input("\x19");
    assert_eq!(editor.get_text(), "1\n2\n3");
}

#[test]
fn alt_d_accumulates_forward_word_kills_for_yank() {
    let mut editor = Editor::new();
    editor.set_text("hello world test");

    editor.handle_input("\x01");
    editor.handle_input("\x1bd");
    assert_eq!(editor.get_text(), " world test");

    editor.handle_input("\x1bd");
    assert_eq!(editor.get_text(), " test");

    editor.handle_input("\x19");
    assert_eq!(editor.get_text(), "hello world test");
}

#[test]
fn undo_is_a_no_op_when_the_stack_is_empty() {
    let mut editor = Editor::new();

    undo(&mut editor);

    assert_eq!(editor.get_text(), "");
}

#[test]
fn undo_coalesces_word_typing_but_keeps_spaces_as_separate_units() {
    let mut editor = Editor::new();
    type_text(&mut editor, "hello world");

    assert_eq!(editor.get_text(), "hello world");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "");
}

#[test]
fn undo_restores_newlines_backspace_and_forward_delete() {
    let mut editor = Editor::new();
    type_text(&mut editor, "hello");
    editor.handle_input("\n");
    type_text(&mut editor, "world");

    assert_eq!(editor.get_text(), "hello\nworld");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello\n");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello");

    editor.handle_input("\x7f");
    assert_eq!(editor.get_text(), "hell");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello");

    editor.handle_input("\x01");
    editor.handle_input("\x1b[C");
    editor.handle_input("\x1b[3~");
    assert_eq!(editor.get_text(), "hllo");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello");
}

#[test]
fn undo_restores_kill_and_yank_operations() {
    let mut editor = Editor::new();
    editor.set_text("hello world");

    editor.handle_input("\x17");
    assert_eq!(editor.get_text(), "hello ");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello world");

    editor.handle_input("\x01");
    move_right(&mut editor, 6);
    editor.handle_input("\x0b");
    assert_eq!(editor.get_text(), "hello ");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello world");

    editor.handle_input("\x15");
    assert_eq!(editor.get_text(), "world");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello world");

    editor.handle_input("\x05");
    editor.handle_input("\x17");
    editor.handle_input("\x19");
    assert_eq!(editor.get_text(), "hello world");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello ");
}

#[test]
fn undo_restores_multiline_paste_and_programmatic_insertions_atomically() {
    let mut editor = Editor::new();
    editor.set_text("hello world");
    editor.handle_input("\x01");
    move_right(&mut editor, 5);

    editor.handle_input("\x1b[200~line1\nline2\x1b[201~");
    assert_eq!(editor.get_text(), "helloline1\nline2 world");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello world");

    editor.insert_text_at_cursor("A\nB");
    assert_eq!(editor.get_text(), "helloA\nB world");
    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello world");
}

#[test]
fn undo_restores_programmatic_set_text_changes() {
    let mut editor = Editor::new();
    type_text(&mut editor, "hello world");

    editor.set_text("");
    assert_eq!(editor.get_text(), "");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello world");
}

#[test]
fn submit_clears_the_undo_stack() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

    let mut editor = Editor::new();
    editor.set_on_submit(move |value| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") = Some(value);
    });

    type_text(&mut editor, "hello");
    editor.handle_input("\r");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("hello")
    );
    assert_eq!(editor.get_text(), "");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "");
}

#[test]
fn undo_exits_history_browsing_and_restores_the_previous_buffer() {
    let mut editor = Editor::new();
    editor.add_to_history("hello");
    type_text(&mut editor, "world");

    editor.handle_input("\x17");
    assert_eq!(editor.get_text(), "");

    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_text(), "hello");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "world");
}

#[test]
fn cursor_movement_starts_a_new_undo_unit() {
    let mut editor = Editor::new();
    type_text(&mut editor, "hello world");

    for _ in 0..5 {
        editor.handle_input("\x1b[D");
    }

    type_text(&mut editor, "lol");
    assert_eq!(editor.get_text(), "hello lolworld");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello world");
}

#[test]
fn no_op_delete_operations_do_not_push_undo_snapshots() {
    let mut editor = Editor::new();
    type_text(&mut editor, "hello");

    editor.handle_input("\x17");
    assert_eq!(editor.get_text(), "");

    editor.handle_input("\x17");
    editor.handle_input("\x17");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "hello");
}

#[test]
fn jump_mode_moves_forward_to_the_next_matching_character() {
    let mut editor = Editor::new();
    editor.set_text("hello world");
    editor.handle_input("\x01");

    editor.handle_input("\x1d");
    editor.handle_input("o");

    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 4 });
}

#[test]
fn jump_mode_moves_backward_across_multiple_lines() {
    let mut editor = Editor::new();
    editor.set_text("abc\ndef\nghi");

    editor.handle_input("\x1b\x1d");
    editor.handle_input("a");

    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 0 });
}

#[test]
fn jump_mode_can_be_canceled_without_moving_the_cursor() {
    let mut editor = Editor::new();
    editor.set_text("hello world");
    editor.handle_input("\x01");

    editor.handle_input("\x1d");
    editor.handle_input("\x1b");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 0 });

    editor.handle_input("o");
    assert_eq!(editor.get_text(), "ohello world");
}

#[test]
fn jump_mode_repeat_shortcut_cancels_pending_jump() {
    let mut editor = Editor::new();
    editor.set_text("hello world");
    editor.handle_input("\x01");

    editor.handle_input("\x1d");
    editor.handle_input("\x1d");
    editor.handle_input("o");

    assert_eq!(editor.get_text(), "ohello world");
}

#[test]
fn jump_mode_leaves_the_cursor_in_place_when_no_match_exists() {
    let mut editor = Editor::new();
    editor.set_text("hello world");
    editor.handle_input("\x01");

    editor.handle_input("\x1d");
    editor.handle_input("z");

    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 0 });
}

#[test]
fn jump_mode_resets_typing_coalescing_for_undo() {
    let mut editor = Editor::new();
    editor.set_text("hello world");
    editor.handle_input("\x01");

    editor.handle_input("x");
    assert_eq!(editor.get_text(), "xhello world");

    editor.handle_input("\x1d");
    editor.handle_input("o");
    editor.handle_input("Y");
    assert_eq!(editor.get_text(), "xhellYo world");

    undo(&mut editor);
    assert_eq!(editor.get_text(), "xhello world");
}

#[test]
fn large_pastes_insert_a_marker_and_preserve_expanded_text() {
    let mut editor = Editor::new();
    let pasted_text = [
        "line 1",
        "line 2",
        "line 3",
        "line 4",
        "line 5",
        "line 6",
        "line 7",
        "line 8",
        "line 9",
        "line 10",
        "tokens $1 $2 $& $$ $` $' end",
    ]
    .join("\n");

    paste_with_marker(&mut editor, &pasted_text);

    assert!(PASTE_MARKER_REGEX.is_match(&editor.get_text()));
    assert_eq!(editor.get_expanded_text(), pasted_text);
}

#[test]
fn paste_markers_are_atomic_for_horizontal_cursor_movement() {
    let mut editor = Editor::new();
    editor.handle_input("A");
    paste_with_marker(&mut editor, &"line\n".repeat(20).trim_end().to_owned());
    editor.handle_input("B");

    let marker = first_marker(&editor.get_text());

    editor.handle_input("\x01");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 0 });

    editor.handle_input("\x1b[C");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 1 });

    editor.handle_input("\x1b[C");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: 1 + marker.len(),
        }
    );

    editor.handle_input("\x1b[C");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: 1 + marker.len() + 1,
        }
    );

    editor.handle_input("\x1b[D");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: 1 + marker.len(),
        }
    );

    editor.handle_input("\x1b[D");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 1 });
}

#[test]
fn paste_markers_are_atomic_for_backspace_delete_and_undo() {
    let mut editor = Editor::new();
    editor.handle_input("A");
    paste_with_marker(&mut editor, &"line\n".repeat(20).trim_end().to_owned());
    editor.handle_input("B");

    let original = editor.get_text();
    let marker = first_marker(&original);

    editor.handle_input("\x01");
    editor.handle_input("\x1b[C");
    editor.handle_input("\x1b[C");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: 1 + marker.len(),
        }
    );

    editor.handle_input("\x7f");
    assert_eq!(editor.get_text(), "AB");

    undo(&mut editor);
    assert_eq!(editor.get_text(), original);

    editor.handle_input("\x01");
    editor.handle_input("\x1b[C");
    editor.handle_input("\x1b[3~");
    assert_eq!(editor.get_text(), "AB");
}

#[test]
fn paste_markers_are_atomic_for_word_movement() {
    let mut editor = Editor::new();
    editor.handle_input("X");
    editor.handle_input(" ");
    paste_with_marker(&mut editor, &"line\n".repeat(20).trim_end().to_owned());
    editor.handle_input(" ");
    editor.handle_input("Y");

    let marker = first_marker(&editor.get_text());

    editor.handle_input("\x01");
    editor.handle_input("\x1b[1;5C");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 1 });

    editor.handle_input("\x1b[1;5C");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: 2 + marker.len(),
        }
    );
}

#[test]
fn multiple_paste_markers_remain_atomic() {
    let mut editor = Editor::new();
    paste_with_marker(&mut editor, &"line\n".repeat(20).trim_end().to_owned());
    editor.handle_input(" ");
    paste_with_marker(&mut editor, &"row\n".repeat(20).trim_end().to_owned());

    let markers = all_markers(&editor.get_text());
    assert_eq!(markers.len(), 2);

    editor.handle_input("\x01");
    editor.handle_input("\x1b[C");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: markers[0].len(),
        }
    );

    editor.handle_input("\x1b[C");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: markers[0].len() + 1,
        }
    );

    editor.handle_input("\x1b[C");
    assert_eq!(
        editor.get_cursor(),
        EditorCursor {
            line: 0,
            col: markers[0].len() + 1 + markers[1].len(),
        }
    );
}

#[test]
fn manually_typed_marker_like_text_is_not_atomic_without_a_valid_paste_id() {
    let mut editor = Editor::new();
    type_text(&mut editor, "[paste #99 +5 lines]");

    editor.handle_input("\x01");
    editor.handle_input("\x1b[C");

    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 1 });
}

#[test]
fn render_wraps_large_paste_markers_without_overflow() {
    let mut editor = Editor::new();
    paste_with_marker(&mut editor, &"line\n".repeat(47).trim_end().to_owned());

    let lines = editor.render(8);
    for line in &lines {
        assert!(
            visible_width(line) <= 8,
            "line exceeds width 8: visible={} text={line:?}",
            visible_width(line)
        );
    }
}

#[test]
fn submit_expands_large_paste_markers() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_for_callback = Arc::clone(&submitted);

    let mut editor = Editor::new();
    editor.set_on_submit(move |value| {
        *submitted_for_callback
            .lock()
            .expect("submitted mutex poisoned") = Some(value);
    });

    let pasted_text = [
        "line 1",
        "line 2",
        "line 3",
        "line 4",
        "line 5",
        "line 6",
        "line 7",
        "line 8",
        "line 9",
        "line 10",
        "tokens $1 $2 $& $$ $` $' end",
    ]
    .join("\n");
    paste_with_marker(&mut editor, &pasted_text);
    editor.handle_input("\r");

    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some(pasted_text.as_str())
    );
}

#[test]
fn typing_initial_slash_auto_triggers_command_autocomplete_and_backspacing_hides_it() {
    let mut editor = Editor::new();
    editor.set_autocomplete_provider(slash_command_provider(vec![
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
    ]));

    editor.handle_input("/");

    assert_eq!(editor.get_text(), "/");
    assert!(editor.is_showing_autocomplete());
    let lines = editor.render(40);
    assert!(lines.iter().any(|line| line.contains("model")));
    assert!(lines.iter().any(|line| line.contains("help")));

    editor.handle_input("\x7f");

    assert_eq!(editor.get_text(), "");
    assert!(!editor.is_showing_autocomplete());
}

#[test]
fn typing_in_prefilled_slash_argument_context_auto_triggers_argument_autocomplete() {
    let mut editor = Editor::new();
    editor.set_autocomplete_provider(slash_command_provider(vec![SlashCommand {
        name: String::from("load-skills"),
        description: Some(String::from("Load skills")),
        argument_completions: Some(Arc::new(|prefix| {
            prefix.starts_with('s').then_some(vec![AutocompleteItem {
                value: String::from("skill-a"),
                label: String::from("skill-a"),
                description: None,
            }])
        })),
    }]));
    editor.set_text("/load-skills ");

    editor.handle_input("s");

    assert!(editor.is_showing_autocomplete());
    editor.handle_input("\t");
    assert_eq!(editor.get_text(), "/load-skills skill-a");
    assert!(!editor.is_showing_autocomplete());
}

#[test]
fn enter_keeps_exact_typed_model_argument_when_autocomplete_is_showing() {
    let mut editor = Editor::new();
    editor.set_autocomplete_provider(slash_command_provider(vec![SlashCommand {
        name: String::from("model"),
        description: Some(String::from("Switch model")),
        argument_completions: Some(Arc::new(|prefix| {
            let items = ["gpt-4o", "gpt-4o-mini", "claude-sonnet"]
                .into_iter()
                .filter(|value| value.starts_with(prefix))
                .map(|value| AutocompleteItem {
                    value: value.to_owned(),
                    label: value.to_owned(),
                    description: None,
                })
                .collect::<Vec<_>>();
            (!items.is_empty()).then_some(items)
        })),
    }]));
    editor.set_text("/model ");
    type_text(&mut editor, "gpt-4o-mini");

    assert!(editor.is_showing_autocomplete());
    editor.handle_input("\r");

    assert_eq!(editor.get_text(), "/model gpt-4o-mini");
    assert!(!editor.is_showing_autocomplete());
}

#[test]
fn enter_submits_a_slash_command_name_after_accepting_completion() {
    let submitted = Arc::new(Mutex::new(None::<String>));
    let submitted_clone = Arc::clone(&submitted);

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(slash_command_provider(vec![
        SlashCommand {
            name: String::from("quit"),
            description: Some(String::from("Quit pi")),
            argument_completions: None,
        },
        SlashCommand {
            name: String::from("model"),
            description: Some(String::from("Switch model")),
            argument_completions: None,
        },
    ]));
    editor.set_on_submit(move |value| {
        *submitted_clone.lock().expect("submitted mutex poisoned") = Some(value);
    });
    type_text(&mut editor, "/quit");

    assert!(editor.is_showing_autocomplete());
    editor.handle_input("\r");

    assert_eq!(editor.get_text(), "");
    assert_eq!(
        submitted
            .lock()
            .expect("submitted mutex poisoned")
            .as_deref(),
        Some("/quit")
    );
}

#[test]
fn typing_initial_at_auto_triggers_attachment_autocomplete_and_backspacing_hides_it() {
    let temp_dir = TestDir::new("pi-editor-attachment");
    write_file(temp_dir.path().join("README.md"), "readme");

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(attachment_provider(temp_dir.path()));

    editor.handle_input("@");

    assert_eq!(editor.get_text(), "@");
    assert!(editor.is_showing_autocomplete());
    let lines = editor.render(40);
    assert!(lines.iter().any(|line| line.contains("README.md")));

    editor.handle_input("\x7f");

    assert_eq!(editor.get_text(), "");
    assert!(!editor.is_showing_autocomplete());
}

#[test]
fn typing_in_prefilled_attachment_context_auto_triggers_and_updates_autocomplete() {
    let temp_dir = TestDir::new("pi-editor-attachment-prefilled");
    write_file(temp_dir.path().join("src/main.rs"), "fn main() {}\n");
    write_file(temp_dir.path().join("src/other.rs"), "pub fn other() {}\n");

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(attachment_provider(temp_dir.path()));
    editor.set_text("@ma");

    editor.handle_input("i");

    assert_eq!(editor.get_text(), "@mai");
    assert!(editor.is_showing_autocomplete());
    let lines = editor.render(60);
    assert!(
        lines.iter().any(|line| line.contains("main.rs")),
        "lines: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("other.rs")),
        "lines: {lines:?}"
    );
}

#[test]
fn backspacing_attachment_query_retriggers_autocomplete_after_no_matches() {
    let temp_dir = TestDir::new("pi-editor-attachment-backspace");
    write_file(temp_dir.path().join("main.rs"), "fn main() {}\n");

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(attachment_provider(temp_dir.path()));

    editor.handle_input("@");
    assert!(editor.is_showing_autocomplete());

    editor.handle_input("z");
    assert_eq!(editor.get_text(), "@z");
    assert!(!editor.is_showing_autocomplete());

    editor.handle_input("\x7f");

    assert_eq!(editor.get_text(), "@");
    assert!(editor.is_showing_autocomplete());
    let lines = editor.render(40);
    assert!(lines.iter().any(|line| line.contains("main.rs")));
}

#[test]
fn typing_inside_accepted_quoted_attachment_directory_retriggers_autocomplete() {
    let temp_dir = TestDir::new("pi-editor-quoted-attachment-dir");
    write_file(temp_dir.path().join("my folder/target.txt"), "target\n");
    write_file(temp_dir.path().join("my folder/other.txt"), "other\n");

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(attachment_provider(temp_dir.path()));

    type_text(&mut editor, "@my");
    assert!(editor.is_showing_autocomplete());

    editor.handle_input("\t");

    assert_eq!(editor.get_text(), "@\"my folder/\"");
    assert!(!editor.is_showing_autocomplete());

    editor.handle_input("t");

    assert_eq!(editor.get_text(), "@\"my folder/t\"");
    assert!(editor.is_showing_autocomplete());
    let lines = editor.render(60);
    assert!(
        lines.iter().any(|line| line.contains("target.txt")),
        "lines: {lines:?}"
    );
}

#[test]
fn backspacing_quoted_attachment_query_retriggers_autocomplete_after_no_matches() {
    let temp_dir = TestDir::new("pi-editor-quoted-attachment-backspace");
    write_file(temp_dir.path().join("my folder/target.txt"), "target\n");

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(attachment_provider(temp_dir.path()));

    type_text(&mut editor, "@my");
    editor.handle_input("\t");
    assert_eq!(editor.get_text(), "@\"my folder/\"");

    editor.handle_input("z");
    assert_eq!(editor.get_text(), "@\"my folder/z\"");
    assert!(!editor.is_showing_autocomplete());

    editor.handle_input("\x7f");

    assert_eq!(editor.get_text(), "@\"my folder/\"");
    assert!(editor.is_showing_autocomplete());
    let lines = editor.render(60);
    assert!(
        lines.iter().any(|line| line.contains("target.txt")),
        "lines: {lines:?}"
    );
}

#[test]
fn force_tab_autocomplete_auto_applies_single_suggestion_and_undo_restores_prefix() {
    struct SingleSuggestionProvider;

    impl AutocompleteProvider for SingleSuggestionProvider {
        fn get_suggestions(
            &self,
            lines: &[String],
            _cursor_line: usize,
            cursor_col: usize,
            force: bool,
        ) -> Option<AutocompleteSuggestions> {
            if !force {
                return None;
            }
            let text = lines.first().map(String::as_str).unwrap_or("");
            let prefix = &text[..cursor_col.min(text.len())];
            (prefix == "rea").then(|| AutocompleteSuggestions {
                items: vec![pi_tui::AutocompleteItem {
                    value: String::from("README.md"),
                    label: String::from("README.md"),
                    description: None,
                }],
                prefix: String::from("rea"),
            })
        }

        fn apply_completion(
            &self,
            lines: &[String],
            cursor_line: usize,
            cursor_col: usize,
            item: &pi_tui::AutocompleteItem,
            prefix: &str,
        ) -> pi_tui::CompletionResult {
            apply_completion(lines, cursor_line, cursor_col, item, prefix)
        }
    }

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(Arc::new(SingleSuggestionProvider));

    type_text(&mut editor, "rea");
    editor.handle_input("\t");

    assert_eq!(editor.get_text(), "README.md");
    assert!(!editor.is_showing_autocomplete());

    undo(&mut editor);
    assert_eq!(editor.get_text(), "rea");
}

#[test]
fn autocomplete_menu_can_be_rendered_navigated_and_accepted() {
    struct MultiSuggestionProvider;

    impl AutocompleteProvider for MultiSuggestionProvider {
        fn get_suggestions(
            &self,
            lines: &[String],
            _cursor_line: usize,
            cursor_col: usize,
            force: bool,
        ) -> Option<AutocompleteSuggestions> {
            if !force {
                return None;
            }
            let text = lines.first().map(String::as_str).unwrap_or("");
            let prefix = &text[..cursor_col.min(text.len())];
            (prefix == "src").then(|| AutocompleteSuggestions {
                items: vec![
                    pi_tui::AutocompleteItem {
                        value: String::from("src/"),
                        label: String::from("src/"),
                        description: None,
                    },
                    pi_tui::AutocompleteItem {
                        value: String::from("src.txt"),
                        label: String::from("src.txt"),
                        description: None,
                    },
                ],
                prefix: String::from("src"),
            })
        }

        fn apply_completion(
            &self,
            lines: &[String],
            cursor_line: usize,
            cursor_col: usize,
            item: &pi_tui::AutocompleteItem,
            prefix: &str,
        ) -> pi_tui::CompletionResult {
            apply_completion(lines, cursor_line, cursor_col, item, prefix)
        }
    }

    let mut editor = Editor::new();
    editor.set_autocomplete_provider(Arc::new(MultiSuggestionProvider));

    type_text(&mut editor, "src");
    editor.handle_input("\t");

    assert!(editor.is_showing_autocomplete());
    let lines = editor.render(20);
    assert!(lines.iter().any(|line| line.contains("src/")));
    assert!(lines.iter().any(|line| line.contains("src.txt")));

    editor.handle_input("\x1b[B");
    editor.handle_input("\t");

    assert_eq!(editor.get_text(), "src.txt");
    assert!(!editor.is_showing_autocomplete());
}

#[test]
fn horizontal_movement_resets_sticky_column_for_future_vertical_navigation() {
    let mut editor = Editor::new();
    editor.set_text("1234567890\n\n1234567890");

    editor.handle_input("\x01");
    move_right(&mut editor, 5);
    assert_eq!(editor.get_cursor(), EditorCursor { line: 2, col: 5 });

    editor.handle_input("\x1b[A");
    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 5 });

    editor.handle_input("\x1b[D");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 4 });

    editor.handle_input("\x1b[B");
    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 2, col: 4 });
}

#[test]
fn right_at_end_of_last_line_updates_sticky_column_for_later_vertical_moves() {
    let mut editor = Editor::new();
    editor.set_text("111111111x1111111111\n\n333333333_");

    editor.handle_input("\x1b[A");
    editor.handle_input("\x1b[A");
    editor.handle_input("\x05");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 20 });

    editor.handle_input("\x1b[B");
    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 2, col: 10 });

    editor.handle_input("\x1b[C");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 2, col: 10 });

    editor.handle_input("\x1b[A");
    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 10 });
}

#[test]
fn sticky_column_survives_resize_when_preferred_column_is_on_another_line() {
    let mut editor = Editor::new();
    editor.set_text("short\n12345678901234567890");

    editor.handle_input("\x01");
    move_right(&mut editor, 15);
    assert_eq!(editor.get_cursor(), EditorCursor { line: 1, col: 15 });

    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 5 });

    editor.render(10);
    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 1, col: 8 });

    editor.handle_input("\x1b[A");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 0, col: 5 });

    editor.render(80);
    editor.handle_input("\x1b[B");
    assert_eq!(editor.get_cursor(), EditorCursor { line: 1, col: 15 });
}
