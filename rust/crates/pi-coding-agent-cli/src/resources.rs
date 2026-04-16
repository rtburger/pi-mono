use crate::{Args, ToolName};
use pi_agent::AgentTool;
use pi_coding_agent_core::{
    BuildSystemPromptOptions, PromptTemplate, Skill, build_system_prompt, expand_prompt_template,
    expand_skill_command, load_prompt_templates, load_skills, load_system_prompt_resources,
    resolve_prompt_input,
};
use pi_coding_agent_tools::{
    create_bash_tool, create_edit_tool, create_find_tool, create_grep_tool, create_ls_tool,
    create_read_tool, create_read_tool_with_auto_resize_flag, create_write_tool,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

const FINALIZED_SYSTEM_PROMPT_PREFIX: &str = "\0pi-final-system-prompt\n";

#[derive(Debug, Clone, Default)]
pub struct LoadedCliResources {
    pub prompt_templates: Vec<PromptTemplate>,
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
}

pub fn load_cli_resources(
    parsed: &Args,
    cwd: &Path,
    agent_dir: Option<&Path>,
) -> LoadedCliResources {
    let mut warnings = Vec::new();

    if let Some(extension_paths) = parsed.extensions.as_ref() {
        warnings.extend(validate_resource_paths(cwd, extension_paths, "Extension"));
    }

    let prompt_templates =
        load_prompt_templates(pi_coding_agent_core::LoadPromptTemplatesOptions {
            cwd: cwd.to_path_buf(),
            agent_dir: agent_dir.map(Path::to_path_buf),
            prompt_paths: parsed.prompt_templates.clone().unwrap_or_default(),
            include_defaults: !parsed.no_prompt_templates,
        });
    warnings.extend(
        prompt_templates
            .diagnostics
            .iter()
            .map(format_resource_diagnostic),
    );

    let skills = load_skills(pi_coding_agent_core::LoadSkillsOptions {
        cwd: cwd.to_path_buf(),
        agent_dir: agent_dir.map(Path::to_path_buf),
        skill_paths: parsed.skills.clone().unwrap_or_default(),
        include_defaults: !parsed.no_skills,
    });
    warnings.extend(skills.diagnostics.iter().map(format_resource_diagnostic));

    if let Some(theme_paths) = parsed.themes.as_ref() {
        warnings.extend(validate_resource_paths(cwd, theme_paths, "Theme"));
    }

    LoadedCliResources {
        prompt_templates: prompt_templates.prompts,
        skills: skills.skills,
        warnings,
    }
}

pub fn build_selected_tools(
    parsed: &Args,
    cwd: &Path,
    auto_resize_images: bool,
) -> (Vec<String>, Vec<AgentTool>) {
    let requested = if parsed.no_tools {
        parsed.tools.clone().unwrap_or_default()
    } else if let Some(tools) = parsed.tools.clone() {
        tools
    } else {
        vec![
            ToolName::Read,
            ToolName::Bash,
            ToolName::Edit,
            ToolName::Write,
        ]
    };

    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    for tool in requested {
        if seen.insert(tool.as_str().to_owned()) {
            names.push(tool.as_str().to_owned());
        }
    }

    let read_auto_resize = Arc::new(AtomicBool::new(auto_resize_images));
    let tools = names
        .iter()
        .filter_map(|name| match name.as_str() {
            "read" => Some(if auto_resize_images {
                create_read_tool(cwd.to_path_buf())
            } else {
                create_read_tool_with_auto_resize_flag(cwd.to_path_buf(), read_auto_resize.clone())
            }),
            "bash" => Some(create_bash_tool(cwd.to_path_buf())),
            "edit" => Some(create_edit_tool(cwd.to_path_buf())),
            "write" => Some(create_write_tool(cwd.to_path_buf())),
            "grep" => Some(create_grep_tool(cwd.to_path_buf())),
            "find" => Some(create_find_tool(cwd.to_path_buf())),
            "ls" => Some(create_ls_tool(cwd.to_path_buf())),
            _ => None,
        })
        .collect();

    (names, tools)
}

pub fn build_runtime_system_prompt(
    default_system_prompt: &str,
    parsed: &Args,
    cwd: &Path,
    agent_dir: Option<&Path>,
    selected_tools: &[String],
    resources: &LoadedCliResources,
) -> String {
    let (custom_prompt, append_system_prompt, context_files) = if let Some(agent_dir) = agent_dir {
        let resources_from_disk = load_system_prompt_resources(cwd, agent_dir);
        let custom_prompt = resolve_prompt_input(parsed.system_prompt.as_deref())
            .or(resources_from_disk.system_prompt);
        let append_system_prompt = resolve_prompt_input(parsed.append_system_prompt.as_deref())
            .or_else(|| join_prompt_sections(&resources_from_disk.append_system_prompt));
        (
            custom_prompt,
            append_system_prompt,
            resources_from_disk.context_files,
        )
    } else {
        let custom_prompt = resolve_prompt_input(parsed.system_prompt.as_deref()).or_else(|| {
            default_system_prompt
                .strip_prefix(FINALIZED_SYSTEM_PROMPT_PREFIX)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    (!default_system_prompt.is_empty()).then(|| default_system_prompt.to_owned())
                })
        });
        (
            custom_prompt,
            resolve_prompt_input(parsed.append_system_prompt.as_deref()),
            Vec::new(),
        )
    };

    build_system_prompt(BuildSystemPromptOptions {
        custom_prompt,
        selected_tools: selected_tools.to_vec(),
        tool_snippets: tool_snippets(),
        append_system_prompt,
        cwd: Some(cwd.to_path_buf()),
        context_files,
        skills: resources.skills.clone(),
        ..BuildSystemPromptOptions::default()
    })
}

pub fn preprocess_prompt_text(text: &str, resources: &LoadedCliResources) -> String {
    let expanded_skill = expand_skill_command(text, &resources.skills);
    expand_prompt_template(&expanded_skill, &resources.prompt_templates)
}

fn resolve_from_cwd(cwd: &Path, input: &str) -> PathBuf {
    let path = Path::new(input);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn format_resource_diagnostic(diagnostic: &pi_coding_agent_core::ResourceDiagnostic) -> String {
    match diagnostic.path.as_deref() {
        Some(path) => format!("Warning: {} ({path})", diagnostic.message),
        None => format!("Warning: {}", diagnostic.message),
    }
}

fn validate_resource_paths(cwd: &Path, paths: &[String], kind: &str) -> Vec<String> {
    paths.iter()
        .filter_map(|path| {
            let resolved = resolve_from_cwd(cwd, path);
            (!resolved.exists())
                .then(|| format!("Warning: {kind} path does not exist: {}", resolved.display()))
        })
        .collect()
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

fn tool_snippets() -> BTreeMap<String, String> {
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
        (
            String::from("grep"),
            String::from("Search file contents for patterns (respects .gitignore)"),
        ),
        (
            String::from("find"),
            String::from("Find files by glob pattern (respects .gitignore)"),
        ),
        (String::from("ls"), String::from("List directory contents")),
    ])
}
