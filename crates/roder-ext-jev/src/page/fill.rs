//! Typing a value into a field, once the press has landed.
//!
//! Jev's own, diverging from upstream, which pressed the platform's
//! select-all accelerator and inserted the text wherever focus happened to
//! be. Here the field that should take the text is found first (following
//! focus to an editor the click opened over the field), focused and
//! selected by script, and the text is checked to have stayed. A native date
//! or time input gets its ISO value through the value setter instead of
//! keys. A fill whose text did not stay is typed once more only when focus
//! has since moved to another field; otherwise it is a refused outcome the
//! loop records, not an error that ends the run. For a password or
//! one-time-code field the check runs inside the page and returns only
//! whether the text stayed, so neither the typed text nor what the field
//! shows comes back. The rules follow fastbrowse's browser/page.py (MIT).

use serde_json::{Value, json};

use super::Page;
use super::act::Point;
use crate::engine::JevActOutcome;

pub(super) const FILL_JS: &str = include_str!("../assets/fill.js");

/// How much of what a field shows a refusal quotes.
const SHOWN_CHARS: usize = 100;

impl Page {
    /// Fill the field clicked at `point` with `text`. `settle` gets the node
    /// that took the text, so the settle waits on its suggestions.
    pub(super) async fn fill(
        &mut self,
        action: &Value,
        text: &str,
        point: Point,
        settle: &mut Value,
    ) -> anyhow::Result<JevActOutcome> {
        let clicked = action["node"].as_i64().unwrap_or_default();
        let Some((mut node, date)) = self.handoff(clicked, point).await? else {
            return Ok(JevActOutcome::refused(
                "The clicked field went away and nothing took its place; nothing was typed.",
            ));
        };
        settle["node"] = json!(node);
        let secret = crate::secret::is_secret(action);
        if date && !secret {
            return self.fill_date(node, text).await;
        }
        let mut retyped = false;
        loop {
            if !self.focus(node).await? {
                return Ok(JevActOutcome::refused(
                    "The field did not take keyboard focus; nothing was typed.",
                ));
            }
            self.call("Input.insertText", json!({"text": text})).await?;
            let landed = self
                .evaluate(&format!(
                    "({FILL_JS}).landed({})",
                    json!({"node": node, "text": text, "x": point.x, "y": point.y,
                        "secret": secret})
                ))
                .await?;
            if landed[0] == json!(true) {
                return Ok(JevActOutcome::done());
            }
            // An editor that took focus after the hand-off looked never got
            // the text; type into it once, as a person would on seeing it.
            if !retyped
                && let Some((handed, _)) = self.handoff(node, point).await?
                && handed != node
            {
                node = handed;
                settle["node"] = json!(node);
                retyped = true;
                continue;
            }
            if secret {
                // Checked inside the page: what a secret field holds is never read.
                return Ok(JevActOutcome::refused(
                    "The field did not keep the typed text (a secret field: what it holds is \
                     not read).",
                ));
            }
            return Ok(JevActOutcome::refused(match landed[1].as_str() {
                Some(shown) => format!(
                    "The field shows {:?}, not the typed text.",
                    shown.chars().take(SHOWN_CHARS).collect::<String>()
                ),
                None => "The field went away while the text was typed.".to_string(),
            }));
        }
    }

    /// The field focus went to after the click, and whether it is a native
    /// date or time input; `None` when the clicked one is gone and nothing
    /// replaced it.
    async fn handoff(&mut self, node: i64, point: Point) -> anyhow::Result<Option<(i64, bool)>> {
        let found = self
            .evaluate_async(&format!(
                "({FILL_JS}).handoff({})",
                json!({"node": node, "x": point.x, "y": point.y})
            ))
            .await?;
        Ok(found[0]
            .as_i64()
            .map(|node| (node, found[1] == json!(true))))
    }

    async fn focus(&mut self, node: i64) -> anyhow::Result<bool> {
        let focused = self.evaluate(&format!("({FILL_JS}).focus({node})")).await?;
        Ok(focused == json!(true))
    }

    async fn fill_date(&mut self, node: i64, value: &str) -> anyhow::Result<JevActOutcome> {
        let kept = self
            .evaluate(&format!(
                "({FILL_JS}).date({})",
                json!({"node": node, "value": value})
            ))
            .await?;
        Ok(match (kept[0].as_str(), kept[1] == json!(true)) {
            (Some(_), true) => JevActOutcome::done(),
            (Some(kept), false) => JevActOutcome::refused(format!(
                "The date field kept {kept:?}, not {value:?}; it takes an ISO value \
                 (2026-09-25 for a date)."
            )),
            (None, _) => JevActOutcome::refused("The field went away before its date was set."),
        })
    }
}
