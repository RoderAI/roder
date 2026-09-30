//! Caller-defined UI postconditions, independent of the decision model's DONE.
use crate::engine::{JevRunResult, JevStatus, JevStopCause};
use crate::fallback::FinalPage;
use anyhow::{Context, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Completion {
    #[serde(default)]
    url_contains: String,
    #[serde(default)]
    text_contains: String,
}
impl Completion {
    pub(crate) fn parse(args: &Value) -> anyhow::Result<Option<Self>> {
        let Some(raw) = args.get("success_condition").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let condition: Self = serde_json::from_value(raw.clone()).context(
            "success_condition must contain only string url_contains/text_contains fields",
        )?;
        if condition.url_contains.is_empty() && condition.text_contains.is_empty() {
            return Ok(None);
        }
        ensure!(
            condition.url_contains.len() <= 4096 && condition.text_contains.len() <= 4096,
            "success_condition strings must be at most 4096 bytes"
        );
        Ok(Some(condition))
    }
    pub(crate) fn matches(&self, page: &FinalPage) -> bool {
        page.url.contains(&self.url_contains) && page.visible_text.contains(&self.text_contains)
    }
    pub(crate) fn report(&self, page: Option<&FinalPage>) -> Value {
        json!({"status": if page.is_some_and(|page|self.matches(page)) {"passed"} else {"failed"},
          "source":"fresh browser observation", "url_contains":self.url_contains, "text_contains":self.text_contains,
          "observation_available":page.is_some(), "scope":"Only the caller-specified URL/text predicates; not a proof of the entire natural-language goal."})
    }
}
pub(crate) fn apply(result: &mut JevRunResult, condition: &Completion, page: Option<&FinalPage>) {
    if let Some(page) = page {
        result.url = page.url.clone();
        result.title = page.title.clone();
        result.visible_text = page.visible_text.clone();
        result.controls = serde_json::from_value(page.controls.clone()).unwrap_or_default();
        result.page = serde_json::from_value(page.page.clone()).ok();
        result.observed_elements = page.observed_elements;
    }
    if !page.is_some_and(|page| condition.matches(page)) {
        result.status = JevStatus::Blocked;
        result.stop_cause = Some(JevStopCause::OutcomeMismatch);
        result.stopped_because=Some("Model reported DONE, but a fresh browser observation did not satisfy the caller's success_condition".into());
    }
}
