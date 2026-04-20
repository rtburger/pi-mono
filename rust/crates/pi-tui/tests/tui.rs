use pi_tui::{
    CURSOR_MARKER, Component, Container, InputListenerResult, OverlayAnchor, OverlayMargin,
    OverlayOptions, SizeValue, Terminal, Tui, TuiError, get_cell_dimensions, set_cell_dimensions,
    visible_width,
};
use std::{
    cell::Cell,
    ffi::OsString,
    sync::{
        Arc, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct StaticComponent {
    lines: Vec<String>,
    requested_width: Option<Arc<Cell<Option<usize>>>>,
}

struct ViewportAwareComponent {
    viewport: Arc<Mutex<Option<(usize, usize)>>>,
}

struct DynamicComponent {
    lines: Arc<Mutex<Vec<String>>>,
}

impl ViewportAwareComponent {
    fn new(viewport: Arc<Mutex<Option<(usize, usize)>>>) -> Self {
        Self { viewport }
    }
}

impl DynamicComponent {
    fn new(lines: impl IntoIterator<Item = impl Into<String>>) -> (Self, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(lines.into_iter().map(Into::into).collect()));
        (
            Self {
                lines: Arc::clone(&lines),
            },
            lines,
        )
    }
}

impl StaticComponent {
    fn new(lines: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
            requested_width: None,
        }
    }

    fn with_requested_width(
        lines: impl IntoIterator<Item = impl Into<String>>,
        requested_width: Arc<Cell<Option<usize>>>,
    ) -> Self {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
            requested_width: Some(requested_width),
        }
    }
}

impl Component for StaticComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if let Some(requested_width) = &self.requested_width {
            requested_width.set(Some(width));
        }
        self.lines.clone()
    }

    fn invalidate(&mut self) {}
}

impl Component for ViewportAwareComponent {
    fn render(&self, _width: usize) -> Vec<String> {
        vec!["viewport".to_owned()]
    }

    fn invalidate(&mut self) {}

    fn set_viewport_size(&self, width: usize, height: usize) {
        *self.viewport.lock().expect("viewport mutex poisoned") = Some((width, height));
    }
}

impl Component for DynamicComponent {
    fn render(&self, _width: usize) -> Vec<String> {
        self.lines
            .lock()
            .expect("dynamic lines mutex poisoned")
            .clone()
    }

