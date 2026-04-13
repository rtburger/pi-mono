use pi_coding_agent_tui::{
    KeybindingsManager, PlainKeyHintStyler, StartupShellComponent, ToolExecutionComponent,
    ToolExecutionOptions, ToolExecutionRendererDefinition, ToolExecutionResult,
};
use pi_events::UserContent;
use pi_tui::{Component, KeyId, Terminal, Tui, TuiError, visible_width};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};

#[derive(Default)]
struct NoopTerminal;

impl Terminal for NoopTerminal {
    fn start(
        &mut self,
        _on_input: Box<dyn FnMut(String) + Send>,
        _on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn drain_input(&mut self, _max: Duration, _idle: Duration) -> Result<(), TuiError> {
        Ok(())
    }

    fn write(&mut self, _data: &str) -> Result<(), TuiError> {
        Ok(())
    }

    fn columns(&self) -> u16 {
        80
    }

    fn rows(&self) -> u16 {
        24
    }

    fn kitty_protocol_active(&self) -> bool {
        false
    }

    fn move_by(&mut self, _lines: i32) -> Result<(), TuiError> {
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn clear_line(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn clear_from_cursor(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn clear_screen(&mut self) -> Result<(), TuiError> {
        Ok(())
    }

    fn set_title(&mut self, _title: &str) -> Result<(), TuiError> {
        Ok(())
    }
}

fn default_keybindings() -> KeybindingsManager {
    KeybindingsManager::new(BTreeMap::new(), None)
}

fn strip_ansi(text: &str) -> String {
    let mut stripped = String::new();
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            index += 1;
            if index < bytes.len() && bytes[index] == b'[' {
                index += 1;
                while index < bytes.len() && !(bytes[index] as char).is_ascii_alphabetic() {
                    index += 1;
                }
                if index < bytes.len() {
                    index += 1;
                }
                continue;
            }
        }

        let character = text[index..].chars().next().expect("valid UTF-8 character");
        stripped.push(character);
        index += character.len_utf8();
    }

    stripped
}

#[test]
fn tool_execution_component_renders_tool_name_args_and_text_result() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new(
        "read",
        "tool-1",
        json!({ "path": "README.md" }),
        ToolExecutionOptions::default(),
        &keybindings,
    );

    let initial_lines = component.render(64);
    assert!(initial_lines.iter().all(|line| visible_width(line) <= 64));
    assert!(initial_lines.iter().any(|line| line.contains("read")));
    assert!(initial_lines.iter().any(|line| line.contains("README.md")));

    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "first line\nsecond line".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let result_lines = component.render(64);
    assert!(result_lines.iter().all(|line| visible_width(line) <= 64));
    assert!(result_lines.iter().any(|line| line.contains("first line")));
    assert!(result_lines.iter().any(|line| line.contains("second line")));
}

#[test]
fn tool_execution_component_updates_args_and_renders_image_fallback_text() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new(
        "read",
        "tool-2",
        json!({ "path": "README.md" }),
        ToolExecutionOptions::default(),
        &keybindings,
    );

