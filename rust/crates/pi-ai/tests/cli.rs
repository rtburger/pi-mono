use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_pi-ai");

#[test]
fn help_lists_login_commands_and_providers() {
    let output = Command::new(BIN).arg("--help").output().unwrap();
    assert_success(&output, "expected --help to succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: pi-ai <command> [provider]"));
    assert!(stdout.contains("login [provider]"));
    assert!(stdout.contains("anthropic"));
    assert!(stdout.contains("openai-codex"));
}

#[test]
fn list_prints_available_oauth_providers() {
    let output = Command::new(BIN).arg("list").output().unwrap();
    assert_success(&output, "expected list to succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Available OAuth providers"));
    assert!(stdout.contains("anthropic"));
    assert!(stdout.contains("openai-codex"));
}

#[test]
fn login_rejects_unknown_provider_without_starting_oauth_flow() {
    let output = Command::new(BIN)
        .args(["login", "unknown-provider"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected unknown provider login to fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown provider: unknown-provider"));
    assert!(stderr.contains("Use 'pi-ai list' to see available providers"));
}

fn assert_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
