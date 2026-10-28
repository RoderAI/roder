//! Reading the page: the look script's result, and the text the model reads.

use serde_json::{Value, json};

use super::client::TabClient;
use super::guard::{DirectGuard, PageFacts};
use super::session::DirectSession;
use crate::observed_render::Budget;

pub(crate) const DIRECT_JS: &str = include_str!("direct.js");
const SNAPSHOT_JS: &str = include_str!("snapshot.js");

/// Bound tree and page-text output, leaving room for the action outcome.
pub(super) const LOOK_LINES: usize = Budget::RESULT.lines - 10;

/// How much one read lists.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Detail {
    pub(crate) elements: usize,
    pub(crate) text: usize,
    publish: bool,
    full: bool,
}

impl Detail {
    /// `jev_tab_look` and its kind: the page as a person would survey it.
    pub(crate) const FULL: Self = Self {
        elements: 120,
        text: 3000,
        publish: true,
        full: true,
    };
    /// A private outcome probe. Its coverage matches the post-action read so
    /// changing excerpt limits cannot manufacture an observed page change.
    pub(crate) const BRIEF: Self = Self {
        elements: 120,
        text: 1200,
        publish: false,
        full: false,
    };
    /// Return changes since the last model-visible read, retaining full control
    /// coverage and a brief text excerpt. Private probes must not advance it.
    pub(crate) const AFTER: Self = Self {
        publish: true,
        full: false,
        ..Self::BRIEF
    };
}

/// Run the look script (installing it first, once per document) and scrub
/// the owner's secrets from what it read.
pub(crate) async fn read(
    client: &mut TabClient,
    guard: &dyn DirectGuard,
    detail: Detail,
) -> anyhow::Result<Value> {
    let expression = format!(
        "{DIRECT_JS}; {SNAPSHOT_JS}; (() => {{ const page = window.__roderDirect.look({}); \
         if ({}) page.snapshot = window.__roderDirect.snapshot(page, {}); return page; }})()",
        json!({"max": detail.elements, "text": detail.text}),
        detail.publish,
        json!({"publish": detail.publish, "full": detail.full}),
    );
    let mut look = client.evaluate_isolated(&expression).await?;
    scrub_strings(&mut look, guard);
    Ok(look)
}

impl DirectSession {
    /// The page as it is now, briefly, for a caller to compare a later look
    /// with. `None` when the page is outside the owner's scope (nothing is
    /// read from it) or cannot be read.
    pub(crate) async fn brief_look(&mut self) -> Option<Value> {
        let url = self.client.evaluate_isolated("location.href").await.ok()?;
        if url
            .as_str()
            .and_then(|url| self.guard.outside(url))
            .is_some()
        {
            return None;
        }
        read(&mut self.client, self.guard.as_ref(), Detail::BRIEF)
            .await
            .ok()
    }
}

/// Call one helper of the page script, installing it first.
pub(crate) async fn helper(client: &mut TabClient, call: &str) -> anyhow::Result<Value> {
    client
        .evaluate_isolated(&format!("{DIRECT_JS}; window.__roderDirect.{call}"))
        .await
}

fn scrub_strings(value: &mut Value, guard: &dyn DirectGuard) {
    match value {
        Value::String(text) => *text = guard.scrub(text),
        Value::Array(items) => items.iter_mut().for_each(|item| scrub_strings(item, guard)),
        Value::Object(map) => map.values_mut().for_each(|item| scrub_strings(item, guard)),
        _ => {}
    }
}

/// The facts a guard checks, from a look.
pub(crate) fn facts(look: &Value) -> PageFacts {
    PageFacts {
        url: look["url"].as_str().unwrap_or_default().to_string(),
        title: look["title"].as_str().unwrap_or_default().to_string(),
        text: look["text"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(600)
            .collect(),
        http_status: look["http_status"].as_u64().map(|code| code as u16),
        controls: look["elements"].as_array().map_or(0, |elements| {
            elements
                .iter()
                .filter(|element| element["kind"] != "graphic")
                .count()
        }),
    }
}

pub(crate) use super::look_render::render;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_scrubbed_from_current_and_removed_rows() {
        struct Hides;
        impl DirectGuard for Hides {
            fn scrub(&self, text: &str) -> String {
                text.replace("hunter22", "[secret]")
            }
        }
        let mut look = json!({"url":"https://a.test/?p=hunter22", "snapshot": {
            "rows":[{"label":"hunter22"}], "removed":[{"value":"hunter22"}]
        }});
        scrub_strings(&mut look, &Hides);
        assert!(!look.to_string().contains("hunter22"));
        assert_eq!(look["snapshot"]["removed"][0]["value"], "[secret]");
    }
}
