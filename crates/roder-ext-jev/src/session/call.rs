//! One `jev_browse` call on a thread's session.

use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{JevSessions, SessionDeps, SessionState, TabNote};
use crate::chrome::ChromeEndpoint;
use crate::engine::{JevEngineConfig, JevRunResult, JevStatus};
use crate::fallback::run::Limits;
use crate::fallback::trigger::trigger;
use crate::fallback::{Brief, FallbackMode, FinalPage, Report, Rules, fall_back as run_fallback};
use crate::runner::{
    Driven, JevRequest, NO_TAB_YET, TabChoice, Task, annotate, close_all, close_quietly, drive,
    look_again, timed_out,
};

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
            .run_locked(thread, &mut state, &request, deps, started, deadline)
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
        thread: &str,
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
        let fallback = match &endpoint {
            Some(endpoint) => {
                fall_back(
                    thread,
                    state,
                    request,
                    deps,
                    &driven.result,
                    endpoint,
                    started,
                )
                .await
            }
            None => Fallback::none(),
        };
        let mut recorded = driven.result.clone();
        if let Report::Ran { outcome, .. } = &fallback.report {
            recorded.status = outcome.status;
            recorded.stopped_because = outcome.stopped_because.clone();
            if let Some(page) = &fallback.page {
                recorded.url = page.url.clone();
                recorded.title = page.title.clone();
            }
            state.totals.fallback_actions += outcome.actions.len();
        }
        state.record(&request.goal, request.url.as_deref(), &recorded);
        let mut value = serde_json::to_value(&driven.result).context("serialize the JEV result")?;
        crate::fallback::report(
            &mut value,
            &driven.result,
            &fallback.report,
            request.fallback.mode,
            state.tabs.current().map(|tab| tab.id.as_str()),
            fallback.page,
        );
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

/// What the fallback came to for one call.
struct Fallback {
    report: Report,
    /// The page the call ended on, read again after the fallback moved it.
    page: Option<FinalPage>,
}

impl Fallback {
    fn none() -> Self {
        Self {
            report: Report::None,
            page: None,
        }
    }
}

/// Fall back after a Jev run that could not progress, as the operator's
/// `JEV_FALLBACK` says: hand over, or run the model-driven loop in the same
/// tab and record on the session what it did (tabs it opened, secrets it
/// typed, where it left the tab).
async fn fall_back(
    thread: &str,
    state: &mut SessionState,
    request: &JevRequest,
    deps: &dyn SessionDeps,
    result: &JevRunResult,
    endpoint: &ChromeEndpoint,
    started: Instant,
) -> Fallback {
    let Some(why) = trigger(result) else {
        return Fallback::none();
    };
    let settings = &request.fallback;
    let handover = |reason: Option<String>| Fallback {
        report: Report::Handover {
            trigger: why,
            why: reason,
        },
        page: None,
    };
    match settings.mode {
        FallbackMode::Off => return Fallback::none(),
        FallbackMode::Handover => return handover(None),
        FallbackMode::Auto => {}
    }
    let Some(tab) = state.tabs.current().cloned() else {
        return handover(Some("the session has no tab to go on in".into()));
    };
    let model = match deps.fallback_model(settings, thread).await {
        Ok(model) => model,
        Err(reason) => return handover(Some(reason)),
    };
    let now = Instant::now();
    let mut deadline = now + settings.max_duration;
    if let Some(seconds) = request.host_seconds {
        deadline = deadline.min(started + Duration::from_secs(seconds));
    }
    if deadline <= now {
        return handover(Some("no time is left in this call".into()));
    }
    let mut secrets = state.secrets.clone();
    secrets.extend(&result.typed_secrets);
    let rules = Rules {
        scope: request.scope.clone(),
        gate: request.confirm_irreversible,
        authorized: request.authorize_irreversible,
        banners: request.refuse_cookie_banners,
        secrets,
    };
    let limits = Limits {
        max_steps: settings.max_steps,
        max_tokens: settings.max_tokens,
        deadline,
    };
    let direct = roder_ext_chrome::direct::DirectTab::Target {
        endpoint: endpoint.url().to_string(),
        target_id: tab.target.clone(),
    };
    let brief = Brief {
        goal: &request.goal,
        trigger: why,
        jev: result,
    };
    let (outcome, secrets) =
        run_fallback(&direct, model.as_ref(), brief, rules, limits, None).await;
    state.secrets.extend(&secrets);
    for opened in &outcome.opened_tabs {
        state.tabs.adopt(&opened.target_id, &opened.opener);
    }
    let closing = state.tabs.cap();
    if !closing.is_empty() {
        close_quietly(async {
            let mut connection = crate::cdp::Connection::connect(endpoint.url()).await?;
            close_all(&mut connection, &closing).await;
            Ok(())
        })
        .await;
    }
    let page = match outcome.actions.is_empty() {
        true => None,
        false => {
            look_again(
                endpoint.url(),
                &state.tabs,
                &state.secrets,
                request.refuse_cookie_banners,
            )
            .await
        }
    };
    let url = page.as_ref().map(|page| page.url.clone()).or_else(|| {
        outcome
            .last_page
            .as_ref()
            .and_then(|page| page["url"].as_str().map(str::to_string))
    });
    if let Some(url) = url.filter(|url| !url.is_empty()) {
        state.tabs.last_url = Some(url);
    }
    Fallback {
        report: Report::Ran {
            trigger: why,
            outcome: Box::new(outcome),
        },
        page,
    }
}
