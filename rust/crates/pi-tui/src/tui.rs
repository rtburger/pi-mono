use crate::{
    Terminal, TuiError, extract_segments, slice_by_column, slice_with_width, visible_width,
};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_COMPONENT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_OVERLAY_ID: AtomicU64 = AtomicU64::new(1);

const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

type OverlayVisibility = dyn Fn(usize, usize) -> bool + Send + Sync + 'static;

pub trait Component {
    fn render(&self, width: usize) -> Vec<String>;
    fn invalidate(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComponentId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayId(u64);

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
    hidden: bool,
    focus_order: u64,
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
    focus_order_counter: u64,
    started: bool,
}

impl<T: Terminal> Tui<T> {
    pub fn new(terminal: T) -> Self {
        Self {
            terminal,
            root: Container::new(),
            overlays: Vec::new(),
            focus_order_counter: 0,
            started: false,
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
        self.root.remove_child(id)
    }

    pub fn clear(&mut self) {
        self.root.clear();
    }

    pub fn show_overlay(
        &mut self,
        component: Box<dyn Component>,
        options: OverlayOptions,
    ) -> OverlayId {
        self.focus_order_counter += 1;
        let id = OverlayId(NEXT_OVERLAY_ID.fetch_add(1, Ordering::Relaxed));
        self.overlays.push(OverlayEntry {
            id,
            component,
            options,
            hidden: false,
            focus_order: self.focus_order_counter,
        });
        id
    }

    pub fn hide_overlay(&mut self) -> bool {
        self.overlays.pop().is_some()
    }

    pub fn hide_overlay_by_id(&mut self, id: OverlayId) -> bool {
        let Some(index) = self.overlays.iter().position(|entry| entry.id == id) else {
            return false;
        };
        self.overlays.remove(index);
        true
    }

    pub fn set_overlay_hidden(&mut self, id: OverlayId, hidden: bool) -> bool {
        let Some(entry) = self.overlays.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        entry.hidden = hidden;
        true
    }

    pub fn overlay_count(&self) -> usize {
        self.overlays.len()
    }

    pub fn invalidate(&mut self) {
        self.root.invalidate();
        for overlay in &mut self.overlays {
            overlay.component.invalidate();
        }
    }

    pub fn render_for_size(&self, width: usize, height: usize) -> Vec<String> {
        let mut lines = self.root.render(width);
        if !self.overlays.is_empty() {
            lines = self.composite_overlays(lines, width, height);
        }
        self.apply_line_resets(lines)
    }

    pub fn render_current(&self) -> Vec<String> {
        self.render_for_size(
            self.terminal.columns() as usize,
            self.terminal.rows() as usize,
        )
    }

    pub fn start(&mut self) -> Result<(), TuiError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        self.terminal.start(Box::new(|_| {}), Box::new(|| {}))?;
        self.terminal.hide_cursor()?;
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
        Ok(())
    }

    fn render_now(&mut self) -> Result<(), TuiError> {
        let lines = self.render_current();
        let mut buffer = String::from("\x1b[2J\x1b[H");
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str(line);
        }
        self.terminal.write(&buffer)
    }

    fn is_overlay_visible(
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
            .filter(|entry| self.is_overlay_visible(entry, term_width, term_height))
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
