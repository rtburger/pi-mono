use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterDataSnapshot {
    pub cwd: String,
    pub git_branch: Option<String>,
    pub available_provider_count: usize,
    pub extension_statuses: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterDataProvider {
    cwd: PathBuf,
    extension_statuses: BTreeMap<String, String>,
    available_provider_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitPaths {
    repo_dir: PathBuf,
    head_path: PathBuf,
}

impl FooterDataProvider {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            extension_statuses: BTreeMap::new(),
            available_provider_count: 0,
        }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn set_cwd(&mut self, cwd: impl Into<PathBuf>) {
        self.cwd = cwd.into();
    }

    pub fn get_git_branch(&self) -> Option<String> {
        resolve_git_branch(&self.cwd)
    }

    pub fn get_extension_statuses(&self) -> &BTreeMap<String, String> {
        &self.extension_statuses
    }

    pub fn set_extension_status(&mut self, key: impl Into<String>, text: Option<String>) {
        let key = key.into();
        match text {
            Some(text) => {
                self.extension_statuses.insert(key, text);
            }
            None => {
                self.extension_statuses.remove(&key);
            }
        }
    }

    pub fn clear_extension_statuses(&mut self) {
        self.extension_statuses.clear();
    }

    pub fn get_available_provider_count(&self) -> usize {
        self.available_provider_count
    }

    pub fn set_available_provider_count(&mut self, count: usize) {
        self.available_provider_count = count;
    }

    pub fn snapshot(&self) -> FooterDataSnapshot {
        FooterDataSnapshot {
            cwd: self.cwd.display().to_string(),
            git_branch: self.get_git_branch(),
            available_provider_count: self.available_provider_count,
            extension_statuses: self.extension_statuses.clone(),
        }
    }
}

fn resolve_git_branch(cwd: &Path) -> Option<String> {
    let git_paths = find_git_paths(cwd)?;
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
                let _ = common_git_dir;
                return Some(GitPaths {
                    repo_dir: dir.to_path_buf(),
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