    fn invalidate(&mut self) {}
}

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct EnvVarGuard {
    _lock: MutexGuard<'static, ()>,
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: Option<&str>) -> Self {
        let lock = ENV_LOCK.lock().expect("env mutex poisoned");
        let previous = std::env::var_os(key);
        match value {
            Some(value) => {
                // SAFETY: tests serialize environment mutation through ENV_LOCK.
                unsafe { std::env::set_var(key, value) }
            }
            None => {
                // SAFETY: tests serialize environment mutation through ENV_LOCK.
                unsafe { std::env::remove_var(key) }
            }
        }
        Self {
            _lock: lock,
            key,
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: tests serialize environment mutation through ENV_LOCK.
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: tests serialize environment mutation through ENV_LOCK.
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

#[derive(Default)]
struct FocusableState {
    focused: bool,
    inputs: Vec<String>,
}

#[derive(Clone)]
struct FocusableProbe {
    state: Arc<Mutex<FocusableState>>,
}

impl FocusableProbe {
    fn focused(&self) -> bool {
        self.state
            .lock()
            .expect("focusable state mutex poisoned")
            .focused
    }

    fn inputs(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("focusable state mutex poisoned")
            .inputs
            .clone()
    }
}

struct FocusableComponent {
    lines: Vec<String>,
    state: Arc<Mutex<FocusableState>>,
    wants_key_release: bool,
}

impl FocusableComponent {
    fn new(lines: impl IntoIterator<Item = impl Into<String>>) -> (Self, FocusableProbe) {
        Self::with_key_release(lines, false)
    }

    fn with_key_release(
        lines: impl IntoIterator<Item = impl Into<String>>,
        wants_key_release: bool,
    ) -> (Self, FocusableProbe) {
        let state = Arc::new(Mutex::new(FocusableState::default()));
        (
            Self {
                lines: lines.into_iter().map(Into::into).collect(),
                state: Arc::clone(&state),
                wants_key_release,
            },
            FocusableProbe { state },
        )
    }
}

impl Component for FocusableComponent {
    fn render(&self, _width: usize) -> Vec<String> {
        self.lines.clone()
    }

    fn invalidate(&mut self) {}

    fn handle_input(&mut self, data: &str) {
        self.state
            .lock()
            .expect("focusable state mutex poisoned")
            .inputs
            .push(data.to_owned());
    }

    fn wants_key_release(&self) -> bool {
        self.wants_key_release
    }

    fn set_focused(&mut self, focused: bool) {
        self.state
            .lock()
            .expect("focusable state mutex poisoned")
            .focused = focused;
    }
}

#[derive(Clone)]
struct MockTerminal {
    state: Arc<Mutex<MockTerminalState>>,
}

#[derive(Default)]
struct MockTerminalState {
    writes: Vec<String>,
    started: usize,
    stopped: usize,
    cursor_hidden: bool,
    columns: u16,
    rows: u16,
    on_input: Option<Box<dyn FnMut(String) + Send>>,
    on_resize: Option<Box<dyn FnMut() + Send>>,
}

impl MockTerminal {
    fn new(columns: u16, rows: u16) -> Self {
        let state = MockTerminalState {
            writes: Vec::new(),
            started: 0,
            stopped: 0,
            cursor_hidden: false,
            columns,
            rows,
            on_input: None,
            on_resize: None,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn writes(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("mock terminal mutex poisoned")
            .writes
            .clone()
    }

    fn clear_writes(&self) {
        self.state
            .lock()
            .expect("mock terminal mutex poisoned")
            .writes
            .clear();
    }

    fn started(&self) -> usize {
        self.state
            .lock()
            .expect("mock terminal mutex poisoned")
            .started
    }

    fn stopped(&self) -> usize {
        self.state
            .lock()
            .expect("mock terminal mutex poisoned")
            .stopped
    }

    fn cursor_hidden(&self) -> bool {
        self.state
            .lock()
            .expect("mock terminal mutex poisoned")
            .cursor_hidden
    }

    fn send_input(&self, data: &str) {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        if let Some(handler) = state.on_input.as_mut() {
            handler(data.to_owned());
        }
    }

    fn resize(&self, columns: u16, rows: u16) {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.columns = columns;
        state.rows = rows;
        if let Some(handler) = state.on_resize.as_mut() {
            handler();
        }
    }
}

impl Terminal for MockTerminal {
    fn start(
        &mut self,
        on_input: Box<dyn FnMut(String) + Send>,
        on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.started += 1;
        state.on_input = Some(on_input);
        state.on_resize = Some(on_resize);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.stopped += 1;
        state.on_input = None;
        state.on_resize = None;
        Ok(())
    }

    fn drain_input(&mut self, _max: Duration, _idle: Duration) -> Result<(), TuiError> {
        Ok(())
    }

    fn write(&mut self, data: &str) -> Result<(), TuiError> {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.writes.push(data.to_owned());
        Ok(())
    }

    fn columns(&self) -> u16 {
        self.state
            .lock()
            .expect("mock terminal mutex poisoned")
            .columns
    }

    fn rows(&self) -> u16 {
        self.state
            .lock()
            .expect("mock terminal mutex poisoned")
            .rows
    }

    fn kitty_protocol_active(&self) -> bool {
        false
    }

    fn move_by(&mut self, _lines: i32) -> Result<(), TuiError> {
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), TuiError> {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.cursor_hidden = true;
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), TuiError> {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.cursor_hidden = false;
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

#[test]
fn render_for_size_propagates_viewport_size_to_root_children() {
    let terminal = MockTerminal::new(30, 7);
    let mut tui = Tui::new(terminal);
    let viewport = Arc::new(Mutex::new(None));
    tui.add_child(Box::new(ViewportAwareComponent::new(Arc::clone(&viewport))));

    let _ = tui.render_for_size(30, 7);

    assert_eq!(
        *viewport.lock().expect("viewport mutex poisoned"),
        Some((30, 7))
    );
}

#[test]
fn container_renders_children_in_order() {
    let mut container = Container::new();
    container.add_child(Box::new(StaticComponent::new(["one", "two"])));
    container.add_child(Box::new(StaticComponent::new(["three"])));

    assert_eq!(container.render(80), vec!["one", "two", "three"]);
}

#[test]
fn overlay_renders_when_content_is_shorter_than_terminal_height() {
    let terminal = MockTerminal::new(80, 24);
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new([
        "Line 1", "Line 2", "Line 3",
    ])));
    tui.show_overlay(
        Box::new(StaticComponent::new([
            "OVERLAY_TOP",
            "OVERLAY_MID",
            "OVERLAY_BOT",
        ])),
        OverlayOptions::default(),
    );

    let lines = tui.render_for_size(80, 24);
    let row = lines
        .iter()
        .position(|line| line.contains("OVERLAY_TOP"))
        .expect("overlay should be visible");

    assert_eq!(row, 10);
}

#[test]
fn overlay_requests_percentage_width_and_respects_min_width() {
    let requested_width = Arc::new(Cell::new(None));
    let terminal = MockTerminal::new(100, 24);
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(std::iter::empty::<String>())));
    tui.show_overlay(
        Box::new(StaticComponent::with_requested_width(
            ["test"],
            Arc::clone(&requested_width),
        )),
        OverlayOptions {
            width: Some(SizeValue::parse("10%").expect("valid percentage")),
            min_width: Some(30),
            ..OverlayOptions::default()
        },
    );

    let _ = tui.render_for_size(100, 24);
    assert_eq!(requested_width.get(), Some(30));
}

#[test]
fn anchor_margin_offset_and_absolute_positioning_are_applied() {
    let terminal = MockTerminal::new(80, 24);
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(std::iter::empty::<String>())));

    tui.show_overlay(
        Box::new(StaticComponent::new(["MARGIN"])),
        OverlayOptions {
            anchor: OverlayAnchor::TopLeft,
            width: Some(10.into()),
            margin: OverlayMargin {
                top: 2,
                left: 3,
                right: 0,
                bottom: 0,
            },
            ..OverlayOptions::default()
        },
    );

    tui.show_overlay(
        Box::new(StaticComponent::new(["OFFSET"])),
        OverlayOptions {
            anchor: OverlayAnchor::TopLeft,
            width: Some(10.into()),
            offset_x: 10,
            offset_y: 5,
            ..OverlayOptions::default()
        },
    );

    tui.show_overlay(
        Box::new(StaticComponent::new(["ABSOLUTE"])),
        OverlayOptions {
            anchor: OverlayAnchor::BottomRight,
            row: Some(SizeValue::absolute(3)),
            col: Some(SizeValue::absolute(5)),
            width: Some(10.into()),
            ..OverlayOptions::default()
        },
    );

    let lines = tui.render_for_size(80, 24);
    let margin_index = lines[2].find("MARGIN").expect("MARGIN should be visible");
    let offset_index = lines[5].find("OFFSET").expect("OFFSET should be visible");
    let absolute_index = lines[3]
        .find("ABSOLUTE")
        .expect("ABSOLUTE should be visible");

    assert_eq!(visible_width(&lines[2][..margin_index]), 3);
    assert_eq!(visible_width(&lines[5][..offset_index]), 10);
    assert_eq!(visible_width(&lines[3][..absolute_index]), 5);
}

