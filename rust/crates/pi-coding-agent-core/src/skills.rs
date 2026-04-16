use crate::{
    ResourceDiagnostic, SourceInfo,
    frontmatter::{parse_frontmatter, strip_frontmatter},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const CONFIG_DIR_NAME: &str = ".pi";
const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub base_dir: String,
    pub source_info: SourceInfo,
    pub disable_model_invocation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadSkillsResult {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<ResourceDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadSkillsOptions {
    pub cwd: PathBuf,
    pub agent_dir: Option<PathBuf>,
    pub skill_paths: Vec<String>,
    pub include_defaults: bool,
}

#[derive(Debug, Deserialize, Default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "disable-model-invocation")]
    disable_model_invocation: Option<bool>,
}

pub fn load_skills(options: LoadSkillsOptions) -> LoadSkillsResult {
    let LoadSkillsOptions {
        cwd,
        agent_dir,
        skill_paths,
        include_defaults,
    } = options;
    let mut skills_by_name = BTreeMap::<String, Skill>::new();
    let mut diagnostics = Vec::new();

    if include_defaults {
        if let Some(agent_dir) = agent_dir.as_deref() {
            load_skills_from_dir(
                &agent_dir.join("skills"),
                &mut skills_by_name,
                &mut diagnostics,
                Some(("user", agent_dir.join("skills"))),
                true,
            );
        }
        let project_dir = cwd.join(CONFIG_DIR_NAME).join("skills");
        load_skills_from_dir(
            &project_dir,
            &mut skills_by_name,
            &mut diagnostics,
            Some(("project", project_dir.clone())),
            true,
        );
    }

    for skill_path in skill_paths {
        let resolved = resolve_from_cwd(&cwd, &skill_path);
        if !resolved.exists() {
            diagnostics.push(ResourceDiagnostic::new(
                "Skill path does not exist",
                Some(resolved.display().to_string()),
            ));
            continue;
        }

        if resolved.is_dir() {
            load_skills_from_dir(&resolved, &mut skills_by_name, &mut diagnostics, None, true);
        } else if resolved
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("md")
        {
            if let Some(skill) = load_skill_file(
                &resolved,
                source_info_for_path(&resolved, None),
                &mut diagnostics,
            ) {
                add_skill(&mut skills_by_name, &mut diagnostics, skill);
            }
        } else {
            diagnostics.push(ResourceDiagnostic::new(
                "Skill path is not a markdown file",
                Some(resolved.display().to_string()),
            ));
        }
    }

    LoadSkillsResult {
        skills: skills_by_name.into_values().collect(),
        diagnostics,
    }
}

pub fn format_skills_for_prompt(skills: &[Skill]) -> String {
    let visible_skills = skills
        .iter()
        .filter(|skill| !skill.disable_model_invocation)
        .collect::<Vec<_>>();
    if visible_skills.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        String::from(
            "\n\nThe following skills provide specialized instructions for specific tasks.",
        ),
        String::from(
            "Use the read tool to load a skill's file when the task matches its description.",
        ),
        String::from(
            "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.",
        ),
        String::new(),
        String::from("<available_skills>"),
    ];

    for skill in visible_skills {
        lines.push(String::from("  <skill>"));
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.file_path)
        ));
        lines.push(String::from("  </skill>"));
    }

    lines.push(String::from("</available_skills>"));
    lines.join("\n")
}

pub fn expand_skill_command(text: &str, skills: &[Skill]) -> String {
    let Some(command) = text.strip_prefix("/skill:") else {
        return text.to_owned();
    };
    let (skill_name, args) = match command.split_once(' ') {
        Some((skill_name, args)) => (skill_name, args.trim()),
        None => (command, ""),
    };

    let Some(skill) = skills.iter().find(|skill| skill.name == skill_name) else {
        return text.to_owned();
    };

    let Ok(content) = fs::read_to_string(&skill.file_path) else {
        return text.to_owned();
    };
    let body = strip_frontmatter(&content).trim();
    let skill_block = format!(
        "<skill name=\"{}\" location=\"{}\">\nReferences are relative to {}.\n\n{}\n</skill>",
        skill.name, skill.file_path, skill.base_dir, body
    );
    if args.is_empty() {
        skill_block
    } else {
        format!("{skill_block}\n\n{args}")
    }
}

