//! One local re-plan for a harmless navigation; consequential repeats still
//! stop. And the light recovery tier (after fastbrowse's `recover`): a run
//! that stalls (three steps that changed nothing, three covered attempts,
//! or going round in circles) first gets up to
//! [`crate::engine::LIGHT_RECOVERIES`] one-step recoveries before it ends
//! `blocked` and the full fallback takes it over. Each asks the consultant
//! (see `consult`) for one control that moves the task on another way. The
//! step it names is taken next, through every gate, in place of a decision;
//! the stall's evidence is spent (the stall and wait rules count only steps
//! after it, and the same-pair counts start again), and Jev decides again
//! from there. A consultant that names nothing, names the control that just
//! failed, or cannot be reached leaves the stop as it was.
use super::consult::{Ask, directed};
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

impl Agent {
    /// After a stall ended the run `blocked`: whether a light recovery
    /// takes it up instead, with the step it names queued for the next
    /// tick and the run `ready` again.
    pub(super) async fn light_recovery(&mut self, reason: &str) -> bool {
        let stall = matches!(
            self.cause,
            Some(JevStopCause::Stalled | JevStopCause::Covered | JevStopCause::Looped)
        );
        if !stall
            || self.config.consult.consultant.is_none()
            || self.consults.recoveries >= self.config.consult.light_recoveries
            || self.budget_spent()
        {
            return false;
        }
        self.consults.recoveries += 1;
        let reason = self.secrets.scrub(reason);
        let Some(answer) = self.ask(Ask::Stalled { reason: &reason }).await else {
            return false;
        };
        let why = self
            .tidy(&json!(answer.diagnosis), repeat::CONTEXT_CHARS)
            .unwrap_or_default();
        let failed = self.last_step().map(|step| step["choice"].clone());
        let named = answer
            .control
            .as_deref()
            .and_then(|id| self.observed(id))
            .filter(|action| self.nameable(action))
            // The control the stall was on again is no recovery.
            .filter(|action| failed.as_ref() != Some(&action["id"]));
        let Some(action) = named else {
            self.recovery
                .record("light_recovery_declined", &why, self.elapsed_ms);
            return false;
        };
        let label = self
            .tidy(&action["label"], repeat::LABEL_CHARS)
            .unwrap_or_default();
        self.recovery.record(
            "light_recovery",
            &format!("{label:?}: {why}"),
            self.elapsed_ms,
        );
        // Spend the stall's evidence: the rules count only what follows.
        if let Some(entry) = self.history.last_mut() {
            entry["stall_reset"] = json!(true);
        }
        self.watch = loops::LoopWatch::default();
        self.directed = Some(directed(&action, json!({})));
        self.status = JevStatus::Ready;
        self.cause = None;
        true
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
        self.form_fill = super::form_fill::FormFill::default();
        self.watch = loops::LoopWatch::default();
        // A page the rescue moved to is a new state: its stalls get the
        // light recoveries afresh, and nothing queued before still applies.
        self.consults.recoveries = 0;
        self.directed = None;
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
