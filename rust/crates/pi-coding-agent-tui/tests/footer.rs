use pi_coding_agent_core::{FooterDataProvider, FooterDataSnapshot};
use pi_coding_agent_tui::{FooterComponent, FooterState};
use pi_events::Model;
use pi_tui::{Component, visible_width};
use std::{
    collections::BTreeMap,
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

fn create_plain_repo(temp_dir: &Path, repo_name: &str, branch: &str) -> PathBuf {
    let repo_dir = temp_dir.join(repo_name);
    fs::create_dir_all(repo_dir.join(".git")).expect("failed to create repo .git dir");
    write_file(
        repo_dir.join(".git/HEAD"),
        &format!("ref: refs/heads/{branch}\n"),
    );
    repo_dir
}

fn model(id: &str, provider: &str, reasoning: bool) -> Model {
    Model {
        id: id.to_owned(),
        name: id.to_owned(),
        api: "openai-responses".to_owned(),
        provider: provider.to_owned(),
        base_url: String::new(),
        reasoning,
        input: vec!["text".to_owned()],
        cost: pi_events::ModelCost {
            input: 1.0,
            output: 1.0,
            cache_read: 0.1,
            cache_write: 0.1,
        },
        context_window: 200_000,
        max_tokens: 8_192,
        compat: None,
    }
}

#[test]
fn footer_lines_stay_within_width_for_wide_session_names() {
    let footer = FooterComponent::new(FooterState {
        cwd: "/tmp/project".to_owned(),
        git_branch: Some("main".to_owned()),
        session_name: Some("한글".repeat(30)),
        model: Some(model("test-model", "test", false)),
        context_window: 200_000,
        context_percent: Some(12.3),
        ..FooterState::default()
    });

    for line in footer.render(93) {
        assert!(visible_width(&line) <= 93);
    }
}

#[test]
fn footer_lines_stay_within_width_for_wide_model_and_provider_names() {
    let footer = FooterComponent::new(FooterState {
        cwd: "/tmp/project".to_owned(),
        git_branch: Some("main".to_owned()),
        model: Some(model(&"模".repeat(30), "공급자", true)),
        thinking_level: "high".to_owned(),
        usage_input: 12_345,
        usage_output: 6_789,
        total_cost: 1.234,
        context_window: 200_000,
        context_percent: Some(12.3),
        available_provider_count: 2,
        ..FooterState::default()
    });

    for line in footer.render(60) {
        assert!(visible_width(&line) <= 60);
    }
}

#[test]
fn footer_sorts_and_sanitizes_extension_statuses() {
    let mut extension_statuses = BTreeMap::new();
    extension_statuses.insert("z-last".to_owned(), "status\ttwo".to_owned());
    extension_statuses.insert("a-first".to_owned(), "status\none".to_owned());

    let footer = FooterComponent::new(FooterState {
        cwd: "/tmp/project".to_owned(),
        model: Some(model("gpt-5", "openai", false)),
        context_window: 200_000,
        context_percent: Some(12.3),
        extension_statuses,
        ..FooterState::default()
    });

    let lines = footer.render(80);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2], "status one status two");
}

#[test]
fn footer_can_apply_core_data_snapshot_without_losing_session_fields() {
    let mut extension_statuses = BTreeMap::new();
    extension_statuses.insert("a-first".to_owned(), "status\none".to_owned());

    let snapshot = FooterDataSnapshot {
        cwd: "/tmp/project".to_owned(),
        git_branch: Some("main".to_owned()),
        available_provider_count: 2,
        extension_statuses,
    };

    let footer = FooterComponent::new(FooterState {
        model: Some(model("gpt-5", "openai", true)),
        thinking_level: "high".to_owned(),
        usage_input: 12_345,
        usage_output: 6_789,
        total_cost: 1.234,
        context_window: 200_000,
        context_percent: Some(12.3),
        ..FooterState::default()
    });
    footer.apply_data_snapshot(&snapshot);

    let lines = footer.render(80);
    assert_eq!(footer.state().cwd, "/tmp/project");
    assert_eq!(footer.state().git_branch.as_deref(), Some("main"));
    assert_eq!(footer.state().available_provider_count, 2);
    assert!(lines[0].contains("/tmp/project (main)"));
    assert!(lines[1].contains("(openai) gpt-5 • high"));
    assert_eq!(lines[2], "status one");
}

#[test]
fn footer_can_bind_live_data_provider_and_refresh_on_cwd_change() {
    let temp_dir = TestDir::new("footer-component");
    let first_repo = create_plain_repo(temp_dir.path(), "first", "main");
    let second_repo = create_plain_repo(temp_dir.path(), "second", "feature");
    let provider = FooterDataProvider::new(&first_repo);
    let footer = FooterComponent::new(FooterState {
        model: Some(model("gpt-5", "openai", true)),
        thinking_level: "high".to_owned(),
        context_window: 200_000,
        context_percent: Some(12.3),
        ..FooterState::default()
    });

    footer.bind_data_provider(&provider);
    let initial_lines = footer.render(80);
    assert!(
        initial_lines
            .iter()
            .any(|line| line.contains("first (main)"))
    );

    provider.set_cwd(&second_repo);
    let updated_lines = footer.render(80);

    assert!(
        updated_lines
            .iter()
            .any(|line| line.contains("second (feature)"))
    );
    assert!(
        updated_lines
            .iter()
            .any(|line| line.contains("gpt-5 • high"))
    );
}
