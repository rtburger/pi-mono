use pi_coding_agent_core::{ParsedSkillBlock, parse_skill_block};

#[test]
fn parses_skill_block_without_user_message() {
    let parsed = parse_skill_block(
        "<skill name=\"test\" location=\"/tmp/SKILL.md\">\nReferences are relative to /tmp.\n\nUse ripgrep first.\n</skill>",
    )
    .expect("skill block should parse");

    assert_eq!(
        parsed,
        ParsedSkillBlock {
            name: "test".into(),
            location: "/tmp/SKILL.md".into(),
            content: "References are relative to /tmp.\n\nUse ripgrep first.".into(),
            user_message: None,
        }
    );
}

#[test]
fn parses_skill_block_with_trimmed_user_message() {
    let parsed = parse_skill_block(
        "<skill name=\"calendar\" location=\"/repo/skills/calendar/SKILL.md\">\nRead the schedule file first.\n</skill>\n\n  Please update the tests.  ",
    )
    .expect("skill block should parse");

    assert_eq!(parsed.name, "calendar");
    assert_eq!(parsed.location, "/repo/skills/calendar/SKILL.md");
    assert_eq!(parsed.content, "Read the schedule file first.");
    assert_eq!(
        parsed.user_message.as_deref(),
        Some("Please update the tests.")
    );
}

#[test]
fn rejects_text_that_is_not_exact_skill_block_shape() {
    assert!(parse_skill_block("plain user message").is_none());
    assert!(
        parse_skill_block(
            "<skill name=\"test\" location=\"/tmp/SKILL.md\">\ncontent\n</skill>\nextra"
        )
        .is_none()
    );
}