#[test]
fn bottom_right_percentage_positioning_and_max_height_are_applied() {
    let terminal = MockTerminal::new(80, 10);
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(std::iter::empty::<String>())));

    tui.show_overlay(
        Box::new(StaticComponent::new(["BTM-RIGHT"])),
        OverlayOptions {
            anchor: OverlayAnchor::BottomRight,
            width: Some(10.into()),
            ..OverlayOptions::default()
        },
    );

    tui.show_overlay(
        Box::new(StaticComponent::new([
            "L1", "L2", "L3", "L4", "L5", "L6", "L7", "L8",
        ])),
        OverlayOptions {
            row: Some(SizeValue::parse("50%").expect("valid row percentage")),
            col: Some(SizeValue::parse("50%").expect("valid col percentage")),
            width: Some(10.into()),
            max_height: Some(SizeValue::parse("50%").expect("valid max height percentage")),
            ..OverlayOptions::default()
        },
    );

    let lines = tui.render_for_size(80, 10);
    let right_index = lines[9]
        .find("BTM-RIGHT")
        .expect("BTM-RIGHT should be visible on the last row");
    assert_eq!(visible_width(&lines[9][..right_index]), 70);

    let content = lines.join("\n");
    assert!(content.contains("L1"));
    assert!(content.contains("L5"));
    assert!(!content.contains("L6"));
}

#[test]
fn later_overlay_renders_on_top_and_hide_overlay_restores_previous_overlay() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(std::iter::empty::<String>())));

    tui.show_overlay(
        Box::new(StaticComponent::new(["FIRST-OVERLAY"])),
        OverlayOptions {
            anchor: OverlayAnchor::TopLeft,
            width: Some(20.into()),
            ..OverlayOptions::default()
        },
    );
    tui.show_overlay(
        Box::new(StaticComponent::new(["SECOND"])),
        OverlayOptions {
            anchor: OverlayAnchor::TopLeft,
            width: Some(10.into()),
            ..OverlayOptions::default()
        },
    );

    let lines = tui.render_for_size(40, 8);
    assert!(lines[0].contains("SECOND"));

    assert!(tui.hide_overlay());
    let lines = tui.render_for_size(40, 8);
    assert!(lines[0].contains("FIRST-OVERLAY"));
}

