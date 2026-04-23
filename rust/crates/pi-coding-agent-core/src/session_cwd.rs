use crate::SessionManager;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCwdIssue {
    pub session_file: Option<String>,
    pub session_cwd: String,
    pub fallback_cwd: String,
}

pub fn get_missing_session_cwd_issue(
    session_manager: &SessionManager,
    fallback_cwd: impl AsRef<Path>,
) -> Option<SessionCwdIssue> {
    let session_file = session_manager.get_session_file().map(str::to_owned);
    if session_file.is_none() {
        return None;
    }

    let session_cwd = session_manager.get_cwd().to_owned();
    if session_cwd.is_empty() || Path::new(&session_cwd).exists() {
        return None;
    }

    Some(SessionCwdIssue {
        session_file,
        session_cwd,
        fallback_cwd: fallback_cwd.as_ref().to_string_lossy().into_owned(),
    })
}

pub fn format_missing_session_cwd_error(issue: &SessionCwdIssue) -> String {
    let session_file = issue
        .session_file
        .as_ref()
        .map(|session_file| format!("\nSession file: {session_file}"))
        .unwrap_or_default();
    format!(
        "Stored session working directory does not exist: {}{}\nCurrent working directory: {}",
        issue.session_cwd, session_file, issue.fallback_cwd
    )
}

pub fn format_missing_session_cwd_prompt(issue: &SessionCwdIssue) -> String {
    format!(
        "cwd from session file does not exist\n{}\n\ncontinue in current cwd\n{}",
        issue.session_cwd, issue.fallback_cwd
    )
}

pub fn assert_session_cwd_exists(
    session_manager: &SessionManager,
    fallback_cwd: impl AsRef<Path>,
) -> Result<(), SessionCwdIssue> {
    get_missing_session_cwd_issue(session_manager, fallback_cwd).map_or(Ok(()), Err)
}

impl std::fmt::Display for SessionCwdIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&format_missing_session_cwd_error(self))
    }
}

impl std::error::Error for SessionCwdIssue {}
