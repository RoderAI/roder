//! Bounded observed evidence carried across pages in one delegation. This is
//! page data, not model-written conclusions or instructions. No extra LLM call.
use crate::secret::Secrets;
use serde_json::Value;

#[derive(Default)]
pub(super) struct Evidence {
    records: Vec<(String, String)>,
    omitted: bool,
}
impl Evidence {
    pub(super) fn remember(&mut self, page: &Value) {
        let url = page["url"].as_str().unwrap_or_default();
        let text = page["text"].as_str().unwrap_or_default();
        if text.trim().is_empty() || url.len() > 2000 {
            return;
        }
        // Keep only new complete lines per URL; scrolling often repeats headers.
        let fresh =
            text.lines()
                .filter(|line| {
                    !line.trim().is_empty()
                        && !self.records.iter().any(|(source, old)| {
                            source == url && old.lines().any(|seen| seen == *line)
                        })
                })
                .collect::<Vec<_>>()
                .join("\n");
        if fresh.is_empty() {
            return;
        }
        if fresh.chars().count() > 6000 {
            self.omitted = true;
            return;
        }
        self.records.push((url.to_string(), fresh));
        while self
            .records
            .iter()
            .map(|(u, t)| u.chars().count() + t.chars().count() + 20)
            .sum::<usize>()
            > 8000
        {
            self.records.remove(0);
            self.omitted = true;
        }
    }
    pub(super) fn sources(&self, secrets: &Secrets) -> Vec<String> {
        let mut sources = Vec::new();
        for (url, _) in &self.records {
            let url = secrets.scrub(url);
            if !sources.contains(&url) && sources.len() < 12 {
                sources.push(url);
            }
        }
        sources
    }
    pub(super) fn render(&self, secrets: &Secrets) -> String {
        let mut out = if self.omitted {
            "Earlier evidence omitted by the bounded history. Revisit a source if a required fact is missing.\n".to_string()
        } else {
            String::new()
        };
        for (url, text) in &self.records {
            out.push_str(&format!(
                "Source: {}\n{}\n\n",
                secrets.scrub(url),
                secrets.scrub(text)
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn retains_multiple_sources_and_new_scroll_text_without_repeated_headers() {
        let mut evidence = Evidence::default();
        evidence.remember(&json!({"url":"https://a.test","text":"Menu\nPrice 25"}));
        evidence.remember(&json!({"url":"https://a.test","text":"Menu\nWeight 70"}));
        evidence.remember(&json!({"url":"https://b.test","text":"Dimensions 11 x 8 x 5"}));
        let text = evidence.render(&Secrets::default());
        assert_eq!(text.matches("Menu").count(), 1);
        for fact in ["Price 25", "Weight 70", "https://b.test", "Dimensions"] {
            assert!(text.contains(fact));
        }
    }
    #[test]
    fn bounds_history_and_scrubs_secrets_learned_after_recording() {
        let mut evidence = Evidence::default();
        for n in 0..20 {
            evidence.remember(
                &json!({"url":format!("https://a.test/{n}"),"text":"sensitive-value".repeat(200)}),
            );
        }
        let mut secrets = Secrets::default();
        secrets.remember("sensitive-value");
        let text = evidence.render(&secrets);
        assert!(text.contains("Earlier evidence omitted"));
        assert!(!text.contains("sensitive-value"));
        assert!(text.chars().count() < 8200);
    }
}
