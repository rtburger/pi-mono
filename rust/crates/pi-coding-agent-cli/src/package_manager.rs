use globset::Glob;
use pi_config::{
    FilteredPackageSource, PackageSource, ResourceSettings, SettingsWarning, load_resource_settings,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

const CONFIG_DIR_NAME: &str = ".pi";
const SETTINGS_FILE_NAME: &str = "settings.json";
const PACKAGE_JSON_FILE_NAME: &str = "package.json";
const IGNORE_FILE_NAMES: &[&str] = &[".gitignore", ".ignore", ".fdignore"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceScope {
    User,
    Project,
    Temporary,
}

impl ResourceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Temporary => "temporary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceOrigin {
    Package,
    TopLevel,
}

impl ResourceOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::TopLevel => "top-level",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMetadata {
    pub source: String,
    pub scope: ResourceScope,
    pub origin: ResourceOrigin,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResource {
    pub path: String,
    pub enabled: bool,
    pub metadata: PathMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedPaths {
    pub extensions: Vec<ResolvedResource>,
    pub skills: Vec<ResolvedResource>,
    pub prompts: Vec<ResolvedResource>,
    pub themes: Vec<ResolvedResource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageResolutionOutput {
    pub resolved: ResolvedPaths,
    pub warnings: Vec<SettingsWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredPackage {
    pub source: String,
    pub scope: ResourceScope,
    pub filtered: bool,
    pub installed_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolveExtensionSourcesOptions {
    pub temporary: bool,
    pub local: bool,
}

#[derive(Debug, Clone)]
pub struct DefaultPackageManager {
    cwd: PathBuf,
    agent_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct PackageEntry {
    package: PackageSource,
    scope: ResourceScope,
}

#[derive(Debug, Clone, Default)]
struct PackageFilter {
    extensions: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    prompts: Option<Vec<String>>,
    themes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
struct ResourceAccumulator {
    extensions: BTreeMap<String, ResourceState>,
    skills: BTreeMap<String, ResourceState>,
    prompts: BTreeMap<String, ResourceState>,
    themes: BTreeMap<String, ResourceState>,
}

#[derive(Debug, Clone)]
struct ResourceState {
    metadata: PathMetadata,
    enabled: bool,
}

#[derive(Debug, Clone)]
struct IgnoreRule {
    pattern: String,
    negated: bool,
}

#[derive(Debug, Clone, Default)]
struct IgnoreState {
    rules: Vec<IgnoreRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceType {
    Extensions,
    Skills,
    Prompts,
    Themes,
}

impl ResourceType {
    fn all() -> &'static [Self] {
        const TYPES: &[ResourceType] = &[
            ResourceType::Extensions,
            ResourceType::Skills,
            ResourceType::Prompts,
            ResourceType::Themes,
        ];
        TYPES
    }

    fn dir_name(self) -> &'static str {
        match self {
            Self::Extensions => "extensions",
            Self::Skills => "skills",
            Self::Prompts => "prompts",
            Self::Themes => "themes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillDiscoveryMode {
    Pi,
    Agents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedSource {
    Npm(NpmSource),
    Git(GitSource),
    Local(LocalSource),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NpmSource {
    spec: String,
    name: String,
    pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSource {
    repo: String,
    host: String,
    path: String,
    reference: Option<String>,
    pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalSource {
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PiManifest {
    extensions: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    prompts: Option<Vec<String>>,
    themes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum StoredPackageSource {
    Plain(String),
    Filtered {
        source: String,
        extensions: Option<Vec<String>>,
        skills: Option<Vec<String>>,
        prompts: Option<Vec<String>>,
        themes: Option<Vec<String>>,
    },
}

impl DefaultPackageManager {
    pub fn new(cwd: PathBuf, agent_dir: PathBuf) -> Self {
        Self { cwd, agent_dir }
    }

    pub fn settings_warnings(&self) -> Vec<SettingsWarning> {
        load_resource_settings(&self.cwd, &self.agent_dir).warnings
    }

    pub fn has_explicit_extension_configuration(&self) -> bool {
        let settings = load_resource_settings(&self.cwd, &self.agent_dir);
        !settings.global.packages.is_empty()
            || !settings.project.packages.is_empty()
            || !settings.global.extensions.is_empty()
            || !settings.project.extensions.is_empty()
    }

    pub fn resolve(&self) -> Result<PackageResolutionOutput, String> {
        let settings = load_resource_settings(&self.cwd, &self.agent_dir);
        let mut accumulator = ResourceAccumulator::default();

        let mut packages = Vec::new();
        for package in &settings.project.packages {
            packages.push(PackageEntry {
                package: package.clone(),
                scope: ResourceScope::Project,
            });
        }
        for package in &settings.global.packages {
            packages.push(PackageEntry {
                package: package.clone(),
                scope: ResourceScope::User,
            });
        }

        let packages = self.dedupe_packages(packages);
        self.resolve_package_sources(&packages, &mut accumulator)?;

        let project_base_dir = self.cwd.join(CONFIG_DIR_NAME);
        let global_base_dir = self.agent_dir.clone();

        for resource_type in ResourceType::all() {
            let target = accumulator.target_map_mut(*resource_type);
            self.resolve_local_entries(
                self.entries_for_type(&settings.project, *resource_type),
                *resource_type,
                target,
                PathMetadata {
                    source: String::from("local"),
                    scope: ResourceScope::Project,
                    origin: ResourceOrigin::TopLevel,
                    base_dir: Some(path_to_string(&project_base_dir)),
                },
                &project_base_dir,
            );
            self.resolve_local_entries(
                self.entries_for_type(&settings.global, *resource_type),
                *resource_type,
                target,
                PathMetadata {
                    source: String::from("local"),
                    scope: ResourceScope::User,
                    origin: ResourceOrigin::TopLevel,
                    base_dir: Some(path_to_string(&global_base_dir)),
                },
                &global_base_dir,
            );
        }

        self.add_auto_discovered_resources(
            &mut accumulator,
            &settings.global,
            &settings.project,
            &global_base_dir,
            &project_base_dir,
        );

        Ok(PackageResolutionOutput {
            resolved: accumulator.into_resolved_paths(),
            warnings: settings.warnings,
        })
    }

    pub fn resolve_extension_sources(
        &self,
        sources: &[String],
        options: ResolveExtensionSourcesOptions,
    ) -> Result<PackageResolutionOutput, String> {
        let settings = load_resource_settings(&self.cwd, &self.agent_dir);
        let scope = if options.temporary {
            ResourceScope::Temporary
        } else if options.local {
            ResourceScope::Project
        } else {
            ResourceScope::User
        };
        let entries = sources
            .iter()
            .cloned()
            .map(|source| PackageEntry {
                package: PackageSource::Plain(source),
                scope,
            })
            .collect::<Vec<_>>();
        let mut accumulator = ResourceAccumulator::default();
        self.resolve_package_sources(&entries, &mut accumulator)?;
        Ok(PackageResolutionOutput {
            resolved: accumulator.into_resolved_paths(),
            warnings: settings.warnings,
        })
    }

    pub fn list_configured_packages(&self) -> Vec<ConfiguredPackage> {
        let settings = load_resource_settings(&self.cwd, &self.agent_dir);
        let mut configured = Vec::new();

        for package in &settings.global.packages {
            let source = package.source().to_owned();
            configured.push(ConfiguredPackage {
                installed_path: self.get_installed_path(&source, ResourceScope::User),
                source,
                scope: ResourceScope::User,
                filtered: package.is_filtered(),
            });
        }
        for package in &settings.project.packages {
            let source = package.source().to_owned();
            configured.push(ConfiguredPackage {
                installed_path: self.get_installed_path(&source, ResourceScope::Project),
                source,
                scope: ResourceScope::Project,
                filtered: package.is_filtered(),
            });
        }

        configured
    }

    pub fn install(&self, source: &str, local: bool) -> Result<(), String> {
        let parsed = self.parse_source(source)?;
        let scope = if local {
            ResourceScope::Project
        } else {
            ResourceScope::User
        };

        match parsed {
            ParsedSource::Npm(source) => self.install_npm(&source, scope),
            ParsedSource::Git(source) => self.install_git(&source, scope),
            ParsedSource::Local(source) => {
                let resolved = self.resolve_path(&source.path);
                if resolved.exists() {
                    Ok(())
                } else {
                    Err(format!("Path does not exist: {}", resolved.display()))
                }
            }
        }
    }

    pub fn install_and_persist(&self, source: &str, local: bool) -> Result<(), String> {
        self.install(source, local)?;
        self.add_source_to_settings(source, local).map(|_| ())
    }

    pub fn remove(&self, source: &str, local: bool) -> Result<(), String> {
        let parsed = self.parse_source(source)?;
        let scope = if local {
            ResourceScope::Project
        } else {
            ResourceScope::User
        };

        match parsed {
            ParsedSource::Npm(source) => self.uninstall_npm(&source, scope),
            ParsedSource::Git(source) => self.remove_git(&source, scope),
            ParsedSource::Local(_) => Ok(()),
        }
    }

    pub fn remove_and_persist(&self, source: &str, local: bool) -> Result<bool, String> {
        self.remove(source, local)?;
        self.remove_source_from_settings(source, local)
    }

    pub fn update(&self, source: Option<&str>) -> Result<(), String> {
        let settings = load_resource_settings(&self.cwd, &self.agent_dir);
        let identity = source
            .map(|value| self.get_package_identity(value, None))
            .transpose()?;
        let mut matched = false;

        for package in &settings.global.packages {
            let package_source = package.source();
            if identity.as_deref().is_some_and(|identity| {
                self.get_package_identity(package_source, Some(ResourceScope::User))
                    .map(|candidate| candidate != identity)
                    .unwrap_or(true)
            }) {
                continue;
            }
            matched = true;
            self.update_source_for_scope(package_source, ResourceScope::User)?;
        }

        for package in &settings.project.packages {
            let package_source = package.source();
            if identity.as_deref().is_some_and(|identity| {
                self.get_package_identity(package_source, Some(ResourceScope::Project))
                    .map(|candidate| candidate != identity)
                    .unwrap_or(true)
            }) {
                continue;
            }
            matched = true;
            self.update_source_for_scope(package_source, ResourceScope::Project)?;
        }

        if source.is_some() && !matched {
            let mut configured = Vec::new();
            configured.extend(settings.global.packages.iter().cloned());
            configured.extend(settings.project.packages.iter().cloned());
            return Err(self
                .build_no_matching_package_message(source.expect("checked is_some"), &configured));
        }

        Ok(())
    }

    pub fn get_installed_path(&self, source: &str, scope: ResourceScope) -> Option<String> {
        let parsed = self.parse_source(source).ok()?;
        match parsed {
            ParsedSource::Npm(source) => {
                let installed = self.get_npm_install_path(&source, scope).ok()?;
                installed.exists().then(|| path_to_string(&installed))
            }
            ParsedSource::Git(source) => {
                let installed = self.get_git_install_path(&source, scope);
                installed.exists().then(|| path_to_string(&installed))
            }
            ParsedSource::Local(source) => {
                let base_dir = self.base_dir_for_scope(scope);
                let installed = self.resolve_path_from_base(&source.path, &base_dir);
                installed.exists().then(|| path_to_string(&installed))
            }
        }
    }

    pub fn add_source_to_settings(&self, source: &str, local: bool) -> Result<bool, String> {
        let scope = if local {
            ResourceScope::Project
        } else {
            ResourceScope::User
        };
        let path = self.settings_path(scope);
        let mut settings = self.read_settings_object(&path)?;
        let mut packages = self.packages_from_settings_object(&settings)?;
        if packages.iter().any(|existing| {
            self.package_sources_match(existing, source, scope)
                .unwrap_or(false)
        }) {
            return Ok(false);
        }

        packages.push(PackageSource::Plain(
            self.normalize_package_source_for_settings(source, scope)?,
        ));
        settings.insert(String::from("packages"), self.packages_to_value(&packages));
        self.write_settings_object(&path, settings)?;
        Ok(true)
    }

    pub fn remove_source_from_settings(&self, source: &str, local: bool) -> Result<bool, String> {
        let scope = if local {
            ResourceScope::Project
        } else {
            ResourceScope::User
        };
        let path = self.settings_path(scope);
        let mut settings = self.read_settings_object(&path)?;
        let packages = self.packages_from_settings_object(&settings)?;
        let mut changed = false;
        let retained = packages
            .into_iter()
            .filter(|existing| {
                let matches = self
                    .package_sources_match(existing, source, scope)
                    .unwrap_or(false);
                if matches {
                    changed = true;
                }
                !matches
            })
            .collect::<Vec<_>>();
        if !changed {
            return Ok(false);
        }
        settings.insert(String::from("packages"), self.packages_to_value(&retained));
        self.write_settings_object(&path, settings)?;
        Ok(true)
    }

    fn resolve_package_sources(
        &self,
        sources: &[PackageEntry],
        accumulator: &mut ResourceAccumulator,
    ) -> Result<(), String> {
        for entry in sources {
            let source = entry.package.source().to_owned();
            let filter = match &entry.package {
                PackageSource::Filtered(filter) => Some(PackageFilter {
                    extensions: filter.extensions.clone(),
                    skills: filter.skills.clone(),
                    prompts: filter.prompts.clone(),
                    themes: filter.themes.clone(),
                }),
                PackageSource::Plain(_) => None,
            };
            let parsed = self.parse_source(&source)?;
            let mut metadata = PathMetadata {
                source: source.clone(),
                scope: entry.scope,
                origin: ResourceOrigin::Package,
                base_dir: None,
            };

            match parsed {
                ParsedSource::Local(local) => {
                    let base_dir = self.base_dir_for_scope(entry.scope);
                    let resolved = self.resolve_path_from_base(&local.path, &base_dir);
                    if !resolved.exists() {
                        continue;
                    }
                    metadata.base_dir = Some(path_to_string(&resolved));
                    if resolved.is_file() {
                        self.add_resource(
                            accumulator.target_map_mut(ResourceType::Extensions),
                            path_to_string(&resolved),
                            metadata,
                            true,
                        );
                        continue;
                    }

                    let has_resources = self.collect_package_resources(
                        &resolved,
                        filter.as_ref(),
                        accumulator,
                        &metadata,
                    );
                    if !has_resources {
                        for extension in self.collect_auto_extension_entries(&resolved) {
                            self.add_resource(
                                accumulator.target_map_mut(ResourceType::Extensions),
                                extension,
                                metadata.clone(),
                                true,
                            );
                        }
                    }
                }
                ParsedSource::Npm(npm) => {
                    let install_path = self.get_npm_install_path(&npm, entry.scope)?;
                    let needs_install = !install_path.exists()
                        || (npm.pinned
                            && !self.installed_npm_matches_pinned_version(&npm, &install_path));
                    if needs_install {
                        if is_offline_mode_enabled() {
                            continue;
                        }
                        self.install_npm(&npm, entry.scope)?;
                    }
                    metadata.base_dir = Some(path_to_string(&install_path));
                    self.collect_package_resources(
                        &install_path,
                        filter.as_ref(),
                        accumulator,
                        &metadata,
                    );
                }
                ParsedSource::Git(git) => {
                    let install_path = self.get_git_install_path(&git, entry.scope);
                    if !install_path.exists() {
                        if is_offline_mode_enabled() {
                            continue;
                        }
                        self.install_git(&git, entry.scope)?;
                    } else if entry.scope == ResourceScope::Temporary
                        && !git.pinned
                        && !is_offline_mode_enabled()
                    {
                        let _ = self.update_git(&git, ResourceScope::Temporary);
                    }
                    metadata.base_dir = Some(path_to_string(&install_path));
                    self.collect_package_resources(
                        &install_path,
                        filter.as_ref(),
                        accumulator,
                        &metadata,
                    );
                }
            }
        }

        Ok(())
    }

    fn collect_package_resources(
        &self,
        package_root: &Path,
        filter: Option<&PackageFilter>,
        accumulator: &mut ResourceAccumulator,
        metadata: &PathMetadata,
    ) -> bool {
        if let Some(filter) = filter {
            for resource_type in ResourceType::all() {
                let patterns = match resource_type {
                    ResourceType::Extensions => filter.extensions.as_ref(),
                    ResourceType::Skills => filter.skills.as_ref(),
                    ResourceType::Prompts => filter.prompts.as_ref(),
                    ResourceType::Themes => filter.themes.as_ref(),
                };
                let target = accumulator.target_map_mut(*resource_type);
                if let Some(patterns) = patterns {
                    self.apply_package_filter(
                        package_root,
                        patterns,
                        *resource_type,
                        target,
                        metadata,
                    );
                } else {
                    self.collect_default_resources(package_root, *resource_type, target, metadata);
                }
            }
            return true;
        }

        if let Some(manifest) = self.read_pi_manifest(package_root) {
            for resource_type in ResourceType::all() {
                let target = accumulator.target_map_mut(*resource_type);
                self.add_manifest_entries(
                    manifest_entries(&manifest, *resource_type),
                    package_root,
                    *resource_type,
                    target,
                    metadata,
                );
            }
            return true;
        }

        let mut has_any_dir = false;
        for resource_type in ResourceType::all() {
            let dir = package_root.join(resource_type.dir_name());
            if dir.exists() {
                let files = self.collect_files_from_paths(&[dir], *resource_type);
                for file in files {
                    self.add_resource(
                        accumulator.target_map_mut(*resource_type),
                        file,
                        metadata.clone(),
                        true,
                    );
                }
                has_any_dir = true;
            }
        }
        has_any_dir
    }

    fn collect_default_resources(
        &self,
        package_root: &Path,
        resource_type: ResourceType,
        target: &mut BTreeMap<String, ResourceState>,
        metadata: &PathMetadata,
    ) {
        if let Some(manifest) = self.read_pi_manifest(package_root) {
            if let Some(entries) = manifest_entries(&manifest, resource_type) {
                self.add_manifest_entries(
                    Some(entries),
                    package_root,
                    resource_type,
                    target,
                    metadata,
                );
                return;
            }
        }

        let dir = package_root.join(resource_type.dir_name());
        if !dir.exists() {
            return;
        }
        for file in self.collect_files_from_paths(&[dir], resource_type) {
            self.add_resource(target, file, metadata.clone(), true);
        }
    }

    fn apply_package_filter(
        &self,
        package_root: &Path,
        user_patterns: &[String],
        resource_type: ResourceType,
        target: &mut BTreeMap<String, ResourceState>,
        metadata: &PathMetadata,
    ) {
        let all_files = self.collect_manifest_files(package_root, resource_type);
        if user_patterns.is_empty() {
            for file in all_files {
                self.add_resource(target, file, metadata.clone(), false);
            }
            return;
        }

        let enabled_by_user = apply_patterns(&all_files, user_patterns, package_root);
        for file in all_files {
            self.add_resource(
                target,
                file.clone(),
                metadata.clone(),
                enabled_by_user.contains(&file),
            );
        }
    }

    fn collect_manifest_files(
        &self,
        package_root: &Path,
        resource_type: ResourceType,
    ) -> Vec<String> {
        if let Some(manifest) = self.read_pi_manifest(package_root)
            && let Some(entries) = manifest_entries(&manifest, resource_type)
            && !entries.is_empty()
        {
            let all_files =
                self.collect_files_from_manifest_entries(entries, package_root, resource_type);
            let manifest_patterns = entries
                .iter()
                .filter(|entry| is_pattern(entry))
                .cloned()
                .collect::<Vec<_>>();
            if manifest_patterns.is_empty() {
                return all_files;
            }
            let enabled = apply_patterns(&all_files, &manifest_patterns, package_root);
            return all_files
                .into_iter()
                .filter(|file| enabled.contains(file))
                .collect();
        }

        let dir = package_root.join(resource_type.dir_name());
        if !dir.exists() {
            return Vec::new();
        }
        self.collect_files_from_paths(&[dir], resource_type)
    }

    fn read_pi_manifest(&self, package_root: &Path) -> Option<PiManifest> {
        let path = package_root.join(PACKAGE_JSON_FILE_NAME);
        let content = fs::read_to_string(path).ok()?;
        let value = serde_json::from_str::<Value>(&content).ok()?;
        serde_json::from_value(value.get("pi")?.clone()).ok()
    }

    fn add_manifest_entries(
        &self,
        entries: Option<&Vec<String>>,
        root: &Path,
        resource_type: ResourceType,
        target: &mut BTreeMap<String, ResourceState>,
        metadata: &PathMetadata,
    ) {
        let Some(entries) = entries else {
            return;
        };
        let all_files = self.collect_files_from_manifest_entries(entries, root, resource_type);
        let patterns = entries
            .iter()
            .filter(|entry| is_pattern(entry))
            .cloned()
            .collect::<Vec<_>>();
        let enabled = apply_patterns(&all_files, &patterns, root);
        for file in all_files {
            if enabled.contains(&file) {
                self.add_resource(target, file, metadata.clone(), true);
            }
        }
    }

    fn collect_files_from_manifest_entries(
        &self,
        entries: &[String],
        root: &Path,
        resource_type: ResourceType,
    ) -> Vec<String> {
        let resolved = entries
            .iter()
            .filter(|entry| !is_pattern(entry))
            .map(|entry| normalize_pathbuf(root.join(entry)))
            .collect::<Vec<_>>();
        self.collect_files_from_paths(&resolved, resource_type)
    }

    fn resolve_local_entries(
        &self,
        entries: &[String],
        resource_type: ResourceType,
        target: &mut BTreeMap<String, ResourceState>,
        metadata: PathMetadata,
        base_dir: &Path,
    ) {
        if entries.is_empty() {
            return;
        }

        let (plain, patterns) = split_patterns(entries);
        let resolved_plain = plain
            .iter()
            .map(|entry| self.resolve_path_from_base(entry, base_dir))
            .collect::<Vec<_>>();
        let all_files = self.collect_files_from_paths(&resolved_plain, resource_type);
        let enabled = apply_patterns(&all_files, &patterns, base_dir);
        for file in all_files {
            self.add_resource(
                target,
                file.clone(),
                metadata.clone(),
                enabled.contains(&file),
            );
        }
    }

    fn add_auto_discovered_resources(
        &self,
        accumulator: &mut ResourceAccumulator,
        global: &ResourceSettings,
        project: &ResourceSettings,
        global_base_dir: &Path,
        project_base_dir: &Path,
    ) {
        let user_metadata = PathMetadata {
            source: String::from("auto"),
            scope: ResourceScope::User,
            origin: ResourceOrigin::TopLevel,
            base_dir: Some(path_to_string(global_base_dir)),
        };
        let project_metadata = PathMetadata {
            source: String::from("auto"),
            scope: ResourceScope::Project,
            origin: ResourceOrigin::TopLevel,
            base_dir: Some(path_to_string(project_base_dir)),
        };

        let user_agents_skills_dir = home_dir().map(|home| home.join(".agents").join("skills"));
        let project_agents_skill_dirs = collect_ancestor_agents_skill_dirs(&self.cwd)
            .into_iter()
            .filter(|dir| {
                user_agents_skills_dir.as_ref().is_none_or(|user_dir| {
                    normalize_pathbuf(dir.clone()) != normalize_pathbuf(user_dir.clone())
                })
            })
            .collect::<Vec<_>>();

        self.add_auto_resources_for_scope(
            accumulator,
            project_base_dir,
            project,
            &project_metadata,
            Some(&project_agents_skill_dirs),
            user_agents_skills_dir.as_deref(),
            true,
        );
        self.add_auto_resources_for_scope(
            accumulator,
            global_base_dir,
            global,
            &user_metadata,
            None,
            user_agents_skills_dir.as_deref(),
            false,
        );
    }

    fn add_auto_resources_for_scope(
        &self,
        accumulator: &mut ResourceAccumulator,
        base_dir: &Path,
        settings: &ResourceSettings,
        metadata: &PathMetadata,
        project_agents_skill_dirs: Option<&[PathBuf]>,
        user_agents_skill_dir: Option<&Path>,
        project_scope: bool,
    ) {
        let extensions_dir = base_dir.join("extensions");
        let skills_dir = base_dir.join("skills");
        let prompts_dir = base_dir.join("prompts");
        let themes_dir = base_dir.join("themes");

        self.add_auto_resources(
            accumulator.target_map_mut(ResourceType::Extensions),
            self.collect_auto_extension_entries(&extensions_dir),
            metadata,
            &settings.extensions,
            base_dir,
        );

        let mut skills = self.collect_skill_entries(&skills_dir, SkillDiscoveryMode::Pi);
        if project_scope {
            for dir in project_agents_skill_dirs.unwrap_or(&[]) {
                skills.extend(self.collect_skill_entries(dir, SkillDiscoveryMode::Agents));
            }
        } else if let Some(user_agents_skill_dir) = user_agents_skill_dir {
            skills.extend(
                self.collect_skill_entries(user_agents_skill_dir, SkillDiscoveryMode::Agents),
            );
        }
        self.add_auto_resources(
            accumulator.target_map_mut(ResourceType::Skills),
            skills,
            metadata,
            &settings.skills,
            base_dir,
        );

        self.add_auto_resources(
            accumulator.target_map_mut(ResourceType::Prompts),
            self.collect_auto_prompt_entries(&prompts_dir),
            metadata,
            &settings.prompts,
            base_dir,
        );
        self.add_auto_resources(
            accumulator.target_map_mut(ResourceType::Themes),
            self.collect_auto_theme_entries(&themes_dir),
            metadata,
            &settings.themes,
            base_dir,
        );
    }

    fn add_auto_resources(
        &self,
        target: &mut BTreeMap<String, ResourceState>,
        paths: Vec<String>,
        metadata: &PathMetadata,
        overrides: &[String],
        base_dir: &Path,
    ) {
        for path in paths {
            let enabled = is_enabled_by_overrides(&path, overrides, base_dir);
            self.add_resource(target, path, metadata.clone(), enabled);
        }
    }

    fn collect_files_from_paths(
        &self,
        paths: &[PathBuf],
        resource_type: ResourceType,
    ) -> Vec<String> {
        let mut files = Vec::new();
        for path in paths {
            if !path.exists() {
                continue;
            }
            if path.is_file() {
                if matches_resource_file(path, resource_type) {
                    files.push(path_to_string(path));
                }
                continue;
            }
            let mut discovered = match resource_type {
                ResourceType::Extensions => self.collect_auto_extension_entries(path),
                ResourceType::Skills => self.collect_skill_entries(path, SkillDiscoveryMode::Pi),
                ResourceType::Prompts => self.collect_recursive_files(path, ResourceType::Prompts),
                ResourceType::Themes => self.collect_recursive_files(path, ResourceType::Themes),
            };
            files.append(&mut discovered);
        }
        files
    }

    fn collect_recursive_files(&self, root: &Path, resource_type: ResourceType) -> Vec<String> {
        let mut files = Vec::new();
        let mut ignore_state = IgnoreState::default();
        self.collect_recursive_files_impl(root, root, resource_type, &mut ignore_state, &mut files);
        files
    }

    fn collect_recursive_files_impl(
        &self,
        current: &Path,
        root: &Path,
        resource_type: ResourceType,
        ignore_state: &mut IgnoreState,
        files: &mut Vec<String>,
    ) {
        let checkpoint = ignore_state.rules.len();
        ignore_state.load_from_dir(current, root);
        let entries = sorted_dir_entries(current);
        for entry in entries {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if is_hidden_name(name) {
                continue;
            }
            if name == "node_modules" {
                continue;
            }
            let path = entry.path();
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                if ignore_state.is_ignored(&path, root, true) {
                    continue;
                }
                self.collect_recursive_files_impl(&path, root, resource_type, ignore_state, files);
                continue;
            }
            if ignore_state.is_ignored(&path, root, false) {
                continue;
            }
            if matches_resource_file(&path, resource_type) {
                files.push(path_to_string(&path));
            }
        }
        ignore_state.rules.truncate(checkpoint);
    }

    fn collect_skill_entries(&self, root: &Path, mode: SkillDiscoveryMode) -> Vec<String> {
        if !root.exists() || !root.is_dir() {
            return Vec::new();
        }

        let mut ignore_state = IgnoreState::default();
        let mut skill_files = Vec::new();
        let mut root_markdown = Vec::new();
        self.collect_skill_entries_impl(
            root,
            root,
            mode,
            &mut ignore_state,
            &mut skill_files,
            &mut root_markdown,
        );

        let root_skill = root.join("SKILL.md");
        if skill_files
            .iter()
            .any(|path| normalize_pathbuf(path.clone()) == normalize_pathbuf(root_skill.clone()))
        {
            return vec![path_to_string(&root_skill)];
        }

        let skill_dirs = skill_files
            .iter()
            .filter_map(|path| path.parent().map(PathBuf::from))
            .collect::<HashSet<_>>();
        let filtered_skills = skill_files
            .into_iter()
            .filter(|path| {
                let mut current = path.parent().and_then(Path::parent);
                while let Some(dir) = current {
                    if skill_dirs.contains(dir) {
                        return false;
                    }
                    if dir == root {
                        break;
                    }
                    current = dir.parent();
                }
                true
            })
            .map(|path| path_to_string(&path))
            .collect::<Vec<_>>();

        let mut result = Vec::new();
        if mode == SkillDiscoveryMode::Pi {
            result.extend(root_markdown.into_iter().map(|path| path_to_string(&path)));
        }
        result.extend(filtered_skills);
        result.sort();
        result.dedup();
        result
    }

    fn collect_skill_entries_impl(
        &self,
        current: &Path,
        root: &Path,
        mode: SkillDiscoveryMode,
        ignore_state: &mut IgnoreState,
        skill_files: &mut Vec<PathBuf>,
        root_markdown: &mut Vec<PathBuf>,
    ) {
        let checkpoint = ignore_state.rules.len();
        ignore_state.load_from_dir(current, root);
        let entries = sorted_dir_entries(current);
        for entry in entries {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if is_hidden_name(name) {
                continue;
            }
            if name == "node_modules" {
                continue;
            }
            let path = entry.path();
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                if ignore_state.is_ignored(&path, root, true) {
                    continue;
                }
                self.collect_skill_entries_impl(
                    &path,
                    root,
                    mode,
                    ignore_state,
                    skill_files,
                    root_markdown,
                );
                continue;
            }
            if ignore_state.is_ignored(&path, root, false) {
                continue;
            }
            if name == "SKILL.md" {
                skill_files.push(path);
                continue;
            }
            if mode == SkillDiscoveryMode::Pi
                && current == root
                && path.extension().and_then(|extension| extension.to_str()) == Some("md")
            {
                root_markdown.push(path);
            }
        }
        ignore_state.rules.truncate(checkpoint);
    }

    fn collect_auto_extension_entries(&self, dir: &Path) -> Vec<String> {
        if !dir.exists() || !dir.is_dir() {
            return Vec::new();
        }
        if let Some(entries) = self.resolve_extension_entries(dir) {
            return entries;
        }

        let mut ignore_state = IgnoreState::default();
        ignore_state.load_from_dir(dir, dir);
        let mut entries = Vec::new();
        for entry in sorted_dir_entries(dir) {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if is_hidden_name(name) {
                continue;
            }
            if name == "node_modules" {
                continue;
            }
            let path = entry.path();
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if ignore_state.is_ignored(&path, dir, metadata.is_dir()) {
                continue;
            }
            if metadata.is_file() {
                if matches_resource_file(&path, ResourceType::Extensions) {
                    entries.push(path_to_string(&path));
                }
                continue;
            }
            if let Some(resolved) = self.resolve_extension_entries(&path) {
                entries.extend(resolved);
            }
        }
        entries.sort();
        entries.dedup();
        entries
    }

    fn resolve_extension_entries(&self, dir: &Path) -> Option<Vec<String>> {
        let package_json_path = dir.join(PACKAGE_JSON_FILE_NAME);
        if package_json_path.exists()
            && let Some(manifest) = self.read_pi_manifest(dir)
            && let Some(entries) = manifest.extensions
            && !entries.is_empty()
        {
            let resolved = entries
                .into_iter()
                .map(|entry| normalize_pathbuf(dir.join(entry)))
                .filter(|entry| entry.exists())
                .flat_map(|entry| {
                    if entry.is_dir() {
                        self.collect_auto_extension_entries(&entry)
                    } else {
                        vec![path_to_string(&entry)]
                    }
                })
                .collect::<Vec<_>>();
            if !resolved.is_empty() {
                return Some(resolved);
            }
        }

        let index_ts = dir.join("index.ts");
        if index_ts.exists() {
            return Some(vec![path_to_string(&index_ts)]);
        }
        let index_js = dir.join("index.js");
        if index_js.exists() {
            return Some(vec![path_to_string(&index_js)]);
        }

        None
    }

    fn collect_auto_prompt_entries(&self, dir: &Path) -> Vec<String> {
        self.collect_top_level_files(dir, ResourceType::Prompts)
    }

    fn collect_auto_theme_entries(&self, dir: &Path) -> Vec<String> {
        self.collect_top_level_files(dir, ResourceType::Themes)
    }

    fn collect_top_level_files(&self, dir: &Path, resource_type: ResourceType) -> Vec<String> {
        if !dir.exists() || !dir.is_dir() {
            return Vec::new();
        }
        let mut ignore_state = IgnoreState::default();
        ignore_state.load_from_dir(dir, dir);
        let mut files = Vec::new();
        for entry in sorted_dir_entries(dir) {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if is_hidden_name(name) || name == "node_modules" {
                continue;
            }
            let path = entry.path();
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if !metadata.is_file() || ignore_state.is_ignored(&path, dir, false) {
                continue;
            }
            if matches_resource_file(&path, resource_type) {
                files.push(path_to_string(&path));
            }
        }
        files
    }

    fn add_resource(
        &self,
        target: &mut BTreeMap<String, ResourceState>,
        path: String,
        metadata: PathMetadata,
        enabled: bool,
    ) {
        target
            .entry(path)
            .or_insert(ResourceState { metadata, enabled });
    }

    fn entries_for_type<'a>(
        &self,
        settings: &'a ResourceSettings,
        resource_type: ResourceType,
    ) -> &'a [String] {
        match resource_type {
            ResourceType::Extensions => &settings.extensions,
            ResourceType::Skills => &settings.skills,
            ResourceType::Prompts => &settings.prompts,
            ResourceType::Themes => &settings.themes,
        }
    }

    fn update_source_for_scope(&self, source: &str, scope: ResourceScope) -> Result<(), String> {
        if is_offline_mode_enabled() {
            return Ok(());
        }
        match self.parse_source(source)? {
            ParsedSource::Npm(source) => {
                if source.pinned {
                    return Ok(());
                }
                self.install_npm(
                    &NpmSource {
                        spec: format!("{}@latest", source.name),
                        name: source.name,
                        pinned: false,
                    },
                    scope,
                )
            }
            ParsedSource::Git(source) => {
                if source.pinned {
                    return Ok(());
                }
                self.update_git(&source, scope)
            }
            ParsedSource::Local(_) => Ok(()),
        }
    }

    fn install_npm(&self, source: &NpmSource, scope: ResourceScope) -> Result<(), String> {
        if scope == ResourceScope::User {
            return self.run_npm_command(
                &[
                    String::from("install"),
                    String::from("-g"),
                    source.spec.clone(),
                ],
                None,
            );
        }
        let install_root = self.npm_install_root_for_source(scope, source);
        self.ensure_npm_project(&install_root)?;
        self.run_npm_command(
            &[
                String::from("install"),
                source.spec.clone(),
                String::from("--prefix"),
                path_to_string(&install_root),
            ],
            None,
        )
    }

    fn uninstall_npm(&self, source: &NpmSource, scope: ResourceScope) -> Result<(), String> {
        if scope == ResourceScope::User {
            return self.run_npm_command(
                &[
                    String::from("uninstall"),
                    String::from("-g"),
                    source.name.clone(),
                ],
                None,
            );
        }
        let install_root = self.npm_install_root_for_source(scope, source);
        if !install_root.exists() {
            return Ok(());
        }
        self.run_npm_command(
            &[
                String::from("uninstall"),
                source.name.clone(),
                String::from("--prefix"),
                path_to_string(&install_root),
            ],
            None,
        )
    }

    fn install_git(&self, source: &GitSource, scope: ResourceScope) -> Result<(), String> {
        let target_dir = self.get_git_install_path(source, scope);
        if target_dir.exists() {
            return Ok(());
        }
        if let Some(root) = self.git_install_root(scope) {
            self.ensure_git_ignore(&root)?;
        }
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        self.run_command(
            "git",
            &[
                String::from("clone"),
                source.repo.clone(),
                path_to_string(&target_dir),
            ],
            None,
        )?;
        if let Some(reference) = source.reference.as_ref() {
            self.run_command(
                "git",
                &[String::from("checkout"), reference.clone()],
                Some(&target_dir),
            )?;
        }
        if target_dir.join(PACKAGE_JSON_FILE_NAME).exists() {
            self.run_npm_command(&[String::from("install")], Some(&target_dir))?;
        }
        Ok(())
    }

    fn update_git(&self, source: &GitSource, scope: ResourceScope) -> Result<(), String> {
        let target_dir = self.get_git_install_path(source, scope);
        if !target_dir.exists() {
            return self.install_git(source, scope);
        }
        self.run_command(
            "git",
            &[String::from("pull"), String::from("--ff-only")],
            Some(&target_dir),
        )?;
        if target_dir.join(PACKAGE_JSON_FILE_NAME).exists() {
            self.run_npm_command(&[String::from("install")], Some(&target_dir))?;
        }
        Ok(())
    }

    fn remove_git(&self, source: &GitSource, scope: ResourceScope) -> Result<(), String> {
        let target_dir = self.get_git_install_path(source, scope);
        if !target_dir.exists() {
            return Ok(());
        }
        fs::remove_dir_all(&target_dir).map_err(|error| error.to_string())?;
        if let Some(root) = self.git_install_root(scope) {
            self.prune_empty_git_parents(&target_dir, &root)?;
        }
        Ok(())
    }

    fn prune_empty_git_parents(
        &self,
        target_dir: &Path,
        install_root: &Path,
    ) -> Result<(), String> {
        let resolved_root = normalize_pathbuf(install_root.to_path_buf());
        let mut current = target_dir.parent().map(PathBuf::from);
        while let Some(dir) = current {
            let normalized = normalize_pathbuf(dir.clone());
            if !normalized.starts_with(&resolved_root) || normalized == resolved_root {
                break;
            }
            if !dir.exists() {
                current = dir.parent().map(PathBuf::from);
                continue;
            }
            if fs::read_dir(&dir)
                .map_err(|error| error.to_string())?
                .next()
                .is_some()
            {
                break;
            }
            fs::remove_dir(&dir).map_err(|error| error.to_string())?;
            current = dir.parent().map(PathBuf::from);
        }
        Ok(())
    }

    fn ensure_npm_project(&self, install_root: &Path) -> Result<(), String> {
        fs::create_dir_all(install_root).map_err(|error| error.to_string())?;
        self.ensure_git_ignore(install_root)?;
        let package_json_path = install_root.join(PACKAGE_JSON_FILE_NAME);
        if !package_json_path.exists() {
            fs::write(
                package_json_path,
                serde_json::to_string_pretty(&json!({
                    "name": "pi-extensions",
                    "private": true,
                }))
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn ensure_git_ignore(&self, dir: &Path) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let ignore_path = dir.join(".gitignore");
        if !ignore_path.exists() {
            fs::write(ignore_path, "*\n!.gitignore\n").map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn npm_install_root_for_source(&self, scope: ResourceScope, source: &NpmSource) -> PathBuf {
        match scope {
            ResourceScope::Temporary => self.temporary_dir("npm", Some(&source.name)),
            ResourceScope::Project => self.cwd.join(CONFIG_DIR_NAME).join("npm"),
            ResourceScope::User => self.agent_dir.join("npm-global-fallback"),
        }
    }

    fn get_npm_install_path(
        &self,
        source: &NpmSource,
        scope: ResourceScope,
    ) -> Result<PathBuf, String> {
        match scope {
            ResourceScope::Temporary | ResourceScope::Project => Ok(self
                .npm_install_root_for_source(scope, source)
                .join("node_modules")
                .join(&source.name)),
            ResourceScope::User => Ok(PathBuf::from(self.global_npm_root()?).join(&source.name)),
        }
    }

    fn global_npm_root(&self) -> Result<String, String> {
        self.run_npm_command_capture(&[String::from("root"), String::from("-g")])
    }

    fn get_git_install_path(&self, source: &GitSource, scope: ResourceScope) -> PathBuf {
        match scope {
            ResourceScope::Temporary => {
                self.temporary_dir("git", Some(&format!("{}:{}", source.host, source.path)))
            }
            ResourceScope::Project => self
                .cwd
                .join(CONFIG_DIR_NAME)
                .join("git")
                .join(&source.host)
                .join(&source.path),
            ResourceScope::User => self
                .agent_dir
                .join("git")
                .join(&source.host)
                .join(&source.path),
        }
    }

    fn git_install_root(&self, scope: ResourceScope) -> Option<PathBuf> {
        match scope {
            ResourceScope::Temporary => None,
            ResourceScope::Project => Some(self.cwd.join(CONFIG_DIR_NAME).join("git")),
            ResourceScope::User => Some(self.agent_dir.join("git")),
        }
    }

    fn temporary_dir(&self, prefix: &str, suffix: Option<&str>) -> PathBuf {
        let key = format!("{prefix}:{}", suffix.unwrap_or_default());
        let hash = stable_hash(&key);
        let mut path = env::temp_dir()
            .join("pi-extensions")
            .join(prefix)
            .join(hash);
        if let Some(suffix) = suffix {
            let sanitized = suffix
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                        character
                    } else {
                        '-'
                    }
                })
                .collect::<String>();
            if !sanitized.is_empty() {
                path = path.join(sanitized);
            }
        }
        path
    }

    fn installed_npm_matches_pinned_version(
        &self,
        source: &NpmSource,
        install_path: &Path,
    ) -> bool {
        let Some(installed_version) = self.installed_npm_version(install_path) else {
            return false;
        };
        let (_, expected_version) = parse_npm_spec(&source.spec);
        expected_version.is_none_or(|expected| expected == installed_version)
    }

    fn installed_npm_version(&self, install_path: &Path) -> Option<String> {
        let package_json_path = install_path.join(PACKAGE_JSON_FILE_NAME);
        let content = fs::read_to_string(package_json_path).ok()?;
        let value = serde_json::from_str::<Value>(&content).ok()?;
        value.get("version")?.as_str().map(ToOwned::to_owned)
    }

    fn parse_source(&self, source: &str) -> Result<ParsedSource, String> {
        let trimmed = source.trim();
        if let Some(spec) = trimmed.strip_prefix("npm:") {
            let spec = spec.trim();
            let (name, version) = parse_npm_spec(spec);
            return Ok(ParsedSource::Npm(NpmSource {
                spec: spec.to_owned(),
                name,
                pinned: version.is_some(),
            }));
        }

        if let Some(git) = parse_git_source(trimmed) {
            return Ok(ParsedSource::Git(git));
        }

        Ok(ParsedSource::Local(LocalSource {
            path: trimmed.to_owned(),
        }))
    }

    fn get_package_identity(
        &self,
        source: &str,
        scope: Option<ResourceScope>,
    ) -> Result<String, String> {
        match self.parse_source(source)? {
            ParsedSource::Npm(source) => Ok(format!("npm:{}", source.name)),
            ParsedSource::Git(source) => Ok(format!("git:{}/{}", source.host, source.path)),
            ParsedSource::Local(source) => {
                let resolved = if let Some(scope) = scope {
                    self.resolve_path_from_base(&source.path, &self.base_dir_for_scope(scope))
                } else {
                    self.resolve_path(&source.path)
                };
                Ok(format!("local:{}", path_to_string(&resolved)))
            }
        }
    }

    fn dedupe_packages(&self, packages: Vec<PackageEntry>) -> Vec<PackageEntry> {
        let mut seen = BTreeMap::<String, PackageEntry>::new();
        for package in packages {
            let Ok(identity) =
                self.get_package_identity(package.package.source(), Some(package.scope))
            else {
                continue;
            };
            if let Some(existing) = seen.get(&identity)
                && existing.scope == ResourceScope::Project
                && package.scope == ResourceScope::User
            {
                continue;
            }
            seen.entry(identity.clone())
                .or_insert_with(|| package.clone());
            if package.scope == ResourceScope::Project {
                seen.insert(identity, package);
            }
        }
        seen.into_values().collect()
    }

    fn package_sources_match(
        &self,
        existing: &PackageSource,
        input_source: &str,
        scope: ResourceScope,
    ) -> Result<bool, String> {
        let left = self.get_source_match_key_for_settings(existing.source(), scope)?;
        let right = self.get_source_match_key_for_input(input_source)?;
        Ok(left == right)
    }

    fn get_source_match_key_for_input(&self, source: &str) -> Result<String, String> {
        self.get_package_identity(source, None)
    }

    fn get_source_match_key_for_settings(
        &self,
        source: &str,
        scope: ResourceScope,
    ) -> Result<String, String> {
        self.get_package_identity(source, Some(scope))
    }

    fn normalize_package_source_for_settings(
        &self,
        source: &str,
        scope: ResourceScope,
    ) -> Result<String, String> {
        match self.parse_source(source)? {
            ParsedSource::Local(local) => {
                let base_dir = self.base_dir_for_scope(scope);
                let resolved = self.resolve_path(&local.path);
                let relative = relative_path(&base_dir, &resolved);
                if relative.is_empty() {
                    return Ok(String::from("."));
                }
                if relative.starts_with('.') {
                    Ok(relative)
                } else {
                    Ok(format!("./{relative}"))
                }
            }
            _ => Ok(source.to_owned()),
        }
    }

    fn build_no_matching_package_message(
        &self,
        source: &str,
        configured: &[PackageSource],
    ) -> String {
        let Some(suggestion) = self.find_suggested_configured_source(source, configured) else {
            return format!("No matching package found for {source}");
        };
        format!("No matching package found for {source}. Did you mean {suggestion}?")
    }

    fn find_suggested_configured_source(
        &self,
        source: &str,
        configured: &[PackageSource],
    ) -> Option<String> {
        let trimmed = source.trim();
        let mut suggestions = HashSet::new();
        for package in configured {
            let source_value = package.source();
            let Ok(parsed) = self.parse_source(source_value) else {
                continue;
            };
            match parsed {
                ParsedSource::Npm(source) => {
                    if trimmed == source.name || trimmed == source.spec {
                        suggestions.insert(source_value.to_owned());
                    }
                }
                ParsedSource::Git(source) => {
                    let shorthand = format!("{}/{}", source.host, source.path);
                    let shorthand_with_ref = source
                        .reference
                        .as_ref()
                        .map(|reference| format!("{shorthand}@{reference}"));
                    if trimmed == shorthand
                        || shorthand_with_ref
                            .as_deref()
                            .is_some_and(|candidate| candidate == trimmed)
                    {
                        suggestions.insert(source_value.to_owned());
                    }
                }
                ParsedSource::Local(_) => {}
            }
        }
        suggestions.into_iter().next()
    }

    fn get_npm_command(&self) -> Result<(String, Vec<String>), String> {
        let settings = load_resource_settings(&self.cwd, &self.agent_dir);
        let argv = settings
            .project
            .npm_command
            .or(settings.global.npm_command)
            .unwrap_or_else(|| vec![String::from("npm")]);
        let Some(command) = argv.first().cloned() else {
            return Err(String::from(
                "Invalid npmCommand: first array entry must be a non-empty command",
            ));
        };
        Ok((command, argv.into_iter().skip(1).collect()))
    }

    fn run_npm_command(&self, args: &[String], cwd: Option<&Path>) -> Result<(), String> {
        let (command, prefix_args) = self.get_npm_command()?;
        let mut full_args = prefix_args;
        full_args.extend(args.iter().cloned());
        self.run_command(&command, &full_args, cwd)
    }

    fn run_npm_command_capture(&self, args: &[String]) -> Result<String, String> {
        let (command, prefix_args) = self.get_npm_command()?;
        let mut full_args = prefix_args;
        full_args.extend(args.iter().cloned());
        self.run_command_capture(&command, &full_args, None)
    }

    fn run_command(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Result<(), String> {
        let output = Command::new(command)
            .args(args)
            .current_dir(cwd.unwrap_or(&self.cwd))
            .output()
            .map_err(|error| format!("Failed to run {command}: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        Err(format_command_error(
            command,
            args,
            &output.stdout,
            &output.stderr,
        ))
    }

    fn run_command_capture(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Result<String, String> {
        let output = Command::new(command)
            .args(args)
            .current_dir(cwd.unwrap_or(&self.cwd))
            .output()
            .map_err(|error| format!("Failed to run {command}: {error}"))?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !stdout.is_empty() {
                return Ok(stdout);
            }
            return Ok(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        Err(format_command_error(
            command,
            args,
            &output.stdout,
            &output.stderr,
        ))
    }

    fn base_dir_for_scope(&self, scope: ResourceScope) -> PathBuf {
        match scope {
            ResourceScope::Project => self.cwd.join(CONFIG_DIR_NAME),
            ResourceScope::User => self.agent_dir.clone(),
            ResourceScope::Temporary => self.cwd.clone(),
        }
    }

    fn resolve_path(&self, input: &str) -> PathBuf {
        self.resolve_path_from_base(input, &self.cwd)
    }

    fn resolve_path_from_base(&self, input: &str, base_dir: &Path) -> PathBuf {
        let trimmed = input.trim();
        if trimmed == "~" {
            return home_dir().unwrap_or_else(|| normalize_pathbuf(base_dir.to_path_buf()));
        }
        if let Some(rest) = trimmed.strip_prefix("~/") {
            if let Some(home) = home_dir() {
                return normalize_pathbuf(home.join(rest));
            }
        }
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            normalize_pathbuf(path)
        } else {
            normalize_pathbuf(base_dir.join(path))
        }
    }

    fn settings_path(&self, scope: ResourceScope) -> PathBuf {
        match scope {
            ResourceScope::User => self.agent_dir.join(SETTINGS_FILE_NAME),
            ResourceScope::Project => self.cwd.join(CONFIG_DIR_NAME).join(SETTINGS_FILE_NAME),
            ResourceScope::Temporary => self.cwd.join(CONFIG_DIR_NAME).join(SETTINGS_FILE_NAME),
        }
    }

    fn read_settings_object(&self, path: &Path) -> Result<Map<String, Value>, String> {
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Map::new());
            }
            Err(error) => return Err(error.to_string()),
        };
        match serde_json::from_str::<Value>(&content).map_err(|error| error.to_string())? {
            Value::Object(object) => Ok(object),
            _ => Err(format!("{} must contain a JSON object", path.display())),
        }
    }

    fn write_settings_object(&self, path: &Path, object: Map<String, Value>) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let content = serde_json::to_string_pretty(&Value::Object(object))
            .map_err(|error| error.to_string())?;
        fs::write(path, format!("{content}\n")).map_err(|error| error.to_string())
    }

    fn packages_from_settings_object(
        &self,
        object: &Map<String, Value>,
    ) -> Result<Vec<PackageSource>, String> {
        let Some(value) = object.get("packages") else {
            return Ok(Vec::new());
        };
        let stored = serde_json::from_value::<Vec<StoredPackageSource>>(value.clone())
            .map_err(|error| format!("Invalid packages setting: {error}"))?;
        Ok(stored
            .into_iter()
            .map(|entry| match entry {
                StoredPackageSource::Plain(source) => PackageSource::Plain(source),
                StoredPackageSource::Filtered {
                    source,
                    extensions,
                    skills,
                    prompts,
                    themes,
                } => PackageSource::Filtered(FilteredPackageSource {
                    source,
                    extensions,
                    skills,
                    prompts,
                    themes,
                }),
            })
            .collect())
    }

    fn packages_to_value(&self, packages: &[PackageSource]) -> Value {
        Value::Array(
            packages
                .iter()
                .map(|package| match package {
                    PackageSource::Plain(source) => Value::String(source.clone()),
                    PackageSource::Filtered(filter) => json!({
                        "source": filter.source,
                        "extensions": filter.extensions,
                        "skills": filter.skills,
                        "prompts": filter.prompts,
                        "themes": filter.themes,
                    }),
                })
                .collect(),
        )
    }
}

impl IgnoreState {
    fn load_from_dir(&mut self, dir: &Path, root: &Path) {
        let relative_dir = relative_path(root, dir);
        let prefix = if relative_dir.is_empty() {
            String::new()
        } else {
            format!("{relative_dir}/")
        };
        for file_name in IGNORE_FILE_NAMES {
            let path = dir.join(file_name);
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };
            for line in content.lines() {
                let Some(rule) = prefix_ignore_pattern(line, &prefix) else {
                    continue;
                };
                self.rules.push(rule);
            }
        }
    }

    fn is_ignored(&self, path: &Path, root: &Path, is_dir: bool) -> bool {
        let relative = relative_path(root, path);
        let mut ignored = false;
        for rule in &self.rules {
            if ignore_rule_matches(&rule.pattern, &relative, is_dir) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

impl ResourceAccumulator {
    fn target_map_mut(
        &mut self,
        resource_type: ResourceType,
    ) -> &mut BTreeMap<String, ResourceState> {
        match resource_type {
            ResourceType::Extensions => &mut self.extensions,
            ResourceType::Skills => &mut self.skills,
            ResourceType::Prompts => &mut self.prompts,
            ResourceType::Themes => &mut self.themes,
        }
    }

    fn into_resolved_paths(self) -> ResolvedPaths {
        ResolvedPaths {
            extensions: to_resolved_resources(self.extensions),
            skills: to_resolved_resources(self.skills),
            prompts: to_resolved_resources(self.prompts),
            themes: to_resolved_resources(self.themes),
        }
    }
}

fn to_resolved_resources(entries: BTreeMap<String, ResourceState>) -> Vec<ResolvedResource> {
    let mut resolved = entries
        .into_iter()
        .map(|(path, state)| ResolvedResource {
            path,
            enabled: state.enabled,
            metadata: state.metadata,
        })
        .collect::<Vec<_>>();
    resolved.sort_by_key(|resource| resource_precedence_rank(&resource.metadata));
    resolved
}

fn resource_precedence_rank(metadata: &PathMetadata) -> u8 {
    if metadata.scope == ResourceScope::Temporary {
        return 0;
    }
    if metadata.origin == ResourceOrigin::Package {
        return 4;
    }
    let base = if metadata.scope == ResourceScope::Project {
        0
    } else {
        2
    };
    base + if metadata.source == "local" { 0 } else { 1 }
}

fn manifest_entries(manifest: &PiManifest, resource_type: ResourceType) -> Option<&Vec<String>> {
    match resource_type {
        ResourceType::Extensions => manifest.extensions.as_ref(),
        ResourceType::Skills => manifest.skills.as_ref(),
        ResourceType::Prompts => manifest.prompts.as_ref(),
        ResourceType::Themes => manifest.themes.as_ref(),
    }
}

fn is_offline_mode_enabled() -> bool {
    let Some(value) = env::var_os("PI_OFFLINE") else {
        return false;
    };
    matches!(
        value.to_string_lossy().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}

fn matches_resource_file(path: &Path, resource_type: ResourceType) -> bool {
    match resource_type {
        ResourceType::Extensions => path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "ts" | "js")),
        ResourceType::Skills | ResourceType::Prompts => path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "md"),
        ResourceType::Themes => path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "json"),
    }
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

fn sorted_dir_entries(dir: &Path) -> Vec<fs::DirEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());
    entries
}

fn parse_npm_spec(spec: &str) -> (String, Option<String>) {
    if spec.starts_with('@') {
        if let Some(index) = spec[1..].rfind('@') {
            let split = index + 1;
            let version = spec[split + 1..].trim();
            if !version.is_empty() {
                return (spec[..split].to_owned(), Some(version.to_owned()));
            }
        }
        return (spec.to_owned(), None);
    }

    if let Some(index) = spec.rfind('@') {
        let version = spec[index + 1..].trim();
        if !version.is_empty() {
            return (spec[..index].to_owned(), Some(version.to_owned()));
        }
    }
    (spec.to_owned(), None)
}

fn parse_git_source(source: &str) -> Option<GitSource> {
    let trimmed = source.trim();
    let has_git_prefix = trimmed.starts_with("git:");
    let url = if has_git_prefix {
        trimmed[4..].trim()
    } else {
        trimmed
    };
    if !has_git_prefix && !has_protocol_prefix(url) {
        return None;
    }

    let (repo_without_ref, reference) = split_git_reference(url);
    let (repo, host, path) = if let Some((host, path)) = parse_scp_like(&repo_without_ref) {
        (repo_without_ref.clone(), host, path)
    } else if has_protocol_prefix(&repo_without_ref) {
        let (scheme, authority, path) = parse_protocol_repo(&repo_without_ref)?;
        let host = extract_host_from_authority(&authority)?;
        (
            format!("{scheme}://{authority}/{}", path.trim_start_matches('/'))
                .trim_end_matches('/')
                .to_owned(),
            host,
            path.trim_start_matches('/').to_owned(),
        )
    } else {
        let (host, path) = parse_host_path(&repo_without_ref)?;
        if !has_git_prefix {
            return None;
        }
        (format!("https://{host}/{path}"), host, path)
    };

    let normalized_path = path
        .trim_start_matches('/')
        .trim_end_matches(".git")
        .to_owned();
    if host.is_empty() || normalized_path.split('/').count() < 2 {
        return None;
    }

    Some(GitSource {
        repo,
        host,
        path: normalized_path,
        pinned: reference.is_some(),
        reference,
    })
}

fn split_git_reference(url: &str) -> (String, Option<String>) {
    if let Some((host, path_with_ref)) = parse_scp_like(url) {
        if let Some(index) = path_with_ref.find('@') {
            let path = path_with_ref[..index].to_owned();
            let reference = path_with_ref[index + 1..].trim();
            if !path.is_empty() && !reference.is_empty() {
                return (format!("git@{host}:{path}"), Some(reference.to_owned()));
            }
        }
        return (url.to_owned(), None);
    }

    if has_protocol_prefix(url) {
        let Some((scheme, authority, path_with_ref)) = parse_protocol_repo(url) else {
            return (url.to_owned(), None);
        };
        if let Some(index) = path_with_ref.find('@') {
            let path = path_with_ref[..index].to_owned();
            let reference = path_with_ref[index + 1..].trim();
            if !path.is_empty() && !reference.is_empty() {
                return (
                    format!("{scheme}://{authority}/{path}")
                        .trim_end_matches('/')
                        .to_owned(),
                    Some(reference.to_owned()),
                );
            }
        }
        return (url.to_owned(), None);
    }

    let Some((host, path_with_ref)) = parse_host_path(url) else {
        return (url.to_owned(), None);
    };
    if let Some(index) = path_with_ref.find('@') {
        let path = path_with_ref[..index].to_owned();
        let reference = path_with_ref[index + 1..].trim();
        if !path.is_empty() && !reference.is_empty() {
            return (format!("{host}/{path}"), Some(reference.to_owned()));
        }
    }
    (url.to_owned(), None)
}

fn parse_scp_like(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("git@")?;
    let (host, path) = rest.split_once(':')?;
    Some((host.to_owned(), path.to_owned()))
}

fn parse_host_path(url: &str) -> Option<(String, String)> {
    let (host, path) = url.split_once('/')?;
    if !host.contains('.') && host != "localhost" {
        return None;
    }
    Some((host.to_owned(), path.to_owned()))
}

fn parse_protocol_repo(url: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let (authority, path) = rest.split_once('/')?;
    Some((scheme.to_owned(), authority.to_owned(), path.to_owned()))
}

fn extract_host_from_authority(authority: &str) -> Option<String> {
    let host_port = authority.rsplit('@').next()?.trim();
    if host_port.is_empty() {
        return None;
    }
    let host = host_port.split(':').next()?.trim();
    if host.is_empty() {
        return None;
    }
    Some(host.to_owned())
}

fn has_protocol_prefix(value: &str) -> bool {
    matches!(
        value,
        value if value.starts_with("https://")
            || value.starts_with("http://")
            || value.starts_with("ssh://")
            || value.starts_with("git://")
    )
}

fn split_patterns(entries: &[String]) -> (Vec<String>, Vec<String>) {
    let mut plain = Vec::new();
    let mut patterns = Vec::new();
    for entry in entries {
        if is_pattern(entry) {
            patterns.push(entry.clone());
        } else {
            plain.push(entry.clone());
        }
    }
    (plain, patterns)
}

fn is_pattern(value: &str) -> bool {
    value.starts_with('!')
        || value.starts_with('+')
        || value.starts_with('-')
        || value.contains('*')
        || value.contains('?')
}

fn apply_patterns(all_paths: &[String], patterns: &[String], base_dir: &Path) -> HashSet<String> {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    let mut force_includes = Vec::new();
    let mut force_excludes = Vec::new();

    for pattern in patterns {
        if let Some(pattern) = pattern.strip_prefix('+') {
            force_includes.push(pattern.to_owned());
        } else if let Some(pattern) = pattern.strip_prefix('-') {
            force_excludes.push(pattern.to_owned());
        } else if let Some(pattern) = pattern.strip_prefix('!') {
            excludes.push(pattern.to_owned());
        } else {
            includes.push(pattern.clone());
        }
    }

    let mut result = if includes.is_empty() {
        all_paths.to_vec()
    } else {
        all_paths
            .iter()
            .filter(|path| matches_any_pattern(path, &includes, base_dir))
            .cloned()
            .collect::<Vec<_>>()
    };

    if !excludes.is_empty() {
        result.retain(|path| !matches_any_pattern(path, &excludes, base_dir));
    }

    if !force_includes.is_empty() {
        for path in all_paths {
            if !result.contains(path) && matches_any_exact_pattern(path, &force_includes, base_dir)
            {
                result.push(path.clone());
            }
        }
    }

    if !force_excludes.is_empty() {
        result.retain(|path| !matches_any_exact_pattern(path, &force_excludes, base_dir));
    }

    result.into_iter().collect()
}

fn is_enabled_by_overrides(path: &str, overrides: &[String], base_dir: &Path) -> bool {
    let excludes = overrides
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('!').map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    let force_includes = overrides
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('+').map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    let force_excludes = overrides
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('-').map(ToOwned::to_owned))
        .collect::<Vec<_>>();

    let mut enabled = true;
    if !excludes.is_empty() && matches_any_pattern(path, &excludes, base_dir) {
        enabled = false;
    }
    if !force_includes.is_empty() && matches_any_exact_pattern(path, &force_includes, base_dir) {
        enabled = true;
    }
    if !force_excludes.is_empty() && matches_any_exact_pattern(path, &force_excludes, base_dir) {
        enabled = false;
    }
    enabled
}

fn matches_any_pattern(path: &str, patterns: &[String], base_dir: &Path) -> bool {
    let path = PathBuf::from(path);
    let rel = relative_path(base_dir, &path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .replace('\\', "/");
    let full = path_to_string(&path);
    let is_skill_file = file_name == "SKILL.md";
    let parent_dir = path.parent().map(PathBuf::from);
    let parent_rel = parent_dir
        .as_ref()
        .map(|parent| relative_path(base_dir, parent))
        .unwrap_or_default();
    let parent_name = parent_dir
        .as_ref()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .replace('\\', "/");
    let parent_full = parent_dir
        .as_ref()
        .map(|parent| path_to_string(parent))
        .unwrap_or_default();

    patterns.iter().any(|pattern| {
        glob_matches(pattern, &rel)
            || glob_matches(pattern, &file_name)
            || glob_matches(pattern, &full)
            || (is_skill_file
                && (glob_matches(pattern, &parent_rel)
                    || glob_matches(pattern, &parent_name)
                    || glob_matches(pattern, &parent_full)))
    })
}

fn matches_any_exact_pattern(path: &str, patterns: &[String], base_dir: &Path) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let path = PathBuf::from(path);
    let rel = normalize_exact_pattern(&relative_path(base_dir, &path));
    let full = normalize_exact_pattern(&path_to_string(&path));
    let is_skill_file = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "SKILL.md");
    let parent_rel = path
        .parent()
        .map(|parent| normalize_exact_pattern(&relative_path(base_dir, parent)))
        .unwrap_or_default();
    let parent_full = path
        .parent()
        .map(|parent| normalize_exact_pattern(&path_to_string(parent)))
        .unwrap_or_default();

    patterns.iter().any(|pattern| {
        let normalized = normalize_exact_pattern(pattern);
        normalized == rel
            || normalized == full
            || (is_skill_file && (normalized == parent_rel || normalized == parent_full))
    })
}

fn normalize_exact_pattern(pattern: &str) -> String {
    pattern
        .strip_prefix("./")
        .or_else(|| pattern.strip_prefix(".\\"))
        .unwrap_or(pattern)
        .replace('\\', "/")
}

fn glob_matches(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    let candidate = candidate.replace('\\', "/");
    if pattern == candidate {
        return true;
    }
    Glob::new(&pattern)
        .ok()
        .is_some_and(|glob| glob.compile_matcher().is_match(&candidate))
}

fn prefix_ignore_pattern(line: &str, prefix: &str) -> Option<IgnoreRule> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('#') && !trimmed.starts_with("\\#") {
        return None;
    }

    let mut pattern = trimmed;
    let mut negated = false;
    if let Some(rest) = pattern.strip_prefix('!') {
        negated = true;
        pattern = rest;
    } else if let Some(rest) = pattern.strip_prefix("\\!") {
        pattern = rest;
    }
    if let Some(rest) = pattern.strip_prefix('/') {
        pattern = rest;
    }
    let pattern = format!("{prefix}{pattern}");
    Some(IgnoreRule { pattern, negated })
}

fn ignore_rule_matches(pattern: &str, relative_path: &str, is_dir: bool) -> bool {
    let mut pattern = pattern.replace('\\', "/");
    let directory_only = pattern.ends_with('/');
    if directory_only {
        pattern.pop();
    }
    if pattern.is_empty() {
        return false;
    }

    let relative_path = relative_path.replace('\\', "/");
    if pattern.contains('/') {
        if glob_matches(&pattern, &relative_path) {
            return !directory_only || is_dir;
        }
        if !pattern.contains('*') && !pattern.contains('?') {
            return relative_path == pattern || relative_path.starts_with(&format!("{pattern}/"));
        }
        return false;
    }

    let mut matched = false;
    for component in relative_path.split('/') {
        if glob_matches(&pattern, component) {
            matched = true;
            break;
        }
    }
    matched && (!directory_only || is_dir)
}

fn format_command_error(command: &str, args: &[String], stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    let output = if stderr.is_empty() { stdout } else { stderr };
    if output.is_empty() {
        format!("{command} {} failed", args.join(" "))
    } else {
        format!("{command} {} failed: {output}", args.join(" "))
    }
}

fn stable_hash(input: &str) -> String {
    let mut value: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100000001b3);
    }
    format!("{value:016x}")
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

fn relative_path(base: &Path, target: &Path) -> String {
    if let Ok(relative) =
        normalize_pathbuf(target.to_path_buf()).strip_prefix(normalize_pathbuf(base.to_path_buf()))
    {
        let string = path_to_string(relative);
        return string.trim_start_matches('/').to_owned();
    }

    let base_components = normalize_pathbuf(base.to_path_buf())
        .components()
        .map(component_to_string)
        .collect::<Vec<_>>();
    let target_components = normalize_pathbuf(target.to_path_buf())
        .components()
        .map(component_to_string)
        .collect::<Vec<_>>();

    let mut common = 0usize;
    while common < base_components.len()
        && common < target_components.len()
        && base_components[common] == target_components[common]
    {
        common += 1;
    }

    let mut relative = Vec::new();
    for _ in common..base_components.len() {
        relative.push(String::from(".."));
    }
    for component in target_components.into_iter().skip(common) {
        if !component.is_empty() {
            relative.push(component);
        }
    }
    relative.join("/")
}

fn component_to_string(component: Component<'_>) -> String {
    component.as_os_str().to_string_lossy().replace('\\', "/")
}

fn normalize_pathbuf(path: PathBuf) -> PathBuf {
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
    normalized
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn find_git_repo_root(start_dir: &Path) -> Option<PathBuf> {
    let mut current = normalize_pathbuf(start_dir.to_path_buf());
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        let Some(parent) = current.parent() else {
            return None;
        };
        if parent == current {
            return None;
        }
        current = parent.to_path_buf();
    }
}

fn collect_ancestor_agents_skill_dirs(start_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut current = normalize_pathbuf(start_dir.to_path_buf());
    let git_repo_root = find_git_repo_root(&current);

    loop {
        dirs.push(current.join(".agents").join("skills"));
        if git_repo_root.as_ref().is_some_and(|root| *root == current) {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }

    dirs
}

#[cfg(test)]
mod tests {
    use super::{
        DefaultPackageManager, PackageSource, ResolveExtensionSourcesOptions, ResourceScope,
        parse_git_source,
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pi-coding-agent-cli-package-manager-{prefix}-{unique}"
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_supported_git_source_formats() {
        let https = parse_git_source("https://github.com/user/repo@v1").unwrap();
        assert_eq!(https.host, "github.com");
        assert_eq!(https.path, "user/repo");
        assert_eq!(https.reference.as_deref(), Some("v1"));
        assert!(https.pinned);

        let ssh = parse_git_source("ssh://git@github.com/user/repo").unwrap();
        assert_eq!(ssh.host, "github.com");
        assert_eq!(ssh.path, "user/repo");
        assert_eq!(ssh.reference, None);

        let shorthand = parse_git_source("git:git@github.com:user/repo").unwrap();
        assert_eq!(shorthand.host, "github.com");
        assert_eq!(shorthand.path, "user/repo");
        assert_eq!(shorthand.reference, None);

        assert!(parse_git_source("github.com/user/repo").is_none());
    }

    #[test]
    fn normalizes_local_sources_for_settings_relative_to_scope_base() {
        let temp_dir = unique_temp_dir("normalize");
        let agent_dir = temp_dir.join("agent");
        let cwd = temp_dir.join("project");
        let package_dir = cwd.join("packages").join("demo");
        fs::create_dir_all(package_dir.join("extensions")).unwrap();
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();

        let manager = DefaultPackageManager::new(cwd.clone(), agent_dir.clone());
        manager
            .add_source_to_settings("./packages/demo", true)
            .expect("expected source to be added");

        let settings = fs::read_to_string(cwd.join(".pi").join("settings.json")).unwrap();
        assert!(
            settings.contains("../packages/demo"),
            "settings: {settings}"
        );
    }

    #[test]
    fn resolves_package_resources_from_project_settings() {
        let temp_dir = unique_temp_dir("resolve");
        let agent_dir = temp_dir.join("agent");
        let cwd = temp_dir.join("project");
        let package_dir = temp_dir.join("package");
        fs::create_dir_all(agent_dir.clone()).unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::create_dir_all(package_dir.join("prompts")).unwrap();
        fs::create_dir_all(package_dir.join("skills").join("demo-skill")).unwrap();
        fs::write(package_dir.join("prompts").join("review.md"), "Review").unwrap();
        fs::write(
            package_dir
                .join("skills")
                .join("demo-skill")
                .join("SKILL.md"),
            "---\ndescription: Demo\n---\nSkill\n",
        )
        .unwrap();
        fs::write(
            cwd.join(".pi").join("settings.json"),
            format!("{{\n  \"packages\": [\"{}\"]\n}}\n", package_dir.display()),
        )
        .unwrap();

        let manager = DefaultPackageManager::new(cwd, agent_dir);
        let resolved = manager
            .resolve()
            .expect("expected resolution to succeed")
            .resolved;
        assert!(
            resolved
                .prompts
                .iter()
                .any(|resource| resource.path.ends_with("review.md") && resource.enabled),
            "prompts: {:?}",
            resolved.prompts
        );
        assert!(
            resolved
                .skills
                .iter()
                .any(|resource| resource.path.ends_with("demo-skill/SKILL.md") && resource.enabled),
            "skills: {:?}",
            resolved.skills
        );
    }

    #[test]
    fn resolve_extension_sources_supports_temporary_local_package_layouts() {
        let temp_dir = unique_temp_dir("temporary");
        let agent_dir = temp_dir.join("agent");
        let cwd = temp_dir.join("project");
        let package_dir = temp_dir.join("package");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(package_dir.join("extensions").join("demo")).unwrap();
        fs::write(
            package_dir.join("extensions").join("demo").join("index.ts"),
            "export default function () {}\n",
        )
        .unwrap();
        fs::write(
            package_dir
                .join("extensions")
                .join("demo")
                .join("helper.ts"),
            "export const helper = true;\n",
        )
        .unwrap();

        let manager = DefaultPackageManager::new(cwd, agent_dir);
        let resolved = manager
            .resolve_extension_sources(
                &[package_dir.to_string_lossy().into_owned()],
                ResolveExtensionSourcesOptions {
                    temporary: true,
                    local: false,
                },
            )
            .expect("expected extension source resolution to succeed")
            .resolved;
        assert!(
            resolved.extensions.iter().any(|resource| {
                resource.path.ends_with("extensions/demo/index.ts")
                    && resource.metadata.scope == ResourceScope::Temporary
            }),
            "extensions: {:?}",
            resolved.extensions
        );
        assert!(
            !resolved
                .extensions
                .iter()
                .any(|resource| resource.path.ends_with("helper.ts")),
            "extensions: {:?}",
            resolved.extensions
        );
    }

    #[test]
    fn project_scope_dedupes_global_packages() {
        let temp_dir = unique_temp_dir("dedupe");
        let agent_dir = temp_dir.join("agent");
        let cwd = temp_dir.join("project");
        let package_dir = temp_dir.join("package");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::create_dir_all(package_dir.join("prompts")).unwrap();
        fs::write(package_dir.join("prompts").join("review.md"), "Review").unwrap();
        fs::write(
            agent_dir.join("settings.json"),
            format!("{{\n  \"packages\": [\"{}\"]\n}}\n", package_dir.display()),
        )
        .unwrap();
        fs::write(
            cwd.join(".pi").join("settings.json"),
            format!("{{\n  \"packages\": [\"{}\"]\n}}\n", package_dir.display()),
        )
        .unwrap();

        let manager = DefaultPackageManager::new(cwd, agent_dir);
        let resolved = manager
            .resolve()
            .expect("expected resolution to succeed")
            .resolved;
        let matches = resolved
            .prompts
            .iter()
            .filter(|resource| resource.path.ends_with("review.md"))
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].metadata.scope, ResourceScope::Project);
    }

    #[test]
    fn list_configured_packages_marks_filtered_entries() {
        let temp_dir = unique_temp_dir("configured");
        let agent_dir = temp_dir.join("agent");
        let cwd = temp_dir.join("project");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(
            agent_dir.join("settings.json"),
            "{\n  \"packages\": [\n    \"npm:plain\",\n    {\n      \"source\": \"npm:filtered\",\n      \"extensions\": [\"extensions\"]\n    }\n  ]\n}\n",
        )
        .unwrap();

        let manager = DefaultPackageManager::new(cwd, agent_dir);
        let packages = manager.list_configured_packages();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].source, "npm:plain");
        assert!(!packages[0].filtered);
        assert_eq!(packages[1].source, "npm:filtered");
        assert!(packages[1].filtered);
        assert_eq!(
            PackageSource::Plain(String::from("npm:plain")).source(),
            "npm:plain"
        );
    }
}
