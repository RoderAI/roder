//! The bounded agent loop.
//!
//! A port of upstream Jev's `agent.py`: observe, ask for one operation, execute
//! it, observe again. The budgets and the rule that three consecutive actions
//! changing nothing mean `blocked` are upstream's, though an unchanged wait no
//! longer hides one. The statuses past `done` and `blocked` are Jev's:
//! upstream raises on a budget, a missing value or a provider failure, and a
//! run stopped that way ends with a [`JevStop`] status (any other error is
//! `error`) instead of reporting `ready`. An
//! action the page refused and the dialogs it opened are recorded on the step;
//! a blocked run whose page asked for a confirmation Jev declined says so.
//! Two more behaviours are Jev's: the opt-in irreversible-action gate stops
//! the run `needs_confirmation` before an action that may not be undone
//! (see [`crate::irreversible`]), and a cookie banner refused (on by
//! default) is recorded as a step of its own that costs no model call and
//! counts toward no budget. A value typed into a password or one-time-code
//! field is recorded as `[secret]` and scrubbed from every later page read
//! (see [`crate::secret`]). A run going round in circles that the stall rule
//! misses ends `blocked` too: the same control a fourth time on the same
//! page, three stale decisions in a row with nothing new on the page, or six
//! waits in a row that changed nothing (see `loops`).

use std::time::Instant;

use serde_json::{Value, json};

use crate::effects;
use crate::engine::{
    Covered, JevBrowser, JevDecision, JevDecisionRecord, JevEngineConfig, JevPageFacts, JevStatus,
    JevStop, JevStopCause, JevSuppressedClick, JevSuppressedKind, JevTextValue, JevWall,
    StaleObservation,
};
use crate::secret::{self, SECRET, Secrets};

mod consult;
mod evidence;
mod execution;
mod form_fill;
mod recovery;
mod research;
mod workflow;

pub(crate) struct Agent {
    evidence: evidence::Evidence,
    recovery: recovery::Recovery,
    research: research::Research,
    requirement: usize,
    browser: Box<dyn JevBrowser>,
    config: JevEngineConfig,
    observation: Value,
    /// Last semantic observation actually accepted by the decision driver.
    model_snapshot: Option<Value>,
    history: Vec<Value>,
    text_calls: Vec<Value>,
    decisions: usize,
    decision_calls: Vec<JevDecisionRecord>,
    /// The usage of decisions that were billed but could not be used.
    unusable_decisions: Vec<Value>,
    /// The unusable replies to the decision being asked, in a row.
    unusable_streak: unusable::UnusableStreak,
    status: JevStatus,
    started_at: Instant,
    elapsed_ms: u64,
    /// Values written for the empty fields of a form, kept for their steps.
    form_fill: form_fill::FormFill,
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
    /// The click a `done` run ended on instead of making.
    suppressed_click: Option<JevSuppressedClick>,
    /// What the caps on a run going round in circles remember.
    watch: loops::LoopWatch,
    /// The fill the text resolver had no value for, when one ended the run.
    missing_field: Option<Value>,
    /// The sign-in wall a `needs_login` run stopped at.
    wall: Option<JevWall>,
    /// The consults and light recoveries made so far (see `consult`).
    consults: consult::Consults,
    /// The step a light recovery named, taken next in place of a decision.
    directed: Option<JevDecision>,
}

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
            evidence: evidence::Evidence::default(),
            recovery: recovery::Recovery::default(),
            research: research::Research::default(),
            requirement: 0,
            browser,
            config,
            observation: Value::Null,
            model_snapshot: None,
            history: Vec::new(),
            text_calls: Vec::new(),
            decisions: 0,
            decision_calls: Vec::new(),
            unusable_decisions: Vec::new(),
            unusable_streak: unusable::UnusableStreak::default(),
            status: JevStatus::Ready,
            started_at: Instant::now(),
            elapsed_ms: 0,
            form_fill: form_fill::FormFill::default(),
            pending_text: None,
            stopped_before: None,
            secrets,
            facts: None,
            looked_again: false,
            cause: None,
            suppressed_click: None,
            watch: loops::LoopWatch::default(),
            missing_field: None,
            wall: None,
            consults: consult::Consults::default(),
            directed: None,
        };
        agent.observe().await?;
        agent.look_at_opening().await?;
        if let Err(error) = agent.check_scope() {
            agent.stopped_before = Some(agent.fail(&error));
        } else if let Some(reason) = agent.stop_refused() {
            agent.elapsed();
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
                Ok(()) => {
                    // A stall gets its light recoveries before it stands.
                    if self.status == JevStatus::Blocked {
                        let reason = self.cause.map(stall_reason).unwrap_or_default();
                        self.light_recovery(reason).await;
                    }
                }
                Err(error) if error.is::<StaleObservation>() => {
                    // The page moved under the decision; observe and choose again.
                    self.status = JevStatus::Ready;

                    let before = loops::settled_view(&self.observation);
                    if let Err(error) = self.observe().await {
                        return Some(self.fail(&error));
                    }
                    if let Err(error) = self.check_scope() {
                        return Some(self.fail(&error));
                    }
                    self.elapsed();
                    let same = before == loops::settled_view(&self.observation);
                    if let Some(stop) = self.after_stale(&error, same) {
                        return Some(self.fail(&stop));
                    }
                }
                Err(error) if Self::is_unusable_reply(&error) => {
                    // The service answered with something that cannot be
                    // used; the page has not moved, so just ask again.
                    if let Some(reason) = self.after_unusable_reply(&error) {
                        return Some(reason);
                    }
                }
                Err(error) => {
                    // Going round in circles is a stall too.
                    if self.cause == Some(JevStopCause::Looped)
                        && JevStop::status_of(&error) == JevStatus::Blocked
                        && self.light_recovery(&error.to_string()).await
                    {
                        continue;
                    }
                    return Some(self.fail(&error));
                }
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
        if self.check_scope().is_ok() {
            self.evidence.remember(&self.observation);
        }
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
        self.record_downloads();
        if let Some(label) = refused {
            self.record_cookie_banner(&self.secrets.scrub(&label));
        }
        self.offer_research().await?;
        Ok(())
    }

    /// Record the files the last action downloaded on its step, as the
    /// observation lists them, with page-supplied text scrubbed.
    fn record_downloads(&mut self) {
        let Some(downloads) = self.observation["downloads"]
            .as_array()
            .filter(|downloads| !downloads.is_empty())
        else {
            return;
        };
        let mut downloads = downloads.clone();
        for download in &mut downloads {
            for key in ["name", "url", "path"] {
                if let Some(text) = download[key].as_str() {
                    download[key] = json!(self.secrets.scrub(text));
                }
            }
        }
        if let Some(entry) = self.history.last_mut() {
            match entry["downloads"].as_array_mut() {
                Some(recorded) => recorded.extend(downloads),
                None => entry["downloads"] = Value::Array(downloads),
            }
        }
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

    pub(crate) async fn close(&mut self) -> anyhow::Result<()> {
        self.browser.close().await
    }
}

