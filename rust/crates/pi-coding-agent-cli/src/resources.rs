use crate::{
    Args, ToolName,
    package_manager::{
        DefaultPackageManager, PathMetadata, ResolveExtensionSourcesOptions, ResolvedPaths,
    },
};
use pi_agent::AgentTool;
use pi_coding_agent_core::{
    BuildSystemPromptOptions, ContextFile, DefaultResourceLoader, DefaultResourceLoaderOptions,
    PromptTemplate, Skill, SourceInfo, build_system_prompt, expand_prompt_template,
    expand_skill_command, load_prompt_templates, load_skills, load_system_prompt_resources,
    resolve_prompt_input,
};
use pi_coding_agent_tools::{
    create_bash_tool, create_edit_tool, create_find_tool, create_grep_tool, create_ls_tool,
    create_read_tool_with_auto_resize_flag, create_write_tool,
};
use pi_coding_agent_tui::{LoadThemesOptions, Theme, load_themes};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Component, Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

const FINALIZED_SYSTEM_PROMPT_PREFIX: &str = "\0pi-final-system-prompt\n";

#[derive(Debug, Clone, Default)]
pub struct LoadedCliResources {
    pub prompt_templates: Vec<PromptTemplate>,
    pub skills: Vec<Skill>,
    pub themes: Vec<Theme>,
    pub warnings: Vec<String>,
    pub context_files: Vec<ContextFile>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CliResourceLoader {
    parsed: Args,
    cwd: PathBuf,
    agent_dir: Option<PathBuf>,
    resources: LoadedCliResources,
}

impl CliResourceLoader {
    pub fn load(parsed: &Args, cwd: &Path, agent_dir: Option<&Path>) -> Self {
        Self {
            parsed: parsed.clone(),
            cwd: cwd.to_path_buf(),
            agent_dir: agent_dir.map(Path::to_path_buf),
            resources: load_cli_resources(parsed, cwd, agent_dir),
        }
    }

    pub fn resources(&self) -> &LoadedCliResources {
        &self.resources
    }

    pub fn warnings(&self) -> &[String] {
        &self.resources.warnings
    }

    pub fn prompt_templates(&self) -> &[PromptTemplate] {
        &self.resources.prompt_templates
    }

    pub fn skills(&self) -> &[Skill] {
        &self.resources.skills
    }

    pub fn themes(&self) -> &[Theme] {
        &self.resources.themes
    }

    pub fn context_files(&self) -> &[ContextFile] {
        &self.resources.context_files
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.resources.system_prompt.as_deref()
    }

    pub fn append_system_prompt(&self) -> &[String] {
        &self.resources.append_system_prompt
    }

    pub fn reload(&mut self) {
        self.resources = load_cli_resources(&self.parsed, &self.cwd, self.agent_dir.as_deref());
    }

    pub fn extend_resources_from_extensions(
        &mut self,
        skill_paths: &[ExtensionResourcePath],
        prompt_paths: &[ExtensionResourcePath],
        theme_paths: &[ExtensionResourcePath],
    ) -> Vec<String> {
        extend_cli_resources_from_extensions(
            &mut self.resources,
            &self.cwd,
            skill_paths,
            prompt_paths,
            theme_paths,
        )
    }

    pub fn build_system_prompt(
        &self,
        default_system_prompt: &str,
        selected_tools: &[String],
    ) -> String {
        build_runtime_system_prompt(
            default_system_prompt,
            &self.parsed,
            &self.cwd,
            self.agent_dir.as_deref(),
            selected_tools,
            &self.resources,
        )
    }

    pub fn preprocess_prompt_text(&self, text: &str) -> String {
        preprocess_prompt_text(text, &self.resources)
    }

