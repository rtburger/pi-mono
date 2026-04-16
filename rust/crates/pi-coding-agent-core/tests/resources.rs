use pi_coding_agent_core::{
    BuildSystemPromptOptions, expand_prompt_template, expand_skill_command, load_prompt_templates,
    load_skills,
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
