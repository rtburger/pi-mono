use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub path: String,
    pub source: String,
    pub scope: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
}

impl SourceInfo {
    pub fn local(path: impl Into<String>, scope: &str, base_dir: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            source: String::from("local"),
            scope: scope.to_owned(),
            origin: String::from("top-level"),
            base_dir: Some(base_dir.into()),
        }
    }

    pub fn temporary(path: impl Into<String>, base_dir: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            source: String::from("local"),
            scope: String::from("temporary"),
            origin: String::from("top-level"),
            base_dir: Some(base_dir.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiagnostic {
    pub message: String,
    pub path: Option<String>,
}

impl ResourceDiagnostic {
    pub fn new(message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            message: message.into(),
            path,
        }
    }
}
