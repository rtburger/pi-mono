use pi_coding_agent_core::{
    build_default_pi_system_prompt, build_system_prompt, load_project_context_files,
    load_system_prompt_resources, resolve_prompt_input, BuildSystemPromptOptions, ContextFile,
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
        "pi-coding-agent-core-system-prompt-{prefix}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn load_project_context_files_returns_global_then_root_to_leaf_contexts() {
    let temp_dir = unique_temp_dir("context-order");
    let agent_dir = temp_dir.join("agent");
    let root_dir = temp_dir.join("workspace");
    let nested_dir = root_dir.join("nested");
    let cwd = nested_dir.join("current");

    fs::create_dir_all(&agent_dir).unwrap();
    fs::create_dir_all(&cwd).unwrap();

    fs::write(agent_dir.join("AGENTS.md"), "global instructions\n").unwrap();
    fs::write(root_dir.join("CLAUDE.md"), "root instructions\n").unwrap();
    fs::write(nested_dir.join("AGENTS.md"), "nested instructions\n").unwrap();
    fs::write(cwd.join("CLAUDE.md"), "cwd instructions\n").unwrap();

    let context_files = load_project_context_files(&cwd, &agent_dir);
    let paths = context_files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        vec![
            agent_dir.join("AGENTS.md").display().to_string(),
            root_dir.join("CLAUDE.md").display().to_string(),
            nested_dir.join("AGENTS.md").display().to_string(),
            cwd.join("CLAUDE.md").display().to_string(),
        ]
    );
}

#[test]
fn load_system_prompt_resources_prefers_project_prompt_files() {
    let temp_dir = unique_temp_dir("discover");
    let agent_dir = temp_dir.join("agent");
    let cwd = temp_dir.join("project");
    let project_pi_dir = cwd.join(".pi");

    fs::create_dir_all(&agent_dir).unwrap();
    fs::create_dir_all(&project_pi_dir).unwrap();

    fs::write(agent_dir.join("SYSTEM.md"), "global system\n").unwrap();
    fs::write(agent_dir.join("APPEND_SYSTEM.md"), "global append\n").unwrap();
    fs::write(project_pi_dir.join("SYSTEM.md"), "project system\n").unwrap();
    fs::write(project_pi_dir.join("APPEND_SYSTEM.md"), "project append\n").unwrap();

    let resources = load_system_prompt_resources(&cwd, &agent_dir);

    assert_eq!(resources.system_prompt.as_deref(), Some("project system\n"));
    assert_eq!(
        resources.append_system_prompt,
        vec![String::from("project append\n")]
    );
}

#[test]
fn build_system_prompt_renders_default_prompt_context_and_footer() {
    let prompt = build_system_prompt(BuildSystemPromptOptions {
        selected_tools: vec![
            String::from("read"),
            String::from("bash"),
            String::from("edit"),
            String::from("write"),
        ],
        tool_snippets: BTreeMap::from([
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
        ]),
        cwd: Some(PathBuf::from("/work/tree")),
        date: Some(String::from("2026-04-15")),
        context_files: vec![ContextFile {
            path: String::from("/work/tree/AGENTS.md"),
            content: String::from("Follow repo rules.\n"),
        }],
        readme_path: Some(PathBuf::from("/pkg/README.md")),
        docs_path: Some(PathBuf::from("/pkg/docs")),
        examples_path: Some(PathBuf::from("/pkg/examples")),
        ..BuildSystemPromptOptions::default()
    });

    assert!(prompt.contains("Available tools:\n- read: Read file contents"));
    assert!(prompt.contains("- Main documentation: /pkg/README.md"));
    assert!(prompt.contains("# Project Context"));
    assert!(prompt.contains("## /work/tree/AGENTS.md"));
    assert!(prompt.ends_with("Current date: 2026-04-15\nCurrent working directory: /work/tree"));
}

#[test]
fn build_system_prompt_defaults_to_rust_reference_bundle() {
    let prompt = build_system_prompt(BuildSystemPromptOptions {
        cwd: Some(PathBuf::from("/work/tree")),
        date: Some(String::from("2026-04-15")),
        ..BuildSystemPromptOptions::default()
    });

    let reference_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../support/coding-agent-reference")
        .canonicalize()
        .unwrap();

    assert!(reference_dir.join("README.md").exists());
    assert!(reference_dir.join("docs").exists());
    assert!(reference_dir.join("examples").exists());
    assert!(prompt.contains(&format!(
        "- Main documentation: {}",
        reference_dir.join("README.md").display()
    )));
    assert!(prompt.contains(&format!(
        "- Additional docs: {}",
        reference_dir.join("docs").display()
    )));
    assert!(prompt.contains(&format!(
        "- Examples: {}",
        reference_dir.join("examples").display()
    )));
    assert!(
        !prompt.contains("packages/coding-agent"),
        "prompt: {prompt}"
    );
}

#[test]
fn build_system_prompt_preserves_explicit_no_tools_selection() {
    let prompt = build_system_prompt(BuildSystemPromptOptions {
        selected_tools: Vec::new(),
        selected_tools_explicit: true,
        tool_snippets: BTreeMap::from([(String::from("read"), String::from("Read file contents"))]),
        cwd: Some(PathBuf::from("/work/tree")),
        date: Some(String::from("2026-04-15")),
        ..BuildSystemPromptOptions::default()
    });

    assert!(
        prompt.contains("Available tools:\n(none)"),
        "prompt: {prompt}"
    );
    assert!(
        !prompt.contains("- read: Read file contents"),
        "prompt: {prompt}"
    );
}

#[test]
fn build_default_pi_system_prompt_applies_cli_override_and_discovered_append() {
    let temp_dir = unique_temp_dir("default-wrapper");
    let agent_dir = temp_dir.join("agent");
    let cwd = temp_dir.join("project");
    let project_pi_dir = cwd.join(".pi");

    fs::create_dir_all(&agent_dir).unwrap();
    fs::create_dir_all(&project_pi_dir).unwrap();

    fs::write(agent_dir.join("AGENTS.md"), "global rules\n").unwrap();
    fs::write(
        project_pi_dir.join("APPEND_SYSTEM.md"),
        "append from project\n",
    )
    .unwrap();

    let prompt = build_default_pi_system_prompt(&cwd, &agent_dir, Some("CLI system"), None);

    assert!(prompt.starts_with("CLI system\n\nappend from project\n"));
    assert!(prompt.contains("## "));
    assert!(prompt.contains(&agent_dir.join("AGENTS.md").display().to_string()));
}

#[test]
fn resolve_prompt_input_reads_existing_files() {
    let temp_dir = unique_temp_dir("resolve-input");
    let prompt_path = temp_dir.join("prompt.md");
    fs::write(&prompt_path, "from file\n").unwrap();

    assert_eq!(
        resolve_prompt_input(Some(prompt_path.to_str().unwrap())).as_deref(),
        Some("from file\n")
    );
    assert_eq!(
        resolve_prompt_input(Some("inline prompt")).as_deref(),
        Some("inline prompt")
    );
    assert_eq!(resolve_prompt_input(Some("   ")), None);
}
