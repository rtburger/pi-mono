use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);
const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterDataSnapshot {
    pub cwd: String,
    pub git_branch: Option<String>,
    pub available_provider_count: usize,
    pub extension_statuses: BTreeMap<String, String>,
}

pub struct FooterDataProvider {
    inner: Arc<FooterDataInner>,
}

pub struct BranchChangeSubscription {
    callback_id: Option<usize>,
    inner: Weak<FooterDataInner>,
}

struct FooterDataInner {
    state: Mutex<FooterDataState>,
    state_changed: Condvar,
    callbacks: Mutex<BTreeMap<usize, Arc<dyn Fn() + Send + Sync>>>,
    next_callback_id: AtomicUsize,
    watcher_handle: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug)]
struct FooterDataState {
    cwd: PathBuf,
    git_paths: Option<GitPaths>,
    cached_branch: CachedBranch,
    extension_statuses: BTreeMap<String, String>,
    available_provider_count: usize,
    generation: u64,
    disposed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitPaths {
    repo_dir: PathBuf,
    common_git_dir: PathBuf,
    head_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CachedBranch {
    Unknown,
    Value(Option<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GitWatchState {
    head_contents: Option<String>,
    reftable_entries: Option<Vec<String>>,
    tables_list_contents: Option<String>,
}

impl FooterDataProvider {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        let cwd = cwd.into();
        let inner = Arc::new(FooterDataInner {
            state: Mutex::new(FooterDataState {
                git_paths: find_git_paths(&cwd),
                cwd,
                cached_branch: CachedBranch::Unknown,
                extension_statuses: BTreeMap::new(),
                available_provider_count: 0,
                generation: 0,
                disposed: false,
            }),
            state_changed: Condvar::new(),
            callbacks: Mutex::new(BTreeMap::new()),
            next_callback_id: AtomicUsize::new(1),
            watcher_handle: Mutex::new(None),
        });

        let watcher_inner = Arc::clone(&inner);
        let handle = thread::Builder::new()
            .name("pi-footer-data-watcher".to_owned())
            .spawn(move || watch_git_branch_changes(watcher_inner))
            .expect("failed to spawn footer data watcher thread");
        *inner
            .watcher_handle
            .lock()
            .expect("watcher handle mutex poisoned") = Some(handle);

        Self { inner }
    }

    pub fn cwd(&self) -> PathBuf {
        self.inner
            .state
            .lock()
            .expect("footer data state mutex poisoned")
            .cwd
            .clone()
    }

    pub fn set_cwd(&self, cwd: impl Into<PathBuf>) {
        let cwd = cwd.into();
        {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("footer data state mutex poisoned");
            if state.cwd == cwd {
                return;
            }
            state.cwd = cwd.clone();
            state.git_paths = find_git_paths(&cwd);
            state.cached_branch = CachedBranch::Unknown;
            state.generation = state.generation.wrapping_add(1);
        }
        self.inner.state_changed.notify_all();
        self.inner.notify_branch_change();
    }

    pub fn get_git_branch(&self) -> Option<String> {
        loop {
            let (generation, git_paths, cached_branch) = {
                let state = self
                    .inner
                    .state
                    .lock()
                    .expect("footer data state mutex poisoned");
                (
                    state.generation,
                    state.git_paths.clone(),
                    state.cached_branch.clone(),
                )
            };

            if let CachedBranch::Value(branch) = cached_branch {
                return branch;
            }

            let resolved = resolve_git_branch_from_paths(git_paths.as_ref());
            let mut state = self
                .inner
                .state
                .lock()
                .expect("footer data state mutex poisoned");
            if state.generation != generation {
                continue;
            }
            state.cached_branch = CachedBranch::Value(resolved.clone());
            return resolved;
        }
    }

    pub fn get_extension_statuses(&self) -> BTreeMap<String, String> {
        self.inner
            .state
            .lock()
            .expect("footer data state mutex poisoned")
            .extension_statuses
            .clone()
    }

    pub fn set_extension_status(&self, key: impl Into<String>, text: Option<String>) {
        let key = key.into();
        let mut state = self
            .inner
            .state
            .lock()
            .expect("footer data state mutex poisoned");
        match text {
            Some(text) => {
                state.extension_statuses.insert(key, text);
            }
            None => {
                state.extension_statuses.remove(&key);
            }
        }
    }

    pub fn clear_extension_statuses(&self) {
        self.inner
            .state
            .lock()
            .expect("footer data state mutex poisoned")
            .extension_statuses
            .clear();
    }

    pub fn get_available_provider_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .expect("footer data state mutex poisoned")
            .available_provider_count
    }

    pub fn set_available_provider_count(&self, count: usize) {
        self.inner
            .state
            .lock()
            .expect("footer data state mutex poisoned")
            .available_provider_count = count;
    }

    pub fn on_branch_change<F>(&self, callback: F) -> BranchChangeSubscription
    where
        F: Fn() + Send + Sync + 'static,
    {
        let callback_id = self.inner.next_callback_id.fetch_add(1, Ordering::Relaxed);
        self.inner
            .callbacks
            .lock()
            .expect("footer data callback mutex poisoned")
            .insert(callback_id, Arc::new(callback));
        BranchChangeSubscription {
            callback_id: Some(callback_id),
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub fn on_snapshot_change<F>(&self, callback: F) -> BranchChangeSubscription
    where
        F: Fn(FooterDataSnapshot) + Send + Sync + 'static,
    {
        let weak_inner = Arc::downgrade(&self.inner);
        self.on_branch_change(move || {
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            callback(snapshot_from_inner(&inner));
        })
    }

    pub fn snapshot(&self) -> FooterDataSnapshot {
        snapshot_from_inner(&self.inner)
    }

    pub fn dispose(&self) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("footer data state mutex poisoned");
            if state.disposed {
                return;
            }
            state.disposed = true;
        }
        self.inner.state_changed.notify_all();

        if let Some(handle) = self
            .inner
            .watcher_handle
            .lock()
            .expect("watcher handle mutex poisoned")
            .take()
        {
            let _ = handle.join();
        }

        self.inner
            .callbacks
            .lock()
            .expect("footer data callback mutex poisoned")
            .clear();
    }
}

impl Drop for FooterDataProvider {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl FooterDataInner {
    fn notify_branch_change(&self) {
        let callbacks = self
            .callbacks
            .lock()
            .expect("footer data callback mutex poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for callback in callbacks {
            callback();
        }
    }

    fn remove_callback(&self, callback_id: usize) {
        self.callbacks
            .lock()
            .expect("footer data callback mutex poisoned")
            .remove(&callback_id);
    }
}

impl BranchChangeSubscription {
    pub fn unsubscribe(&mut self) {
        let Some(callback_id) = self.callback_id.take() else {
            return;
        };
        if let Some(inner) = self.inner.upgrade() {
            inner.remove_callback(callback_id);
        }
    }
}

impl Drop for BranchChangeSubscription {
    fn drop(&mut self) {
        self.unsubscribe();
    }
}

fn snapshot_from_inner(inner: &FooterDataInner) -> FooterDataSnapshot {
    loop {
        let (
            generation,
            cwd,
            git_paths,
            cached_branch,
            extension_statuses,
            available_provider_count,
        ) = {
            let state = inner
                .state
                .lock()
                .expect("footer data state mutex poisoned");
            (
                state.generation,
                state.cwd.clone(),
                state.git_paths.clone(),
                state.cached_branch.clone(),
                state.extension_statuses.clone(),
                state.available_provider_count,
            )
        };

        let git_branch = match cached_branch {
            CachedBranch::Unknown => resolve_git_branch_from_paths(git_paths.as_ref()),
            CachedBranch::Value(branch) => branch,
        };

        let mut state = inner
            .state
            .lock()
            .expect("footer data state mutex poisoned");
        if state.generation != generation {
            continue;
        }
        if matches!(state.cached_branch, CachedBranch::Unknown) {
            state.cached_branch = CachedBranch::Value(git_branch.clone());
        }
        return FooterDataSnapshot {
            cwd: cwd.display().to_string(),
            git_branch,
            available_provider_count,
            extension_statuses,
        };
    }
}

fn watch_git_branch_changes(inner: Arc<FooterDataInner>) {
    let mut watched_generation = None;
    let mut watched_state = GitWatchState::default();
    let mut refresh_deadline = None;

    loop {
        let (generation, git_paths) = {
            let state = inner
                .state
                .lock()
                .expect("footer data state mutex poisoned");
            if state.disposed {
                return;
            }
            (state.generation, state.git_paths.clone())
        };

        if watched_generation != Some(generation) {
            watched_generation = Some(generation);
            watched_state = capture_git_watch_state(git_paths.as_ref());
            refresh_deadline = None;
        } else {
            let current_watch_state = capture_git_watch_state(git_paths.as_ref());
            if current_watch_state != watched_state {
                watched_state = current_watch_state;
                refresh_deadline = Some(Instant::now() + WATCH_DEBOUNCE);
            }
        }

        if let Some(deadline) = refresh_deadline {
            if Instant::now() >= deadline {
                refresh_deadline = None;
                let next_branch = resolve_git_branch_from_paths(git_paths.as_ref());
                let should_notify = {
                    let mut state = inner
                        .state
                        .lock()
                        .expect("footer data state mutex poisoned");
                    if state.disposed {
                        return;
                    }
                    if state.generation != generation {
                        false
                    } else {
                        let should_notify = matches!(
                            &state.cached_branch,
                            CachedBranch::Value(current_branch) if current_branch != &next_branch
                        );
                        state.cached_branch = CachedBranch::Value(next_branch.clone());
                        should_notify
                    }
                };

                if should_notify {
                    inner.notify_branch_change();
                }
                continue;
            }
        }

        let wait_duration = refresh_deadline
            .map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(WATCH_POLL_INTERVAL)
            })
            .unwrap_or(WATCH_POLL_INTERVAL);

        let state = inner
            .state
            .lock()
            .expect("footer data state mutex poisoned");
        if state.disposed {
            return;
        }
        let _ = inner
            .state_changed
            .wait_timeout(state, wait_duration)
            .expect("footer data condvar wait failed");
    }
}

fn resolve_git_branch_from_paths(git_paths: Option<&GitPaths>) -> Option<String> {
    let git_paths = git_paths?;
    let content = fs::read_to_string(&git_paths.head_path).ok()?;
    let head = content.trim();
    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        return if branch == ".invalid" {
            Some(
                resolve_branch_with_git_sync(&git_paths.repo_dir)
                    .unwrap_or_else(|| "detached".to_owned()),
            )
        } else {
            Some(branch.to_owned())
        };
    }
    Some("detached".to_owned())
}

