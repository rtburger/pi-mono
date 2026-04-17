use pi_coding_agent_core::{
    BuildSystemPromptOptions, DefaultResourceLoader, DefaultResourceLoaderOptions,
    ResourcePathEntry, SourceInfo, expand_prompt_template, expand_skill_command,
    load_prompt_templates, load_skills,
};
use std::{
    collections::BTreeMap,
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
        "pi-coding-agent-core-resources-{prefix}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn load_prompt_templates_reads_defaults_and_expands_arguments() {
    let temp_dir = unique_temp_dir("prompts");
    let agent_dir = temp_dir.join("agent");
    let cwd = temp_dir.join("project");
    fs::create_dir_all(agent_dir.join("prompts")).unwrap();
    fs::create_dir_all(cwd.join(".pi").join("prompts")).unwrap();

    fs::write(
        agent_dir.join("prompts").join("review.md"),
        "---\ndescription: Review code\n---\nReview $1 and $ARGUMENTS\n",
    )
    .unwrap();
    fs::write(
        cwd.join(".pi").join("prompts").join("summarize.md"),
        "Summarize ${@:2}\n",
    )
    .unwrap();

    let loaded = load_prompt_templates(pi_coding_agent_core::LoadPromptTemplatesOptions {
        cwd: cwd.clone(),
        agent_dir: Some(agent_dir.clone()),
        prompt_paths: Vec::new(),
        include_defaults: true,
    });

    assert!(
        loaded.diagnostics.is_empty(),
        "diagnostics: {:?}",
        loaded.diagnostics
    );
    assert_eq!(loaded.prompts.len(), 2);
    let review = loaded
        .prompts
        .iter()
        .find(|prompt| prompt.name == "review")
        .expect("expected review prompt");
    assert_eq!(review.description, "Review code");
    assert_eq!(
        expand_prompt_template("/review src/lib.rs src/main.rs", &loaded.prompts),
        "Review src/lib.rs and src/lib.rs src/main.rs\n"
    );
    assert_eq!(
        expand_prompt_template("/summarize ignore these remaining words", &loaded.prompts),
        "Summarize these remaining words\n"
    );
}

#[test]
fn load_skills_reads_defaults_and_expands_skill_commands() {
    let temp_dir = unique_temp_dir("skills");
    let agent_dir = temp_dir.join("agent");
    let cwd = temp_dir.join("project");
    let skill_dir = cwd.join(".pi").join("skills").join("review-code");
    fs::create_dir_all(agent_dir.join("skills")).unwrap();
    fs::create_dir_all(&skill_dir).unwrap();

    fs::write(
        skill_dir.join("SKILL.md"),
        "---\ndescription: Review code safely\n---\n# Review Code\nRead the target file first.\n",
    )
    .unwrap();

    let loaded = load_skills(pi_coding_agent_core::LoadSkillsOptions {
        cwd: cwd.clone(),
        agent_dir: Some(agent_dir),
        skill_paths: Vec::new(),
        include_defaults: true,
    });

    assert!(
        loaded.diagnostics.is_empty(),
        "diagnostics: {:?}",
        loaded.diagnostics
    );
    assert_eq!(loaded.skills.len(), 1);
    let skill = &loaded.skills[0];
    assert_eq!(skill.name, "review-code");
    assert_eq!(skill.description, "Review code safely");

    let expanded = expand_skill_command("/skill:review-code src/lib.rs", &loaded.skills);
    assert!(
        expanded.contains("<skill name=\"review-code\""),
        "expanded: {expanded}"
    );
    assert!(
        expanded.contains("References are relative to"),
        "expanded: {expanded}"
    );
    assert!(expanded.contains("src/lib.rs"), "expanded: {expanded}");
}

