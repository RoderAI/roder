//! The bounded agent loop.
//!
//! A port of upstream Jev's `agent.py`: observe, ask for one operation, execute
//! it, observe again. The budgets and the rule that three consecutive actions
//! changing nothing mean `blocked` are upstream's. The statuses past `done` and
//! `blocked` are Jev's: upstream raises on a budget, a missing value or a
//! provider failure, and a run stopped that way ends with a [`JevStop`]
//! status (any other error is `error`) instead of reporting `ready`. An
//! action the page refused and the dialogs it opened are recorded on the step;
//! a blocked run whose page asked for a confirmation Jev declined says so.
//! Two more behaviours are Jev's: the opt-in irreversible-action gate stops
//! the run `needs_confirmation` before an action that may not be undone
//! (see [`crate::irreversible`]), and a cookie banner refused (on by
//! default) is recorded as a step of its own that costs no model call and
//! counts toward no budget. A value typed into a password or one-time-code
//! field is recorded as `[secret]` and scrubbed from every later page read
//! (see [`crate::secret`]).

use std::time::Instant;

use serde_json::{Value, json};

use crate::effects;
use crate::engine::{
    Covered, JevBrowser, JevDecision, JevDecisionRecord, JevEngineConfig, JevPageFacts, JevStatus,
    JevStop, JevStopCause, JevTextValue, StaleObservation,
};
use crate::secret::{self, SECRET, Secrets};
use crate::usage::JevBilled;

pub(crate) struct Agent {
    browser: Box<dyn JevBrowser>,
    config: JevEngineConfig,
    observation: Value,
    history: Vec<Value>,
    text_calls: Vec<Value>,
    decisions: usize,
    decision_calls: Vec<JevDecisionRecord>,
    /// The usage of decisions that were billed but could not be used.
    unusable_decisions: Vec<Value>,
    status: JevStatus,
    started_at: Instant,
    elapsed_ms: u64,
    /// A value already written for a field whose action had to be retried.
    pending_text: Option<(Value, JevTextValue)>,
    /// Why the run ended before its loop started (a start page outside the
    /// allowed origins).
    stopped_before: Option<String>,
    /// The secrets typed so far, scrubbed from every later observation.
    secrets: Secrets,
    /// What the browser said of the page besides the observation.
    facts: Option<JevPageFacts>,
    /// A BLOCKED on an empty page has already been looked at again.
    looked_again: bool,
    /// What ended the run, once it stopped short of its goal.
    cause: Option<JevStopCause>,
}

/// Below this, a repeat of the last effective click is read as DONE.
const REPEAT_CONFIDENCE: f64 = 0.7;

/// The kind of a history entry recording a refused cookie banner: not a
/// model step, so no budget, stall or repeat rule counts it.
pub(crate) const COOKIE_BANNER: &str = "cookie_banner";

/// Whether a history entry is an action the model chose.
fn chosen(entry: &&Value) -> bool {
    entry["kind"].as_str() != Some(COOKIE_BANNER)
}

impl Agent {
    pub(crate) async fn start(
        browser: Box<dyn JevBrowser>,
        mut config: JevEngineConfig,
    ) -> anyhow::Result<Self> {
        let secrets = std::mem::take(&mut config.secrets);
        let mut agent = Self {
            browser,
            config,
            observation: Value::Null,
            history: Vec::new(),
            text_calls: Vec::new(),
            decisions: 0,
            decision_calls: Vec::new(),
            unusable_decisions: Vec::new(),
            status: JevStatus::Ready,
            started_at: Instant::now(),
            elapsed_ms: 0,
            pending_text: None,
            stopped_before: None,
            secrets,
            facts: None,
            looked_again: false,
            cause: None,
        };
        agent.observe().await?;
        agent.look_again_while_empty().await?;
        if let Err(error) = agent.check_scope() {
            agent.stopped_before = Some(agent.fail(&error));
        } else if let Some(reason) = agent.refused_access().await {
            agent.stop(JevStatus::AccessDenied);
            agent.stopped_before = Some(reason);
        }
        Ok(agent)
    }

    pub(crate) fn max_duration(&self) -> Option<std::time::Duration> {
        self.config.max_duration
    }

    /// End the run, `blocked`, on a page outside the allowed origins.
    fn check_scope(&mut self) -> anyhow::Result<()> {
        let url = self.observation["url"].as_str().unwrap_or_default();
        if self.config.scope.allows(url) {
            return Ok(());
        }
        self.cause = Some(JevStopCause::OutsideScope);
        Err(JevStop::new(JevStatus::Blocked, self.config.scope.outside(url)).into())
    }

