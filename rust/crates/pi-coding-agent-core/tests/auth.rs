use pi_coding_agent_core::{AuthFileSource, AuthSource, ChainedAuthSource, MemoryAuthStorage};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-auth-{prefix}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn auth_file_source_reads_api_key_entries() {
    let temp_dir = unique_temp_dir("api-key");
    let auth_path = temp_dir.join("auth.json");
    fs::write(
        &auth_path,
        serde_json::json!({
            "anthropic": {
                "type": "api_key",
                "key": "stored-token"
            }
        })
        .to_string(),
    )
    .unwrap();

    let auth = AuthFileSource::new(auth_path);

    assert!(auth.has_auth("anthropic"));
    assert_eq!(
        auth.get_api_key("anthropic").as_deref(),
        Some("stored-token")
    );
}

#[test]
fn auth_file_source_reads_openai_codex_oauth_credentials() {
    let temp_dir = unique_temp_dir("oauth");
    let auth_path = temp_dir.join("auth.json");
    fs::write(
        &auth_path,
        serde_json::json!({
            "openai-codex": {
                "type": "oauth",
                "access": "oauth-access-token",
                "refresh": "oauth-refresh-token",
                "expires": i64::MAX,
                "accountId": "acc_123"
            }
        })
        .to_string(),
    )
    .unwrap();

    let auth = AuthFileSource::new(auth_path);

    assert!(auth.has_auth("openai-codex"));
    assert_eq!(
        auth.get_api_key("openai-codex").as_deref(),
        Some("oauth-access-token")
    );
}

#[test]
fn chained_auth_source_falls_back_to_later_sources() {
    let chained = ChainedAuthSource::new(vec![
        Arc::new(AuthFileSource::new(
            unique_temp_dir("missing").join("auth.json"),
        )),
        Arc::new(MemoryAuthStorage::with_api_keys([("openai", "env-token")])),
    ]);

    assert!(chained.has_auth("openai"));
    assert_eq!(chained.get_api_key("openai").as_deref(), Some("env-token"));
}
