use crate::{Skill, format_skills_for_prompt, load_skills};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DEFAULT_SELECTED_TOOLS: [&str; 4] = ["read", "bash", "edit", "write"];
const CONTEXT_FILE_CANDIDATES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
const CONFIG_DIR_NAME: &str = ".pi";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSystemPromptResources {
    pub context_files: Vec<ContextFile>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BuildSystemPromptOptions {
    pub custom_prompt: Option<String>,
    pub selected_tools: Vec<String>,
    pub tool_snippets: BTreeMap<String, String>,
    pub prompt_guidelines: Vec<String>,
    pub append_system_prompt: Option<String>,
    pub cwd: Option<PathBuf>,
    pub context_files: Vec<ContextFile>,
    pub skills: Vec<Skill>,
    pub date: Option<String>,
    pub readme_path: Option<PathBuf>,
    pub docs_path: Option<PathBuf>,
    pub examples_path: Option<PathBuf>,
}

pub fn build_default_pi_system_prompt(
    cwd: &Path,
    agent_dir: &Path,
    custom_prompt: Option<&str>,
    append_system_prompt: Option<&str>,
) -> String {
    let resources = load_system_prompt_resources(cwd, agent_dir);
    let custom_prompt = resolve_prompt_input(custom_prompt).or(resources.system_prompt);
    let append_system_prompt = resolve_prompt_input(append_system_prompt)
        .or_else(|| join_prompt_sections(&resources.append_system_prompt));

    let skills = load_skills(crate::LoadSkillsOptions {
        cwd: cwd.to_path_buf(),
        agent_dir: Some(agent_dir.to_path_buf()),
        skill_paths: Vec::new(),
        include_defaults: true,
    })
    .skills;

    build_system_prompt(BuildSystemPromptOptions {
        custom_prompt,
        selected_tools: DEFAULT_SELECTED_TOOLS
            .into_iter()
            .map(str::to_owned)
            .collect(),
        tool_snippets: default_tool_snippets(),
        append_system_prompt,
        cwd: Some(cwd.to_path_buf()),
        context_files: resources.context_files,
        skills,
        ..BuildSystemPromptOptions::default()
    })
}

pub fn build_system_prompt(options: BuildSystemPromptOptions) -> String {
    let BuildSystemPromptOptions {
        custom_prompt,
        selected_tools,
        tool_snippets,
        prompt_guidelines,
        append_system_prompt,
        cwd,
        context_files,
        skills,
        date,
        readme_path,
        docs_path,
        examples_path,
    } = options;

    let resolved_cwd = cwd.unwrap_or_else(current_dir_fallback);
    let prompt_cwd = display_path_posix(&resolved_cwd);
    let date = date.unwrap_or_else(current_utc_date_string);
    let append_section = append_system_prompt
        .filter(|value| !value.is_empty())
        .map(|value| format!("\n\n{value}"))
        .unwrap_or_default();
    let selected_tools = if selected_tools.is_empty() {
        DEFAULT_SELECTED_TOOLS
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        selected_tools
    };

    if let Some(mut prompt) = custom_prompt.filter(|value| !value.is_empty()) {
        if !append_section.is_empty() {
            prompt.push_str(&append_section);
        }
        append_context_files(&mut prompt, &context_files);
        if selected_tools.iter().any(|tool| tool == "read") && !skills.is_empty() {
            prompt.push_str(&format_skills_for_prompt(&skills));
        }
        append_prompt_footer(&mut prompt, &date, &prompt_cwd);
        return prompt;
    }

    let (readme_path, docs_path, examples_path) =
        default_pi_docs_paths(readme_path, docs_path, examples_path);
    let visible_tools = selected_tools
        .iter()
        .filter_map(|name| {
            tool_snippets
                .get(name)
                .map(|snippet| format!("- {name}: {snippet}"))
        })
        .collect::<Vec<_>>();
    let tools_list = if visible_tools.is_empty() {
        String::from("(none)")
    } else {
        visible_tools.join("\n")
    };

    let mut guidelines_list = Vec::new();
    let mut guidelines_seen = HashSet::new();
    let mut add_guideline = |guideline: &str| {
        if guideline.is_empty() || !guidelines_seen.insert(guideline.to_owned()) {
            return;
        }
        guidelines_list.push(guideline.to_owned());
    };

    let has_bash = selected_tools.iter().any(|tool| tool == "bash");
    let has_grep = selected_tools.iter().any(|tool| tool == "grep");
    let has_find = selected_tools.iter().any(|tool| tool == "find");
    let has_ls = selected_tools.iter().any(|tool| tool == "ls");

    if has_bash && !has_grep && !has_find && !has_ls {
        add_guideline("Use bash for file operations like ls, rg, find");
    } else if has_bash && (has_grep || has_find || has_ls) {
        add_guideline(
            "Prefer grep/find/ls tools over bash for file exploration (faster, respects .gitignore)",
        );
    }

    for guideline in prompt_guidelines {
        let trimmed = guideline.trim();
        if !trimmed.is_empty() {
            add_guideline(trimmed);
        }
    }

    add_guideline("Be concise in your responses");
    add_guideline("Show file paths clearly when working with files");

    let guidelines = guidelines_list
        .into_iter()
        .map(|guideline| format!("- {guideline}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut prompt = format!(
        "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.\n\nAvailable tools:\n{tools_list}\n\nIn addition to the tools above, you may have access to other custom tools depending on the project.\n\nGuidelines:\n{guidelines}\n\nPi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):\n- Main documentation: {}\n- Additional docs: {}\n- Examples: {} (extensions, custom tools, SDK)\n- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md)\n- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing\n- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)",
        readme_path.display(),
        docs_path.display(),
        examples_path.display(),
    );

    if !append_section.is_empty() {
        prompt.push_str(&append_section);
    }
    append_context_files(&mut prompt, &context_files);
    if selected_tools.iter().any(|tool| tool == "read") && !skills.is_empty() {
        prompt.push_str(&format_skills_for_prompt(&skills));
    }
    append_prompt_footer(&mut prompt, &date, &prompt_cwd);

    prompt
}

pub fn load_project_context_files(cwd: &Path, agent_dir: &Path) -> Vec<ContextFile> {
    let mut context_files = Vec::new();
    let mut seen_paths = HashSet::new();

    if let Some(global_context) = load_context_file_from_dir(agent_dir) {
        seen_paths.insert(global_context.path.clone());
        context_files.push(global_context);
    }

    let mut current_dir = cwd.to_path_buf();
    let mut ancestor_context_files = Vec::new();

    loop {
        if let Some(context_file) = load_context_file_from_dir(&current_dir)
            && seen_paths.insert(context_file.path.clone())
        {
            ancestor_context_files.push(context_file);
        }

        if !current_dir.pop() {
            break;
        }
    }

    ancestor_context_files.reverse();
    context_files.extend(ancestor_context_files);
    context_files
}

pub fn load_system_prompt_resources(cwd: &Path, agent_dir: &Path) -> LoadedSystemPromptResources {
    let system_prompt =
        discover_system_prompt_file(cwd, agent_dir).and_then(|path| fs::read_to_string(path).ok());
    let append_system_prompt = discover_append_system_prompt_file(cwd, agent_dir)
        .and_then(|path| fs::read_to_string(path).ok())
        .into_iter()
        .collect();

    LoadedSystemPromptResources {
        context_files: load_project_context_files(cwd, agent_dir),
        system_prompt,
        append_system_prompt,
    }
}

pub fn discover_system_prompt_file(cwd: &Path, agent_dir: &Path) -> Option<PathBuf> {
    let project_path = cwd.join(CONFIG_DIR_NAME).join("SYSTEM.md");
    if project_path.exists() {
        return Some(project_path);
    }

    let global_path = agent_dir.join("SYSTEM.md");
    global_path.exists().then_some(global_path)
}

pub fn discover_append_system_prompt_file(cwd: &Path, agent_dir: &Path) -> Option<PathBuf> {
    let project_path = cwd.join(CONFIG_DIR_NAME).join("APPEND_SYSTEM.md");
    if project_path.exists() {
        return Some(project_path);
    }

    let global_path = agent_dir.join("APPEND_SYSTEM.md");
    global_path.exists().then_some(global_path)
}

pub fn resolve_prompt_input(input: Option<&str>) -> Option<String> {
    let value = input?.trim();
    if value.is_empty() {
        return None;
    }

    let path = Path::new(value);
    match fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(_) => Some(value.to_owned()),
    }
}

fn append_context_files(prompt: &mut String, context_files: &[ContextFile]) {
    if context_files.is_empty() {
        return;
    }

    prompt.push_str("\n\n# Project Context\n\n");
    prompt.push_str("Project-specific instructions and guidelines:\n\n");
    for context_file in context_files {
        prompt.push_str(&format!(
            "## {}\n\n{}\n\n",
            context_file.path, context_file.content
        ));
    }
}

fn append_prompt_footer(prompt: &mut String, date: &str, cwd: &str) {
    prompt.push_str(&format!("\nCurrent date: {date}"));
    prompt.push_str(&format!("\nCurrent working directory: {cwd}"));
}

fn default_tool_snippets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (String::from("read"), String::from("Read file contents")),
        (String::from("bash"), String::from("Execute bash commands")),
        (
            String::from("edit"),
            String::from("Make exact text replacements in files"),
        ),
        (
            String::from("write"),
            String::from("Create or overwrite files"),
        ),
    ])
}

