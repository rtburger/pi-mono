use crate::{KeyHintStyler, KeybindingsManager, key_text, raw_key_hint};
use pi_tui::{Component, wrap_text_with_ansi};

pub trait StartupHeaderStyler: KeyHintStyler {
    fn accent_bold(&self, text: &str) -> String;
}

impl StartupHeaderStyler for crate::PlainKeyHintStyler {
    fn accent_bold(&self, text: &str) -> String {
        text.to_owned()
    }
}

pub fn build_startup_header_text(
    app_name: &str,
    version: &str,
    keybindings: &KeybindingsManager,
    styler: &impl StartupHeaderStyler,
    quiet: bool,
) -> String {
    if quiet {
        return String::new();
    }

    let logo = styler.accent_bold(app_name) + &styler.dim(&format!(" v{version}"));

    let mut instructions = Vec::new();

    push_hint(
        &mut instructions,
        binding_hint(keybindings, styler, "app.interrupt", "to interrupt"),
    );
    push_hint(
        &mut instructions,
        binding_hint(keybindings, styler, "app.clear", "to clear"),
    );
    if let Some(clear_key) = binding_key_text(keybindings, "app.clear") {
        instructions.push(raw_key_hint(
            styler,
            &format!("{clear_key} twice"),
            "to exit",
        ));
    }
    push_hint(
        &mut instructions,
        binding_hint(keybindings, styler, "app.exit", "to exit (empty)"),
    );
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "tui.editor.deleteToLineEnd",
            "to delete to end",
        ),
    );
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "app.thinking.cycle",
            "to cycle thinking level",
        ),
    );
    if let (Some(forward), Some(backward)) = (
        binding_key_text(keybindings, "app.model.cycleForward"),
        binding_key_text(keybindings, "app.model.cycleBackward"),
    ) {
        instructions.push(raw_key_hint(
            styler,
            &format!("{forward}/{backward}"),
            "to cycle models",
        ));
    }
    push_hint(
        &mut instructions,
        binding_hint(keybindings, styler, "app.model.select", "to select model"),
    );
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "app.tools.expand",
            "to toggle tool output",
        ),
    );
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "app.thinking.toggle",
            "to toggle thinking",
        ),
    );
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "app.editor.external",
            "for external editor",
        ),
    );
    instructions.push(raw_key_hint(styler, "/", "for commands"));
    instructions.push(raw_key_hint(styler, "!", "to run bash"));
    instructions.push(raw_key_hint(styler, "!!", "to run bash (no context)"));
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "app.message.followUp",
            "to queue follow-up",
        ),
    );
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "app.message.dequeue",
            "to edit all queued messages",
        ),
    );
    push_hint(
        &mut instructions,
        binding_hint(
            keybindings,
            styler,
            "app.clipboard.pasteImage",
            "to paste image",
        ),
    );

    format!("{logo}\n{}", instructions.join("\n"))
}

fn push_hint(instructions: &mut Vec<String>, hint: Option<String>) {
    if let Some(hint) = hint {
        instructions.push(hint);
    }
}

fn binding_key_text(keybindings: &KeybindingsManager, keybinding: &str) -> Option<String> {
    let text = key_text(keybindings, keybinding);
    (!text.is_empty()).then_some(text)
}

fn binding_hint(
    keybindings: &KeybindingsManager,
    styler: &impl StartupHeaderStyler,
    keybinding: &str,
    description: &str,
) -> Option<String> {
    binding_key_text(keybindings, keybinding).map(|key| raw_key_hint(styler, &key, description))
}

#[derive(Debug, Clone)]
pub struct StartupHeaderComponent {
    text: String,
}

impl StartupHeaderComponent {
    pub fn new(
        app_name: &str,
        version: &str,
        keybindings: &KeybindingsManager,
        styler: &impl StartupHeaderStyler,
        quiet: bool,
    ) -> Self {
        Self {
            text: build_startup_header_text(app_name, version, keybindings, styler, quiet),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl Component for StartupHeaderComponent {
    fn render(&self, width: usize) -> Vec<String> {
        render_wrapped_text(&self.text, width)
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone)]
pub struct BuiltInHeaderComponent {
    startup_header: StartupHeaderComponent,
    quiet: bool,
}

impl BuiltInHeaderComponent {
    pub fn new(
        app_name: &str,
        version: &str,
        keybindings: &KeybindingsManager,
        styler: &impl StartupHeaderStyler,
        quiet: bool,
    ) -> Self {
        Self {
            startup_header: StartupHeaderComponent::new(
                app_name,
                version,
                keybindings,
                styler,
                quiet,
            ),
            quiet,
        }
    }
}

impl Component for BuiltInHeaderComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let render_width = width.max(1);
        let mut lines = Vec::new();

        if !self.quiet {
            lines.push(String::new());
            lines.extend(self.startup_header.render(render_width));
            lines.push(String::new());
        }

        lines
    }

    fn invalidate(&mut self) {}
}

fn render_wrapped_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let render_width = width.max(1);
    let mut lines = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(wrap_text_with_ansi(line, render_width));
        }
    }

    lines
}
