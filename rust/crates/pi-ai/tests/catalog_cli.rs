use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const BIN: &str = env!("CARGO_BIN_EXE_pi-ai-catalog");

#[test]
fn check_succeeds_for_repo_catalog() {
    let output = Command::new(BIN).arg("check").output().unwrap();
    assert_success(&output, "expected repo catalog check to succeed");
}

#[test]
fn fmt_rewrites_non_canonical_catalog_and_check_passes() {
    let temp_dir = unique_temp_dir("catalog-fmt");
    let catalog_path = temp_dir.join("models.catalog.json");
    fs::write(&catalog_path, sample_catalog_json()).unwrap();

    let check_before = Command::new(BIN)
        .args(["check", &catalog_path.to_string_lossy()])
        .output()
        .unwrap();
    assert!(
        !check_before.status.success(),
        "expected non-canonical catalog check to fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&check_before.stdout),
        String::from_utf8_lossy(&check_before.stderr)
    );
    assert!(
        String::from_utf8_lossy(&check_before.stderr).contains("not in canonical format"),
        "expected canonical format error\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&check_before.stdout),
        String::from_utf8_lossy(&check_before.stderr)
    );

    let fmt_output = Command::new(BIN)
        .args(["fmt", &catalog_path.to_string_lossy()])
        .output()
        .unwrap();
    assert_success(&fmt_output, "expected fmt to rewrite catalog");

    let check_after = Command::new(BIN)
        .args(["check", &catalog_path.to_string_lossy()])
        .output()
        .unwrap();
    assert_success(&check_after, "expected canonical catalog check to succeed");

    let rewritten = fs::read_to_string(&catalog_path).unwrap();
    assert!(rewritten.starts_with("{\n  \"anthropic\""));
    assert!(rewritten.ends_with("}\n"));

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn check_fails_when_model_key_does_not_match_inner_id() {
    let temp_dir = unique_temp_dir("catalog-invalid-id");
    let catalog_path = temp_dir.join("models.catalog.json");
    fs::write(&catalog_path, invalid_model_key_catalog_json()).unwrap();

    let output = Command::new(BIN)
        .args(["check", &catalog_path.to_string_lossy()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "expected invalid catalog check to fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("model key `wrong-openai-id` does not match model id `gpt-5.4`"),
        "expected model id mismatch error\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    fs::remove_dir_all(temp_dir).unwrap();
}

fn assert_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "pi-ai-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn sample_catalog_json() -> String {
    serde_json::to_string(&json!({
        "openai": {
            "gpt-5.4": model_entry("gpt-5.4", "GPT-5.4", "openai-responses", "openai", "https://api.openai.com/v1", true, ["text", "image"], 2.5, 15.0, 0.25, 0.0, 272000, 128000)
        },
        "anthropic": {
            "claude-opus-4-6": model_entry("claude-opus-4-6", "Claude Opus 4.6", "anthropic-messages", "anthropic", "https://api.anthropic.com", true, ["text", "image"], 5.0, 25.0, 0.5, 6.25, 1000000, 128000)
        },
        "openai-codex": {
            "gpt-5.4": model_entry("gpt-5.4", "GPT-5.4", "openai-codex-responses", "openai-codex", "https://chatgpt.com/backend-api", true, ["text", "image"], 2.5, 15.0, 0.25, 0.0, 272000, 128000)
        }
    }))
    .unwrap()
}

fn invalid_model_key_catalog_json() -> String {
    serde_json::to_string_pretty(&json!({
        "anthropic": {
            "claude-opus-4-6": model_entry("claude-opus-4-6", "Claude Opus 4.6", "anthropic-messages", "anthropic", "https://api.anthropic.com", true, ["text", "image"], 5.0, 25.0, 0.5, 6.25, 1000000, 128000)
        },
        "openai": {
            "wrong-openai-id": model_entry("gpt-5.4", "GPT-5.4", "openai-responses", "openai", "https://api.openai.com/v1", true, ["text", "image"], 2.5, 15.0, 0.25, 0.0, 272000, 128000)
        },
        "openai-codex": {
            "gpt-5.4": model_entry("gpt-5.4", "GPT-5.4", "openai-codex-responses", "openai-codex", "https://chatgpt.com/backend-api", true, ["text", "image"], 2.5, 15.0, 0.25, 0.0, 272000, 128000)
        }
    }))
    .unwrap()
}

#[expect(clippy::too_many_arguments, reason = "test helper keeps catalog fixtures readable")]
fn model_entry(
    id: &str,
    name: &str,
    api: &str,
    provider: &str,
    base_url: &str,
    reasoning: bool,
    input: [&str; 2],
    input_cost: f64,
    output_cost: f64,
    cache_read_cost: f64,
    cache_write_cost: f64,
    context_window: u64,
    max_tokens: u64,
) -> serde_json::Value {
    json!({
        "id": id,
        "name": name,
        "api": api,
        "provider": provider,
        "baseUrl": base_url,
        "reasoning": reasoning,
        "input": input,
        "cost": {
            "input": input_cost,
            "output": output_cost,
            "cacheRead": cache_read_cost,
            "cacheWrite": cache_write_cost,
        },
        "contextWindow": context_window,
        "maxTokens": max_tokens,
    })
}

