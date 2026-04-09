use crate::{
    CellDimensions, Terminal, TuiError, extract_segments, get_capabilities, is_key_release,
    matches_key, set_cell_dimensions, slice_by_column, slice_with_width, visible_width,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT_COMPONENT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_OVERLAY_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_INPUT_LISTENER_ID: AtomicU64 = AtomicU64::new(1);

const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

type OverlayVisibility = dyn Fn(usize, usize) -> bool + Send + Sync + 'static;
type InputListener = dyn FnMut(String) -> InputListenerResult + Send + 'static;

pub trait Component {
    fn render(&self, width: usize) -> Vec<String>;
    fn invalidate(&mut self);

    fn handle_input(&mut self, _data: &str) {}

    fn wants_key_release(&self) -> bool {
        false
    }

    fn set_focused(&mut self, _focused: bool) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputListenerId(u64);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputListenerResult {
    pub consume: bool,
    pub data: Option<String>,
}

impl InputListenerResult {
    pub fn passthrough() -> Self {
        Self::default()
    }

    pub fn replace(data: impl Into<String>) -> Self {
        Self {
            consume: false,
            data: Some(data.into()),
        }
    }

    pub fn consume() -> Self {
        Self {
            consume: true,
            data: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayHandle {
    id: OverlayId,
}

impl OverlayHandle {
    pub fn id(self) -> OverlayId {
        self.id
    }

    pub fn hide<T: Terminal>(self, tui: &mut Tui<T>) -> bool {
        tui.hide_overlay_by_id(self.id)
    }

    pub fn set_hidden<T: Terminal>(self, tui: &mut Tui<T>, hidden: bool) -> bool {
        tui.set_overlay_hidden(self.id, hidden)
    }

    pub fn is_hidden<T: Terminal>(self, tui: &Tui<T>) -> bool {
        tui.is_overlay_hidden(self.id)
    }

    pub fn focus<T: Terminal>(self, tui: &mut Tui<T>) -> bool {
        tui.focus_overlay(self.id)
    }

    pub fn unfocus<T: Terminal>(self, tui: &mut Tui<T>) -> bool {
        tui.unfocus_overlay(self.id)
    }

    pub fn is_focused<T: Terminal>(self, tui: &Tui<T>) -> bool {
        tui.is_overlay_focused(self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAnchor {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverlayMargin {
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
    pub left: usize,
}

impl OverlayMargin {
    pub fn all(value: usize) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeValue {
    Absolute(usize),
    Percent(f64),
}

impl SizeValue {
    pub fn absolute(value: usize) -> Self {
        Self::Absolute(value)
    }

    pub fn percent(value: f64) -> Self {
        Self::Percent(value)
    }

    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if let Some(percent) = trimmed.strip_suffix('%') {
            return percent.parse::<f64>().ok().map(Self::Percent);
        }
        trimmed.parse::<usize>().ok().map(Self::Absolute)
    }

    fn resolve(self, reference: usize) -> usize {
        match self {
            Self::Absolute(value) => value,
            Self::Percent(percent) => ((reference as f64 * percent) / 100.0).floor() as usize,
        }
    }
}

impl From<usize> for SizeValue {
    fn from(value: usize) -> Self {
        Self::Absolute(value)
    }
}

pub struct OverlayOptions {
    pub width: Option<SizeValue>,
    pub min_width: Option<usize>,
    pub max_height: Option<SizeValue>,
    pub anchor: OverlayAnchor,
    pub offset_x: isize,
    pub offset_y: isize,
    pub row: Option<SizeValue>,
    pub col: Option<SizeValue>,
    pub margin: OverlayMargin,
    pub visible: Option<Box<OverlayVisibility>>,
    pub non_capturing: bool,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            width: None,
            min_width: None,
            max_height: None,
            anchor: OverlayAnchor::Center,
            offset_x: 0,
            offset_y: 0,
            row: None,
            col: None,
            margin: OverlayMargin::default(),
            visible: None,
            non_capturing: false,
        }
    }
}

struct ComponentEntry {
    id: ComponentId,
    component: Box<dyn Component>,
}

struct OverlayEntry {
    id: OverlayId,
    component: Box<dyn Component>,
    options: OverlayOptions,
    pre_focus: Option<FocusTarget>,
    hidden: bool,
    focus_order: u64,
}

struct InputListenerEntry {
    id: InputListenerId,
    listener: Box<InputListener>,
}

enum TerminalEvent {
    Input(String),
    Resize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FocusTarget {
    Child(ComponentId),
    Overlay(OverlayId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorPosition {
    row: usize,
    col: usize,
}

struct RenderedFrame {
    lines: Vec<String>,
    cursor_pos: Option<CursorPosition>,
}

pub struct Container {
    children: Vec<ComponentEntry>,
}

impl Container {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) -> ComponentId {
        let id = ComponentId(NEXT_COMPONENT_ID.fetch_add(1, Ordering::Relaxed));
        self.children.push(ComponentEntry { id, component });
        id
    }

    pub fn remove_child(&mut self, id: ComponentId) -> bool {
        let Some(index) = self.children.iter().position(|entry| entry.id == id) else {
            return false;
        };
        self.children.remove(index);
        true
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }

    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    fn child_mut(&mut self, id: ComponentId) -> Option<&mut (dyn Component + '_)> {
        for entry in &mut self.children {
            if entry.id == id {
                return Some(entry.component.as_mut());
            }
        }
        None
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for Container {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for child in &self.children {
            lines.extend(child.component.render(width));
        }
        lines
    }

    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.component.invalidate();
        }
    }
}

pub struct Tui<T: Terminal> {
    terminal: T,
    root: Container,
    overlays: Vec<OverlayEntry>,
    input_listeners: Vec<InputListenerEntry>,
    pending_terminal_events: Arc<Mutex<VecDeque<TerminalEvent>>>,
    on_debug: Option<Box<dyn FnMut() + Send>>,
    focus_order_counter: u64,
    focused_target: Option<FocusTarget>,
    started: bool,
    show_hardware_cursor: bool,
}

impl<T: Terminal> Tui<T> {
    pub fn new(terminal: T) -> Self {
        Self {
            terminal,
            root: Container::new(),
            overlays: Vec::new(),
            input_listeners: Vec::new(),
            pending_terminal_events: Arc::new(Mutex::new(VecDeque::new())),
            on_debug: None,
            focus_order_counter: 0,
            focused_target: None,
            started: false,
            show_hardware_cursor: matches!(std::env::var("PI_HARDWARE_CURSOR").as_deref(), Ok("1")),
        }
    }

    pub fn terminal(&self) -> &T {
        &self.terminal
    }

    pub fn terminal_mut(&mut self) -> &mut T {
        &mut self.terminal
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) -> ComponentId {
        self.root.add_child(component)
    }

    pub fn remove_child(&mut self, id: ComponentId) -> bool {
        if self.focused_target == Some(FocusTarget::Child(id)) {
            self.set_focus_target(None);
        }
        self.root.remove_child(id)
    }

    pub fn clear(&mut self) {
        self.set_focus_target(None);
        self.root.clear();
    }

    pub fn set_focus_child(&mut self, id: ComponentId) -> bool {
        self.set_focus_target(Some(FocusTarget::Child(id)))
    }

    pub fn clear_focus(&mut self) {
        self.set_focus_target(None);
    }

    pub fn is_child_focused(&self, id: ComponentId) -> bool {
        self.focused_target == Some(FocusTarget::Child(id))
    }

    pub fn add_input_listener<F>(&mut self, listener: F) -> InputListenerId
    where
        F: FnMut(String) -> InputListenerResult + Send + 'static,
    {
        let id = InputListenerId(NEXT_INPUT_LISTENER_ID.fetch_add(1, Ordering::Relaxed));
        self.input_listeners.push(InputListenerEntry {
            id,
            listener: Box::new(listener),
        });
        id
    }

    pub fn remove_input_listener(&mut self, id: InputListenerId) -> bool {
        let Some(index) = self.input_listeners.iter().position(|entry| entry.id == id) else {
            return false;
        };
        self.input_listeners.remove(index);
        true
    }

    pub fn clear_input_listeners(&mut self) {
        self.input_listeners.clear();
    }

    pub fn set_debug_handler<F>(&mut self, handler: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.on_debug = Some(Box::new(handler));
    }

    pub fn clear_debug_handler(&mut self) {
        self.on_debug = None;
    }

    pub fn show_overlay(
        &mut self,
        component: Box<dyn Component>,
        options: OverlayOptions,
    ) -> OverlayId {
        self.focus_order_counter += 1;
        let id = OverlayId(NEXT_OVERLAY_ID.fetch_add(1, Ordering::Relaxed));
        let auto_focus = !options.non_capturing && self.overlay_options_visible(&options);
        let pre_focus = self.focused_target;
        self.overlays.push(OverlayEntry {
            id,
            component,
            options,
            pre_focus,
            hidden: false,
            focus_order: self.focus_order_counter,
        });
        if auto_focus {
            self.set_focus_target(Some(FocusTarget::Overlay(id)));
        }
        id
    }

    pub fn show_overlay_handle(
        &mut self,
        component: Box<dyn Component>,
        options: OverlayOptions,
    ) -> OverlayHandle {
        OverlayHandle {
            id: self.show_overlay(component, options),
        }
    }

    pub fn hide_overlay(&mut self) -> bool {
        let Some(mut entry) = self.overlays.pop() else {
            return false;
        };
        if self.focused_target == Some(FocusTarget::Overlay(entry.id)) {
            entry.component.set_focused(false);
            self.focused_target = None;
            let restore = self
                .topmost_visible_capturing_overlay_id()
                .map(FocusTarget::Overlay)
                .or(entry.pre_focus);
            self.set_focus_target(restore);
        }
        true
    }

    pub fn hide_overlay_by_id(&mut self, id: OverlayId) -> bool {
        let Some(index) = self.overlays.iter().position(|entry| entry.id == id) else {
            return false;
        };
        let mut entry = self.overlays.remove(index);
        if self.focused_target == Some(FocusTarget::Overlay(id)) {
            entry.component.set_focused(false);
            self.focused_target = None;
            let restore = self
                .topmost_visible_capturing_overlay_id()
                .map(FocusTarget::Overlay)
                .or(entry.pre_focus);
            self.set_focus_target(restore);
        }
        true
    }

    pub fn set_overlay_hidden(&mut self, id: OverlayId, hidden: bool) -> bool {
        let Some(index) = self.overlays.iter().position(|entry| entry.id == id) else {
            return false;
        };

        if self.overlays[index].hidden == hidden {
            return true;
        }

        self.overlays[index].hidden = hidden;
        if hidden {
            if self.focused_target == Some(FocusTarget::Overlay(id)) {
                let restore = self
                    .topmost_visible_capturing_overlay_id()
                    .map(FocusTarget::Overlay)
                    .or(self.overlays[index].pre_focus);
                self.set_focus_target(restore);
            }
            return true;
        }

        if !self.overlays[index].options.non_capturing && self.is_overlay_visible_by_id(id) {
            self.focus_order_counter += 1;
            self.overlays[index].focus_order = self.focus_order_counter;
            self.set_focus_target(Some(FocusTarget::Overlay(id)));
        }
        true
    }

    pub fn focus_overlay(&mut self, id: OverlayId) -> bool {
        if !self.overlays.iter().any(|entry| entry.id == id) || !self.is_overlay_visible_by_id(id) {
            return false;
        }
        self.focus_order_counter += 1;
        if let Some(entry) = self.overlays.iter_mut().find(|entry| entry.id == id) {
            entry.focus_order = self.focus_order_counter;
        }
        self.set_focus_target(Some(FocusTarget::Overlay(id)));
        true
    }

    pub fn unfocus_overlay(&mut self, id: OverlayId) -> bool {
        if self.focused_target != Some(FocusTarget::Overlay(id)) {
            return false;
        }
        let restore = self
            .topmost_visible_capturing_overlay_id_excluding(id)
            .map(FocusTarget::Overlay)
            .or_else(|| self.overlay_pre_focus(id));
        self.set_focus_target(restore);
        true
    }

    pub fn is_overlay_focused(&self, id: OverlayId) -> bool {
        self.focused_target == Some(FocusTarget::Overlay(id))
    }

    pub fn is_overlay_hidden(&self, id: OverlayId) -> bool {
        self.overlays
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.hidden)
            .unwrap_or(false)
    }

    pub fn overlay_count(&self) -> usize {
        self.overlays.len()
    }

    pub fn has_overlay(&self) -> bool {
        self.overlays
            .iter()
            .any(|entry| self.is_overlay_visible(entry))
    }

    pub fn handle_input(&mut self, data: &str) -> Result<(), TuiError> {
        let mut current = data.to_owned();
        for entry in &mut self.input_listeners {
            let result = (entry.listener)(current.clone());
            if result.consume {
                return Ok(());
            }
            if let Some(next) = result.data {
                current = next;
            }
        }
        if current.is_empty() {
            return Ok(());
        }

        if self.consume_cell_size_response(&current)? {
            return Ok(());
        }

        if matches_key(&current, "shift+ctrl+d") {
            if let Some(on_debug) = &mut self.on_debug {
                on_debug();
                return Ok(());
            }
        }

        self.redirect_focus_from_invisible_overlay();

        let Some(target) = self.focused_target else {
            return Ok(());
        };

        let wants_key_release = self
            .component_ref(target)
            .map(|component| component.wants_key_release())
            .unwrap_or(false);
        if is_key_release(&current) && !wants_key_release {
            return Ok(());
        }

        let delivered = self
            .visit_component_mut(target, |component| component.handle_input(&current))
            .is_some();
        if delivered && self.started {
            self.render_now()?;
        }
        Ok(())
    }

    pub fn drain_terminal_events(&mut self) -> Result<(), TuiError> {
        loop {
            let next_event = self
                .pending_terminal_events
                .lock()
                .expect("pending terminal events mutex poisoned")
                .pop_front();
            let Some(event) = next_event else {
                break;
            };
            match event {
                TerminalEvent::Input(data) => self.handle_input(&data)?,
                TerminalEvent::Resize => {
                    if self.started {
                        self.render_now()?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn invalidate(&mut self) {
        self.root.invalidate();
        for overlay in &mut self.overlays {
            overlay.component.invalidate();
        }
    }

    pub fn show_hardware_cursor(&self) -> bool {
        self.show_hardware_cursor
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) -> Result<(), TuiError> {
        if self.show_hardware_cursor == enabled {
            return Ok(());
        }
        self.show_hardware_cursor = enabled;
        if !enabled {
            self.terminal.hide_cursor()?;
        }
        self.request_render(false)
    }

    pub fn render_for_size(&self, width: usize, height: usize) -> Vec<String> {
        self.render_frame_for_size(width, height).lines
    }

    pub fn render_current(&self) -> Vec<String> {
        self.render_for_size(
            self.terminal.columns() as usize,
            self.terminal.rows() as usize,
        )
    }

    fn render_frame_for_size(&self, width: usize, height: usize) -> RenderedFrame {
        let mut lines = self.root.render(width);
        if !self.overlays.is_empty() {
            lines = self.composite_overlays(lines, width, height);
        }
        let cursor_pos = self.extract_cursor_position(&mut lines, height);
        let lines = self.apply_line_resets(lines);
        RenderedFrame { lines, cursor_pos }
    }

    pub fn start(&mut self) -> Result<(), TuiError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        let pending_input = Arc::clone(&self.pending_terminal_events);
        let pending_resize = Arc::clone(&self.pending_terminal_events);
        self.terminal.start(
            Box::new(move |data| {
                pending_input
                    .lock()
                    .expect("pending terminal events mutex poisoned")
                    .push_back(TerminalEvent::Input(data));
            }),
            Box::new(move || {
                pending_resize
                    .lock()
                    .expect("pending terminal events mutex poisoned")
                    .push_back(TerminalEvent::Resize);
            }),
        )?;
        self.terminal.hide_cursor()?;
        self.query_cell_size()?;
        self.render_now()
    }

    pub fn request_render(&mut self, _force: bool) -> Result<(), TuiError> {
        if !self.started {
            return Ok(());
        }
        self.render_now()
    }

    pub fn stop(&mut self) -> Result<(), TuiError> {
        if !self.started {
            return Ok(());
        }
        self.started = false;
        self.terminal.show_cursor()?;
        self.terminal.stop()?;
        self.pending_terminal_events
            .lock()
            .expect("pending terminal events mutex poisoned")
            .clear();
        Ok(())
    }

    fn render_now(&mut self) -> Result<(), TuiError> {
        let frame = self.render_frame_for_size(
            self.terminal.columns() as usize,
            self.terminal.rows() as usize,
        );
        let mut buffer = String::from("\x1b[2J\x1b[H");
        for (index, line) in frame.lines.iter().enumerate() {
            if index > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str(line);
        }
        self.terminal.write(&buffer)?;
        self.position_hardware_cursor(frame.cursor_pos, frame.lines.len())
    }

    fn set_focus_target(&mut self, target: Option<FocusTarget>) -> bool {
        if self.focused_target == target {
            return true;
        }

        if let Some(previous) = self.focused_target {
            let _ = self.visit_component_mut(previous, |component| component.set_focused(false));
        }

        self.focused_target = None;
        if let Some(target) = target {
            if self
                .visit_component_mut(target, |component| component.set_focused(true))
                .is_some()
            {
                self.focused_target = Some(target);
                return true;
            }
            return false;
        }

        true
    }

    fn visit_component_mut<R>(
        &mut self,
        target: FocusTarget,
        mut visit: impl FnMut(&mut dyn Component) -> R,
    ) -> Option<R> {
        match target {
            FocusTarget::Child(id) => self.root.child_mut(id).map(|component| visit(component)),
            FocusTarget::Overlay(id) => self
                .overlays
                .iter_mut()
                .find(|entry| entry.id == id)
                .map(|entry| visit(entry.component.as_mut())),
        }
    }

    fn component_ref(&self, target: FocusTarget) -> Option<&(dyn Component + '_)> {
        match target {
            FocusTarget::Child(id) => {
                for entry in &self.root.children {
                    if entry.id == id {
                        return Some(entry.component.as_ref());
                    }
                }
                None
            }
            FocusTarget::Overlay(id) => {
                for entry in &self.overlays {
                    if entry.id == id {
                        return Some(entry.component.as_ref());
                    }
                }
                None
            }
        }
    }

    fn overlay_pre_focus(&self, id: OverlayId) -> Option<FocusTarget> {
        self.overlays
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.pre_focus)
    }

    fn overlay_options_visible(&self, options: &OverlayOptions) -> bool {
        let term_width = self.terminal.columns() as usize;
        let term_height = self.terminal.rows() as usize;
        if let Some(visible) = &options.visible {
            return visible(term_width, term_height);
        }
        true
    }

    fn query_cell_size(&mut self) -> Result<(), TuiError> {
        if get_capabilities().images.is_some() {
            self.terminal.write("\x1b[16t")?;
        }
        Ok(())
    }

    fn consume_cell_size_response(&mut self, data: &str) -> Result<bool, TuiError> {
        let Some(rest) = data.strip_prefix("\x1b[6;") else {
            return Ok(false);
        };
        let Some(rest) = rest.strip_suffix('t') else {
            return Ok(false);
        };
        let mut parts = rest.split(';');
        let Some(height_text) = parts.next() else {
            return Ok(false);
        };
        let Some(width_text) = parts.next() else {
            return Ok(false);
        };
        if parts.next().is_some() {
            return Ok(false);
        }
        let Ok(height_px) = height_text.parse::<u32>() else {
            return Ok(false);
        };
        let Ok(width_px) = width_text.parse::<u32>() else {
            return Ok(false);
        };
        if height_px == 0 || width_px == 0 {
            return Ok(true);
        }

        set_cell_dimensions(CellDimensions {
            width_px,
            height_px,
        });
        self.invalidate();
        if self.started {
            self.render_now()?;
        }
        Ok(true)
    }

    fn is_overlay_visible(&self, overlay: &OverlayEntry) -> bool {
        self.is_overlay_visible_in(
            overlay,
            self.terminal.columns() as usize,
            self.terminal.rows() as usize,
        )
    }

    fn is_overlay_visible_in(
        &self,
        overlay: &OverlayEntry,
        term_width: usize,
        term_height: usize,
    ) -> bool {
        if overlay.hidden {
            return false;
        }
        if let Some(visible) = &overlay.options.visible {
            return visible(term_width, term_height);
        }
        true
    }

    fn is_overlay_visible_by_id(&self, id: OverlayId) -> bool {
        self.overlays
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| self.is_overlay_visible(entry))
            .unwrap_or(false)
    }

    fn topmost_visible_capturing_overlay_id(&self) -> Option<OverlayId> {
        self.topmost_visible_capturing_overlay_id_excluding(OverlayId(0))
    }

    fn topmost_visible_capturing_overlay_id_excluding(
        &self,
        excluded: OverlayId,
    ) -> Option<OverlayId> {
        self.overlays
            .iter()
            .rev()
            .find(|entry| {
                entry.id != excluded
                    && !entry.options.non_capturing
                    && self.is_overlay_visible(entry)
            })
            .map(|entry| entry.id)
    }

    fn redirect_focus_from_invisible_overlay(&mut self) {
        let Some(FocusTarget::Overlay(id)) = self.focused_target else {
            return;
        };
        if self.is_overlay_visible_by_id(id) {
            return;
        }
        let restore = self
            .topmost_visible_capturing_overlay_id_excluding(id)
            .map(FocusTarget::Overlay)
            .or_else(|| self.overlay_pre_focus(id));
        self.set_focus_target(restore);
    }

    fn composite_overlays(
        &self,
        lines: Vec<String>,
        term_width: usize,
        term_height: usize,
    ) -> Vec<String> {
        if self.overlays.is_empty() {
            return lines;
        }

        let mut result = lines;
        let mut min_lines_needed = result.len();
        let mut rendered = Vec::new();

        let mut visible_entries = self
            .overlays
            .iter()
            .filter(|entry| self.is_overlay_visible_in(entry, term_width, term_height))
            .collect::<Vec<_>>();
        visible_entries.sort_by_key(|entry| entry.focus_order);

        for entry in visible_entries {
            let provisional =
                self.resolve_overlay_layout(&entry.options, 0, term_width, term_height);
            let mut overlay_lines = entry.component.render(provisional.width);
            if let Some(max_height) = provisional.max_height {
                overlay_lines.truncate(max_height);
            }
            let layout = self.resolve_overlay_layout(
                &entry.options,
                overlay_lines.len(),
                term_width,
                term_height,
            );
            min_lines_needed = min_lines_needed.max(layout.row + overlay_lines.len());
            rendered.push((overlay_lines, layout.row, layout.col, layout.width));
        }

        let working_height = result.len().max(term_height).max(min_lines_needed);
        while result.len() < working_height {
            result.push(String::new());
        }

        let viewport_start = working_height.saturating_sub(term_height);

        for (overlay_lines, row, col, width) in rendered {
            for (index, overlay_line) in overlay_lines.iter().enumerate() {
                let target = viewport_start + row + index;
                if target >= result.len() {
                    continue;
                }
                let truncated_overlay = if visible_width(overlay_line) > width {
                    slice_by_column(overlay_line, 0, width, true)
                } else {
                    overlay_line.clone()
                };
                result[target] = self.composite_line_at(
                    &result[target],
                    &truncated_overlay,
                    col,
                    width,
                    term_width,
                );
            }
        }

        result
    }

    fn apply_line_resets(&self, mut lines: Vec<String>) -> Vec<String> {
        for line in &mut lines {
            line.push_str(SEGMENT_RESET);
        }
        lines
    }

    fn composite_line_at(
        &self,
        base_line: &str,
        overlay_line: &str,
        start_col: usize,
        overlay_width: usize,
        total_width: usize,
    ) -> String {
        let after_start = start_col + overlay_width;
        let base = extract_segments(
            base_line,
            start_col,
            after_start,
            total_width.saturating_sub(after_start),
            true,
        );
        let overlay = slice_with_width(overlay_line, 0, overlay_width, true);

        let before_pad = start_col.saturating_sub(base.before_width);
        let overlay_pad = overlay_width.saturating_sub(overlay.width);
        let actual_before_width = start_col.max(base.before_width);
        let actual_overlay_width = overlay_width.max(overlay.width);
        let after_target = total_width.saturating_sub(actual_before_width + actual_overlay_width);
        let after_pad = after_target.saturating_sub(base.after_width);

        let result = format!(
            "{}{}{}{}{}{}{}",
            base.before,
            " ".repeat(before_pad),
            SEGMENT_RESET,
            overlay.text,
            " ".repeat(overlay_pad),
            SEGMENT_RESET,
            base.after
        ) + &" ".repeat(after_pad);

        if visible_width(&result) <= total_width {
            return result;
        }

        slice_by_column(&result, 0, total_width, true)
    }

    fn extract_cursor_position(
        &self,
        lines: &mut [String],
        height: usize,
    ) -> Option<CursorPosition> {
        let viewport_top = lines.len().saturating_sub(height);
        for row in (viewport_top..lines.len()).rev() {
            let line = lines[row].clone();
            let Some(marker_index) = line.find(CURSOR_MARKER) else {
                continue;
            };
            let col = visible_width(&line[..marker_index]);
            let mut stripped =
                String::with_capacity(line.len().saturating_sub(CURSOR_MARKER.len()));
            stripped.push_str(&line[..marker_index]);
            stripped.push_str(&line[marker_index + CURSOR_MARKER.len()..]);
            lines[row] = stripped;
            return Some(CursorPosition { row, col });
        }
        None
    }

    fn position_hardware_cursor(
        &mut self,
        cursor_pos: Option<CursorPosition>,
        total_lines: usize,
    ) -> Result<(), TuiError> {
        let Some(cursor_pos) = cursor_pos else {
            self.terminal.hide_cursor()?;
            return Ok(());
        };
        if total_lines == 0 {
            self.terminal.hide_cursor()?;
            return Ok(());
        }

        let target_row = cursor_pos.row.min(total_lines - 1);
        let target_col = cursor_pos.col;
        let current_row = total_lines - 1;

        let mut buffer = String::new();
        if target_row < current_row {
            buffer.push_str(&format!("\x1b[{}A", current_row - target_row));
        } else if target_row > current_row {
            buffer.push_str(&format!("\x1b[{}B", target_row - current_row));
        }
        buffer.push_str(&format!("\x1b[{}G", target_col + 1));
        self.terminal.write(&buffer)?;

        if self.show_hardware_cursor {
            self.terminal.show_cursor()?;
        } else {
            self.terminal.hide_cursor()?;
        }
        Ok(())
    }

    fn resolve_overlay_layout(
        &self,
        options: &OverlayOptions,
        overlay_height: usize,
        term_width: usize,
        term_height: usize,
    ) -> ResolvedOverlayLayout {
        let margin_top = options.margin.top;
        let margin_right = options.margin.right;
        let margin_bottom = options.margin.bottom;
        let margin_left = options.margin.left;

        let avail_width = term_width.saturating_sub(margin_left + margin_right).max(1);
        let avail_height = term_height
            .saturating_sub(margin_top + margin_bottom)
            .max(1);

        let mut width = options
            .width
            .map(|value| value.resolve(term_width))
            .unwrap_or_else(|| 80.min(avail_width));
        if let Some(min_width) = options.min_width {
            width = width.max(min_width);
        }
        width = width.clamp(1, avail_width);

        let max_height = options
            .max_height
            .map(|value| value.resolve(term_height).clamp(1, avail_height));
        let effective_height =
            max_height.map_or(overlay_height, |max_height| overlay_height.min(max_height));

        let mut row = if let Some(row) = options.row {
            match row {
                SizeValue::Absolute(value) => value,
                SizeValue::Percent(percent) => {
                    let max_row = avail_height.saturating_sub(effective_height);
                    margin_top + ((max_row as f64 * percent) / 100.0).floor() as usize
                }
            }
        } else {
            self.resolve_anchor_row(options.anchor, effective_height, avail_height, margin_top)
        };

        let mut col = if let Some(col) = options.col {
            match col {
                SizeValue::Absolute(value) => value,
                SizeValue::Percent(percent) => {
                    let max_col = avail_width.saturating_sub(width);
                    margin_left + ((max_col as f64 * percent) / 100.0).floor() as usize
                }
            }
        } else {
            self.resolve_anchor_col(options.anchor, width, avail_width, margin_left)
        };

        row = apply_offset(row, options.offset_y);
        col = apply_offset(col, options.offset_x);

        row = row.clamp(
            margin_top,
            term_height.saturating_sub(margin_bottom + effective_height),
        );
        col = col.clamp(margin_left, term_width.saturating_sub(margin_right + width));

        ResolvedOverlayLayout {
            width,
            row,
            col,
            max_height,
        }
    }

    fn resolve_anchor_row(
        &self,
        anchor: OverlayAnchor,
        height: usize,
        avail_height: usize,
        margin_top: usize,
    ) -> usize {
        match anchor {
            OverlayAnchor::TopLeft | OverlayAnchor::TopCenter | OverlayAnchor::TopRight => {
                margin_top
            }
            OverlayAnchor::BottomLeft
            | OverlayAnchor::BottomCenter
            | OverlayAnchor::BottomRight => margin_top + avail_height.saturating_sub(height),
            OverlayAnchor::LeftCenter | OverlayAnchor::Center | OverlayAnchor::RightCenter => {
                margin_top + avail_height.saturating_sub(height) / 2
            }
        }
    }

    fn resolve_anchor_col(
        &self,
        anchor: OverlayAnchor,
        width: usize,
        avail_width: usize,
        margin_left: usize,
    ) -> usize {
        match anchor {
            OverlayAnchor::TopLeft | OverlayAnchor::LeftCenter | OverlayAnchor::BottomLeft => {
                margin_left
            }
            OverlayAnchor::TopRight | OverlayAnchor::RightCenter | OverlayAnchor::BottomRight => {
                margin_left + avail_width.saturating_sub(width)
            }
            OverlayAnchor::TopCenter | OverlayAnchor::Center | OverlayAnchor::BottomCenter => {
                margin_left + avail_width.saturating_sub(width) / 2
            }
        }
    }
}

struct ResolvedOverlayLayout {
    width: usize,
    row: usize,
    col: usize,
    max_height: Option<usize>,
}

fn apply_offset(value: usize, offset: isize) -> usize {
    if offset.is_negative() {
        value.saturating_sub(offset.unsigned_abs())
    } else {
        value.saturating_add(offset as usize)
    }
}