fn load_context_file_from_dir(dir: &Path) -> Option<ContextFile> {
    for file_name in CONTEXT_FILE_CANDIDATES {
        let path = dir.join(file_name);
        if let Ok(content) = fs::read_to_string(&path) {
            return Some(ContextFile {
                path: path.display().to_string(),
                content,
            });
        }
    }

    None
}

fn join_prompt_sections(sections: &[String]) -> Option<String> {
    let non_empty = sections
        .iter()
        .map(|section| section.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>();
    if non_empty.is_empty() {
        None
    } else {
        Some(non_empty.join("\n\n"))
    }
}

fn default_pi_docs_paths(
    readme_path: Option<PathBuf>,
    docs_path: Option<PathBuf>,
    examples_path: Option<PathBuf>,
) -> (PathBuf, PathBuf, PathBuf) {
    let package_dir = coding_agent_package_dir();
    (
        readme_path.unwrap_or_else(|| package_dir.join("README.md")),
        docs_path.unwrap_or_else(|| package_dir.join("docs")),
        examples_path.unwrap_or_else(|| package_dir.join("examples")),
    )
}

fn coding_agent_package_dir() -> PathBuf {
    let candidate =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages/coding-agent");
    candidate.canonicalize().unwrap_or(candidate)
}

fn current_dir_fallback() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn display_path_posix(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn current_utc_date_string() -> String {
    let days_since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    let (year, month, day) = civil_from_days(days_since_epoch as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}