#[test]
fn overlay_lines_are_truncated_to_declared_width_and_terminal_width() {
    let terminal = MockTerminal::new(20, 6);
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new([format!(
        "\x1b[3m{}\x1b[23m",
        "X".repeat(20)
    )])));
    tui.show_overlay(
        Box::new(StaticComponent::new(["中".repeat(20)])),
        OverlayOptions {
            width: Some(15.into()),
            anchor: OverlayAnchor::TopLeft,
            ..OverlayOptions::default()
        },
    );

    let lines = tui.render_for_size(20, 6);
    assert!(visible_width(&lines[0]) <= 20);
}

#[test]
fn render_for_size_strips_cursor_marker_from_visible_output() {
    let terminal = MockTerminal::new(20, 5);
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new([
        format!("ab{CURSOR_MARKER}cd"),
        "tail".to_owned(),
    ])));

    let lines = tui.render_for_size(20, 5);
    assert!(lines[0].contains("abcd"));
    assert!(!lines[0].contains(CURSOR_MARKER));
}

#[test]
fn start_positions_cursor_at_marker_and_shows_it_when_enabled() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new([
        format!("ab{CURSOR_MARKER}cd"),
        "tail".to_owned(),
    ])));

    assert!(!tui.show_hardware_cursor());
    tui.set_show_hardware_cursor(true)
        .expect("enabling hardware cursor should succeed");
    assert!(tui.show_hardware_cursor());

    tui.start().expect("start should succeed");

    let writes = inspector.writes().join("");
    assert!(writes.contains("\x1b[?2026habcd"));
    assert!(writes.contains("\x1b[?2026l"));
    assert!(writes.contains("\x1b[1A\x1b[3G"));
    assert!(!inspector.cursor_hidden());

    tui.stop().expect("stop should succeed");
}

#[test]
fn missing_cursor_marker_keeps_cursor_hidden_when_enabled() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(["hello", "world"])));

    tui.set_show_hardware_cursor(true)
        .expect("enabling hardware cursor should succeed");
    tui.start().expect("start should succeed");

    assert!(inspector.cursor_hidden());

    tui.stop().expect("stop should succeed");
}

#[test]
fn capturing_and_non_capturing_overlays_manage_focus() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));
    assert!(editor_probe.focused());

    let (non_capturing, non_capturing_probe) = FocusableComponent::new(["NC"]);
    let non_capturing_id = tui.show_overlay(
        Box::new(non_capturing),
        OverlayOptions {
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );
    assert!(tui.is_child_focused(editor_id));
    assert!(editor_probe.focused());
    assert!(!tui.is_overlay_focused(non_capturing_id));
    assert!(!non_capturing_probe.focused());

    let (capturing, capturing_probe) = FocusableComponent::new(["CAP"]);
    let capturing_id = tui.show_overlay(Box::new(capturing), OverlayOptions::default());
    assert!(tui.is_overlay_focused(capturing_id));
    assert!(capturing_probe.focused());
    assert!(!editor_probe.focused());
    assert!(!non_capturing_probe.focused());
}

#[test]
fn focus_and_unfocus_overlay_restore_previous_child_focus() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    let (overlay, overlay_probe) = FocusableComponent::new(["OVERLAY"]);
    let overlay_id = tui.show_overlay(
        Box::new(overlay),
        OverlayOptions {
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );

    assert!(tui.focus_overlay(overlay_id));
    assert!(tui.is_overlay_focused(overlay_id));
    assert!(overlay_probe.focused());
    assert!(!editor_probe.focused());

    assert!(tui.unfocus_overlay(overlay_id));
    assert!(tui.is_child_focused(editor_id));
    assert!(editor_probe.focused());
    assert!(!overlay_probe.focused());

    assert!(tui.focus_overlay(overlay_id));
    assert!(tui.hide_overlay_by_id(overlay_id));
    assert!(tui.is_child_focused(editor_id));
    assert!(editor_probe.focused());
    assert!(!overlay_probe.focused());
}