    component.update_args(json!({ "path": "CHANGELOG.md" }));
    component.update_result(
        ToolExecutionResult {
            content: vec![
                UserContent::Text {
                    text: "captured screenshot".into(),
                },
                UserContent::Image {
                    data: "ZmFrZQ==".into(),
                    mime_type: "image/png".into(),
                },
            ],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let lines = component.render(72);
    assert!(lines.iter().all(|line| visible_width(line) <= 72));
    assert!(lines.iter().any(|line| line.contains("CHANGELOG.md")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("captured screenshot"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("[Image: [image/png]]"))
    );
}

#[test]
fn custom_call_and_result_renderers_stack_for_custom_tools() {
    let keybindings = default_keybindings();
    let renderers = ToolExecutionRendererDefinition::new()
        .with_render_call(|_args, _context| Box::new(pi_tui::Text::new("custom call", 0, 0)))
        .with_render_result(|_result, _options, _context| {
            Box::new(pi_tui::Text::new("custom result", 0, 0))
        });
    let mut component = ToolExecutionComponent::new_with_definition(
        "custom_tool",
        "tool-custom-1",
        json!({}),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(renderers),
        (),
    );

    let initial = strip_ansi(&component.render(120).join("\n"));
    assert!(initial.contains("custom call"));

    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "done".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("custom call"));
    assert!(rendered.contains("custom result"));
}

#[test]
fn built_in_renderers_still_apply_for_built_in_overrides_without_custom_slots() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new_with_definition(
        "edit",
        "tool-edit-override-1",
        json!({ "path": "README.md", "oldText": "before", "newText": "after" }),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(ToolExecutionRendererDefinition::<()>::default()),
        (),
    );
    component.update_result(
        ToolExecutionResult {
            content: Vec::new(),
            details: json!({ "diff": "+1 after", "firstChangedLine": 1 }),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("edit README.md"));
    assert!(!rendered.contains(":1"));
}

#[test]
fn built_in_result_renderer_is_inherited_when_only_custom_call_renderer_is_provided() {
    let keybindings = default_keybindings();
    let renderers = ToolExecutionRendererDefinition::new()
        .with_render_call(|_args, _context| Box::new(pi_tui::Text::new("override call", 0, 0)));
    let mut component = ToolExecutionComponent::new_with_definition(
        "read",
        "tool-read-override-1",
        json!({ "path": "README.md" }),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(renderers),
        (),
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "hello".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("override call"));
    assert!(rendered.contains("hello"));
}

#[test]
fn built_in_call_renderer_is_inherited_when_only_custom_result_renderer_is_provided() {
    let keybindings = default_keybindings();
    let renderers =
        ToolExecutionRendererDefinition::new().with_render_result(|_result, _options, _context| {
            Box::new(pi_tui::Text::new("override result", 0, 0))
        });
    let mut component = ToolExecutionComponent::new_with_definition(
        "read",
        "tool-read-override-2",
        json!({ "path": "README.md" }),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(renderers),
        (),
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "hello".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("read README.md"));
    assert!(rendered.contains("override result"));
}

#[test]
fn custom_renderers_override_built_in_renderers_when_both_slots_are_present() {
    let keybindings = default_keybindings();
    let renderers = ToolExecutionRendererDefinition::new()
        .with_render_call(|_args, _context| Box::new(pi_tui::Text::new("override call", 0, 0)))
        .with_render_result(|_result, _options, _context| {
            Box::new(pi_tui::Text::new("override result", 0, 0))
        });
    let mut component = ToolExecutionComponent::new_with_definition(
        "read",
        "tool-read-override-3",
        json!({ "path": "README.md" }),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(renderers),
        (),
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "hello".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("override call"));
    assert!(rendered.contains("override result"));
    assert!(!rendered.contains("read README.md"));
}

#[derive(Default)]
struct RendererState {
    token: Option<String>,
}

#[test]
fn custom_call_and_result_renderers_share_state() {
    let keybindings = default_keybindings();
    let renderers = ToolExecutionRendererDefinition::<RendererState>::new()
        .with_render_call(|_args, context| {
            context
                .state
                .token
                .get_or_insert_with(|| "shared-token".to_owned());
            Box::new(pi_tui::Text::new(
                format!(
                    "custom call {}",
                    context
                        .state
                        .token
                        .as_deref()
                        .expect("token set by call renderer")
                ),
                0,
                0,
            ))
        })
        .with_render_result(|_result, _options, context| {
            Box::new(pi_tui::Text::new(
                format!(
                    "custom result {}",
                    context
                        .state
                        .token
                        .as_deref()
                        .expect("token should be shared across renderers")
                ),
                0,
                0,
            ))
        });
    let mut component = ToolExecutionComponent::new_with_definition(
        "custom_tool",
        "tool-custom-2",
        json!({}),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(renderers),
        RendererState::default(),
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "done".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("custom call shared-token"));
    assert!(rendered.contains("custom result shared-token"));
}

#[test]
fn result_renderer_context_exposes_current_args() {
    let keybindings = default_keybindings();
    let renderers = ToolExecutionRendererDefinition::new()
        .with_render_call(|_args, _context| Box::new(pi_tui::Text::new("call", 0, 0)))
        .with_render_result(|_result, _options, context| {
            let argument = context
                .args
                .get("foo")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("missing");
            Box::new(pi_tui::Text::new(format!("arg:{argument}"), 0, 0))
        });
    let mut component = ToolExecutionComponent::new_with_definition(
        "custom_tool",
        "tool-custom-3",
        json!({ "foo": "bar" }),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(renderers),
        (),
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "done".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("arg:bar"));
}

#[test]
fn custom_tools_without_renderers_use_component_fallbacks_without_argument_dump() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new_with_definition(
        "custom_tool",
        "tool-custom-4",
        json!({ "foo": "bar" }),
        ToolExecutionOptions::default(),
        &keybindings,
        Some(ToolExecutionRendererDefinition::<()>::default()),
        (),
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "done".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = strip_ansi(&component.render(120).join("\n"));
    assert!(rendered.contains("custom_tool"));
    assert!(rendered.contains("done"));
    assert!(!rendered.contains("\"foo\""));
}

#[test]
fn built_in_read_renderer_supports_legacy_file_path_and_line_ranges() {
    let keybindings = default_keybindings();
    let component = ToolExecutionComponent::new(
        "read",
        "tool-legacy-read",
        json!({ "file_path": "README.md", "offset": 5, "limit": 3 }),
        ToolExecutionOptions::default(),
        &keybindings,
    );

    let rendered = component.render(80).join("\n");

    assert!(rendered.contains("read README.md:5-7"));
    assert!(!rendered.contains("\"file_path\""));
}

#[test]
fn built_in_write_renderer_trims_trailing_blank_preview_lines_and_hides_success_text() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new(
        "write",
        "tool-write-1",
        json!({ "path": "README.md", "content": "one\ntwo\n" }),
        ToolExecutionOptions::default(),
        &keybindings,
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "Successfully wrote 8 bytes to README.md".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = component.render(120).join("\n");

    assert!(rendered.contains("write README.md"));
    assert!(rendered.contains("one"));
    assert!(rendered.contains("two"));
    assert!(!rendered.contains("Successfully wrote 8 bytes to README.md"));
    assert!(!rendered.contains("two\n\n"));
}

#[test]
fn built_in_read_renderer_trims_trailing_blank_lines_from_results() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new(
        "read",
        "tool-read-1",
        json!({ "path": "README.md" }),
        ToolExecutionOptions::default(),
        &keybindings,
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "one\ntwo\n".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = component.render(120).join("\n");

    assert!(rendered.contains("read README.md"));
    assert!(rendered.contains("one"));
    assert!(rendered.contains("two"));
    assert!(!rendered.contains("two\n\n"));
}

#[test]
fn built_in_write_renderer_collapses_long_preview_with_configurable_expand_hint() {
    let keybindings = KeybindingsManager::new(
        BTreeMap::from([("app.tools.expand".to_owned(), vec![KeyId::from("alt+o")])]),
        None,
    );
    let mut component = ToolExecutionComponent::new(
        "write",
        "tool-write-expand-1",
        json!({
            "path": "README.md",
            "content": "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nline 11\nline 12\n"
        }),
        ToolExecutionOptions::default(),
        &keybindings,
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "Successfully wrote 75 bytes to README.md".into(),
            }],
            details: json!({}),
            is_error: false,
        },
        false,
    );

    let rendered = component.render(120).join("\n");

    assert!(rendered.contains("line 10"));
    assert!(!rendered.contains("line 11"));
    assert!(rendered.contains("... (2 more lines, 12 total, alt+o to expand)"));
}

