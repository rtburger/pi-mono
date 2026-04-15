use pi_ai::{StreamOptions, built_in_models};
use pi_coding_agent_cli::{
    AppMode, AuthFileSource, ChainedAuthSource, EnvAuthSource, RunCommandOptions,
    finalize_system_prompt, parse_args, resolve_app_mode, run_command, run_interactive_command,
    run_rpc_command,
};
use pi_coding_agent_core::{build_default_pi_system_prompt, refresh_auth_file_oauth};
use pi_coding_agent_tui::migrate_keybindings_file;
use std::{
    env,
    io::{self, IsTerminal as _, Read as _},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

const CONFIG_DIR_NAME: &str = ".pi";
const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let stdin_is_tty = io::stdin().is_terminal();

    let agent_dir = get_agent_dir();
    run_startup_migrations(&agent_dir);
    let auth_path = agent_dir.join("auth.json");
    refresh_auth_file_oauth(&auth_path).await;

    let args = env::args().skip(1).collect::<Vec<_>>();
    let parsed = parse_args(&args);
    let app_mode = resolve_app_mode(&parsed, stdin_is_tty);
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let auth_source = Arc::new(ChainedAuthSource::new(vec![
        Arc::new(AuthFileSource::new(auth_path)),
        Arc::new(EnvAuthSource::new()),
    ]));
    let built_in_models = built_in_models().to_vec();
    let models_json_path = Some(agent_dir.join("models.json"));
    let version = env!("CARGO_PKG_VERSION").to_string();
    let finalized_default_system_prompt = finalize_system_prompt(build_default_pi_system_prompt(
        &cwd,
        &agent_dir,
        parsed.system_prompt.as_deref(),
        parsed.append_system_prompt.as_deref(),
    ));

    if matches!(app_mode, AppMode::Interactive)
        && !parsed.help
        && !parsed.version
        && parsed.list_models.is_none()
        && parsed.export.is_none()
    {
        let exit_code = run_interactive_command(RunCommandOptions {
            args: args.clone(),
            stdin_is_tty,
            stdin_content: None,
            auth_source: auth_source.clone(),
            built_in_models: built_in_models.clone(),
            models_json_path: models_json_path.clone(),
            agent_dir: Some(agent_dir.clone()),
            cwd: cwd.clone(),
            default_system_prompt: finalized_default_system_prompt.clone(),
            version: version.clone(),
            stream_options: StreamOptions::default(),
        })
        .await;

        return if exit_code == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(exit_code as u8)
        };
    }

    if matches!(app_mode, AppMode::Rpc)
        && !parsed.help
        && !parsed.version
        && parsed.list_models.is_none()
        && parsed.export.is_none()
    {
        let exit_code = run_rpc_command(RunCommandOptions {
            args,
            stdin_is_tty,
            stdin_content: None,
            auth_source,
            built_in_models,
            models_json_path,
            agent_dir: Some(agent_dir),
            cwd,
            default_system_prompt: finalized_default_system_prompt,
            version,
            stream_options: StreamOptions::default(),
        })
        .await;

        return if exit_code == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(exit_code as u8)
        };
    }

    let stdin_content = if stdin_is_tty {
        None
    } else {
        let mut buffer = String::new();
        match io::stdin().read_to_string(&mut buffer) {
            Ok(_) => Some(buffer),
            Err(_) => None,
        }
    };

    let result = run_command(RunCommandOptions {
        args,
        stdin_is_tty,
        stdin_content,
        auth_source,
        built_in_models,
        models_json_path,
        agent_dir: Some(agent_dir),
        cwd,
        default_system_prompt: finalized_default_system_prompt,
        version,
        stream_options: StreamOptions::default(),
    })
    .await;

    if !result.stdout.is_empty() {
        print!("{}", result.stdout);
    }
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }

    if result.exit_code == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(result.exit_code as u8)
    }
}

fn run_startup_migrations(agent_dir: &Path) {
    let _ = migrate_keybindings_file(agent_dir.join("keybindings.json"));
}

fn get_agent_dir() -> PathBuf {
    if let Some(agent_dir) = env::var_os(ENV_AGENT_DIR) {
        return expand_home_path(PathBuf::from(agent_dir));
    }

    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIR_NAME)
        .join("agent")
}

fn expand_home_path(path: PathBuf) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str == "~" {
        return home_dir().unwrap_or(path);
    }
    if let Some(suffix) = path_str.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(suffix);
    }
    path
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::run_startup_migrations;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn startup_migrations_rewrite_legacy_keybindings_file_when_present() {
        let agent_dir = tempdir().unwrap();
        fs::write(
            agent_dir.path().join("keybindings.json"),
            "{\n  \"cursorUp\": [\"up\", \"ctrl+p\"]\n}\n",
        )
        .unwrap();

        run_startup_migrations(agent_dir.path());

        let content = fs::read_to_string(agent_dir.path().join("keybindings.json")).unwrap();
        assert_eq!(
            content,
            "{\n  \"tui.editor.cursorUp\": [\n    \"up\",\n    \"ctrl+p\"\n  ]\n}\n"
        );
    }

    #[test]
    fn startup_migrations_ignore_malformed_keybindings_files() {
        let agent_dir = tempdir().unwrap();
        fs::write(
            agent_dir.path().join("keybindings.json"),
            "{ not valid json\n",
        )
        .unwrap();

        run_startup_migrations(agent_dir.path());

        let content = fs::read_to_string(agent_dir.path().join("keybindings.json")).unwrap();
        assert_eq!(content, "{ not valid json\n");
    }
}