#[test]
fn handle_input_redirects_away_from_invisible_overlay_and_skips_non_capturing() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);

    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    let (fallback, fallback_probe) = FocusableComponent::new(["FALLBACK"]);
    let fallback_id = tui.show_overlay(Box::new(fallback), OverlayOptions::default());
    assert!(tui.is_overlay_focused(fallback_id));

    let (non_capturing, non_capturing_probe) = FocusableComponent::new(["NC"]);
    let _non_capturing_id = tui.show_overlay(
        Box::new(non_capturing),
        OverlayOptions {
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );

    let visible = Arc::new(AtomicBool::new(true));
    let visible_for_overlay = Arc::clone(&visible);
    let (primary, primary_probe) = FocusableComponent::new(["PRIMARY"]);
    let primary_id = tui.show_overlay(
        Box::new(primary),
        OverlayOptions {
            visible: Some(Box::new(move |_, _| {
                visible_for_overlay.load(Ordering::Relaxed)
            })),
            ..OverlayOptions::default()
        },
    );
    assert!(tui.is_overlay_focused(primary_id));

    visible.store(false, Ordering::Relaxed);
    tui.handle_input("x").expect("input routing should succeed");

    assert_eq!(primary_probe.inputs(), Vec::<String>::new());
    assert_eq!(non_capturing_probe.inputs(), Vec::<String>::new());
    assert_eq!(fallback_probe.inputs(), vec!["x".to_owned()]);
    assert!(tui.is_overlay_focused(fallback_id));
    assert!(fallback_probe.focused());
    assert!(!editor_probe.focused());
}

#[test]
fn focus_overlay_bumps_visual_order_for_overlapping_overlays() {
    let terminal = MockTerminal::new(20, 6);
    let mut tui = Tui::new(terminal);
    let lower = tui.show_overlay(
        Box::new(StaticComponent::new(["A"])),
        OverlayOptions {
            row: Some(SizeValue::absolute(0)),
            col: Some(SizeValue::absolute(0)),
            width: Some(1.into()),
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );
    tui.show_overlay(
        Box::new(StaticComponent::new(["B"])),
        OverlayOptions {
            row: Some(SizeValue::absolute(0)),
            col: Some(SizeValue::absolute(0)),
            width: Some(1.into()),
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );

    let lines = tui.render_for_size(20, 6);
    assert!(lines[0].contains("B"));

    assert!(tui.focus_overlay(lower));
    assert!(tui.is_overlay_focused(lower));
    let lines = tui.render_for_size(20, 6);
    assert!(lines[0].contains("A"));
}

#[test]
fn non_capturing_overlay_unhide_does_not_auto_focus() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    let (overlay, overlay_probe) = FocusableComponent::new(["OVERLAY"]);
    let overlay_id = tui.show_overlay(
        Box::new(overlay),
        OverlayOptions {
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );

    assert!(tui.set_overlay_hidden(overlay_id, true));
    assert!(tui.set_overlay_hidden(overlay_id, false));
    assert!(tui.is_child_focused(editor_id));
    assert!(editor_probe.focused());
    assert!(!overlay_probe.focused());
}

#[test]
fn overlay_handle_focus_unfocus_and_hidden_state_follow_tui_overlay_controls() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    let (overlay, overlay_probe) = FocusableComponent::new(["OVERLAY"]);
    let handle = tui.show_overlay_handle(
        Box::new(overlay),
        OverlayOptions {
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );

    assert!(!handle.is_hidden(&tui));
    assert!(!handle.is_focused(&tui));
    assert!(handle.focus(&mut tui));
    assert!(handle.is_focused(&tui));
    assert!(overlay_probe.focused());
    assert!(!editor_probe.focused());

    assert!(handle.unfocus(&mut tui));
    assert!(tui.is_child_focused(editor_id));
    assert!(editor_probe.focused());
    assert!(!overlay_probe.focused());

    assert!(handle.set_hidden(&mut tui, true));
    assert!(handle.is_hidden(&tui));
    assert!(editor_probe.focused());
    assert!(!overlay_probe.focused());

    assert!(handle.set_hidden(&mut tui, false));
    assert!(!handle.is_hidden(&tui));
    assert!(editor_probe.focused());
    assert!(!handle.is_focused(&tui));
}

#[test]
fn overlay_handle_focus_on_hidden_overlay_and_after_hide_are_no_ops() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    let (overlay, overlay_probe) = FocusableComponent::new(["OVERLAY"]);
    let handle = tui.show_overlay_handle(
        Box::new(overlay),
        OverlayOptions {
            non_capturing: true,
            ..OverlayOptions::default()
        },
    );

    assert!(handle.set_hidden(&mut tui, true));
    assert!(!handle.focus(&mut tui));
    assert!(handle.is_hidden(&tui));
    assert!(editor_probe.focused());
    assert!(!overlay_probe.focused());

    assert!(handle.set_hidden(&mut tui, false));
    assert!(handle.focus(&mut tui));
    assert!(handle.hide(&mut tui));
    assert!(editor_probe.focused());
    assert!(!overlay_probe.focused());

    assert!(!handle.focus(&mut tui));
    assert!(!handle.unfocus(&mut tui));
    assert!(!handle.is_focused(&tui));
}

#[test]
fn input_listeners_transform_consume_and_can_be_removed() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    let transform_id = tui.add_input_listener(|data| {
        if data == "x" {
            InputListenerResult::replace("y")
        } else {
            InputListenerResult::passthrough()
        }
    });
    let _consume_id = tui.add_input_listener(|data| {
        if data == "stop" {
            InputListenerResult::consume()
        } else {
            InputListenerResult::passthrough()
        }
    });

    tui.handle_input("x")
        .expect("transformed input should be delivered");
    assert_eq!(editor_probe.inputs(), vec!["y".to_owned()]);

    tui.handle_input("stop")
        .expect("consumed input should succeed");
    assert_eq!(editor_probe.inputs(), vec!["y".to_owned()]);

    assert!(tui.remove_input_listener(transform_id));
    tui.handle_input("x")
        .expect("raw input should be delivered after removal");
    assert_eq!(editor_probe.inputs(), vec!["y".to_owned(), "x".to_owned()]);
}