fn load_skills_from_dir(
    dir: &Path,
    skills_by_name: &mut BTreeMap<String, Skill>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    default_scope: Option<(&str, PathBuf)>,
    include_root_markdown_files: bool,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut entries = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    let root_skill = dir.join("SKILL.md");
    if root_skill.is_file() {
        if let Some(skill) = load_skill_file(
            &root_skill,
            source_info_for_path(&root_skill, default_scope.clone()),
            diagnostics,
        ) {
            add_skill(skills_by_name, diagnostics, skill);
        }
        return;
    }

    for path in entries {
        let Some(file_name) = path.file_name().and_then(|file_name| file_name.to_str()) else {
            continue;
        };
        if file_name.starts_with('.') || file_name == "node_modules" {
            continue;
        }

        if path.is_dir() {
            load_skills_from_dir(
                &path,
                skills_by_name,
                diagnostics,
                default_scope.clone(),
                false,
            );
            continue;
        }

        if !include_root_markdown_files {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        if let Some(skill) = load_skill_file(
            &path,
            source_info_for_path(&path, default_scope.clone()),
            diagnostics,
        ) {
            add_skill(skills_by_name, diagnostics, skill);
        }
    }
}

fn add_skill(
    skills_by_name: &mut BTreeMap<String, Skill>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    skill: Skill,
) {
    if let Some(existing) = skills_by_name.get(&skill.name) {
        diagnostics.push(ResourceDiagnostic::new(
            format!(
                "Skill name collision for {} (keeping {})",
                skill.name, existing.file_path
            ),
            Some(skill.file_path.clone()),
        ));
        return;
    }
    skills_by_name.insert(skill.name.clone(), skill);
}

fn load_skill_file(
    path: &Path,
    source_info: SourceInfo,
    diagnostics: &mut Vec<ResourceDiagnostic>,
) -> Option<Skill> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            diagnostics.push(ResourceDiagnostic::new(
                error.to_string(),
                Some(path.display().to_string()),
            ));
            return None;
        }
    };
    let (frontmatter, _body) = parse_frontmatter::<SkillFrontmatter>(&raw);
    let frontmatter = frontmatter.unwrap_or_default();
    let skill_dir = path.parent()?.to_path_buf();
    let parent_dir_name = skill_dir.file_name()?.to_string_lossy().into_owned();
    let name = frontmatter.name.unwrap_or_else(|| parent_dir_name.clone());
    let Some(description) = frontmatter
        .description
        .filter(|description| !description.trim().is_empty())
    else {
        diagnostics.push(ResourceDiagnostic::new(
            "Skill description is required",
            Some(path.display().to_string()),
        ));
        return None;
    };

    validate_name(&name, &parent_dir_name, path, diagnostics);
    validate_description(&description, path, diagnostics);

    Some(Skill {
        name,
        description,
        file_path: path.display().to_string(),
        base_dir: skill_dir.display().to_string(),
        source_info,
        disable_model_invocation: frontmatter.disable_model_invocation.unwrap_or(false),
    })
}

fn validate_name(
    name: &str,
    parent_dir_name: &str,
    path: &Path,
    diagnostics: &mut Vec<ResourceDiagnostic>,
) {
    let path = Some(path.display().to_string());
    if name != parent_dir_name {
        diagnostics.push(ResourceDiagnostic::new(
            format!("Skill name \"{name}\" does not match parent directory \"{parent_dir_name}\""),
            path.clone(),
        ));
    }
    if name.len() > MAX_NAME_LENGTH {
        diagnostics.push(ResourceDiagnostic::new(
            format!("Skill name exceeds {MAX_NAME_LENGTH} characters"),
            path.clone(),
        ));
    }
    if !name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        diagnostics.push(ResourceDiagnostic::new(
            String::from("Skill name contains invalid characters"),
            path.clone(),
        ));
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        diagnostics.push(ResourceDiagnostic::new(
            String::from("Skill name has invalid hyphen placement"),
            path,
        ));
    }
}

fn validate_description(description: &str, path: &Path, diagnostics: &mut Vec<ResourceDiagnostic>) {
    if description.len() > MAX_DESCRIPTION_LENGTH {
        diagnostics.push(ResourceDiagnostic::new(
            format!("Skill description exceeds {MAX_DESCRIPTION_LENGTH} characters"),
            Some(path.display().to_string()),
        ));
    }
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

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
