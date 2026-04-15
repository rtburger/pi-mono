use pi_ai::BUILT_IN_MODEL_PROVIDERS;
use pi_events::ModelCost;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const DEFAULT_CATALOG_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/models.catalog.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogModelEntry {
    id: String,
    name: String,
    api: String,
    provider: String,
    #[serde(rename = "baseUrl")]
    base_url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<String, String>,
    reasoning: bool,
    input: Vec<String>,
    cost: ModelCost,
    context_window: u64,
    max_tokens: u64,
    #[serde(default, flatten)]
    extra: BTreeMap<String, Value>,
}

type CatalogFile = BTreeMap<String, BTreeMap<String, CatalogModelEntry>>;

enum Command {
    Check { path: PathBuf },
    Format { path: PathBuf },
    Summary { path: PathBuf },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let command = parse_args(env::args().skip(1).collect())?;
    match command {
        Command::Check { path } => check_catalog(&path),
        Command::Format { path } => format_catalog(&path),
        Command::Summary { path } => print_summary(&path),
    }
}

fn parse_args(args: Vec<String>) -> Result<Command, String> {
    if args.is_empty() {
        return Err(usage(None));
    }

    if matches!(args.as_slice(), [help] if help == "-h" || help == "--help") {
        return Err(usage(None));
    }

    match args.as_slice() {
        [command] => parse_command(command, None),
        [command, path] => parse_command(command, Some(PathBuf::from(path))),
        _ => Err(usage(Some("Expected at most two arguments."))),
    }
}

fn parse_command(command: &str, path: Option<PathBuf>) -> Result<Command, String> {
    let path = path.unwrap_or_else(default_catalog_path);
    match command {
        "check" => Ok(Command::Check { path }),
        "fmt" | "format" => Ok(Command::Format { path }),
        "summary" => Ok(Command::Summary { path }),
        _ => Err(usage(Some(&format!("Unknown command: {command}")))),
    }
}

fn usage(error: Option<&str>) -> String {
    let mut message = String::new();
    if let Some(error) = error {
        message.push_str(error);
        message.push_str("\n\n");
    }
    message.push_str("Usage: pi-ai-catalog <check|fmt|summary> [path]\n\n");
    message.push_str("Defaults to the Rust-owned built-in catalog:\n");
    message.push_str(DEFAULT_CATALOG_PATH);
    message.push_str("\n\nCommands:\n");
    message.push_str("  check   Validate catalog semantics and canonical formatting\n");
    message.push_str("  fmt     Rewrite catalog with canonical Rust-owned formatting\n");
    message.push_str("  summary Print provider/model counts for the catalog\n");
    message
}

fn default_catalog_path() -> PathBuf {
    PathBuf::from(DEFAULT_CATALOG_PATH)
}

fn check_catalog(path: &Path) -> Result<(), String> {
    let input = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read catalog {}: {error}", path.display()))?;
    let catalog = parse_catalog(&input, path)?;
    validate_catalog(&catalog)?;
    let canonical = canonical_catalog_json(&catalog, path)?;

    if input != canonical {
        return Err(format!(
            "Catalog {} is not in canonical format.\nRun: cargo run -p pi-ai --bin pi-ai-catalog -- fmt {}",
            path.display(),
            path.display()
        ));
    }

    let (provider_count, model_count) = counts(&catalog);
    println!(
        "OK: {} ({} providers, {} models)",
        path.display(),
        provider_count,
        model_count
    );
    Ok(())
}

fn format_catalog(path: &Path) -> Result<(), String> {
    let input = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read catalog {}: {error}", path.display()))?;
    let catalog = parse_catalog(&input, path)?;
    validate_catalog(&catalog)?;
    let canonical = canonical_catalog_json(&catalog, path)?;

    if input == canonical {
        println!("Already canonical: {}", path.display());
        return Ok(());
    }

    fs::write(path, canonical)
        .map_err(|error| format!("Failed to write catalog {}: {error}", path.display()))?;
    println!("Rewrote catalog: {}", path.display());
    Ok(())
}

fn print_summary(path: &Path) -> Result<(), String> {
    let input = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read catalog {}: {error}", path.display()))?;
    let catalog = parse_catalog(&input, path)?;
    validate_catalog(&catalog)?;

    let (provider_count, model_count) = counts(&catalog);
    println!("Catalog: {}", path.display());
    println!("Providers: {provider_count}");
    println!("Models: {model_count}");
    for (provider, models) in &catalog {
        println!("- {provider}: {}", models.len());
    }

    Ok(())
}

