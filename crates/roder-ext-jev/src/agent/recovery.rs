//! One local re-plan for a harmless navigation; consequential repeats still stop.
use super::*;
#[derive(Default)]
pub(super) struct Recovery {
    pub(super) events: Vec<Value>,
    retried: bool,
    excluded: Option<(Value, Value)>,
}
impl Recovery {
    pub(super) fn record(&mut self, kind: &str, reason: &str, elapsed: u64) {
        if self.events.len() < 24 {
            self.events
                .push(json!({"kind":kind,"reason":reason,"elapsed_ms":elapsed}));
        }
    }
    pub(super) fn prepare(&self, observed: &mut Value) {
        if let Some((url, id)) = &self.excluded
            && observed["url"] == *url
        {
            if let Some(actions) = observed["actions"].as_array_mut() {
                actions.retain(|a| a["id"] != *id);
            }
            observed["loop_feedback"] = json!(
                "The previous navigation succeeded. Its uncertain duplicate was suppressed and is unavailable for this recovery. Use the current page and retained evidence to continue unmet requirements; do not claim success solely because navigation happened."
            );
        }
        if let Some(last) = self.events.last() {
            observed["last_recovery"] = last.clone();
        }
    }
}
impl Agent {
    pub(super) async fn retry_navigation(
        &mut self,
        action: &Value,
        decision: &JevDecision,
    ) -> anyhow::Result<bool> {
        let label = action["label"].as_str().unwrap_or_default();
        // Only observed links, never buttons, form submission, or commitment labels.
        if self.recovery.retried
            || !navigation(action, &self.observation)
            || crate::irreversible::names_commitment(label)
            || decision.irreversible.is_some_and(|p| p >= 0.5)
        {
            return Ok(false);
        }
        self.recovery.retried = true;
        self.recovery.excluded = Some((self.observation["url"].clone(), action["id"].clone()));
        self.recovery.record(
            "navigation_repeat",
            "Suppressed duplicate navigation; refreshed and replanned once",
            self.elapsed_ms,
        );
        self.observe().await?;
        self.check_scope()?;
        Ok(true)
    }
}

impl Agent {
    /// Reuse a read-only navigation only when a refreshed observation proves the
    /// exact target and all form values unchanged. Never retarget by label.
    pub(super) async fn revalidate_navigation(
        &mut self,
        previous: &Value,
        decision: &JevDecision,
    ) -> anyhow::Result<bool> {
        let Some(action) = previous["actions"]
            .as_array()
            .and_then(|a| a.iter().find(|a| a["id"] == decision.choice))
        else {
            return Ok(false);
        };
        if action["kind"] != "click"
            || !navigation(action, previous)
            || crate::irreversible::names_commitment(action["label"].as_str().unwrap_or_default())
        {
            return Ok(false);
        }
        let node = action["node"].to_string();
        if !previous["guards"][&node].is_array() {
            return Ok(false);
        }
        self.observe().await?;
        self.check_scope()?;
        let current = &self.observation;
        let same = current["guards"][&node] == previous["guards"][&node]
            && [0, 1, 6]
                .iter()
                .all(|i| current["page_key"][*i] == previous["page_key"][*i])
            && current["actions"].as_array().is_some_and(|a| {
                a.iter().any(|a| {
                    a["id"] == action["id"]
                        && a["node"] == action["node"]
                        && a["label"] == action["label"]
                        && a["kind"] == action["kind"]
                })
            });
        if same {
            self.recovery.record("stale_revalidated", "Refreshed exact navigation target and form state unchanged; retrying once without another model call", self.elapsed_ms);
        }
        Ok(same)
    }
}

#[cfg(test)]
impl Agent {
    pub(crate) async fn resume_after_rescue(
        &mut self,
        page: &Value,
        secrets: &Secrets,
    ) -> anyhow::Result<()> {
        if !matches!(self.status, JevStatus::Blocked | JevStatus::BudgetExceeded)
            || self.budget_spent()
        {
            return Err(JevStop::new(
                self.status,
                "No native budget remains for a rescue hand-back",
            )
            .into());
        }
        self.secrets.extend(secrets);
        let mut page = page.clone();
        self.secrets.scrub_observation(&mut page);
        self.evidence.remember(&page);
        self.status = JevStatus::Ready;
        self.cause = None;
        self.model_snapshot = None;
        self.research = research::Research::default();
        self.pending_text = None;
        self.watch = loops::LoopWatch::default();
        self.recovery.record("rescue_handback","A bounded external driver moved the tab; continue unmet requirements using fresh controls and retained evidence",self.elapsed_ms);
        self.observe().await?;
        self.check_scope()
    }
}

/// An ARIA menuitem can still be an observed navigation anchor. Require its
/// observed HTTP(S) destination rather than treating every menuitem as safe.
fn navigation(action: &Value, observation: &Value) -> bool {
    if action["role"] == "link" {
        return true;
    }
    let node = action["node"].to_string();
    observation["guards"][&node][12]
        .as_str()
        .is_some_and(|url| url.starts_with("https://") || url.starts_with("http://"))
}
