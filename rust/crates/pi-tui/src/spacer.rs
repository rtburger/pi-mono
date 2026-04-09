use crate::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spacer {
    lines: usize,
}

impl Spacer {
    pub fn new(lines: usize) -> Self {
        Self { lines }
    }

    pub fn set_lines(&mut self, lines: usize) {
        self.lines = lines;
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Component for Spacer {
    fn render(&self, _width: usize) -> Vec<String> {
        (0..self.lines).map(|_| String::new()).collect()
    }

    fn invalidate(&mut self) {}
}
