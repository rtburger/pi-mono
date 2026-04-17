use crate::{
    ContextFile, LoadPromptTemplatesOptions, LoadSkillsOptions, PromptTemplate, ResourceDiagnostic,
    Skill, SourceInfo, expand_prompt_template, expand_skill_command, load_prompt_templates,
    load_skills, load_system_prompt_resources, resolve_prompt_input,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePathEntry {
    pub path: String,
    pub source_info: SourceInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedResources {
    pub prompt_templates: Vec<PromptTemplate>,
    pub skills: Vec<Skill>,
    pub warnings: Vec<String>,
    pub context_files: Vec<ContextFile>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefaultResourceLoaderOptions {
    pub cwd: PathBuf,
    pub agent_dir: Option<PathBuf>,
    pub prompt_paths: Vec<String>,
    pub skill_paths: Vec<String>,
    pub include_prompt_defaults: bool,
    pub include_skill_defaults: bool,
    pub system_prompt_input: Option<String>,
    pub append_system_prompt_input: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DefaultResourceLoader {
    options: DefaultResourceLoaderOptions,
    resources: LoadedResources,
    prompt_source_infos: BTreeMap<String, SourceInfo>,
    skill_source_infos: BTreeMap<String, SourceInfo>,
    last_prompt_paths: Vec<String>,
    last_skill_paths: Vec<String>,
}

impl DefaultResourceLoader {
    pub fn load(options: DefaultResourceLoaderOptions) -> Self {
        let last_prompt_paths = merge_paths(&options.cwd, &options.prompt_paths, &[]);
        let last_skill_paths = merge_paths(&options.cwd, &options.skill_paths, &[]);
        let mut loader = Self {
            options,
            resources: LoadedResources::default(),
            prompt_source_infos: BTreeMap::new(),
            skill_source_infos: BTreeMap::new(),
            last_prompt_paths,
            last_skill_paths,
        };
        loader.reload();
        loader
    }

    pub fn resources(&self) -> &LoadedResources {
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
        self.resources = load_resources(
            &self.options,
            &self.last_prompt_paths,
            &self.last_skill_paths,
            &self.prompt_source_infos,
            &self.skill_source_infos,
        );
    }

    pub fn extend_resources(
        &mut self,
        skill_paths: &[ResourcePathEntry],
        prompt_paths: &[ResourcePathEntry],
    ) {
        for entry in skill_paths {
            let resolved = resolve_from_cwd(&self.options.cwd, &entry.path);
            self.skill_source_infos
                .insert(normalize_path(&resolved), entry.source_info.clone());
        }
        for entry in prompt_paths {
            let resolved = resolve_from_cwd(&self.options.cwd, &entry.path);
            self.prompt_source_infos
                .insert(normalize_path(&resolved), entry.source_info.clone());
        }

        self.last_skill_paths = merge_paths(
            &self.options.cwd,
            &self.last_skill_paths,
            &skill_paths
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
        );
        self.last_prompt_paths = merge_paths(
            &self.options.cwd,
            &self.last_prompt_paths,
            &prompt_paths
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
        );
        self.reload();
    }

    pub fn preprocess_prompt_text(&self, text: &str) -> String {
        let expanded_skill = expand_skill_command(text, &self.resources.skills);
        expand_prompt_template(&expanded_skill, &self.resources.prompt_templates)
    }
}

fn load_resources(
    options: &DefaultResourceLoaderOptions,
    prompt_paths: &[String],
    skill_paths: &[String],
    prompt_source_infos: &BTreeMap<String, SourceInfo>,
    skill_source_infos: &BTreeMap<String, SourceInfo>,
) -> LoadedResources {
    let prompt_result = load_prompt_templates(LoadPromptTemplatesOptions {
        cwd: options.cwd.clone(),
        agent_dir: options.agent_dir.clone(),
        prompt_paths: prompt_paths.to_vec(),
        include_defaults: options.include_prompt_defaults,
    });
    let mut prompt_templates = prompt_result.prompts;
    for prompt in &mut prompt_templates {
        if let Some(source_info) = find_source_info_for_path(&prompt.file_path, prompt_source_infos)
        {
            prompt.source_info = source_info;
        }
    }

    let skill_result = load_skills(LoadSkillsOptions {
        cwd: options.cwd.clone(),
        agent_dir: options.agent_dir.clone(),
        skill_paths: skill_paths.to_vec(),
        include_defaults: options.include_skill_defaults,
    });
    let mut skills = skill_result.skills;
    for skill in &mut skills {
        if let Some(source_info) = find_source_info_for_path(&skill.file_path, skill_source_infos) {
            skill.source_info = source_info;
        }
    }

    let mut warnings = prompt_result
        .diagnostics
        .iter()
        .map(format_resource_diagnostic)
        .collect::<Vec<_>>();
    warnings.extend(
        skill_result
            .diagnostics
            .iter()
            .map(format_resource_diagnostic),
    );

    let prompt_resources = options
        .agent_dir
        .as_deref()
        .map(|agent_dir| load_system_prompt_resources(&options.cwd, agent_dir));
    let context_files = prompt_resources
        .as_ref()
        .map(|resources| resources.context_files.clone())
        .unwrap_or_default();
    let system_prompt =
        resolve_prompt_input(options.system_prompt_input.as_deref()).or_else(|| {
            prompt_resources
                .as_ref()
                .and_then(|resources| resources.system_prompt.clone())
        });
    let append_system_prompt =
        match resolve_prompt_input(options.append_system_prompt_input.as_deref()) {
            Some(prompt) => vec![prompt],
            None => prompt_resources
                .map(|resources| resources.append_system_prompt)
                .unwrap_or_default(),
        };

    LoadedResources {
        prompt_templates,
        skills,
        warnings,
        context_files,
        system_prompt,
        append_system_prompt,
    }
}

fn find_source_info_for_path(
    resource_path: &str,
    source_infos: &BTreeMap<String, SourceInfo>,
) -> Option<SourceInfo> {
    if resource_path.is_empty() {
        return None;
    }

    let normalized_resource_path = normalize_path(&PathBuf::from(resource_path));
    for (source_path, source_info) in source_infos {
        if normalized_resource_path == *source_path
            || normalized_resource_path.starts_with(&format!("{source_path}/"))
        {
            let mut resolved = source_info.clone();
            resolved.path = resource_path.to_owned();
            return Some(resolved);
        }
    }

    None
}

fn merge_paths(cwd: &Path, primary: &[String], additional: &[String]) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();

    for path in primary.iter().chain(additional.iter()) {
        let resolved = resolve_from_cwd(cwd, path);
        let normalized = normalize_path(&resolved);
        if seen.insert(normalized) {
            merged.push(resolved.display().to_string());
        }
    }

    merged
}

fn format_resource_diagnostic(diagnostic: &ResourceDiagnostic) -> String {
    match diagnostic.path.as_deref() {
        Some(path) => format!("Warning: {} ({path})", diagnostic.message),
        None => format!("Warning: {}", diagnostic.message),
    }
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

fn normalize_path(path: &Path) -> String {
    let mut normalized = PathBuf::new();
    for component in path.components() {
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
