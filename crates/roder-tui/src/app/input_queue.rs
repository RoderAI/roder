use roder_api::transcript::InputImage;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingPrompt {
    pub display: String,
    pub message: String,
    pub images: Vec<InputImage>,
}

impl PendingPrompt {
    pub fn with_images(
        display: impl Into<String>,
        message: impl Into<String>,
        images: Vec<InputImage>,
    ) -> Self {
        Self {
            display: display.into(),
            message: message.into(),
            images,
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

    pub fn insert(&mut self, index: usize, prompt: PendingPrompt) {
        self.items.insert(index.min(self.items.len()), prompt);
    }

    pub fn pop_front(&mut self) -> Option<PendingPrompt> {
        if self.items.is_empty() {
            return None;
        }
        Some(self.items.remove(0))
    }

    pub fn pop_back(&mut self) -> Option<PendingPrompt> {
        self.items.pop()
    }

    pub fn remove(&mut self, index: usize) -> Option<PendingPrompt> {
        if index >= self.items.len() {
            return None;
        }
        Some(self.items.remove(index))
    }

    pub fn clear(&mut self) -> usize {
        let count = self.items.len();
        self.items.clear();
        count
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PendingPrompt> {
        self.items.iter()
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
        queue.push(PendingPrompt::with_images(
            "first",
            "first message",
            Vec::new(),
        ));
        queue.push(PendingPrompt::with_images(
            "second",
            "second message",
            Vec::new(),
        ));

        assert_eq!(queue.len(), 2);
        assert_eq!(queue_status(queue.len()), "queued 2 prompts");
        assert_eq!(queue.pop_front().unwrap().message, "first message");
        assert_eq!(queue.pop_front().unwrap().message, "second message");
        assert!(queue.pop_front().is_none());
    }

    #[test]
    fn queue_allows_editing_latest_prompt_first() {
        let mut queue = PromptQueue::default();
        queue.push(PendingPrompt::with_images(
            "first",
            "first message",
            Vec::new(),
        ));
        queue.push(PendingPrompt::with_images(
            "second",
            "second message",
            Vec::new(),
        ));

        assert_eq!(queue.pop_back().unwrap().message, "second message");
        assert_eq!(queue.pop_front().unwrap().message, "first message");
    }

    #[test]
    fn queue_removes_prompt_by_index() {
        let mut queue = PromptQueue::default();
        queue.push(PendingPrompt::with_images(
            "first",
            "first message",
            Vec::new(),
        ));
        queue.push(PendingPrompt::with_images(
            "second",
            "second message",
            Vec::new(),
        ));

        assert_eq!(queue.remove(0).unwrap().message, "first message");
        assert_eq!(queue.pop_front().unwrap().message, "second message");
        assert!(queue.remove(7).is_none());
    }

    #[test]
    fn queue_clear_returns_removed_count() {
        let mut queue = PromptQueue::default();
        queue.push(PendingPrompt::with_images("first", "first", Vec::new()));
        queue.push(PendingPrompt::with_images("second", "second", Vec::new()));

        assert_eq!(queue.clear(), 2);
        assert_eq!(queue.len(), 0);
        assert!(queue.pop_front().is_none());
    }
}
