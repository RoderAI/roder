//! A decision may page through bounded text/DOM and actionable targets before
//! executing anything. Original action IDs survive reindexing each page.
use serde_json::{Value, json};

pub(super) struct Pages<'a> {
    source: &'a Value,
    history: &'a [Value],
    targets: Vec<Value>,
    controls: Vec<Value>,
    offset: usize,
    text_offset: usize,
    count: usize,
    bytes: usize,
    paged: bool,
    reads: usize,
    retried: bool,
    read_history: Vec<Value>,
}
fn end(text: &str, start: usize, bytes: usize) -> usize {
    let mut end = (start + bytes).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}
fn slice(text: &str, offset: usize, bytes: usize) -> String {
    let start = end(text, 0, offset);
    text[start..end(text, start, bytes)].to_string()
}
impl<'a> Pages<'a> {
    pub fn new(source: &'a Value, history: &'a [Value]) -> Self {
        let (targets, controls) = source["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .cloned()
            .partition(|a| a.get("node").is_some());
        Self {
            source,
            history,
            targets,
            controls,
            offset: 0,
            text_offset: 0,
            count: usize::MAX,
            bytes: usize::MAX,
            paged: false,
            reads: 0,
            retried: false,
            read_history: Vec::new(),
        }
    }
    pub fn history(&self) -> Vec<Value> {
        self.history
            .iter()
            .skip(
                self.history
                    .len()
                    .saturating_sub(if self.paged { 4 } else { 10 }),
            )
            .cloned()
            .collect()
    }
    pub fn reads(&self) -> usize {
        self.reads
    }
    pub fn read_history(&self) -> &[Value] {
        &self.read_history
    }
    pub fn shrink(&mut self) -> bool {
        if !self.paged {
            self.count = 24;
            self.bytes = 4000;
            self.paged = true;
            return true;
        }
        if self.count <= 1 && self.bytes <= 256 {
            return false;
        }
        self.count = (self.count / 2).max(1);
        self.bytes = (self.bytes / 2).max(256);
        true
    }
    pub fn retry_smaller(&mut self) -> bool {
        if self.retried {
            return false;
        }
        self.retried = true;
        self.shrink()
    }
    fn text_len(&self) -> usize {
        ["text", "dom_snapshot"]
            .iter()
            .map(|key| self.source[*key].as_str().map_or(0, str::len))
            .max()
            .unwrap_or(0)
    }
    pub fn advance(&mut self, operation: &str) -> bool {
        if !self.paged {
            return false;
        }
        match operation {
            "READ_MORE_TARGETS" if self.offset + self.count < self.targets.len() => {
                self.offset += self.count
            }
            "READ_PREVIOUS_TARGETS" if self.offset > 0 => {
                self.offset = self.offset.saturating_sub(self.count)
            }
            "READ_MORE_TEXT" if self.text_offset + self.bytes < self.text_len() => {
                self.text_offset += self.bytes
            }
            "READ_PREVIOUS_TEXT" if self.text_offset > 0 => {
                self.text_offset = self.text_offset.saturating_sub(self.bytes)
            }
            _ => return false,
        }
        self.reads += 1;
        self.read_history.push(json!({"operation":operation,"targets_start":self.offset,"text_start_utf8_byte":self.text_offset}));
        true
    }
    pub fn view(&self) -> Value {
        if !self.paged {
            return self.source.clone();
        }
        let mut view = self.source.clone();
        let mut actions = self
            .targets
            .iter()
            .skip(self.offset)
            .take(self.count)
            .cloned()
            .collect::<Vec<_>>();
        actions.extend(self.controls.iter().cloned());
        for (id, label, available) in [
            (
                "read_more_targets",
                "Read next page of offered controls without interacting with the website",
                self.offset + self.count < self.targets.len(),
            ),
            (
                "read_previous_targets",
                "Read previous page of offered controls",
                self.offset > 0,
            ),
            (
                "read_more_text",
                "Read next page of page text and DOM evidence",
                self.text_offset + self.bytes < self.text_len(),
            ),
            (
                "read_previous_text",
                "Read previous page of text and DOM evidence",
                self.text_offset > 0,
            ),
        ] {
            if available {
                actions.push(json!({"id":id,"kind":"context","label":label}));
            }
        }
        view["actions"] = json!(actions);
        for key in ["text", "dom_snapshot"] {
            if let Some(text) = self.source[key].as_str() {
                view[key] = json!(slice(text, self.text_offset, self.bytes));
            }
        }
        if let Some(disabled) = view["disabled_controls"].as_array_mut() {
            disabled.truncate(self.count);
        }
        view["context_window"] = json!({"targets_start":self.offset,"targets_shown":self.count.min(self.targets.len().saturating_sub(self.offset)),
            "targets_total":self.targets.len(),"text_start_utf8_byte":self.text_offset,"text_page_bytes":self.bytes,
            "text_total_utf8_bytes":self.text_len(),
            "rule":"This is a bounded observation, not the whole page. READ_MORE_TARGETS or READ_MORE_TEXT requests more evidence without changing the site. Absence from this page is not absence from the website. Do not claim DONE without current evidence for every requirement. Only offered targets are actionable."});
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_preserve_action_identity_and_offer_navigation_only_when_available() {
        let page = json!({"text":"路".repeat(6000),"actions":(0..40).map(|n|json!({"id":format!("e{n}"),"node":n,"kind":"click"})).collect::<Vec<_>>()});
        let mut pages = Pages::new(&page, &[]);
        assert!(pages.shrink());
        let first = pages.view();
        assert_eq!(first["actions"][0]["id"], "e0");
        assert!(pages.advance("READ_MORE_TARGETS"));
        assert_eq!(pages.view()["actions"][0]["id"], "e24");
        assert!(pages.advance("READ_MORE_TEXT"));
        assert!(!pages.view()["text"].as_str().unwrap().is_empty());
        assert!(pages.advance("READ_PREVIOUS_TARGETS"));
        assert_eq!(pages.view()["actions"][0]["id"], "e0");
        assert!(!pages.advance("CLICK"));
    }
}
