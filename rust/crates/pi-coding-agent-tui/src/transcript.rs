use pi_tui::{Component, ComponentId, Container};
use std::cell::Cell;

pub struct TranscriptComponent {
    container: Container,
    scroll_offset: usize,
    viewport_height: Cell<Option<usize>>,
    last_width: Cell<Option<usize>>,
}

impl TranscriptComponent {
    pub fn new() -> Self {
        Self {
            container: Container::new(),
            scroll_offset: 0,
            viewport_height: Cell::new(None),
            last_width: Cell::new(None),
        }
    }

    pub fn add_item(&mut self, component: Box<dyn Component>) -> ComponentId {
        if self.scroll_offset > 0
            && let Some(width) = self.last_width.get()
        {
            self.scroll_offset = self
                .scroll_offset
                .saturating_add(component.render(width).len());
        }
        self.container.add_child(component)
    }

    pub fn remove_item(&mut self, id: ComponentId) -> bool {
        self.container.remove_child(id)
    }

    pub fn clear_items(&mut self) {
        self.container.clear();
        self.scroll_offset = 0;
    }

    pub fn item_count(&self) -> usize {
        self.container.child_count()
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn set_viewport_height(&self, height: Option<usize>) {
        self.viewport_height.set(height);
    }
}

impl Default for TranscriptComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for TranscriptComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.last_width.set(Some(width));
        let lines = self.container.render(width);
        let Some(viewport_height) = self.viewport_height.get() else {
            return lines;
        };
        if viewport_height == 0 {
            return Vec::new();
        }

        let max_scroll = lines.len().saturating_sub(viewport_height);
        let scroll_offset = self.scroll_offset.min(max_scroll);
        let end = lines.len().saturating_sub(scroll_offset);
        let start = end.saturating_sub(viewport_height);
        lines[start..end].to_vec()
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.last_width.set(Some(width));
        self.viewport_height.set(Some(height));
        self.container.set_viewport_size(width, height);
    }
}