    fn elapsed(&mut self) -> u64 {
        self.elapsed_ms = self.started_at.elapsed().as_millis() as u64;
        self.elapsed_ms
    }

    /// Run until the goal is met, nothing can progress, or a budget is reached.
    /// A budget or helper failure ends the run but keeps the trace, which is
    /// what upstream raises and callers want reported; the error's
    /// [`JevStop`] status, else `error`, says which.
    pub(crate) async fn run(&mut self) -> Option<String> {
        if self.stopped_before.is_some() {
            return self.stopped_before.clone();
        }
        while !self.status.stopped() {
            match self.tick().await {
                Ok(()) => {}
                Err(error) if error.is::<StaleObservation>() => {
                    // The page moved under the decision; observe and choose again.
                    self.status = JevStatus::Ready;
                    if let Err(error) = self.observe().await {
                        return Some(self.fail(&error));
                    }
                    if let Err(error) = self.check_scope() {
                        return Some(self.fail(&error));
                    }
                    self.elapsed();
                }
                Err(error) => return Some(self.fail(&error)),
            }
        }
        self.dismissed_dialog()
    }

    /// Read the page, recording any dialog it opened since the last read on
    /// the step that preceded it. With cookie-banner refusal on, the browser
    /// first gets its chance to refuse one, which is recorded after that.
    async fn observe(&mut self) -> anyhow::Result<()> {
        let refused = if self.config.refuse_cookie_banners {
            // Best effort: a check that fails never ends the run, and the
            // observation that follows reports the page as it is.
            self.browser.refuse_cookie_banner().await.ok().flatten()
        } else {
            None
        };
        self.observation = self.browser.observe().await?;
        // A page that shows a typed secret back does not pass it on.
        self.secrets.scrub_observation(&mut self.observation);
        let dialogs = self.observation["dialogs"]
            .as_array()
            .filter(|dialogs| !dialogs.is_empty())
            .cloned();
        if let (Some(dialogs), Some(entry)) = (dialogs, self.history.last_mut()) {
            match entry["dialogs"].as_array_mut() {
                Some(recorded) => recorded.extend(dialogs),
                None => entry["dialogs"] = Value::Array(dialogs),
            }
        }
        if self.observation["opened_tab"] == json!(true)
            && let Some(entry) = self.history.last_mut()
        {
            entry["opened_tab"] = json!(true);
        }
        if let Some(label) = refused {
            self.record_cookie_banner(&self.secrets.scrub(&label));
        }
        Ok(())
    }

    /// The last action the model chose.
    fn last_step(&self) -> Option<&Value> {
        self.history.iter().rev().find(chosen)
    }

    /// Why a blocked run stopped, when the page asked for consent or input
    /// Jev declined: a confirm the task may have needed accepted is for the
    /// caller to decide, and the only thing to do about it.
    fn dismissed_dialog(&self) -> Option<String> {
        if self.status != JevStatus::Blocked {
            return None;
        }
        let dialog = self
            .history
            .iter()
            .rev()
            .filter_map(|entry| entry["dialogs"].as_array())
            .flat_map(|dialogs| dialogs.iter().rev())
            .find(|dialog| dialog["accepted"] == json!(false))?;
        Some(format!(
            "The page asked {:?} in a {} dialog and Jev dismissed it: Jev never accepts a \
             confirm or answers a prompt. If the task needs it accepted, do that step yourself.",
            dialog["message"].as_str().unwrap_or_default(),
            dialog["type"].as_str().unwrap_or("confirm"),
        ))
    }

    fn fail(&mut self, error: &anyhow::Error) -> String {
        let status = JevStop::status_of(error);
        self.stop(status);
        self.cause.get_or_insert(match status {
            JevStatus::BudgetExceeded if self.budget_spent() => JevStopCause::Budget,
            _ => JevStopCause::Stopped,
        });
        error.to_string()
    }

    /// Whether the run has spent its own action or model-call budget, as
    /// opposed to an embedder ending it with that status.
    fn budget_spent(&self) -> bool {
        self.decisions >= self.config.max_actions.saturating_mul(2)
            || self.history.iter().filter(chosen).count() >= self.config.max_actions
    }

