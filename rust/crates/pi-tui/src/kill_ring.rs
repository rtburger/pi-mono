#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct KillRing {
    ring: Vec<String>,
}

impl KillRing {
    pub(crate) fn push(&mut self, text: impl AsRef<str>, prepend: bool, accumulate: bool) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }

        if accumulate {
            if let Some(last) = self.ring.pop() {
                let merged = if prepend {
                    format!("{text}{last}")
                } else {
                    format!("{last}{text}")
                };
                self.ring.push(merged);
                return;
            }
        }

        self.ring.push(text.to_owned());
    }

    pub(crate) fn peek(&self) -> Option<&str> {
        self.ring.last().map(String::as_str)
    }

    pub(crate) fn rotate(&mut self) {
        if self.ring.len() > 1 {
            let last = self.ring.pop().expect("kill ring should contain an entry");
            self.ring.insert(0, last);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.ring.len()
    }
}