#[test]
fn default_resource_loader_reloads_disk_backed_resources() {
    let temp_dir = unique_temp_dir("loader-reload");
    let agent_dir = temp_dir.join("agent");
    let cwd = temp_dir.join("project");
    let prompts_dir = cwd.join(".pi").join("prompts");
    let skill_dir = cwd.join(".pi").join("skills").join("review-code");
    fs::create_dir_all(&prompts_dir).unwrap();
    fs::create_dir_all(&skill_dir).unwrap();
    fs::create_dir_all(&agent_dir).unwrap();

    fs::write(prompts_dir.join("review.md"), "Review $1\n").unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\ndescription: Review code safely\n---\nRead the target file first.\n",
    )
    .unwrap();
    fs::write(agent_dir.join("AGENTS.md"), "global rules\n").unwrap();
    fs::write(cwd.join(".pi").join("SYSTEM.md"), "project system\n").unwrap();
    fs::write(cwd.join(".pi").join("APPEND_SYSTEM.md"), "project append\n").unwrap();

    let mut loader = DefaultResourceLoader::load(DefaultResourceLoaderOptions {
        cwd: cwd.clone(),
        agent_dir: Some(agent_dir.clone()),
        prompt_paths: vec![prompts_dir.display().to_string()],
        skill_paths: vec![skill_dir.display().to_string()],
        ..DefaultResourceLoaderOptions::default()
    });

    assert!(
        loader.warnings().is_empty(),
        "warnings: {:?}",
        loader.warnings()
    );
    assert_eq!(loader.prompt_templates().len(), 1);
    assert_eq!(loader.skills().len(), 1);
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
fn default_resource_loader_extends_resources_with_source_info() {
    let temp_dir = unique_temp_dir("loader-extend");
    let cwd = temp_dir.join("project");
    let extension_dir = temp_dir.join("extension");
    let extension_skill_dir = extension_dir.join("skills").join("extension-review");
    let extension_prompts_dir = extension_dir.join("prompts");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&extension_skill_dir).unwrap();
    fs::create_dir_all(&extension_prompts_dir).unwrap();

    let skill_path = extension_skill_dir.join("SKILL.md");
    let prompt_path = extension_prompts_dir.join("review.md");
    fs::write(
        &skill_path,
        "---\ndescription: Review extension code\n---\nCheck extension assets first.\n",
    )
    .unwrap();
    fs::write(&prompt_path, "Extension review $1\n").unwrap();

    let mut loader = DefaultResourceLoader::load(DefaultResourceLoaderOptions {
        cwd: cwd.clone(),
        ..DefaultResourceLoaderOptions::default()
    });
    loader.extend_resources(
        &[ResourcePathEntry {
            path: skill_path.display().to_string(),
            source_info: SourceInfo {
                path: skill_path.display().to_string(),
                source: String::from("extension:demo"),
                scope: String::from("temporary"),
                origin: String::from("top-level"),
                base_dir: Some(extension_dir.display().to_string()),
            },
        }],
        &[ResourcePathEntry {
            path: prompt_path.display().to_string(),
            source_info: SourceInfo {
                path: prompt_path.display().to_string(),
                source: String::from("extension:demo"),
                scope: String::from("temporary"),
                origin: String::from("top-level"),
                base_dir: Some(extension_dir.display().to_string()),
            },
        }],
    );

    assert!(
        loader.warnings().is_empty(),
        "warnings: {:?}",
        loader.warnings()
    );
    assert_eq!(loader.skills().len(), 1);
    assert_eq!(loader.prompt_templates().len(), 1);
    assert_eq!(
        loader.preprocess_prompt_text("/review src/lib.rs"),
        "Extension review src/lib.rs\n"
    );
    assert!(
        loader
            .skills()
            .iter()
            .all(|skill| skill.source_info.source.starts_with("extension:"))
    );
    assert!(
        loader
            .prompt_templates()
            .iter()
            .all(|prompt| prompt.source_info.source.starts_with("extension:"))
    );
}

#[cfg(unix)]
#[test]
fn load_skills_respects_ignore_files_and_skips_symlink_duplicates() {
    let temp_dir = unique_temp_dir("skills-ignore");
    let skills_dir = temp_dir.join("skills");
    let primary_skill_dir = skills_dir.join("review-code");
    let ignored_skill_dir = skills_dir.join("ignored-skill");
    let linked_skill_dir = skills_dir.join("linked-review");
    fs::create_dir_all(&primary_skill_dir).unwrap();
    fs::create_dir_all(&ignored_skill_dir).unwrap();
    fs::write(skills_dir.join(".gitignore"), "ignored-skill/\n").unwrap();
    fs::write(
        primary_skill_dir.join("SKILL.md"),
        "---\ndescription: Review code safely\n---\nRead the target file first.\n",
    )
    .unwrap();
    fs::write(
        ignored_skill_dir.join("SKILL.md"),
        "---\ndescription: This should be ignored\n---\nIgnored skill.\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&primary_skill_dir, &linked_skill_dir).unwrap();

    let loaded = load_skills(pi_coding_agent_core::LoadSkillsOptions {
        cwd: temp_dir.clone(),
        agent_dir: None,
        skill_paths: vec![skills_dir.display().to_string()],
        include_defaults: false,
    });

    assert!(
        loaded.diagnostics.is_empty(),
        "diagnostics: {:?}",
        loaded.diagnostics
    );
    assert_eq!(loaded.skills.len(), 1);
    assert_eq!(loaded.skills[0].description, "Review code safely");
    assert!(
        loaded
            .skills
            .iter()
            .all(|skill| skill.name != "ignored-skill")
    );
}

#[test]
fn build_system_prompt_appends_skills_when_read_tool_is_available() {
    let temp_dir = unique_temp_dir("build-skills");
    let skill_dir = temp_dir.join("skills").join("review-code");
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    fs::write(
        &skill_path,
        "---\ndescription: Review code safely\n---\n# Review Code\n",
    )
    .unwrap();

    let loaded = load_skills(pi_coding_agent_core::LoadSkillsOptions {
        cwd: temp_dir.clone(),
        agent_dir: None,
        skill_paths: vec![skill_path.to_string_lossy().into_owned()],
        include_defaults: false,
    });

    let prompt = pi_coding_agent_core::build_system_prompt(BuildSystemPromptOptions {
        selected_tools: vec![String::from("read"), String::from("grep")],
        tool_snippets: BTreeMap::from([
            (String::from("read"), String::from("Read file contents")),
            (String::from("grep"), String::from("Search file contents")),
        ]),
        skills: loaded.skills,
        cwd: Some(temp_dir),
        date: Some(String::from("2026-04-15")),
        ..BuildSystemPromptOptions::default()
    });

    assert!(prompt.contains("<available_skills>"), "prompt: {prompt}");
    assert!(prompt.contains("review-code"), "prompt: {prompt}");
    assert!(prompt.contains("Review code safely"), "prompt: {prompt}");
}
