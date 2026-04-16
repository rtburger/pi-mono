use crate::{Component, ComponentId, Container, visible_width};
use std::{boxed::Box as StdBox, cell::RefCell};

type BgFn = dyn Fn(&str) -> String + Send + Sync + 'static;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderCache {
    child_lines: Vec<String>,
    width: usize,
    bg_sample: Option<String>,
    lines: Vec<String>,
}

pub struct Box {
    container: Container,
    padding_x: usize,
    padding_y: usize,
    bg_fn: Option<StdBox<BgFn>>,
    cache: RefCell<Option<RenderCache>>,
}

impl Box {
    pub fn new(padding_x: usize, padding_y: usize) -> Self {
        Self {
            container: Container::new(),
            padding_x,
            padding_y,
            bg_fn: None,
            cache: RefCell::new(None),
        }
    }

    pub fn with_bg_fn<F>(padding_x: usize, padding_y: usize, bg_fn: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let mut component = Self::new(padding_x, padding_y);
        component.set_bg_fn(bg_fn);
        component
    }

    pub fn add_child(&mut self, component: StdBox<dyn Component>) -> ComponentId {
        self.invalidate_cache();
        self.container.add_child(component)
    }

    pub fn remove_child(&mut self, id: ComponentId) -> bool {
        let removed = self.container.remove_child(id);
        if removed {
            self.invalidate_cache();
        }
        removed
    }

    pub fn clear(&mut self) {
        self.container.clear();
        self.invalidate_cache();
    }

    pub fn set_bg_fn<F>(&mut self, bg_fn: F)
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.bg_fn = Some(StdBox::new(bg_fn));
        self.invalidate_cache();
    }

    pub fn clear_bg_fn(&mut self) {
        self.bg_fn = None;
        self.invalidate_cache();
    }

    fn invalidate_cache(&self) {
        *self.cache.borrow_mut() = None;
    }

    fn cache_matches(
        &self,
        width: usize,
        child_lines: &[String],
        bg_sample: Option<&String>,
    ) -> bool {
        let cache_ref = self.cache.borrow();
        let Some(cache) = cache_ref.as_ref() else {
            return false;
        };

        cache.width == width
            && cache.bg_sample.as_ref() == bg_sample
            && cache.child_lines == child_lines
    }

    fn apply_bg(&self, line: &str, width: usize) -> String {
        let visible_len = visible_width(line);
        let padded = format!("{line}{}", " ".repeat(width.saturating_sub(visible_len)));
        match &self.bg_fn {
            Some(bg_fn) => bg_fn(&padded),
            None => padded,
        }
    }
}

impl Default for Box {
    fn default() -> Self {
        Self::new(1, 1)
    }
}

impl Component for Box {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![String::new()];
        }

        let content_width = width.saturating_sub(self.padding_x * 2).max(1);
        let left_pad = " ".repeat(self.padding_x);
        let mut child_lines = Vec::new();
        for line in self.container.render(content_width) {
            child_lines.push(format!("{left_pad}{line}"));
        }

        if child_lines.is_empty() {
            return Vec::new();
        }

        let bg_sample = self.bg_fn.as_ref().map(|bg_fn| bg_fn("test"));
        if self.cache_matches(width, &child_lines, bg_sample.as_ref())
            && let Some(cache) = self.cache.borrow().as_ref()
        {
            return cache.lines.clone();
        }

        let mut lines = Vec::new();
        for _ in 0..self.padding_y {
            lines.push(self.apply_bg("", width));
        }
        for line in &child_lines {
            lines.push(self.apply_bg(line, width));
        }
        for _ in 0..self.padding_y {
            lines.push(self.apply_bg("", width));
        }

        *self.cache.borrow_mut() = Some(RenderCache {
            child_lines,
            width,
            bg_sample,
            lines: lines.clone(),
        });
        lines
    }

    fn invalidate(&mut self) {
        self.invalidate_cache();
        self.container.invalidate();
    }

    fn set_viewport_size(&self, width: usize, height: usize) {
        self.container.set_viewport_size(width, height);
    }
}
