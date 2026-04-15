use pi_coding_agent_core::ModelRegistry;
use pi_tui::fuzzy_filter;
use std::borrow::Cow;

pub fn render_list_models(model_registry: &ModelRegistry, search_pattern: Option<&str>) -> String {
    let models = model_registry.get_available();

    if models.is_empty() {
        return String::from("No models available. Set API keys in environment variables.\n");
    }

    let mut filtered_models = models;
    if let Some(search_pattern) = search_pattern {
        filtered_models = fuzzy_filter(&filtered_models, search_pattern, |model| {
            Cow::Owned(format!("{} {}", model.provider, model.id))
        })
        .into_iter()
        .cloned()
        .collect();
    }

    if filtered_models.is_empty() {
        return format!(
            "No models matching \"{}\"\n",
            search_pattern.unwrap_or_default()
        );
    }

    filtered_models.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.id.cmp(&right.id))
    });

    let rows = filtered_models
        .iter()
        .map(|model| ModelRow {
            provider: model.provider.clone(),
            model: model.id.clone(),
            context: format_token_count(model.context_window),
            max_out: format_token_count(model.max_tokens),
            thinking: yes_or_no(model.reasoning),
            images: yes_or_no(model.input.iter().any(|input| input == "image")),
        })
        .collect::<Vec<_>>();

    let headers = Headers {
        provider: "provider",
        model: "model",
        context: "context",
        max_out: "max-out",
        thinking: "thinking",
        images: "images",
    };

    let widths = Widths {
        provider: max_width(headers.provider, rows.iter().map(|row| row.provider.len())),
        model: max_width(headers.model, rows.iter().map(|row| row.model.len())),
        context: max_width(headers.context, rows.iter().map(|row| row.context.len())),
        max_out: max_width(headers.max_out, rows.iter().map(|row| row.max_out.len())),
        thinking: max_width(headers.thinking, rows.iter().map(|row| row.thinking.len())),
        images: max_width(headers.images, rows.iter().map(|row| row.images.len())),
    };

    let mut output = String::new();
    push_line(
        &mut output,
        &format_row(
            &[
                headers.provider,
                headers.model,
                headers.context,
                headers.max_out,
                headers.thinking,
                headers.images,
            ],
            &widths,
        ),
    );

    for row in rows {
        push_line(
            &mut output,
            &format_row(
                &[
                    &row.provider,
                    &row.model,
                    &row.context,
                    &row.max_out,
                    &row.thinking,
                    &row.images,
                ],
                &widths,
            ),
        );
    }

    output
}

fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        if count % 1_000_000 == 0 {
            return format!("{}M", count / 1_000_000);
        }

        let tenths = (count + 50_000) / 100_000;
        return format!("{}.{}M", tenths / 10, tenths % 10);
    }

    if count >= 1_000 {
        if count % 1_000 == 0 {
            return format!("{}K", count / 1_000);
        }

        let tenths = (count + 50) / 100;
        return format!("{}.{}K", tenths / 10, tenths % 10);
    }

    count.to_string()
}

fn yes_or_no(value: bool) -> String {
    if value {
        String::from("yes")
    } else {
        String::from("no")
    }
}

fn max_width(header: &str, values: impl Iterator<Item = usize>) -> usize {
    values.fold(header.len(), usize::max)
}

fn format_row(columns: &[&str; 6], widths: &Widths) -> String {
    [
        pad(columns[0], widths.provider),
        pad(columns[1], widths.model),
        pad(columns[2], widths.context),
        pad(columns[3], widths.max_out),
        pad(columns[4], widths.thinking),
        pad(columns[5], widths.images),
    ]
    .join("  ")
}

fn pad(value: &str, width: usize) -> String {
    format!("{value:<width$}")
}

fn push_line(buffer: &mut String, line: &str) {
    buffer.push_str(line);
    buffer.push('\n');
}

struct Headers<'a> {
    provider: &'a str,
    model: &'a str,
    context: &'a str,
    max_out: &'a str,
    thinking: &'a str,
    images: &'a str,
}

struct Widths {
    provider: usize,
    model: usize,
    context: usize,
    max_out: usize,
    thinking: usize,
    images: usize,
}

struct ModelRow {
    provider: String,
    model: String,
    context: String,
    max_out: String,
    thinking: String,
    images: String,
}