    pub fn into_resources(self) -> LoadedCliResources {
        self.resources
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionResourcePath {
    pub path: String,
    pub extension_path: String,
}

pub fn load_cli_resources(
    parsed: &Args,
    cwd: &Path,
    agent_dir: Option<&Path>,
) -> LoadedCliResources {
    let mut warnings = Vec::new();
    let mut metadata_by_path = BTreeMap::<String, PathMetadata>::new();

    if let Some(extension_paths) = parsed.extensions.as_ref() {
        warnings.extend(validate_local_extension_sources(cwd, extension_paths));
    }
    warnings.extend(validate_resource_paths(
        cwd,
        parsed.skills.as_deref().unwrap_or(&[]),
        "Skill",
    ));
    warnings.extend(validate_resource_paths(
        cwd,
        parsed.prompt_templates.as_deref().unwrap_or(&[]),
        "Prompt template",
    ));
    warnings.extend(validate_resource_paths(
        cwd,
        parsed.themes.as_deref().unwrap_or(&[]),
        "Theme",
    ));

    let mut configured_paths = ResolvedPaths::default();
    let mut temporary_extension_paths = ResolvedPaths::default();

    if let Some(agent_dir) = agent_dir {
        let package_manager =
            DefaultPackageManager::new(cwd.to_path_buf(), agent_dir.to_path_buf());
        match package_manager.resolve() {
            Ok(output) => {
                warnings.extend(output.warnings.iter().map(format_settings_warning));
                configured_paths = output.resolved;
            }
            Err(error) => warnings.push(format!("Warning: {error}")),
        }

        if let Some(extension_sources) = parsed.extensions.as_ref() {
            match package_manager.resolve_extension_sources(
                extension_sources,
                ResolveExtensionSourcesOptions {
                    temporary: true,
                    local: false,
                },
            ) {
                Ok(output) => {
                    temporary_extension_paths = output.resolved;
                }
                Err(error) => warnings.push(format!("Warning: {error}")),
            }
        }
    }

    let configured_prompt_paths = if agent_dir.is_some() && !parsed.no_prompt_templates {
        enabled_resource_paths(&configured_paths.prompts, &mut metadata_by_path)
    } else {
        Vec::new()
    };
    let configured_skill_paths = if agent_dir.is_some() && !parsed.no_skills {
        enabled_resource_paths(&configured_paths.skills, &mut metadata_by_path)
    } else {
        Vec::new()
    };
    let configured_theme_paths = if agent_dir.is_some() && !parsed.no_themes {
        enabled_resource_paths(&configured_paths.themes, &mut metadata_by_path)
    } else {
        Vec::new()
    };

    let temporary_prompt_paths =
        enabled_resource_paths(&temporary_extension_paths.prompts, &mut metadata_by_path);
    let temporary_skill_paths =
        enabled_resource_paths(&temporary_extension_paths.skills, &mut metadata_by_path);
    let temporary_theme_paths =
        enabled_resource_paths(&temporary_extension_paths.themes, &mut metadata_by_path);

    let prompt_paths = merge_unique_paths(
        &temporary_prompt_paths,
        &configured_prompt_paths,
        parsed.prompt_templates.as_deref().unwrap_or(&[]),
        cwd,
    );
    let skill_paths = merge_unique_paths(
        &temporary_skill_paths,
        &configured_skill_paths,
        parsed.skills.as_deref().unwrap_or(&[]),
        cwd,
    );
    let theme_paths = merge_unique_paths(
        &temporary_theme_paths,
        &configured_theme_paths,
        parsed.themes.as_deref().unwrap_or(&[]),
        cwd,
    );

    let base_resources = DefaultResourceLoader::load(DefaultResourceLoaderOptions {
        cwd: cwd.to_path_buf(),
        agent_dir: agent_dir.map(Path::to_path_buf),
        prompt_paths,
        skill_paths,
        include_prompt_defaults: agent_dir.is_none() && !parsed.no_prompt_templates,
        include_skill_defaults: agent_dir.is_none() && !parsed.no_skills,
        ..DefaultResourceLoaderOptions::default()
    });
    warnings.extend(base_resources.warnings().iter().cloned());

    let mut prompt_templates = base_resources.prompt_templates().to_vec();
    apply_prompt_source_info(&mut prompt_templates, &metadata_by_path);

    let mut skills = base_resources.skills().to_vec();
    apply_skill_source_info(&mut skills, &metadata_by_path);

    let themes = load_themes(LoadThemesOptions {
        cwd: cwd.to_path_buf(),
        agent_dir: agent_dir.map(Path::to_path_buf),
        theme_paths,
        include_defaults: agent_dir.is_none() && !parsed.no_themes,
    });
    warnings.extend(themes.diagnostics.iter().map(format_resource_diagnostic));
    let themes = themes.themes;

    LoadedCliResources {
        prompt_templates,
        skills,
        themes,
        warnings,
        context_files: base_resources.context_files().to_vec(),
        system_prompt: base_resources.system_prompt().map(ToOwned::to_owned),
        append_system_prompt: base_resources.append_system_prompt().to_vec(),
    }
}

fn enabled_resource_paths(
    resources: &[crate::package_manager::ResolvedResource],
    metadata_by_path: &mut BTreeMap<String, PathMetadata>,
) -> Vec<String> {
    let mut enabled = Vec::new();
    for resource in resources {
        if !resource.enabled {
            continue;
        }
        metadata_by_path.insert(normalize_path(&resource.path), resource.metadata.clone());
        enabled.push(resource.path.clone());
    }
    enabled
}

fn merge_unique_paths(
    primary: &[String],
    secondary: &[String],
    additional: &[String],
    cwd: &Path,
) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();

    for path in primary.iter().chain(secondary.iter()) {
        let normalized = normalize_path(path);
        if seen.insert(normalized) {
            merged.push(path.clone());
        }
    }

    for path in additional {
        let resolved = resolve_from_cwd(cwd, path);
        let normalized = normalize_path(&resolved.display().to_string());
        if seen.insert(normalized) {
            merged.push(path.clone());
        }
    }

    merged
}

fn apply_skill_source_info(
    skills: &mut [Skill],
    metadata_by_path: &BTreeMap<String, PathMetadata>,
) {
    for skill in skills {
        if let Some(metadata) = metadata_by_path.get(&normalize_path(&skill.file_path)) {
            skill.source_info = source_info_from_metadata(&skill.file_path, metadata);
        }
    }
}

fn apply_prompt_source_info(
    prompts: &mut [PromptTemplate],
    metadata_by_path: &BTreeMap<String, PathMetadata>,
) {
    for prompt in prompts {
        if let Some(metadata) = metadata_by_path.get(&normalize_path(&prompt.file_path)) {
            prompt.source_info = source_info_from_metadata(&prompt.file_path, metadata);
        }
    }
}

fn source_info_from_metadata(path: &str, metadata: &PathMetadata) -> SourceInfo {
    SourceInfo {
        path: path.to_owned(),
        source: metadata.source.clone(),
        scope: metadata.scope.as_str().to_owned(),
        origin: metadata.origin.as_str().to_owned(),
        base_dir: metadata.base_dir.clone(),
    }
}

fn normalize_path(path: &str) -> String {
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }
    normalized.display().to_string().replace('\\', "/")
}

fn format_settings_warning(warning: &pi_config::SettingsWarning) -> String {
    format!(
        "Warning ({} settings): {}",
        warning.scope.label(),
        warning.message
    )
}

fn validate_local_extension_sources(cwd: &Path, paths: &[String]) -> Vec<String> {
    let local_paths = paths
        .iter()
        .filter(|path| is_local_source(path))
        .cloned()
        .collect::<Vec<_>>();
    validate_resource_paths(cwd, &local_paths, "Extension")
}

fn is_local_source(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.starts_with("npm:")
        && !trimmed.starts_with("git:")
        && !trimmed.starts_with("http://")
        && !trimmed.starts_with("https://")
        && !trimmed.starts_with("ssh://")
        && !trimmed.starts_with("git://")
}

pub fn extend_cli_resources_from_extensions(
    resources: &mut LoadedCliResources,
    cwd: &Path,
    skill_paths: &[ExtensionResourcePath],
    prompt_paths: &[ExtensionResourcePath],
    theme_paths: &[ExtensionResourcePath],
) -> Vec<String> {
    let mut warnings = Vec::new();

    for entry in skill_paths {
        let loaded = load_skills(pi_coding_agent_core::LoadSkillsOptions {
            cwd: cwd.to_path_buf(),
            agent_dir: None,
            skill_paths: vec![entry.path.clone()],
            include_defaults: false,
        });
        warnings.extend(loaded.diagnostics.iter().map(format_resource_diagnostic));

        for mut skill in loaded.skills {
            skill.source_info =
                source_info_for_extension_resource(&skill.file_path, &entry.extension_path);
            if let Some(existing) = resources
                .skills
                .iter()
                .find(|existing| existing.name == skill.name)
            {
                warnings.push(format!(
                    "Warning: Skill name collision for {} (keeping {}) ({})",
                    skill.name, existing.file_path, skill.file_path
                ));
                continue;
            }
            resources.skills.push(skill);
        }
    }

    for entry in prompt_paths {
        let loaded = load_prompt_templates(pi_coding_agent_core::LoadPromptTemplatesOptions {
            cwd: cwd.to_path_buf(),
            agent_dir: None,
            prompt_paths: vec![entry.path.clone()],
            include_defaults: false,
        });
        warnings.extend(loaded.diagnostics.iter().map(format_resource_diagnostic));

        for mut prompt in loaded.prompts {
            prompt.source_info =
                source_info_for_extension_resource(&prompt.file_path, &entry.extension_path);
            if let Some(existing) = resources
                .prompt_templates
                .iter()
                .find(|existing| existing.name == prompt.name)
            {
                warnings.push(format!(
                    "Warning: Prompt template name collision for /{} (keeping {}) ({})",
                    prompt.name, existing.file_path, prompt.file_path
                ));
                continue;
            }
            resources.prompt_templates.push(prompt);
        }
    }

    for entry in theme_paths {
        let loaded = load_themes(LoadThemesOptions {
            cwd: cwd.to_path_buf(),
            agent_dir: None,
            theme_paths: vec![entry.path.clone()],
            include_defaults: false,
        });
        warnings.extend(loaded.diagnostics.iter().map(format_resource_diagnostic));

        for theme in loaded.themes {
            if let Some(existing) = resources
                .themes
                .iter()
                .find(|existing| existing.name() == theme.name())
            {
                warnings.push(format!(
                    "Warning: Theme name collision for {} (keeping {}) ({})",
                    theme.name(),
                    existing.source_path().unwrap_or("<builtin>"),
                    theme.source_path().unwrap_or("<in-memory>")
                ));
                continue;
            }
            let source_path = theme
                .source_path()
                .unwrap_or(entry.path.as_str())
                .to_owned();
            resources.themes.push(theme.with_source_info(Some(
                source_info_for_extension_resource(&source_path, &entry.extension_path),
            )));
        }
    }

    warnings
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
            "read" => Some(create_read_tool_with_auto_resize_flag(
                cwd.to_path_buf(),
                read_auto_resize.clone(),
            )),
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
        selected_tools_explicit: true,
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
    let normalized = normalize_input_path(input);
    let path = Path::new(&normalized);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn normalize_input_path(input: &str) -> String {
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

fn format_resource_diagnostic(diagnostic: &pi_coding_agent_core::ResourceDiagnostic) -> String {
    match diagnostic.path.as_deref() {
        Some(path) => format!("Warning: {} ({path})", diagnostic.message),
        None => format!("Warning: {}", diagnostic.message),
    }
}

fn validate_resource_paths(cwd: &Path, paths: &[String], kind: &str) -> Vec<String> {
    paths
        .iter()
        .filter_map(|path| {
            let resolved = resolve_from_cwd(cwd, path);
            (!resolved.exists()).then(|| {
                format!(
                    "Warning: {kind} path does not exist: {}",
                    resolved.display()
                )
            })
        })
        .collect()
}

fn extension_source_label(extension_path: &str) -> String {
    if extension_path.starts_with('<') {
        return format!("extension:{}", extension_path.trim_matches(&['<', '>'][..]));
    }

    let base = Path::new(extension_path)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or(extension_path);
    let name = base
        .strip_suffix(".ts")
        .or_else(|| base.strip_suffix(".js"))
        .unwrap_or(base);
    format!("extension:{name}")
}

fn source_info_for_extension_resource(resource_path: &str, extension_path: &str) -> SourceInfo {
    let base_dir = if extension_path.starts_with('<') {
        None
    } else {
        Path::new(extension_path)
            .parent()
            .map(|parent| parent.display().to_string())
    };
    SourceInfo {
        path: resource_path.to_owned(),
        source: extension_source_label(extension_path),
        scope: String::from("temporary"),
        origin: String::from("top-level"),
        base_dir,
    }
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

#[cfg(test)]
mod tests {
    use super::{CliResourceLoader, ExtensionResourcePath};
    use crate::Args;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    const DARK_THEME_JSON: &str = include_str!("../../pi-coding-agent-tui/src/theme/dark.json");

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pi-coding-agent-cli-resource-loader-{prefix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn custom_theme_json(name: &str) -> String {
        DARK_THEME_JSON.replace("\"name\": \"dark\"", &format!("\"name\": \"{name}\""))
    }

    #[test]
    fn cli_resource_loader_reload_refreshes_disk_backed_resources() {
        let temp_dir = unique_temp_dir("reload");
        let cwd = temp_dir.join("project");
        let agent_dir = temp_dir.join("agent");
        let prompts_dir = cwd.join(".pi").join("prompts");
        let skill_dir = cwd.join(".pi").join("skills").join("review-code");
        let themes_dir = cwd.join(".pi").join("themes");
        fs::create_dir_all(&prompts_dir).unwrap();
        fs::create_dir_all(&skill_dir).unwrap();
        fs::create_dir_all(&themes_dir).unwrap();
        fs::create_dir_all(&agent_dir).unwrap();

        fs::write(prompts_dir.join("review.md"), "Review $1\n").unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: Review code safely\n---\nRead the target file first.\n",
        )
        .unwrap();
        fs::write(themes_dir.join("custom.json"), custom_theme_json("custom")).unwrap();
        fs::write(agent_dir.join("AGENTS.md"), "global rules\n").unwrap();
        fs::write(cwd.join(".pi").join("SYSTEM.md"), "project system\n").unwrap();
        fs::write(cwd.join(".pi").join("APPEND_SYSTEM.md"), "project append\n").unwrap();

        let mut loader = CliResourceLoader::load(&Args::default(), &cwd, Some(&agent_dir));
        assert!(
            loader.warnings().is_empty(),
            "warnings: {:?}",
            loader.warnings()
        );
        assert_eq!(loader.prompt_templates().len(), 1);
        assert_eq!(loader.skills().len(), 1);
        assert_eq!(loader.themes().len(), 1);
        assert!(
            loader
                .context_files()
                .iter()
                .any(|file| file.path.ends_with("AGENTS.md")),
            "context files: {:?}",
            loader.context_files()
        );
        assert_eq!(loader.system_prompt(), Some("project system\n"));
        assert_eq!(
            loader.append_system_prompt(),
            &[String::from("project append\n")]
        );
        assert_eq!(
            loader.preprocess_prompt_text("/review src/lib.rs"),
            "Review src/lib.rs\n"
        );

        fs::write(prompts_dir.join("review.md"), "Updated review $1\n").unwrap();
        fs::write(cwd.join(".pi").join("SYSTEM.md"), "updated system\n").unwrap();
        fs::write(cwd.join(".pi").join("APPEND_SYSTEM.md"), "updated append\n").unwrap();

        loader.reload();

        assert_eq!(
            loader.preprocess_prompt_text("/review src/main.rs"),
            "Updated review src/main.rs\n"
        );
        assert_eq!(loader.system_prompt(), Some("updated system\n"));
        assert_eq!(
            loader.append_system_prompt(),
            &[String::from("updated append\n")]
        );
    }

    #[test]
    fn cli_resource_loader_extends_resources_from_extension_paths() {
        let temp_dir = unique_temp_dir("extend");
        let cwd = temp_dir.join("project");
        let agent_dir = temp_dir.join("agent");
        let extension_dir = temp_dir.join("extension");
        let extension_skill_dir = extension_dir.join("skills").join("extension-review");
        let extension_prompts_dir = extension_dir.join("prompts");
        let extension_themes_dir = extension_dir.join("themes");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(&extension_skill_dir).unwrap();
        fs::create_dir_all(&extension_prompts_dir).unwrap();
        fs::create_dir_all(&extension_themes_dir).unwrap();

        fs::write(
            extension_skill_dir.join("SKILL.md"),
            "---\ndescription: Review extension code\n---\nCheck extension assets first.\n",
        )
        .unwrap();
        fs::write(
            extension_prompts_dir.join("review.md"),
            "Extension review $1\n",
        )
        .unwrap();
        fs::write(
            extension_themes_dir.join("extension.json"),
            custom_theme_json("extension"),
        )
        .unwrap();

        let mut loader = CliResourceLoader::load(&Args::default(), &cwd, Some(&agent_dir));
        let warnings = loader.extend_resources_from_extensions(
            &[ExtensionResourcePath {
                path: extension_skill_dir.join("SKILL.md").display().to_string(),
                extension_path: extension_dir.join("index.ts").display().to_string(),
            }],
            &[ExtensionResourcePath {
                path: extension_prompts_dir
                    .join("review.md")
                    .display()
                    .to_string(),
                extension_path: extension_dir.join("index.ts").display().to_string(),
            }],
            &[ExtensionResourcePath {
                path: extension_themes_dir
                    .join("extension.json")
                    .display()
                    .to_string(),
                extension_path: extension_dir.join("index.ts").display().to_string(),
            }],
        );

        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(loader.skills().len(), 1);
        assert_eq!(loader.prompt_templates().len(), 1);
        assert_eq!(loader.themes().len(), 1);
        assert_eq!(
            loader.preprocess_prompt_text("/review src/lib.rs"),
            "Extension review src/lib.rs\n"
        );
        assert!(
            loader
                .resources()
                .skills
                .iter()
                .all(|skill| skill.source_info.source.starts_with("extension:")),
            "skills: {:?}",
            loader.resources().skills
        );
        assert!(
            loader
                .resources()
                .prompt_templates
                .iter()
                .all(|prompt| prompt.source_info.source.starts_with("extension:")),
            "prompts: {:?}",
            loader.resources().prompt_templates
        );
        assert!(
            loader.resources().themes.iter().all(|theme| theme
                .source_info()
                .is_some_and(|source| source.source.starts_with("extension:"))),
            "themes: {:?}",
            loader.resources().themes
        );
    }
}
