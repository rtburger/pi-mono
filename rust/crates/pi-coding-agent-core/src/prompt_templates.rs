use crate::{ResourceDiagnostic, SourceInfo, frontmatter::parse_frontmatter};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const CONFIG_DIR_NAME: &str = ".pi";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source_info: SourceInfo,
    pub file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadPromptTemplatesResult {
    pub prompts: Vec<PromptTemplate>,
    pub diagnostics: Vec<ResourceDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadPromptTemplatesOptions {
    pub cwd: PathBuf,
    pub agent_dir: Option<PathBuf>,
    pub prompt_paths: Vec<String>,
    pub include_defaults: bool,
}

#[derive(Debug, Deserialize, Default)]
struct PromptFrontmatter {
    description: Option<String>,
}

pub fn parse_command_args(args: &str) -> Vec<String> {
    let mut parsed = Vec::new();
    let mut current = String::new();
    let mut quote = None::<char>;

    for character in args.chars() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }

        match character {
            '"' | '\'' => quote = Some(character),
            ' ' | '\t' => {
                if !current.is_empty() {
                    parsed.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }

    if !current.is_empty() {
        parsed.push(current);
    }

    parsed
}

pub fn substitute_args(content: &str, args: &[String]) -> String {
    let mut output = replace_positional_args(content, args);
    output = replace_sliced_args(&output, args);
    let all_args = args.join(" ");
    output = output.replace("$ARGUMENTS", &all_args);
    output.replace("$@", &all_args)
}

pub fn expand_prompt_template(text: &str, templates: &[PromptTemplate]) -> String {
    if !text.starts_with('/') {
        return text.to_owned();
    }

    let (template_name, args_string) = split_command(text);
    let Some(template) = templates
        .iter()
        .find(|template| template.name == template_name)
    else {
        return text.to_owned();
    };

    let args = parse_command_args(args_string);
    substitute_args(&template.content, &args)
}

pub fn load_prompt_templates(options: LoadPromptTemplatesOptions) -> LoadPromptTemplatesResult {
    let LoadPromptTemplatesOptions {
        cwd,
        agent_dir,
        prompt_paths,
        include_defaults,
    } = options;
    let mut prompts_by_name = BTreeMap::<String, PromptTemplate>::new();
    let mut diagnostics = Vec::new();

    if include_defaults {
        if let Some(agent_dir) = agent_dir.as_deref() {
            load_templates_from_dir(
                &agent_dir.join("prompts"),
                &mut prompts_by_name,
                &mut diagnostics,
                Some(("user", agent_dir.join("prompts"))),
            );
        }
        let project_dir = cwd.join(CONFIG_DIR_NAME).join("prompts");
        load_templates_from_dir(
            &project_dir,
            &mut prompts_by_name,
            &mut diagnostics,
            Some(("project", project_dir.clone())),
        );
    }

    for prompt_path in prompt_paths {
        let resolved = resolve_from_cwd(&cwd, &prompt_path);
        if !resolved.exists() {
            diagnostics.push(ResourceDiagnostic::new(
                "Prompt template path does not exist",
                Some(resolved.display().to_string()),
            ));
            continue;
        }

        if resolved.is_dir() {
            load_templates_from_dir(&resolved, &mut prompts_by_name, &mut diagnostics, None);
        } else if resolved
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("md")
        {
            if let Some(prompt) =
                load_template_file(&resolved, source_info_for_path(&resolved, None))
            {
                add_prompt(&mut prompts_by_name, &mut diagnostics, prompt);
            }
        } else {
            diagnostics.push(ResourceDiagnostic::new(
                "Prompt template path is not a markdown file",
                Some(resolved.display().to_string()),
            ));
        }
    }

    LoadPromptTemplatesResult {
        prompts: prompts_by_name.into_values().collect(),
        diagnostics,
    }
}

fn load_templates_from_dir(
    dir: &Path,
    prompts_by_name: &mut BTreeMap<String, PromptTemplate>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    default_scope: Option<(&str, PathBuf)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let source_info = source_info_for_path(&path, default_scope.clone());
        if let Some(prompt) = load_template_file(&path, source_info) {
            add_prompt(prompts_by_name, diagnostics, prompt);
        }
    }
}

fn add_prompt(
    prompts_by_name: &mut BTreeMap<String, PromptTemplate>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    prompt: PromptTemplate,
) {
    if let Some(existing) = prompts_by_name.get(&prompt.name) {
        diagnostics.push(ResourceDiagnostic::new(
            format!(
                "Prompt template name collision for /{} (keeping {})",
                prompt.name, existing.file_path
            ),
            Some(prompt.file_path.clone()),
        ));
        return;
    }

    prompts_by_name.insert(prompt.name.clone(), prompt);
}

fn load_template_file(path: &Path, source_info: SourceInfo) -> Option<PromptTemplate> {
    let raw = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = parse_frontmatter::<PromptFrontmatter>(&raw);
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let description = frontmatter
        .and_then(|frontmatter| frontmatter.description)
        .filter(|description| !description.trim().is_empty())
        .or_else(|| {
            body.lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.trim().chars().take(60).collect::<String>())
        })
        .unwrap_or_default();

    Some(PromptTemplate {
        name,
        description,
        content: body.to_owned(),
        source_info,
        file_path: path.display().to_string(),
    })
}