#[cfg(test)]
mod tests {
    use super::{format_token_count, render_list_models};
    use pi_coding_agent_core::{MemoryAuthStorage, ModelRegistry};
    use pi_events::Model;
    use std::{
        env, fs,
        path::{Path, PathBuf},
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn model(
        provider: &str,
        id: &str,
        context_window: u64,
        max_tokens: u64,
        reasoning: bool,
        input: &[&str],
    ) -> Model {
        Model {
            id: id.to_string(),
            name: id.to_string(),
            api: String::from("openai-responses"),
            provider: provider.to_string(),
            base_url: String::from("https://example.invalid/v1"),
            reasoning,
            input: input.iter().map(|value| (*value).to_string()).collect(),
            cost: pi_events::ModelCost {
                input: 1.0,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.1,
            },
            context_window,
            max_tokens,
            compat: None,
        }
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = env::temp_dir().join(format!("pi-coding-agent-cli-{prefix}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_models_json(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
    }

    fn split_columns(line: &str) -> Vec<String> {
        let mut columns = Vec::new();
        let mut current = String::new();
        let mut spaces = 0usize;

        for character in line.chars() {
            if character == ' ' {
                spaces += 1;
                continue;
            }

            if spaces >= 2 && !current.is_empty() {
                columns.push(current.trim_end().to_string());
                current.clear();
            } else if spaces == 1 {
                current.push(' ');
            }

            spaces = 0;
            current.push(character);
        }

        if !current.is_empty() {
            columns.push(current.trim_end().to_string());
        }

        columns
    }

    #[test]
    fn formats_token_counts_like_typescript() {
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_000), "1K");
        assert_eq!(format_token_count(1_500), "1.5K");
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_250_000), "1.3M");
    }

    #[test]
    fn renders_available_models_table() {
        let temp_dir = unique_temp_dir("list-models-table");
        let models_json_path = temp_dir.join("models.json");
        write_models_json(
            &models_json_path,
            r#"{
  "providers": {
    "custom": {
      "baseUrl": "https://custom.example.com/v1",
      "apiKey": "literal-token",
      "api": "openai-responses",
      "models": [
        {
          "id": "tiny-model",
          "contextWindow": 999,
          "maxTokens": 1000
        }
      ]
    }
  }
}"#,
        );

        let registry = ModelRegistry::new(
            Arc::new(MemoryAuthStorage::with_api_keys([("openai", "token")])),
            vec![
                model(
                    "anthropic",
                    "claude-sonnet-4-5",
                    200_000,
                    64_000,
                    true,
                    &["text"],
                ),
                model(
                    "openai",
                    "gpt-5.2-codex",
                    1_000_000,
                    1_500,
                    true,
                    &["text", "image"],
                ),
            ],
            Some(models_json_path),
        );

        let output = render_list_models(&registry, None);
        let lines = output.lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 3);
        assert_eq!(
            split_columns(lines[0]),
            vec![
                "provider", "model", "context", "max-out", "thinking", "images"
            ]
        );
        assert_eq!(
            split_columns(lines[1]),
            vec!["custom", "tiny-model", "999", "1K", "no", "no"]
        );
        assert_eq!(
            split_columns(lines[2]),
            vec!["openai", "gpt-5.2-codex", "1M", "1.5K", "yes", "yes"]
        );
    }

    #[test]
    fn applies_fuzzy_search_before_rendering() {
        let registry = ModelRegistry::in_memory(
            Arc::new(MemoryAuthStorage::with_api_keys([("openai", "token")])),
            vec![
                model(
                    "openai",
                    "gpt-5.2-codex",
                    1_000_000,
                    1_500,
                    true,
                    &["text", "image"],
                ),
                model("openai", "gpt-4o-mini", 128_000, 16_384, false, &["text"]),
            ],
        );

        let output = render_list_models(&registry, Some("codex52"));

        assert!(output.contains("gpt-5.2-codex"));
        assert!(!output.contains("gpt-4o-mini"));
    }

    #[test]
    fn reports_no_models_matching_search_pattern() {
        let registry = ModelRegistry::in_memory(
            Arc::new(MemoryAuthStorage::with_api_keys([("openai", "token")])),
            vec![model(
                "openai",
                "gpt-4o-mini",
                128_000,
                16_384,
                false,
                &["text"],
            )],
        );

        let output = render_list_models(&registry, Some("sonnet"));

        assert_eq!(output, "No models matching \"sonnet\"\n");
    }

    #[test]
    fn reports_when_no_models_are_available() {
        let registry = ModelRegistry::in_memory(
            Arc::new(MemoryAuthStorage::new()),
            vec![model(
                "openai",
                "gpt-4o-mini",
                128_000,
                16_384,
                false,
                &["text"],
            )],
        );

        let output = render_list_models(&registry, None);

        assert_eq!(
            output,
            "No models available. Set API keys in environment variables.\n"
        );
    }
}