fn capture_git_watch_state(git_paths: Option<&GitPaths>) -> GitWatchState {
    let Some(git_paths) = git_paths else {
        return GitWatchState::default();
    };

    let reftable_dir = git_paths.common_git_dir.join("reftable");
    let tables_list_path = reftable_dir.join("tables.list");

    GitWatchState {
        head_contents: fs::read_to_string(&git_paths.head_path).ok(),
        reftable_entries: read_sorted_directory_entries(&reftable_dir),
        tables_list_contents: fs::read_to_string(&tables_list_path).ok(),
    }
}

fn read_sorted_directory_entries(path: &Path) -> Option<Vec<String>> {
    if !path.exists() {
        return None;
    }
    let mut entries = fs::read_dir(path)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    entries.sort();
    Some(entries)
}

fn find_git_paths(cwd: &Path) -> Option<GitPaths> {
    let mut dir = cwd;
    loop {
        let git_path = dir.join(".git");
        if git_path.exists() {
            let metadata = fs::metadata(&git_path).ok()?;
            if metadata.is_file() {
                let content = fs::read_to_string(&git_path).ok()?;
                let git_dir = content
                    .trim()
                    .strip_prefix("gitdir: ")
                    .map(|path| resolve_path(dir, Path::new(path.trim())))?;
                let head_path = git_dir.join("HEAD");
                if !head_path.exists() {
                    return None;
                }
                let common_git_dir = {
                    let common_dir_path = git_dir.join("commondir");
                    if common_dir_path.exists() {
                        let common_dir = fs::read_to_string(common_dir_path).ok()?;
                        resolve_path(&git_dir, Path::new(common_dir.trim()))
                    } else {
                        git_dir.clone()
                    }
                };
                return Some(GitPaths {
                    repo_dir: dir.to_path_buf(),
                    common_git_dir,
                    head_path,
                });
            }
            if metadata.is_dir() {
                let head_path = git_path.join("HEAD");
                if !head_path.exists() {
                    return None;
                }
                return Some(GitPaths {
                    repo_dir: dir.to_path_buf(),
                    common_git_dir: git_path,
                    head_path,
                });
            }
        }

        let parent = dir.parent()?;
        dir = parent;
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn resolve_branch_with_git_sync(repo_dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg("symbolic-ref")
        .arg("--quiet")
        .arg("--short")
        .arg("HEAD")
        .current_dir(repo_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}
