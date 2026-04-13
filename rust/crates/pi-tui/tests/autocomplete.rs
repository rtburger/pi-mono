use pi_tui::{AutocompleteProvider, CombinedAutocompleteProvider};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

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

fn write_file(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent dir");
    }
    fs::write(path, content).expect("failed to write file");
}

#[test]
fn force_completion_lists_current_directory_entries() {
    let temp_dir = TestDir::new("pi-autocomplete");
    fs::create_dir_all(temp_dir.path().join("src")).expect("failed to create dir");
    write_file(temp_dir.path().join("README.md"), "readme");

    let provider = CombinedAutocompleteProvider::new(Vec::new(), temp_dir.path());
    let suggestions = provider
        .get_suggestions(&[String::new()], 0, 0, true)
        .expect("expected forced suggestions");
    let values = suggestions
        .items
        .iter()
        .map(|item| item.value.as_str())
        .collect::<Vec<_>>();

    assert!(values.contains(&"README.md"));
    assert!(values.contains(&"src/"));
}

#[test]
fn at_completion_searches_nested_paths_and_excludes_git() {
    let temp_dir = TestDir::new("pi-autocomplete-at");
    write_file(temp_dir.path().join("src/main.rs"), "fn main() {}");
    write_file(temp_dir.path().join(".git/config"), "[core]");

    let provider = CombinedAutocompleteProvider::new(Vec::new(), temp_dir.path());
    let line = String::from("@main");
    let suggestions = provider
        .get_suggestions(std::slice::from_ref(&line), 0, line.len(), false)
        .expect("expected @ suggestions");
    let values = suggestions
        .items
        .iter()
        .map(|item| item.value.as_str())
        .collect::<Vec<_>>();

    assert!(values.contains(&"@src/main.rs"));
    assert!(!values.iter().any(|value| value.starts_with("@.git")));
}

#[test]
fn quoted_completion_reuses_existing_closing_quote() {
    let temp_dir = TestDir::new("pi-autocomplete-quoted");
    write_file(temp_dir.path().join("my folder/test.txt"), "content");

    let provider = CombinedAutocompleteProvider::new(Vec::new(), temp_dir.path());
    let line = String::from("\"my folder/te\"");
    let cursor_col = line.len() - 1;
    let suggestions = provider
        .get_suggestions(std::slice::from_ref(&line), 0, cursor_col, true)
        .expect("expected quoted suggestions");
    let item = suggestions
        .items
        .iter()
        .find(|item| item.value == "\"my folder/test.txt\"")
        .expect("expected quoted file suggestion");

    let applied = provider.apply_completion(
        std::slice::from_ref(&line),
        0,
        cursor_col,
        item,
        &suggestions.prefix,
    );
    assert_eq!(applied.lines[0], "\"my folder/test.txt\"");
}
