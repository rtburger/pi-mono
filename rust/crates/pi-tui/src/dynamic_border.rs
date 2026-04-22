use crate::Component;

pub type BorderColorFn = dyn Fn(&str) -> String + Send + Sync + 'static;

pub struct DynamicBorder {
    color_fn: Box<BorderColorFn>,
}

impl DynamicBorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_color_fn<F>(color_fn: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            color_fn: Box::new(color_fn),
        }
    }

    pub fn set_color_fn<F>(&mut self, color_fn: F)
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.color_fn = Box::new(color_fn);
    }

    pub fn clear_color_fn(&mut self) {
        self.color_fn = Box::new(str::to_owned);
    }
}

impl Default for DynamicBorder {
    fn default() -> Self {
        Self::with_color_fn(str::to_owned)
    }
}

impl Component for DynamicBorder {
    fn render(&self, width: usize) -> Vec<String> {
        let width = width.max(1);
        vec![(self.color_fn)(&"─".repeat(width))]
    }

    fn invalidate(&mut self) {}
}