    /// End the run with `status`, as of now.
    pub(crate) fn stop(&mut self, status: JevStatus) {
        self.status = status;
        self.elapsed();
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        let decision = self.predict().await?;
        self.act(decision).await
    }

    async fn predict(&mut self) -> anyhow::Result<JevDecision> {
        if !self.browser.fresh(&self.observation, None).await? {
            self.observe().await?;
            self.check_scope()?;
        }
        if self.decisions >= self.config.max_actions.saturating_mul(2) {
            return Err(JevStop::new(
                JevStatus::BudgetExceeded,
                "Reached the demo's model-call budget",
            )
            .into());
        }
        let decision = self.config.decision.as_ref();
        let (observation, goal, history) = (&self.observation, &self.config.goal, &self.history);
        let chosen = if self.config.irreversible_gate {
            decision.choose_gated(observation, goal, history).await
        } else {
            decision.choose(observation, goal, history).await
        };
        let decision = match chosen {
            Ok(decision) => decision,
            Err(error) => {
                // An answer that failed validation was still billed.
                if let Some(usage) = JevBilled::usage_of(&error) {
                    self.decisions += 1;
                    self.unusable_decisions.push(usage.clone());
                }
                return Err(error);
            }
        };
        self.decisions += 1;
        self.decision_calls.push(JevDecisionRecord {
            choice: decision.choice.clone(),
            operation: decision.operation.clone(),
            target: decision.target.clone(),
            confidence: decision.confidence,
            target_confidence: decision.target_confidence,
            probabilities: decision.probabilities.clone(),
            latency_ms: decision.latency_ms,
            usage: decision.usage.clone(),
            model: decision.model.clone(),
            irreversible: decision.irreversible,
        });
        Ok(decision)
    }

    /// An unsure repeat of the click that just took effect. Live runs show
    /// the model split between that click and DONE once the page confirms
    /// the first one (confidence 0.43 to 0.66), while a click that is meant
    /// to repeat stays above 0.75. Clicking again is the costly mistake: a
    /// second item in the cart.
    fn repeats_a_finished_click(&self, action: &Value, decision: &JevDecision) -> bool {
        let Some(last) = self.last_step() else {
            return false;
        };
        action["kind"] == json!("click")
            && decision.confidence < REPEAT_CONFIDENCE
            && last["kind"] == action["kind"]
            && last["choice"] == json!(decision.choice)
            && last["action"] == action["label"]
            && last["page_changed"] == json!(true)
    }