#[test]
fn built_in_write_renderer_expands_long_preview_when_expanded() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new(
        "write",
        "tool-write-expand-2",
        json!({
            "path": "README.md",
            "content": "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nline 11\nline 12\n"
        }),
        ToolExecutionOptions::default(),
        &keybindings,
    );

    let collapsed = component.render(120).join("\n");
    assert!(!collapsed.contains("line 11"));
    assert!(collapsed.contains("ctrl+o to expand"));

    component.set_expanded(true);
    let expanded = component.render(120).join("\n");
    assert!(expanded.contains("line 11"));
    assert!(expanded.contains("line 12"));
    assert!(!expanded.contains("more lines"));
}

#[test]
fn built_in_edit_renderer_applies_colored_intraline_diff_rendering() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new(
        "edit",
        "tool-edit-inline-1",
        json!({ "path": "README.md", "oldText": "\tbefore value", "newText": "\tafter value" }),
        ToolExecutionOptions::default(),
        &keybindings,
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "Successfully replaced 1 block(s) in README.md.".into(),
            }],
            details: json!({
                "diff": " 1 \talpha\n-2 \tbefore value\n+2 \tafter value\n 3 gamma",
                "firstChangedLine": 2,
            }),
            is_error: false,
        },
        false,
    );

    let rendered = component.render(120).join("\n");
    let plain = strip_ansi(&rendered);

    assert!(rendered.contains("\x1b[90m 1    alpha\x1b[0m"));
    assert!(rendered.contains("\x1b[31m-2    "));
    assert!(rendered.contains("\x1b[32m+2    "));
    assert!(rendered.contains("\x1b[7mbefore "));
    assert!(rendered.contains("\x1b[7mafter "));
    assert!(!rendered.contains("\x1b[7m   before"));
    assert!(plain.contains("-2    before value"));
    assert!(plain.contains("+2    after value"));
}