#[test]
fn debug_handler_runs_before_focused_component() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    let debug_count = Arc::new(Mutex::new(0u32));
    let debug_count_for_handler = Arc::clone(&debug_count);
    tui.set_debug_handler(move || {
        *debug_count_for_handler
            .lock()
            .expect("debug counter mutex poisoned") += 1;
    });

    tui.handle_input("\x1b[27;6;100~")
        .expect("debug key should be handled");
    assert_eq!(
        *debug_count.lock().expect("debug counter mutex poisoned"),
        1
    );
    assert_eq!(editor_probe.inputs(), Vec::<String>::new());

    tui.clear_debug_handler();
    tui.handle_input("\x1b[27;6;100~")
        .expect("raw input should reach the focused component once debug is cleared");
    assert_eq!(editor_probe.inputs(), vec!["\x1b[27;6;100~".to_owned()]);
}

#[test]
fn cell_size_responses_are_consumed_and_escape_is_forwarded() {
    let terminal = MockTerminal::new(40, 8);
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));

    set_cell_dimensions(pi_tui::CellDimensions {
        width_px: 9,
        height_px: 18,
    });

    tui.handle_input("\x1b")
        .expect("bare escape should still be forwarded");
    tui.handle_input("\x1b[6;20;10t")
        .expect("cell size response should be consumed");
    assert_eq!(editor_probe.inputs(), vec!["\x1b".to_owned()]);
    assert_eq!(
        get_cell_dimensions(),
        pi_tui::CellDimensions {
            width_px: 10,
            height_px: 20,
        }
    );

    tui.handle_input("q")
        .expect("later user input should still be forwarded");
    assert_eq!(
        editor_probe.inputs(),
        vec!["\x1b".to_owned(), "q".to_owned()]
    );
}

#[test]
fn terminal_input_callbacks_drain_into_existing_handle_input_pipeline() {
    let terminal = MockTerminal::new(40, 8);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let (editor, editor_probe) = FocusableComponent::new(["EDITOR"]);
    let editor_id = tui.add_child(Box::new(editor));
    assert!(tui.set_focus_child(editor_id));
    tui.add_input_listener(|data| {
        if data == "x" {
            InputListenerResult::replace("y")
        } else {
            InputListenerResult::passthrough()
        }
    });

    tui.start().expect("start should succeed");
    inspector.send_input("x");
    tui.drain_terminal_events()
        .expect("queued terminal input should drain successfully");

    assert_eq!(editor_probe.inputs(), vec!["y".to_owned()]);
    tui.stop().expect("stop should succeed");
}

#[test]
fn terminal_resize_callbacks_trigger_rerender_when_drained() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(["hello", "world"])));

    tui.start().expect("start should succeed");
    let writes_before = inspector.writes().len();

    inspector.resize(30, 7);
    tui.drain_terminal_events()
        .expect("queued resize should drain successfully");

    let writes_after = inspector.writes();
    assert!(writes_after.len() > writes_before);
    assert!(
        writes_after
            .last()
            .is_some_and(|write| { write.starts_with("\x1b[?2026h\x1b[2J\x1b[H\x1b[3J") })
    );
    tui.stop().expect("stop should succeed");
}