    async fn act(&mut self, decision: JevDecision) -> anyhow::Result<()> {
        let selected = decision.choice.clone();
        // A page that showed nothing yet is read again once before a
        // BLOCKED about it is believed; the next decision sees what came.
        if selected == "BLOCKED" && !self.looked_again && self.shows_nothing() {
            self.looked_again = true;
            self.look_again_while_empty().await?;
            return Ok(());
        }
        if selected == "DONE" || selected == "BLOCKED" {
            if !self.browser.fresh(&self.observation, None).await? {
                return Err(StaleObservation::new(
                    "Page changed since the decision. Choose again.",
                )
                .into());
            }
            // DONE straight after a covered attempt claims an effect that
            // never happened: nothing was dispatched.
            let covered = self
                .last_step()
                .is_some_and(|entry| entry["covered"] == json!(true));
            (self.status, self.cause) = match (selected.as_str(), covered) {
                ("DONE", false) => (JevStatus::Done, None),
                ("DONE", true) => (JevStatus::Blocked, Some(JevStopCause::DoneAfterCovered)),
                _ => (JevStatus::Blocked, Some(JevStopCause::ModelBlocked)),
            };
            self.elapsed();
            return Ok(());
        }
        let action = self
            .observation
            .get("actions")
            .and_then(Value::as_array)
            .and_then(|actions| {
                actions
                    .iter()
                    .find(|action| action["id"].as_str() == Some(selected.as_str()))
            })
            .cloned();
        let Some(action) = action else {
            return Err(StaleObservation::new("Chosen action is no longer observed").into());
        };
        if self.repeats_a_finished_click(&action, &decision) {
            self.status = JevStatus::Done;
            self.elapsed();
            return Ok(());
        }
        if self.history.iter().filter(chosen).count() >= self.config.max_actions {
            return Err(JevStop::new(
                JevStatus::BudgetExceeded,
                format!(
                    "Stopped at the {}-action demo budget",
                    self.config.max_actions
                ),
            )
            .into());
        }
        self.gate(&action, &decision).await?;

        let written = self.write_text_if_needed(&action).await?;
        let text = written.as_ref().map(|written| written.value.clone());
        let secret = secret::is_secret(&action);
        if secret && let Some(text) = &text {
            self.secrets.remember(text);
        }
        let (covered, refused, uncovered) = match self
            .browser
            .act(
                &action,
                &self.observation,
                text.as_deref(),
                self.config.wait,
            )
            .await
        {
            // A refusal is recorded and the run goes on: the next decision
            // sees the page as it stands.
            Ok(outcome) => (false, outcome.refused, outcome.uncovered),
            // Nothing was dispatched, but the attempt is a step that changed
            // nothing: a target that stays covered stalls the run instead of
            // paying for a decision per retry until the model-call budget.
            Err(error) if error.is::<Covered>() => (true, None, None),
            Err(error) => return Err(error),
        };
        self.pending_text = None;
        self.elapsed();

        // Record execution before observing: a stale observation afterwards
        // must not erase the fact that the action ran.
        let previous = self.observation.clone();
        self.history.push(json!({
            "step": self.history.len() + 1,
            "action": action["label"],
            "kind": action["kind"],
            // Where it sat, for the caller's trace; not sent to the model.
            "context": action.get("context").or_else(|| action.get("section")),
            "choice": selected,
            "probability": decision.probability_of(&selected),
            "confidence": decision.confidence,
            "target_confidence": decision.target_confidence,
            "latency_ms": decision.latency_ms,
            // A covered fill typed nothing, and a secret is never recorded.
            "text": match (covered, secret) {
                (true, _) => None,
                (false, true) => text.map(|_| SECRET.to_string()),
                (false, false) => text,
            },
            "covered": covered,
            "refused": refused.map(|reason| self.secrets.scrub(&reason)),
            "uncovered": uncovered.map(|how| self.secrets.scrub(&how)),
            "text_helper": written.as_ref().map(|written| written.model.clone()),
            "text_latency_ms": written.as_ref().map_or(0, |written| written.latency_ms),
            "operation": decision.operation,
            "target": decision.target,
            "page_changed": Value::Null,
            "effect": Value::Null,
            "url": self.observation["url"],
            "usage": decision.usage,
            "executed_ms": self.elapsed_ms,
            "elapsed_ms": self.elapsed_ms,
        }));

        // A refused cookie banner may be recorded after this step.
        let step = self.history.len() - 1;
        self.observe().await?;
        let elapsed = self.elapsed();
        if let Some(entry) = self.history.get_mut(step) {
            // A covered target may have been scrolled into view first; that
            // is not the action taking effect.
            entry["page_changed"] =
                json!(!covered && self.observation["fingerprint"] != previous["fingerprint"]);
            entry["effect"] = json!(effects::effect(&previous, &self.observation));
            entry["url"] = self.observation["url"].clone();
            entry["elapsed_ms"] = json!(elapsed);
        }
        self.check_scope()?;
        self.status = match stalled(&self.history) {
            Some(cause) => {
                self.cause = Some(cause);
                JevStatus::Blocked
            }
            None => JevStatus::Ready,
        };
        Ok(())
    }

    pub(crate) async fn close(&mut self) -> anyhow::Result<()> {
        self.browser.close().await
    }
}

/// Upstream's stall rule: three consecutive executed actions, none of them a
/// wait, none of which changed the page.
/// A refused cookie banner is not an action the model chose, so it neither
/// counts nor breaks the run of three.
/// Which rule stalled the run: three covered attempts in a row, or any
/// three that changed nothing.
fn stalled(history: &[Value]) -> Option<JevStopCause> {
    let recent = history
        .iter()
        .rev()
        .filter(chosen)
        .take(3)
        .collect::<Vec<_>>();
    let stalled = recent.len() == 3
        && recent.iter().all(|entry| {
            entry["page_changed"] == json!(false) && entry["kind"].as_str() != Some("wait")
        });
    let covered = recent.iter().all(|entry| entry["covered"] == json!(true));
    match (stalled, covered) {
        (false, _) => None,
        (true, true) => Some(JevStopCause::Covered),
        (true, false) => Some(JevStopCause::Stalled),
    }
}

mod look;
mod opt_in;
mod result;
pub(crate) use result::controls;
#[cfg(test)]
mod tests;
mod typing;
