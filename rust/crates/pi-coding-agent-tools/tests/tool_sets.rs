use pi_coding_agent_tools::{
    create_all_tool_definitions, create_all_tools, create_coding_tool_definitions,
    create_read_only_tool_definitions, create_read_only_tools,
};
use std::path::PathBuf;

#[test]
fn coding_tool_definitions_match_typescript_default_set() {
    let names = create_coding_tool_definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["read", "bash", "edit", "write"]);
}

#[test]
fn read_only_tool_definitions_include_search_and_listing_tools() {
    let names = create_read_only_tool_definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["read", "grep", "find", "ls"]);
}

#[test]
fn all_tool_definitions_are_available_by_name() {
    let definitions = create_all_tool_definitions();

    assert_eq!(definitions.len(), 7);
    for name in ["read", "bash", "edit", "write", "grep", "find", "ls"] {
        assert_eq!(
            definitions
                .get(name)
                .map(|definition| definition.name.as_str()),
            Some(name),
            "missing definition for {name}"
        );
    }
}

#[test]
fn read_only_tools_match_read_only_helper_order() {
    let tools = create_read_only_tools(PathBuf::from("/tmp"));
    let names = tools
        .into_iter()
        .map(|tool| tool.definition.name)
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["read", "grep", "find", "ls"]);
}

#[test]
fn all_tools_are_available_by_name() {
    let tools = create_all_tools(PathBuf::from("/tmp"));

    assert_eq!(tools.len(), 7);
    for name in ["read", "bash", "edit", "write", "grep", "find", "ls"] {
        assert_eq!(
            tools.get(name).map(|tool| tool.definition.name.as_str()),
            Some(name),
            "missing tool for {name}"
        );
    }
}