#[test]
fn built_in_edit_renderer_prefers_diff_details_over_success_text() {
    let keybindings = default_keybindings();
    let mut component = ToolExecutionComponent::new(
        "edit",
        "tool-edit-1",
        json!({ "path": "README.md", "oldText": "beta", "newText": "BETA" }),
        ToolExecutionOptions::default(),
        &keybindings,
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "Successfully replaced 1 block(s) in README.md.".into(),
            }],
            details: json!({
                "diff": " 1 alpha\n-2 beta\n+2 BETA\n 3 gamma",
                "firstChangedLine": 2,
            }),
            is_error: false,
        },
        false,
    );

    let rendered = component.render(120).join("\n");
    let plain = strip_ansi(&rendered);

    assert!(plain.contains("edit README.md"));
    assert!(plain.contains("-2 beta"));
    assert!(plain.contains("+2 BETA"));
    assert!(!plain.contains("Successfully replaced 1 block(s) in README.md."));
}

#[test]
fn startup_shell_can_render_tool_execution_component_in_transcript() {
    let keybindings = KeybindingsManager::new(BTreeMap::new(), None);
    let mut shell = StartupShellComponent::new(
        "Pi",
        "1.2.3",
        &keybindings,
        &PlainKeyHintStyler,
        true,
        None,
        false,
    );

    let mut component = ToolExecutionComponent::new(
        "edit",
        "tool-3",
        json!({ "path": "README.md", "oldText": "before", "newText": "after" }),
        ToolExecutionOptions::default(),
        &keybindings,
    );
    component.update_result(
        ToolExecutionResult {
            content: vec![UserContent::Text {
                text: "Successfully replaced 1 block in README.md.".into(),
            }],
            details: json!({
                "diff": " 1 alpha\n-2 beta\n+2 after\n 3 gamma",
                "firstChangedLine": 2,
            }),
            is_error: false,
        },
        false,
    );

    shell.add_transcript_item(Box::new(component));
    shell.set_pending_messages(
        &PlainKeyHintStyler,
        ["queued message"],
        std::iter::empty::<&str>(),
    );

    let mut tui = Tui::new(NoopTerminal);
    let shell_id = tui.add_child(Box::new(shell));
    assert!(tui.set_focus_child(shell_id));

    let lines = tui.render_for_size(72, 20);
    let plain_lines = lines
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>();
    let tool_line = plain_lines
        .iter()
        .position(|line| line.contains("+2 after"))
        .expect("tool execution should render diff output");
    let pending_line = plain_lines
        .iter()
        .position(|line| line.contains("Steering: queued message"))
        .expect("pending message should render");
    let prompt_start = plain_lines.len().saturating_sub(3);

    assert!(tool_line < pending_line);
    assert!(pending_line < prompt_start);
}
