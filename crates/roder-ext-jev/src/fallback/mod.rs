//! When Jev cannot progress: Roder's full browser tools on the same tab.
//!
//! Jev acts only on the controls its snapshot offers, with a small action
//! vocabulary: no screenshots, coordinates, hover, drag or arbitrary keys.
//! When a run ends because of that ([`trigger`]), what happens next is the
//! operator's `JEV_FALLBACK` ([`settings`]):
//!
//! - `auto` (default): `jev_browse` itself goes on, in the same tab, with a
//!   bounded loop ([`run`]) driven by the model the session is on (or
//!   `JEV_FALLBACK_MODEL`) and Roder's direct CDP tools, and returns one
//!   result that says which driver did what, with each one's steps, time
//!   and tokens. When no model can drive it here, or it also fails, the
//!   result hands over instead.
//! - `handover`: the result tells the caller that the full tools
//!   (`jev_tab_*`, [`handover`]) work on this same tab, names them and the
//!   tab, and says what Jev did and where it stopped.
//! - `off`: Jev's result as it was.
//!
//! Either way the fallback inherits Jev's rules ([`guard`]) and never runs
//! after `needs_input`, `needs_confirmation`, `access_denied` or a page that
//! did not load: a different driver does not fix those, and it must never
//! be a way around a block or a confirmation.

pub(crate) mod guard;
pub(crate) mod handover;
pub(crate) mod model;
mod prompt;
pub(crate) mod run;
pub(crate) mod settings;
#[cfg(test)]
mod tests;
pub(crate) mod trigger;

use std::sync::Arc;

use roder_ext_chrome::direct::{DirectSession, DirectTab};
use serde_json::{Value, json};

use crate::engine::{JevRunResult, JevStatus};
use crate::scope::JevOriginScope;
use crate::secret::Secrets;
use guard::JevGuard;
use model::FallbackModel;
pub(crate) use run::{Brief, EndCheck, FallbackOutcome, Limits};
pub(crate) use settings::{FallbackMode, FallbackSettings};
use trigger::Trigger;

/// The rules the fallback runs under, Jev's own for this call.
#[derive(Clone)]
pub(crate) struct Rules {
    pub(crate) scope: JevOriginScope,
    /// The irreversible-action gate is on.
    pub(crate) gate: bool,
    /// The call authorized an irreversible step; the fallback may then
    /// press a gated control only when it passes `authorize_irreversible`
    /// for that press too.
    pub(crate) authorized: bool,
    /// Cookie-banner refusal is on: a banner may be refused, never
    /// accepted; off, no banner is touched.
    pub(crate) banners: bool,
    pub(crate) secrets: Secrets,
}

/// Run the fallback on `tab` and return how it went and every secret known
/// after it (the session's and any it typed).
pub(crate) async fn fall_back(
    tab: &DirectTab,
    model: &dyn FallbackModel,
    brief: Brief<'_>,
    rules: Rules,
    limits: Limits,
    end: Option<&dyn EndCheck>,
) -> (FallbackOutcome, Secrets) {
    let guard = Arc::new(JevGuard::new(
        rules.scope,
        rules.gate,
        rules.banners,
        rules.secrets,
    ));
    let attached = tokio::time::timeout_at(
        limits.deadline,
        DirectSession::attach(tab, guard.clone(), rules.authorized),
    )
    .await;
    let mut session = match attached {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => {
            let reason = format!("could not attach to Jev's tab: {error:#}");
            return (FallbackOutcome::failed(model, reason), guard.secrets());
        }
        Err(_) => {
            let reason = "ran out of time attaching to Jev's tab".to_string();
            return (FallbackOutcome::failed(model, reason), guard.secrets());
        }
    };
    let outcome = run::run(&mut session, &guard, model, brief, limits, end).await;
    session.detach().await;
    (outcome, guard.secrets())
}