fn parse_catalog(input: &str, path: &Path) -> Result<CatalogFile, String> {
    serde_json::from_str(input)
        .map_err(|error| format!("Failed to parse catalog {}: {error}", path.display()))
}

fn validate_catalog(catalog: &CatalogFile) -> Result<(), String> {
    let mut errors = Vec::new();
    let supported_providers = BUILT_IN_MODEL_PROVIDERS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    if catalog.is_empty() {
        errors.push(String::from("Catalog must contain at least one provider."));
    }

    for provider in catalog.keys() {
        if !supported_providers.contains(provider.as_str()) {
            errors.push(format!(
                "Unsupported provider `{provider}` in Rust-owned built-in catalog. Supported providers: {}",
                BUILT_IN_MODEL_PROVIDERS.join(", ")
            ));
        }
    }

    for provider in BUILT_IN_MODEL_PROVIDERS {
        if !catalog.contains_key(*provider) {
            errors.push(format!(
                "Missing built-in provider `{provider}` in Rust-owned catalog."
            ));
        }
    }

    for (provider_key, models) in catalog {
        if models.is_empty() {
            errors.push(format!(
                "Provider `{provider_key}` must define at least one model."
            ));
        }

        for (model_key, model) in models {
            if model.id != *model_key {
                errors.push(format!(
                    "Provider `{provider_key}` model key `{model_key}` does not match model id `{}`.",
                    model.id
                ));
            }
            if model.provider != *provider_key {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` has mismatched provider field `{}`.",
                    model.provider
                ));
            }
            if model.name.trim().is_empty() {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` must have a non-empty name."
                ));
            }
            if model.api.trim().is_empty() {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` must have a non-empty api."
                ));
            }
            if model.base_url.trim().is_empty() {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` must have a non-empty baseUrl."
                ));
            }
            if model.input.is_empty() {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` must declare at least one input kind."
                ));
            }
            if model.input.iter().any(|input| input.trim().is_empty()) {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` contains an empty input kind."
                ));
            }
            if model.context_window == 0 {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` must have contextWindow > 0."
                ));
            }
            if model.max_tokens == 0 {
                errors.push(format!(
                    "Provider `{provider_key}` model `{model_key}` must have maxTokens > 0."
                ));
            }
            validate_costs(provider_key, model_key, model.cost, &mut errors);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn validate_costs(provider_key: &str, model_key: &str, cost: ModelCost, errors: &mut Vec<String>) {
    if cost.input < 0.0 {
        errors.push(format!(
            "Provider `{provider_key}` model `{model_key}` has negative input cost."
        ));
    }
    if cost.output < 0.0 {
        errors.push(format!(
            "Provider `{provider_key}` model `{model_key}` has negative output cost."
        ));
    }
    if cost.cache_read < 0.0 {
        errors.push(format!(
            "Provider `{provider_key}` model `{model_key}` has negative cacheRead cost."
        ));
    }
    if cost.cache_write < 0.0 {
        errors.push(format!(
            "Provider `{provider_key}` model `{model_key}` has negative cacheWrite cost."
        ));
    }
}

fn canonical_catalog_json(catalog: &CatalogFile, path: &Path) -> Result<String, String> {
    let mut canonical = catalog.clone();
    for models in canonical.values_mut() {
        for model in models.values_mut() {
            for value in model.extra.values_mut() {
                canonicalize_value(value);
            }
        }
    }

    let mut output = serde_json::to_string_pretty(&canonical).map_err(|error| {
        format!(
            "Failed to serialize canonical catalog {}: {error}",
            path.display()
        )
    })?;
    output.push('\n');
    Ok(output)
}

fn canonicalize_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let entries = std::mem::take(object)
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            let mut sorted = Map::new();
            for (key, mut nested) in entries {
                canonicalize_value(&mut nested);
                sorted.insert(key, nested);
            }
            *object = sorted;
        }
        Value::Array(values) => {
            for nested in values {
                canonicalize_value(nested);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn counts(catalog: &CatalogFile) -> (usize, usize) {
    let provider_count = catalog.len();
    let model_count = catalog.values().map(BTreeMap::len).sum();
    (provider_count, model_count)
}