fn source_info_for_path(path: &Path, default_scope: Option<(&str, PathBuf)>) -> SourceInfo {
    if let Some((scope, base_dir)) = default_scope {
        return SourceInfo::local(
            path.display().to_string(),
            scope,
            base_dir.display().to_string(),
        );
    }

    let base_dir = path
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_else(|| String::from("."));
    SourceInfo::temporary(path.display().to_string(), base_dir)
}

fn resolve_from_cwd(cwd: &Path, input: &str) -> PathBuf {
    let path = Path::new(input);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn split_command(text: &str) -> (&str, &str) {
    let command = &text[1..];
    match command.split_once(' ') {
        Some((name, args)) => (name, args.trim()),
        None => (command, ""),
    }
}

fn replace_positional_args(content: &str, args: &[String]) -> String {
    let mut output = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '$' {
            output.push(character);
            continue;
        }

        let mut digits = String::new();
        while let Some(next) = chars.peek().copied() {
            if next.is_ascii_digit() {
                digits.push(next);
                chars.next();
            } else {
                break;
            }
        }

        if digits.is_empty() {
            output.push(character);
            continue;
        }

        let index = digits.parse::<usize>().unwrap_or(0).saturating_sub(1);
        output.push_str(args.get(index).map(String::as_str).unwrap_or(""));
    }

    output
}

fn replace_sliced_args(content: &str, args: &[String]) -> String {
    let mut output = String::new();
    let mut index = 0usize;

    while let Some(start) = content[index..].find("${@:") {
        let absolute_start = index + start;
        output.push_str(&content[index..absolute_start]);
        let rest = &content[absolute_start + 4..];
        let Some(end) = rest.find('}') else {
            output.push_str(&content[absolute_start..]);
            return output;
        };
        let expression = &rest[..end];
        let replacement = render_slice_expression(expression, args)
            .unwrap_or_else(|| format!("${{@:{expression}}}"));
        output.push_str(&replacement);
        index = absolute_start + 4 + end + 1;
    }

    output.push_str(&content[index..]);
    output
}

fn render_slice_expression(expression: &str, args: &[String]) -> Option<String> {
    let mut parts = expression.split(':');
    let start = parts.next()?.parse::<usize>().ok()?.saturating_sub(1);
    let length = parts.next().and_then(|part| part.parse::<usize>().ok());
    let sliced = match length {
        Some(length) => args.iter().skip(start).take(length),
        None => args
            .iter()
            .skip(start)
            .take(args.len().saturating_sub(start)),
    };
    Some(sliced.cloned().collect::<Vec<_>>().join(" "))
}