impl FallbackOutcome {
    fn failed(model: &dyn FallbackModel, reason: String) -> Self {
        Self {
            status: JevStatus::Error,
            stopped_because: Some(reason),
            message: String::new(),
            model: model.label(),
            actions: Vec::new(),
            model_calls: 0,
            usage: run::FallbackUsage::default(),
            elapsed_ms: 0,
            opened_tabs: Vec::new(),
            target_id: String::new(),
            last_page: None,
        }
    }
}

/// What a call's result says about the fallback.
pub(crate) enum Report {
    /// Jev's result stands: no fallback applies, or the operator turned it
    /// off.
    None,
    /// The caller goes on with the full tools; `why` when `auto` could not
    /// run.
    Handover {
        trigger: Trigger,
        why: Option<String>,
    },
    /// The automatic fallback ran.
    Ran {
        trigger: Trigger,
        outcome: Box<FallbackOutcome>,
    },
}

/// A page as a result reports it, read after the fallback moved the tab.
#[derive(Debug, Clone, Default)]
pub(crate) struct FinalPage {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) visible_text: String,
    pub(crate) controls: Value,
    pub(crate) page: Value,
    pub(crate) observed_elements: usize,
}

/// Put the fallback into a call's result data. With a fallback that ran,
/// the top-level status, page and time are the call's end state (the
/// fallback's), Jev's own status moves to `jev_status`, and `drivers` says
/// which driver did what, at what cost.
pub(crate) fn report(
    data: &mut Value,
    jev: &JevRunResult,
    report: &Report,
    mode: FallbackMode,
    tab: Option<&str>,
    final_page: Option<FinalPage>,
) {
    let jev_driver = json!({
        "driver": "jev",
        "status": jev.status,
        "actions": jev.actions.len(),
        "decisions": jev.model_calls,
        "text_calls": jev.text_calls,
        "elapsed_ms": jev.elapsed_ms,
        "usage": jev.usage,
    });
    let tools = || json!(handover::tool_names());
    match report {
        Report::None => {}
        Report::Handover { trigger, why } => {
            let mut fallback = json!({
                "mode": mode.name(),
                "trigger": trigger::recorded(*trigger),
                "ran": false,
                "tools": tools(),
                "tab": tab,
            });
            if let Some(why) = why {
                fallback["not_run_because"] = json!(why);
            }
            data["fallback"] = fallback;
            data["drivers"] = json!([jev_driver]);
        }
        Report::Ran { trigger, outcome } => {
            let mut fallback = json!({
                "mode": mode.name(),
                "trigger": trigger::recorded(*trigger),
                "ran": true,
                "tab": tab,
            });
            if let Value::Object(fields) =
                serde_json::to_value(outcome.as_ref()).unwrap_or_default()
                && let Some(object) = fallback.as_object_mut()
            {
                object.extend(fields);
            }
            if outcome.status != JevStatus::Done {
                fallback["tools"] = tools();
            }
            data["fallback"] = fallback;
            data["drivers"] = json!([jev_driver, {
                "driver": "fallback",
                "model": outcome.model,
                "status": outcome.status,
                "actions": outcome.actions.len(),
                "model_calls": outcome.model_calls,
                "elapsed_ms": outcome.elapsed_ms,
                "usage": outcome.usage,
            }]);
            data["jev_status"] = data["status"].take();
            data["status"] = json!(outcome.status);
            data["stopped_because"] = json!(outcome.stopped_because);
            data["elapsed_ms"] = json!(jev.elapsed_ms + outcome.elapsed_ms);
            match final_page {
                Some(page) => {
                    data["url"] = json!(page.url);
                    data["title"] = json!(page.title);
                    data["visible_text"] = json!(page.visible_text);
                    data["controls"] = page.controls;
                    data["page"] = page.page;
                    data["observed_elements"] = json!(page.observed_elements);
                }
                None => {
                    if let Some(page) = &outcome.last_page {
                        data["url"] = page["url"].clone();
                        data["title"] = page["title"].clone();
                        data["visible_text"] = page["text"].clone();
                        data["controls"] = json!([]);
                        data["page"] = json!(null);
                    }
                }
            }
        }
    }
}
