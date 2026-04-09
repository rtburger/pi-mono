use pi_tui::{Component, ComponentId, Container};

pub struct TranscriptComponent {
    container: Container,
}

impl TranscriptComponent {
    pub fn new() -> Self {
        Self {
            container: Container::new(),
        }
    }

    pub fn add_item(&mut self, component: Box<dyn Component>) -> ComponentId {
        self.container.add_child(component)
    }

    pub fn remove_item(&mut self, id: ComponentId) -> bool {
        self.container.remove_child(id)
    }

    pub fn clear_items(&mut self) {
        self.container.clear();
    }

    pub fn item_count(&self) -> usize {
        self.container.child_count()
    }
}

impl Default for TranscriptComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for TranscriptComponent {
    fn render(&self, width: usize) -> Vec<String> {
        self.container.render(width)
    }

    fn invalidate(&mut self) {
        self.container.invalidate();
    }
}
