use crate::{
    ResourceDiagnostic, SourceInfo,
    frontmatter::{parse_frontmatter, strip_frontmatter},
};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

const CONFIG_DIR_NAME: &str = ".pi";
const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;
const IGNORE_FILE_NAMES: &[&str] = &[".gitignore", ".ignore", ".fdignore"];

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
    let mut real_paths = BTreeSet::new();

    if include_defaults {
        if let Some(agent_dir) = agent_dir.as_deref() {
            load_skills_from_dir(
                &agent_dir.join("skills"),
                &mut skills_by_name,
                &mut diagnostics,
                Some(("user", agent_dir.join("skills"))),
                true,
                &mut real_paths,
                &[],
                None,
            );
        }
        let project_dir = cwd.join(CONFIG_DIR_NAME).join("skills");
        load_skills_from_dir(
            &project_dir,
            &mut skills_by_name,
            &mut diagnostics,
            Some(("project", project_dir.clone())),
            true,
            &mut real_paths,
            &[],
            None,
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

        let default_scope = default_scope_for_path(&resolved, &cwd, agent_dir.as_deref());
        if resolved.is_dir() {
            load_skills_from_dir(
                &resolved,
                &mut skills_by_name,
                &mut diagnostics,
                default_scope,
                true,
                &mut real_paths,
                &[],
                None,
            );
        } else if resolved
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("md")
        {
            if let Some(skill) = load_skill_file(
                &resolved,
                source_info_for_path(&resolved, default_scope),
                &mut diagnostics,
            ) {
                add_skill(
                    &mut skills_by_name,
                    &mut diagnostics,
                    &mut real_paths,
                    skill,
                );
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
    real_paths: &mut BTreeSet<String>,
    inherited_ignore_patterns: &[String],
    root_dir: Option<&Path>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let root_dir = root_dir.unwrap_or(dir);
    let mut ignore_patterns = inherited_ignore_patterns.to_vec();
    extend_ignore_patterns(&mut ignore_patterns, dir, root_dir);
    let ignore_matcher = build_ignore_matcher(root_dir, &ignore_patterns);

    let mut entries = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    let root_skill = dir.join("SKILL.md");
    if is_file_path(&root_skill) && !is_ignored(&ignore_matcher, &root_skill, false) {
        if let Some(skill) = load_skill_file(
            &root_skill,
            source_info_for_path(&root_skill, default_scope.clone()),
            diagnostics,
        ) {
            add_skill(skills_by_name, diagnostics, real_paths, skill);
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

        let is_dir = path.is_dir();
        let is_file = path.is_file();
        if !is_dir && !is_file {
            continue;
        }
        if is_ignored(&ignore_matcher, &path, is_dir) {
            continue;
        }

        if is_dir {
            load_skills_from_dir(
                &path,
                skills_by_name,
                diagnostics,
                default_scope.clone(),
                false,
                real_paths,
                &ignore_patterns,
                Some(root_dir),
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
            add_skill(skills_by_name, diagnostics, real_paths, skill);
        }
    }
}

fn add_skill(
    skills_by_name: &mut BTreeMap<String, Skill>,
    diagnostics: &mut Vec<ResourceDiagnostic>,
    real_paths: &mut BTreeSet<String>,
    skill: Skill,
) {
    let real_path = fs::canonicalize(&skill.file_path)
        .unwrap_or_else(|_| PathBuf::from(&skill.file_path))
        .display()
        .to_string();
    if !real_paths.insert(real_path) {
        return;
    }

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

fn extend_ignore_patterns(patterns: &mut Vec<String>, dir: &Path, root_dir: &Path) {
    let relative_dir = dir.strip_prefix(root_dir).unwrap_or(dir);
    let prefix = if relative_dir.as_os_str().is_empty() {
        String::new()
    } else {
        format!("{}/", to_posix_path(relative_dir))
    };

    for file_name in IGNORE_FILE_NAMES {
        let ignore_path = dir.join(file_name);
        let Ok(content) = fs::read_to_string(ignore_path) else {
            continue;
        };
        for line in content.lines() {
            if let Some(pattern) = prefix_ignore_pattern(line, &prefix) {
                patterns.push(pattern);
            }
        }
    }
}

fn prefix_ignore_pattern(line: &str, prefix: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('#') && !trimmed.starts_with("\\#") {
        return None;
    }

    let mut pattern = line.to_owned();
    let mut negated = false;
    if pattern.starts_with('!') {
        negated = true;
        pattern.remove(0);
    } else if pattern.starts_with("\\!") {
        pattern.remove(0);
    }
    if pattern.starts_with('/') {
        pattern.remove(0);
    }

    let prefixed = if prefix.is_empty() {
        pattern
    } else {
        format!("{prefix}{pattern}")
    };
    Some(if negated {
        format!("!{prefixed}")
    } else {
        prefixed
    })
}

fn build_ignore_matcher(root_dir: &Path, patterns: &[String]) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root_dir);
    for pattern in patterns {
        let _ = builder.add_line(None, pattern);
    }
    builder.build().unwrap_or_else(|_| {
        GitignoreBuilder::new(root_dir)
            .build()
            .expect("empty gitignore builder must succeed")
    })
}

fn is_ignored(matcher: &Gitignore, path: &Path, is_dir: bool) -> bool {
    matcher
        .matched_path_or_any_parents(path, is_dir)
        .is_ignore()
}

fn is_file_path(path: &Path) -> bool {
    path.is_file()
}

fn default_scope_for_path(
    path: &Path,
    cwd: &Path,
    agent_dir: Option<&Path>,
) -> Option<(&'static str, PathBuf)> {
    let project_dir = cwd.join(CONFIG_DIR_NAME).join("skills");
    if is_under_path(path, &project_dir) {
        return Some(("project", project_dir));
    }

    let user_dir = agent_dir.map(|agent_dir| agent_dir.join("skills"));
    user_dir.and_then(|user_dir| is_under_path(path, &user_dir).then_some(("user", user_dir)))
}

fn is_under_path(target: &Path, root: &Path) -> bool {
    if target == root {
        return true;
    }
    target.starts_with(root)
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
    let normalized = normalize_path(input);
    let path = Path::new(&normalized);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn normalize_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed == "~" {
        return home_dir();
    }
    if let Some(path) = trimmed.strip_prefix("~/") {
        return Path::new(&home_dir()).join(path).display().to_string();
    }
    trimmed.to_owned()
}

fn home_dir() -> String {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| String::from("~"))
}

fn to_posix_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