#[test]
fn termux_height_changes_stay_on_the_differential_path() {
    let _guard = EnvVarGuard::set("TERMUX_VERSION", Some("1"));
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(
        (0..8).map(|index| format!("Line {index}")),
    )));

    tui.start().expect("start should succeed");
    let redraws_before = tui.full_redraws();
    inspector.clear_writes();

    inspector.resize(20, 8);
    tui.drain_terminal_events()
        .expect("queued termux resize should drain successfully");

    assert_eq!(tui.full_redraws(), redraws_before);
    let writes = inspector.writes().join("");
    assert!(!writes.contains("\x1b[2J"));
    assert!(!writes.contains("\x1b[3J"));

    tui.stop().expect("stop should succeed");
}

#[test]
fn start_and_request_render_write_a_full_frame_to_terminal() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(["hello", "world"])));

    tui.start().expect("start should succeed");
    tui.request_render(true)
        .expect("request_render should succeed");
    tui.stop().expect("stop should succeed");

    let writes = inspector.writes();
    assert!(
        writes
            .iter()
            .any(|write| write.starts_with("\x1b[?2026hhello"))
    );
    assert!(
        writes
            .iter()
            .any(|write| write.starts_with("\x1b[?2026h\x1b[2J\x1b[H\x1b[3Jhello"))
    );
    assert_eq!(inspector.started(), 1);
    assert_eq!(inspector.stopped(), 1);
    assert!(!inspector.cursor_hidden());
}

#[test]
fn rendered_lines_append_segment_resets_to_prevent_style_leaks() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(["\x1b[3mItalic", "Plain"])));

    tui.start().expect("start should succeed");

    let writes = inspector.writes().join("");
    assert!(writes.contains("\x1b[3mItalic\x1b[0m\x1b]8;;\x07\r\nPlain\x1b[0m\x1b]8;;\x07"));

    tui.stop().expect("stop should succeed");
}

#[test]
fn request_render_updates_only_changed_lines_without_full_clear() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let (component, lines) = DynamicComponent::new(["Header", "Working...", "Footer"]);
    tui.add_child(Box::new(component));

    tui.start().expect("start should succeed");
    inspector.clear_writes();

    *lines.lock().expect("dynamic lines mutex poisoned") = vec![
        "Header".to_owned(),
        "Working |".to_owned(),
        "Footer".to_owned(),
    ];
    tui.request_render(false)
        .expect("differential render should succeed");

    let writes = inspector.writes();
    let diff = writes.last().expect("expected a differential render write");
    assert!(diff.starts_with("\x1b[?2026h"));
    assert!(!diff.contains("\x1b[2J"));
    assert!(diff.contains("\x1b[2KWorking |"));
    assert!(!diff.contains("Header"));
    assert!(!diff.contains("Footer"));

    tui.stop().expect("stop should succeed");
}

#[test]
fn render_handle_queues_rerender_until_terminal_events_are_drained() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let (component, lines) = DynamicComponent::new(["hello", "world"]);
    tui.add_child(Box::new(component));
    let render_handle = tui.render_handle();

    tui.start().expect("start should succeed");
    let writes_before = inspector.writes().len();

    *lines.lock().expect("dynamic lines mutex poisoned") =
        vec!["hello".to_owned(), "there".to_owned()];
    render_handle.request_render();

    assert_eq!(inspector.writes().len(), writes_before);
    tui.drain_terminal_events()
        .expect("queued render request should drain successfully");
    let writes_after = inspector.writes();
    assert!(writes_after.len() > writes_before);
    assert!(
        writes_after
            .last()
            .is_some_and(|write| write.contains("there"))
    );

    tui.stop().expect("stop should succeed");
}

#[test]
fn render_handle_coalesces_multiple_pending_redraw_requests() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let (component, lines) = DynamicComponent::new(["hello", "world"]);
    tui.add_child(Box::new(component));
    let render_handle = tui.render_handle();

    tui.start().expect("start should succeed");
    inspector.clear_writes();

    *lines.lock().expect("dynamic lines mutex poisoned") =
        vec!["hello".to_owned(), "there".to_owned()];
    render_handle.request_render();
    render_handle.request_render();
    render_handle.request_render();
    tui.drain_terminal_events()
        .expect("queued render requests should drain successfully");

    let writes = inspector.writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].starts_with("\x1b[?2026h"));
    assert!(writes[0].contains("there"));

    tui.stop().expect("stop should succeed");
}

