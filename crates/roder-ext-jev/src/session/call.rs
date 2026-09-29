//! One `jev_browse` call on a thread's session.

use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{JevSessions, SessionDeps, SessionState, TabNote};
use crate::engine::{JevEngineConfig, JevRunResult, JevStatus};
use crate::runner::{Driven, JevRequest, NO_TAB_YET, TabChoice, Task, annotate, drive, timed_out};

/// Why a call that could not get its session in time did nothing.
pub(crate) const BUSY: &str = "Another jev_browse call is still using this thread's Jev tab. \
     Wait for its result and continue from there.";
/// Earlier calls a result recalls.
const RECALLED_CALLS: usize = 4;
/// A wait for the session shorter than this is not worth reporting.
const REPORTED_WAIT: Duration = Duration::from_millis(500);

impl JevSessions {
    /// Run `request` on `thread`'s session and return the tool's data.
    pub(crate) async fn call(
        &self,
        thread: &str,
        request: JevRequest,
        deps: &dyn SessionDeps,
    ) -> anyhow::Result<Value> {
        let started = Instant::now();
        let deadline = started + request.timeout;
        if request.tab == TabChoice::Close {
            let closed = self.close(thread, deadline).await?;
            return Ok(json!({
                "status": "closed",
                "tabs_closed": closed,
                "session": {"closed": true},
            }));
        }
        // Checked again under the lock; this one spares starting anything.
        if request.url.is_none() && !self.has_tab(thread) {
            anyhow::bail!(NO_TAB_YET);
        }
        let Some((session, mut state)) = self.lock(thread, deadline).await else {
            return Ok(json!({"status": "busy", "stopped_because": BUSY}));
        };
        let waited = started.elapsed();
        if request.url.is_none() && state.tabs.is_empty() {
            anyhow::bail!(NO_TAB_YET);
        }
        let outcome = self
            .run_locked(&mut state, &request, deps, started, deadline)
            .await;
        state.last_used = std::time::Instant::now();
        session.publish(&state);
        drop(state);
        self.write_ledger();
        let mut value = outcome?;
        if waited >= REPORTED_WAIT {
            value["session"]["waited_ms"] = json!(waited.as_millis() as u64);
        }
        Ok(value)
    }

    async fn run_locked(
        &self,
        state: &mut SessionState,
        request: &JevRequest,
        deps: &dyn SessionDeps,
        started: Instant,
        deadline: Instant,
    ) -> anyhow::Result<Value> {
        let key = deps.model_key();
        if state.models.as_ref().is_none_or(|models| models.key != key) {
            state.models = Some(deps.models().await?);
        }
        let models = state.models.as_ref().context("models resolved above")?;
        let mut config = JevEngineConfig::new(request.goal.clone(), models.decision.clone())
            .with_wait(Duration::from_millis(request.wait_ms))
            .with_scope(request.scope.clone())
            .with_cookie_banner_refusal(request.refuse_cookie_banners)
            .with_secrets(state.secrets.clone());
        if let Some(actions) = request.max_actions {
            config = config.with_max_actions(actions);
        }
        if request.confirm_irreversible {
            config = config.with_irreversible_gate();
        }
        if request.authorize_irreversible {
            config = config.with_irreversible_authorized();
        }
        if let Some(text) = &models.text {
            config = config.with_text_resolver(text.clone());
        }
        let helper = models.helper.clone();
        // A tab whose page refused access holds nothing worth keeping, so a
        // new tab asked for after it would only leave a dead one open: the
        // call loads its url there instead.
        let dead_end = request.tab == TabChoice::New
            && request.url.is_some()
            && state.tabs.current().is_some()
            && state
                .trail
                .back()
                .is_some_and(|call| call.status == JevStatus::AccessDenied);
        let (tab, refused_tab) = match dead_end {
            true => (
                TabChoice::Current,
                state.tabs.current().map(|tab| tab.target.clone()),
            ),
            false => (request.tab, None),
        };

        // Starting Chrome counts against the task's timeout like everything else.
        let endpoint = match tokio::time::timeout_at(deadline, deps.endpoint()).await {
            Ok(endpoint) => Some(endpoint?),
            Err(_) => None,
        };
        let driven = match &endpoint {
            Some(endpoint) => {
                if state.endpoint.as_ref() != Some(endpoint) {
                    // Tabs of another browser: found gone on the next resume.
                    state.endpoint = Some(endpoint.clone());
                }
                let task = Task {
                    url: request.url.clone(),
                    tab,
                    refused_tab,
                    foreground: request.foreground,
                    started,
                    deadline,
                    config,
                };
                let reused = state.tabs.current().map(|tab| tab.id.clone());
                let mut driven = drive(endpoint.url(), &mut state.tabs, task).await?;
                if let (true, TabNote::Navigated, Some(reused)) = (dead_end, &driven.note, reused) {
                    driven.note = TabNote::Reused(format!(
                        "asked for a new tab, but {reused}'s page had refused access, so the \
                         url loaded in {reused} instead of leaving a dead tab open"
                    ));
                }
                driven
            }
            None => Driven {
                result: JevRunResult::before_start(
                    JevStatus::TimedOut,
                    request.url.as_deref().unwrap_or_default(),
                    started.elapsed(),
                    timed_out("starting Chrome"),
                ),
                note: TabNote::Continued,
                moved_to: None,
            },
        };
        state.record(&request.goal, request.url.as_deref(), &driven.result);
        let mut value = serde_json::to_value(&driven.result).context("serialize the JEV result")?;
        // The model that wrote values, which is not the one resolved when an
        // unusable Codex sign-in fell back.
        let text = helper.map(|helper| helper.current());
        annotate(
            &mut value,
            endpoint.as_ref(),
            text.as_ref(),
            request.foreground,
        );
        value["session"] = view(state, &driven);
        Ok(value)
    }
}

/// The session as a result reports it: this call's number and tab, how it
/// came to be on that tab, the tabs open, earlier calls and running totals.
fn view(state: &SessionState, driven: &Driven) -> Value {
    let calls = state.trail.len();
    let earlier = state
        .trail
        .iter()
        .take(calls.saturating_sub(1))
        .rev()
        .take(RECALLED_CALLS)
        .rev()
        .collect::<Vec<_>>();
    let mut view = json!({
        "call": state.totals.calls,
        "tab": state.tabs.current().map(|tab| tab.id.clone()),
        "tab_note": driven.note.name(),
        "tabs": state.tabs.records,
        "tabs_open": state.tabs.records.len(),
        "max_tabs": state.tabs.max_tabs,
        "earlier_calls": earlier,
        "totals": state.totals,
        "secrets_typed": state.secrets.len(),
        "began_on": state.began_on,
    });
    if let Some(detail) = driven.note.detail() {
        view["tab_detail"] = json!(detail);
    }
    if let Some(moved_to) = &driven.moved_to {
        view["moved_to"] = json!(moved_to);
    }
    view
}
