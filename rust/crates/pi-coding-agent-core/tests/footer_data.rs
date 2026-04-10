use pi_coding_agent_core::FooterDataProvider;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
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

fn create_plain_repo(temp_dir: &Path, branch: &str) -> PathBuf {
    let repo_dir = temp_dir.join("repo");
    fs::create_dir_all(repo_dir.join(".git")).expect("failed to create repo .git dir");
    write_file(
        repo_dir.join(".git/HEAD"),
        &format!("ref: refs/heads/{branch}\n"),
    );
    repo_dir
}

fn create_plain_reftable_repo(temp_dir: &Path) -> PathBuf {
    let repo_dir = temp_dir.join("repo");
    fs::create_dir_all(repo_dir.join(".git/reftable")).expect("failed to create reftable dir");
    write_file(repo_dir.join(".git/HEAD"), "ref: refs/heads/.invalid\n");
    repo_dir
}

fn create_reftable_worktree(temp_dir: &Path) -> (PathBuf, PathBuf) {
    let repo_dir = temp_dir.join("repo");
    let common_git_dir = repo_dir.join(".git");
    let git_dir = common_git_dir.join("worktrees/src");
    let worktree_dir = temp_dir.join("worktree");
    let reftable_dir = common_git_dir.join("reftable");

    fs::create_dir_all(&git_dir).expect("failed to create worktree git dir");
    fs::create_dir_all(&reftable_dir).expect("failed to create reftable dir");
    fs::create_dir_all(&worktree_dir).expect("failed to create worktree dir");

    write_file(
        worktree_dir.join(".git"),
        &format!("gitdir: {}\n", git_dir.display()),
    );
    write_file(git_dir.join("HEAD"), "ref: refs/heads/.invalid\n");
    write_file(git_dir.join("commondir"), "../..\n");
    write_file(reftable_dir.join("tables.list"), "0\n");

    (worktree_dir, reftable_dir)
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct PathGuard {
    previous_path: Option<std::ffi::OsString>,
}

impl PathGuard {
    fn set(path: &Path) -> Self {
        let previous_path = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", path) };
        Self { previous_path }
    }

    fn clear() -> Self {
        let previous_path = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", "") };
        Self { previous_path }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        match &self.previous_path {
            Some(path) => unsafe { std::env::set_var("PATH", path) },
            None => unsafe { std::env::remove_var("PATH") },
        }
    }
}

#[cfg(unix)]
fn install_fake_git(bin_dir: &Path, branch: Option<&str>) {
    use std::os::unix::fs::PermissionsExt;

    let body = match branch {
        Some(branch) => format!(
            "#!/bin/sh\nif [ \"$1\" = \"--no-optional-locks\" ] && [ \"$2\" = \"symbolic-ref\" ] && [ \"$3\" = \"--quiet\" ] && [ \"$4\" = \"--short\" ] && [ \"$5\" = \"HEAD\" ]; then\n  printf '%s\\n' '{branch}'\n  exit 0\nfi\nexit 1\n"
        ),
        None => "#!/bin/sh\nexit 1\n".to_owned(),
    };
    let path = bin_dir.join("git");
    write_file(&path, &body);
    let mut permissions = fs::metadata(&path)
        .expect("failed to stat fake git")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("failed to chmod fake git");
}

#[test]
fn resolves_head_branch_directly_from_nested_repo_without_git_on_path() {
    let _env_guard = env_lock().lock().expect("env lock poisoned");
    let _path_guard = PathGuard::clear();
    let temp_dir = TestDir::new("footer-data-provider");
    let repo_dir = create_plain_repo(temp_dir.path(), "main");
    let nested_dir = repo_dir.join("src/nested");
    fs::create_dir_all(&nested_dir).expect("failed to create nested dir");

    let provider = FooterDataProvider::new(&nested_dir);

    assert_eq!(provider.get_git_branch().as_deref(), Some("main"));
}

#[cfg(unix)]
#[test]
fn resolves_invalid_reftable_head_via_git_for_plain_repo() {
    let _env_guard = env_lock().lock().expect("env lock poisoned");
    let temp_dir = TestDir::new("footer-data-provider");
    let bin_dir = temp_dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("failed to create bin dir");
    install_fake_git(&bin_dir, Some("main"));
    let _path_guard = PathGuard::set(&bin_dir);
    let repo_dir = create_plain_reftable_repo(temp_dir.path());

    let provider = FooterDataProvider::new(&repo_dir);

    assert_eq!(provider.get_git_branch().as_deref(), Some("main"));
}

#[cfg(unix)]
#[test]
fn resolves_invalid_reftable_head_via_git_for_worktree() {
    let _env_guard = env_lock().lock().expect("env lock poisoned");
    let temp_dir = TestDir::new("footer-data-provider");
    let bin_dir = temp_dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("failed to create bin dir");
    install_fake_git(&bin_dir, Some("main"));
    let _path_guard = PathGuard::set(&bin_dir);
    let (worktree_dir, _) = create_reftable_worktree(temp_dir.path());

    let provider = FooterDataProvider::new(&worktree_dir);

    assert_eq!(provider.get_git_branch().as_deref(), Some("main"));
}

#[cfg(unix)]
#[test]
fn treats_unresolved_invalid_reftable_head_as_detached() {
    let _env_guard = env_lock().lock().expect("env lock poisoned");
    let temp_dir = TestDir::new("footer-data-provider");
    let bin_dir = temp_dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("failed to create bin dir");
    install_fake_git(&bin_dir, None);
    let _path_guard = PathGuard::set(&bin_dir);
    let repo_dir = create_plain_reftable_repo(temp_dir.path());

    let provider = FooterDataProvider::new(&repo_dir);

    assert_eq!(provider.get_git_branch().as_deref(), Some("detached"));
}

#[test]
fn set_cwd_switches_the_repo_used_for_branch_detection() {
    let temp_dir = TestDir::new("footer-data-provider");
    let first_root = temp_dir.path().join("first");
    let second_root = temp_dir.path().join("second");
    let first_repo = create_plain_repo(&first_root, "main");
    let second_repo = create_plain_repo(&second_root, "feature");

    let mut provider = FooterDataProvider::new(first_repo.join("src"));
    fs::create_dir_all(provider.cwd()).expect("failed to create first nested cwd");
    assert_eq!(provider.get_git_branch().as_deref(), Some("main"));

    let second_nested = second_repo.join("nested/path");
    fs::create_dir_all(&second_nested).expect("failed to create second nested cwd");
    provider.set_cwd(&second_nested);

    assert_eq!(provider.get_git_branch().as_deref(), Some("feature"));
}

#[test]
fn snapshot_carries_extension_statuses_and_provider_count() {
    let temp_dir = TestDir::new("footer-data-provider");
    let repo_dir = create_plain_repo(temp_dir.path(), "main");
    let mut provider = FooterDataProvider::new(&repo_dir);
    provider.set_extension_status("z-last", Some("status\ttwo".to_owned()));
    provider.set_extension_status("a-first", Some("status\none".to_owned()));
    provider.set_available_provider_count(2);

    let snapshot = provider.snapshot();

    assert_eq!(snapshot.cwd, repo_dir.display().to_string());
    assert_eq!(snapshot.git_branch.as_deref(), Some("main"));
    assert_eq!(snapshot.available_provider_count, 2);
    assert_eq!(
        snapshot.extension_statuses.keys().collect::<Vec<_>>(),
        vec!["a-first", "z-last"]
    );
    assert_eq!(
        snapshot
            .extension_statuses
            .get("a-first")
            .map(String::as_str),
        Some("status\none")
    );
}
