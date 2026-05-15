use serde_json::json;

#[derive(Debug, Clone)]
pub struct BoundedTranscript {
    max_chars: usize,
    used_chars: usize,
    entries: Vec<serde_json::Value>,
    truncated: bool,
}

impl BoundedTranscript {
    pub fn new(max_chars: usize) -> Self {
        Self {
            max_chars,
            used_chars: 0,
            entries: Vec::new(),
            truncated: false,
        }
    }

    pub fn push_text(&mut self, role: impl Into<String>, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() || self.max_chars == 0 {
            return;
        }
        let remaining = self.max_chars.saturating_sub(self.used_chars);
        if remaining == 0 {
            self.truncated = true;
            return;
        }
        let (text, was_truncated) = truncate_text_with_flag(&text, remaining);
        self.used_chars += text.chars().count();
        self.truncated |= was_truncated;
        self.entries.push(json!({
            "role": role.into(),
            "text": text,
        }));
    }

    pub fn push_tool_call(&mut self, id: impl Into<String>, name: impl Into<String>) {
        if self.max_chars == 0 {
            return;
        }
        self.entries.push(json!({
            "role": "tool_call",
            "id": id.into(),
            "name": name.into(),
        }));
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "entries": self.entries,
            "truncated": self.truncated,
            "max_result_chars": self.max_chars,
        })
    }
}

pub fn truncate_text(text: &str, max_chars: usize) -> String {
    truncate_text_with_flag(text, max_chars).0
}

fn truncate_text_with_flag(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_string(), false);
    }
    if max_chars == 0 {
        return (String::new(), true);
    }
    let marker = "...[truncated]";
    if max_chars <= marker.chars().count() {
        return (text.chars().take(max_chars).collect(), true);
    }
    let keep = max_chars - marker.chars().count();
    let mut out = text.chars().take(keep).collect::<String>();
    out.push_str(marker);
    (out, true)
}
