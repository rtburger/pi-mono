# packages/coding-agent milestone 70: startup-shell prompt extension-editor integration slice

1. Files analyzed
- `packages/tui/src/editor-component.ts`
- `packages/tui/src/components/editor.ts`
- `packages/tui/test/editor.test.ts`
- `packages/coding-agent/src/modes/interactive/components/custom-editor.ts`
- `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`
- `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
- `rust/crates/pi-tui/src/editor.rs`
- `rust/crates/pi-tui/tests/editor.rs`
- `rust/crates/pi-coding-agent-tui/src/custom_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/extension_editor.rs`
- `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- `rust/crates/pi-coding-agent-tui/tests/extension_editor.rs`
- `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- `migration/packages/tui.md`
- `migration/packages/coding-agent.md`

2. Behavior inventory summary
- TypeScript `ExtensionEditorComponent` owns the external-editor lifecycle itself: resolve `VISUAL`/`EDITOR`, write the current editor content to a temp file, stop the TUI, run the editor, reload the edited file on success, restart the TUI, and force a rerender.
- The Rust `ExtensionEditorComponent` already had the default external-editor round-trip, but `StartupShellComponent` was not yet carrying external-editor command/runner/host configuration into the prompt-level extension-editor flow.
- That meant the shell-level `app.editor.external` path could open the prompt extension editor, but the embedded external-editor behavior was not configurable from the startup shell boundary.
- The highest-value shell-level parity gap was therefore prompt-extension-editor integration, not more standalone editor work.

3. Rust design summary
- Keep the external-editor process/lifecycle behavior in `pi-coding-agent-tui`, matching the TypeScript ownership boundary.
- Extend `ExtensionEditorComponent` with Arc-based setter variants so a parent shell can hold shared runner/host configuration and inject it into a newly created extension editor.
- Extend `StartupShellComponent` with stored prompt-extension-editor configuration:
  - external editor command override
  - external editor runner override
  - external editor host override
- Apply those overrides whenever the shell creates an `ExtensionEditorComponent`, including the default prompt external-editor flow triggered by `app.editor.external`.
- Freeze the behavior with startup-shell tests instead of widening the interactive runtime wiring further in the same milestone.

4. Files created/modified
- Modified: `rust/crates/pi-coding-agent-tui/src/extension_editor.rs`
- Modified: `rust/crates/pi-coding-agent-tui/src/startup_shell.rs`
- Modified: `rust/crates/pi-coding-agent-tui/tests/startup_shell.rs`
- Created: `migration/packages/coding-agent-milestone-70.md`

5. Tests added
- `startup_shell_app_editor_external_opens_prompt_extension_editor_and_restores_edited_prompt`
- `startup_shell_registered_external_editor_action_overrides_default_prompt_editor`

6. Validation results
- `cd rust && cargo test -p pi-coding-agent-tui --test startup_shell` passed
- `cd rust && cargo test -p pi-coding-agent-tui --test extension_editor` passed
- `cd rust && cargo fmt --all` passed
- `cd rust && cargo test -q --workspace` passed
- `npm run check` passed

7. Open questions
- The shell now propagates external-editor command/runner/host configuration, but the real interactive runner still does not install a concrete host that stops and restarts the live `Tui` around the external editor.
- If that runner-level lifecycle wiring is done next, it may require either a small TUI lifecycle handle or a narrowly scoped host adapter in the CLI interactive path.
- The multiline editor still intentionally trails full TypeScript parity in autocomplete, paste markers, and some sticky-column edge cases.

8. Recommended next step
- Stay in `packages/coding-agent/src/modes/interactive`, `rust/crates/pi-coding-agent-tui`, and `rust/crates/pi-coding-agent-cli`.
- Wire a real external-editor host into the live interactive runner so the shell-level prompt extension editor uses the same stop/start/rerender lifecycle as the TypeScript TUI path.
- Keep broader editor parity and session/resource-loader work deferred until that real interactive lifecycle path is closed.