#[test]
fn showing_and_hiding_overlay_after_start_requests_rerender() {
    let terminal = MockTerminal::new(20, 6);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new(["base"])));

    tui.start().expect("start should succeed");
    inspector.clear_writes();

    tui.show_overlay(
        Box::new(StaticComponent::new(["overlay"])),
        OverlayOptions {
            anchor: OverlayAnchor::TopLeft,
            width: Some(7.into()),
            ..OverlayOptions::default()
        },
    );

    let overlay_writes = inspector.writes();
    let overlay_write = overlay_writes
        .last()
        .expect("showing an overlay should trigger a rerender");
    assert!(overlay_write.starts_with("\x1b[?2026h"));
    assert!(overlay_write.contains("overlay"));

    inspector.clear_writes();
    assert!(tui.hide_overlay());

    let hide_writes = inspector.writes();
    let hide_write = hide_writes
        .last()
        .expect("hiding an overlay should trigger a rerender");
    assert!(hide_write.starts_with("\x1b[?2026h"));
    assert!(hide_write.contains("base"));
    assert!(!hide_write.contains("overlay"));

    tui.stop().expect("stop should succeed");
}

#[test]
fn clear_on_shrink_triggers_full_redraw_and_tracks_count() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let (component, lines) = DynamicComponent::new(["Line 0", "Line 1", "Line 2", "Line 3"]);
    tui.add_child(Box::new(component));

    tui.set_clear_on_shrink(false);
    assert!(!tui.clear_on_shrink());
    tui.set_clear_on_shrink(true);
    assert!(tui.clear_on_shrink());

    tui.start().expect("start should succeed");
    assert_eq!(tui.full_redraws(), 1);
    inspector.clear_writes();

    *lines.lock().expect("dynamic lines mutex poisoned") =
        vec!["Line 0".to_owned(), "Line 1".to_owned()];
    tui.request_render(false)
        .expect("shrink render should succeed");

    assert_eq!(tui.full_redraws(), 2);
    assert!(
        inspector
            .writes()
            .last()
            .is_some_and(|write| write.starts_with("\x1b[?2026h\x1b[2J\x1b[H\x1b[3JLine 0"))
    );

    tui.stop().expect("stop should succeed");
}

#[test]
fn viewport_reset_after_shrink_allows_append_without_another_full_redraw() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    let (component, lines) = DynamicComponent::new((0..8).map(|index| format!("Line {index}")));
    tui.add_child(Box::new(component));

    tui.start().expect("start should succeed");
    assert_eq!(tui.full_redraws(), 1);
    inspector.clear_writes();

    *lines.lock().expect("dynamic lines mutex poisoned") =
        vec!["Line 0".to_owned(), "Line 1".to_owned()];
    tui.request_render(false)
        .expect("viewport-reset render should succeed");

    assert_eq!(tui.full_redraws(), 2);
    assert!(
        inspector
            .writes()
            .last()
            .is_some_and(|write| write.starts_with("\x1b[?2026h\x1b[2J\x1b[H\x1b[3JLine 0"))
    );

    inspector.clear_writes();
    *lines.lock().expect("dynamic lines mutex poisoned") = vec![
        "Line 0".to_owned(),
        "Line 1".to_owned(),
        "Line 2".to_owned(),
    ];
    tui.request_render(false)
        .expect("append render should stay differential");

    assert_eq!(tui.full_redraws(), 2);
    let append_writes = inspector.writes();
    let append_write = append_writes
        .last()
        .expect("expected a differential append write");
    assert!(append_write.starts_with("\x1b[?2026h"));
    assert!(!append_write.contains("\x1b[2J"));
    assert!(append_write.contains("\r\n\x1b[2KLine 2"));

    tui.stop().expect("stop should succeed");
}

#[test]
fn stop_moves_cursor_below_rendered_content_before_exiting() {
    let terminal = MockTerminal::new(20, 5);
    let inspector = terminal.clone();
    let mut tui = Tui::new(terminal);
    tui.add_child(Box::new(StaticComponent::new([
        format!("ab{CURSOR_MARKER}cd"),
        "tail".to_owned(),
    ])));

    tui.start().expect("start should succeed");
    inspector.clear_writes();

    tui.stop().expect("stop should succeed");

    assert_eq!(inspector.writes(), vec!["\x1b[2B\r\n".to_owned()]);
    assert_eq!(inspector.stopped(), 1);
    assert!(!inspector.cursor_hidden());
}
