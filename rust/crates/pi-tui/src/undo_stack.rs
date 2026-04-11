#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UndoStack<S> {
    stack: Vec<S>,
}

impl<S> Default for UndoStack<S> {
    fn default() -> Self {
        Self { stack: Vec::new() }
    }
}

impl<S: Clone> UndoStack<S> {
    pub(crate) fn push(&mut self, state: &S) {
        self.stack.push(state.clone());
    }
}

impl<S> UndoStack<S> {
    pub(crate) fn pop(&mut self) -> Option<S> {
        self.stack.pop()
    }

    pub(crate) fn clear(&mut self) {
        self.stack.clear();
    }
}
