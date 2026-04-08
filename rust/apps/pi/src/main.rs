use pi_ai::{StreamOptions, built_in_models};
use pi_coding_agent_cli::{
    AuthFileSource, ChainedAuthSource, EnvAuthSource, RunCommandOptions, run_command,
};
use std::{
    env,
    io::{self, IsTerminal as _, Read as _},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};

const CONFIG_DIR_NAME: &str = ".pi";
const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let stdin_is_tty = io::stdin().is_terminal();
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
        args: env::args().skip(1).collect(),
        stdin_is_tty,
        stdin_content,
        auth_source: Arc::new(ChainedAuthSource::new(vec![
            Arc::new(AuthFileSource::new(get_auth_path())),
            Arc::new(EnvAuthSource::new()),
        ])),
        built_in_models: built_in_models().to_vec(),
        models_json_path: Some(get_models_path()),
        cwd: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        default_system_prompt: String::new(),
        version: env!("CARGO_PKG_VERSION").to_string(),
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

fn get_models_path() -> PathBuf {
    get_agent_dir().join("models.json")
}

fn get_auth_path() -> PathBuf {
    get_agent_dir().join("auth.json")
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
