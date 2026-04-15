use pi_ai::{
    OAuthAuthInfo, OAuthCredentials, OAuthLoginCallbacks, OAuthPrompt, OAuthProviderInfo,
    get_oauth_provider, get_oauth_provider_info_list,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write as _},
};

const AUTH_FILE: &str = "auth.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredOAuthCredentials {
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    credentials: OAuthCredentials,
}

impl StoredOAuthCredentials {
    fn oauth(credentials: OAuthCredentials) -> Self {
        Self {
            kind: "oauth".into(),
            credentials,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run(args: Vec<String>) -> Result<(), String> {
    let command = args.first().map(String::as_str);

    match command {
        None | Some("help" | "--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some("list") => {
            print_provider_list();
            Ok(())
        }
        Some("login") => {
            let providers = get_oauth_provider_info_list();
            let provider_id = match args.get(1) {
                Some(provider_id) => provider_id.clone(),
                None => select_provider(&providers)?,
            };

            if !providers.iter().any(|provider| provider.id == provider_id) {
                return Err(format!(
                    "Unknown provider: {provider_id}\nUse 'pi-ai list' to see available providers"
                ));
            }

            println!("Logging in to {provider_id}...");
            login(&provider_id).await
        }
        Some(command) => Err(format!(
            "Unknown command: {command}\nUse 'pi-ai --help' for usage"
        )),
    }
}

fn print_help() {
    let providers = get_oauth_provider_info_list();
    let provider_list = format_provider_list(&providers);
    println!(
        "Usage: pi-ai <command> [provider]\n\nCommands:\n  login [provider]  Login to an OAuth provider\n  list              List available providers\n\nProviders:\n{provider_list}\n\nExamples:\n  pi-ai login              # interactive provider selection\n  pi-ai login anthropic    # login to specific provider\n  pi-ai list               # list providers"
    );
}

fn print_provider_list() {
    let providers = get_oauth_provider_info_list();
    println!("Available OAuth providers:\n");
    for provider in providers {
        println!("  {:<20} {}", provider.id, provider.name);
    }
}

fn format_provider_list(providers: &[OAuthProviderInfo]) -> String {
    providers
        .iter()
        .map(|provider| format!("  {:<20} {}", provider.id, provider.name))
        .collect::<Vec<_>>()
        .join("\n")
}

fn select_provider(providers: &[OAuthProviderInfo]) -> Result<String, String> {
    if providers.is_empty() {
        return Err("No OAuth providers available".into());
    }

    println!("Select a provider:\n");
    for (index, provider) in providers.iter().enumerate() {
        println!("  {}. {}", index + 1, provider.name);
    }
    println!();

    let choice = prompt_line(&format!("Enter number (1-{}): ", providers.len()))?;
    let index = choice
        .parse::<usize>()
        .map_err(|_| "Invalid selection".to_string())?
        .checked_sub(1)
        .ok_or_else(|| "Invalid selection".to_string())?;

    providers
        .get(index)
        .map(|provider| provider.id.clone())
        .ok_or_else(|| "Invalid selection".to_string())
}

async fn login(provider_id: &str) -> Result<(), String> {
    let provider = get_oauth_provider(provider_id)
        .ok_or_else(|| format!("Unknown provider: {provider_id}"))?;

    let callbacks =
        OAuthLoginCallbacks::new(
            handle_auth,
            |prompt| async move { prompt_oauth(prompt).await },
        )
        .with_progress(|message| {
            println!("{message}");
            Ok(())
        });

    let credentials = provider.login(callbacks).await?;
    let mut auth = load_auth();
    auth.insert(
        provider_id.to_string(),
        StoredOAuthCredentials::oauth(credentials),
    );
    save_auth(&auth)?;

    println!("\nCredentials saved to {AUTH_FILE}");
    Ok(())
}

fn handle_auth(info: OAuthAuthInfo) -> Result<(), String> {
    println!("\nOpen this URL in your browser:\n{}", info.url);
    if let Some(instructions) = info.instructions {
        println!("{instructions}");
    }
    println!();
    Ok(())
}

async fn prompt_oauth(prompt: OAuthPrompt) -> Result<String, String> {
    let OAuthPrompt {
        message,
        placeholder,
        allow_empty,
    } = prompt;

    loop {
        let mut question = message.clone();
        if let Some(placeholder) = placeholder.as_deref() {
            question.push_str(&format!(" ({placeholder})"));
        }
        question.push(':');
        let input = prompt_line(&question)?;
        if allow_empty || !input.is_empty() {
            return Ok(input);
        }
    }
}

fn prompt_line(question: &str) -> Result<String, String> {
    print!("{question}");
    io::stdout()
        .flush()
        .map_err(|error| format!("Failed to flush stdout: {error}"))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|error| format!("Failed to read input: {error}"))?;

    Ok(input.trim().to_string())
}

fn load_auth() -> BTreeMap<String, StoredOAuthCredentials> {
    let Ok(content) = fs::read_to_string(AUTH_FILE) else {
        return BTreeMap::new();
    };

    serde_json::from_str(&content).unwrap_or_default()
}

fn save_auth(auth: &BTreeMap<String, StoredOAuthCredentials>) -> Result<(), String> {
    let content = serde_json::to_string_pretty(auth)
        .map_err(|error| format!("Failed to serialize {AUTH_FILE}: {error}"))?;
    fs::write(AUTH_FILE, format!("{content}\n"))
        .map_err(|error| format!("Failed to write {AUTH_FILE}: {error}"))
}
