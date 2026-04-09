use pi_tui::{
    Component, Container, OverlayAnchor, OverlayMargin, OverlayOptions, SizeValue, Terminal, Tui,
    TuiError, visible_width,
};
use std::{
    cell::Cell,
    sync::{Arc, Mutex},
    time::Duration,
};

struct StaticComponent {
    lines: Vec<String>,
    requested_width: Option<Arc<Cell<Option<usize>>>>,
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
}

impl Terminal for MockTerminal {
    fn start(
        &mut self,
        _on_input: Box<dyn FnMut(String) + Send>,
        _on_resize: Box<dyn FnMut() + Send>,
    ) -> Result<(), TuiError> {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.started += 1;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), TuiError> {
        let mut state = self.state.lock().expect("mock terminal mutex poisoned");
        state.stopped += 1;
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
            .any(|write| write.starts_with("\x1b[2J\x1b[Hhello"))
    );
    assert_eq!(inspector.started(), 1);
    assert_eq!(inspector.stopped(), 1);
    assert!(!inspector.cursor_hidden());
}