/// Upstream's stall rule: three consecutive executed actions, none of them a
/// wait, none of which changed the page. A choice that set no value (see
/// `effects`) counts as unchanged even when its page redrew.
/// A refused cookie banner is not an action the model chose, so it neither
/// counts nor breaks the run of three. Nor does a wait after which the page
/// was the same: it is skipped, so no-ops with such waits between them are
/// still a run of three. (Six of those waits in a row are a stop of their
/// own, see `loops`.) A wait that changed the page breaks the run.
/// Which rule stalled the run: three covered attempts in a row, or any
/// three that changed nothing.
///
/// A light recovery spends the evidence: only the steps after the one it
/// marked `stall_reset` count (see `recovery`).
fn stalled(history: &[Value]) -> Option<JevStopCause> {
    let recent = since_recovery(history)
        .filter(chosen)
        .filter(|entry| !loops::idle_wait(entry) && entry["kind"] != "read_document")
        .take(3)
        .collect::<Vec<_>>();
    let stalled = recent.len() == 3
        && recent.iter().all(|entry| {
            // A choice that set no value changed the page (a menu closed)
            // but did not take.
            (entry["page_changed"] == json!(false) || entry["set_something"] == json!(false))
                && entry["kind"].as_str() != Some("wait")
        });
    let covered = recent.iter().all(|entry| entry["covered"] == json!(true));
    match (stalled, covered) {
        (false, _) => None,
        (true, true) => Some(JevStopCause::Covered),
        (true, false) => Some(JevStopCause::Stalled),
    }
}

/// The history back to the last step a light recovery spent the stall
/// evidence at, newest first.
fn since_recovery(history: &[Value]) -> impl Iterator<Item = &Value> {
    history
        .iter()
        .rev()
        .take_while(|entry| entry["stall_reset"] != json!(true))
}

/// What a stall cause tells the consultant.
fn stall_reason(cause: JevStopCause) -> &'static str {
    match cause {
        JevStopCause::Covered => "its last three attempts found their target covered",
        _ => "its last three actions changed nothing on the page",
    }
}

mod cover;
mod look;
mod loops;
mod opt_in;
mod predict;
mod repeat;
mod result;
pub(crate) use consult::configure as configure_consult;
pub(crate) use cover::{offers, tidy_cover};
pub(crate) use repeat::REPEAT_CONFIDENCE;
pub(crate) use result::{controls, omitted};
#[cfg(test)]
mod tests;
mod typing;
mod unusable;
