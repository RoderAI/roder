#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingPrompt {
    pub display: String,
    pub message: String,
}

impl PendingPrompt {
    pub fn new(display: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            display: display.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PromptQueue {
    items: Vec<PendingPrompt>,
}

impl PromptQueue {
    pub fn push(&mut self, prompt: PendingPrompt) {
        self.items.push(prompt);
    }

    pub fn pop_front(&mut self) -> Option<PendingPrompt> {
        if self.items.is_empty() {
            return None;
        }
        Some(self.items.remove(0))
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn displays(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|prompt| prompt.display.as_str())
    }
}

pub(super) fn queue_status(count: usize) -> String {
    if count == 1 {
        "queued 1 prompt".to_string()
    } else {
        format!("queued {count} prompts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_preserves_order_and_status() {
        let mut queue = PromptQueue::default();
        queue.push(PendingPrompt::new("first", "first message"));
        queue.push(PendingPrompt::new("second", "second message"));

        assert_eq!(queue.len(), 2);
        assert_eq!(queue_status(queue.len()), "queued 2 prompts");
        assert_eq!(queue.pop_front().unwrap().message, "first message");
        assert_eq!(queue.pop_front().unwrap().message, "second message");
        assert!(queue.pop_front().is_none());
    }
}
