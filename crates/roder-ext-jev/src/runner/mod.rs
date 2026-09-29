//! Running one bounded browser task.
//!
//! Resolves everything a task needs from Roder — the Chrome endpoint, the
//! decision key, the text model — then runs it on the thread's Jev session
//! (see [`crate::session`]), which drives the ported agent loop in the
//! session's tab and reports the observed trace. One timeout covers the
//! whole task, from waiting for the session and finding Chrome to the last
//! step (see [`drive`]).

mod ceilings;
mod drive;
mod request;
#[cfg(test)]
mod secret_tests;

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use roder_api::inference::ModelSelection;
use serde_json::{Map, Value, json};

use crate::chrome::{self, ChromeEndpoint};
use crate::decide::JevTypeSafeDecisionClient;
use crate::engine::JevTextValueResolver;
use crate::session::{JevSessions, SessionDeps, SessionModels};
use crate::text_helper::TextHelper;
use crate::text_model::{self, Effort, Explicit, RoderCodexSignIn, RoderKeys, TextModel};
pub(crate) use ceilings::Ceilings;
#[cfg(test)]
pub(crate) use ceilings::switch;
pub(crate) use drive::{Driven, Task, close_all, close_quietly, drive, timed_out};
pub(crate) use request::{JevRequest, TabChoice};

/// Why a call with no url on a thread without a tab did nothing.
pub(crate) const NO_TAB_YET: &str = "Nothing was browsed: this thread has no Jev tab yet, so \
     there is no page to continue on. Call jev_browse again with the same goal and url set to \
     the http(s) page to start on, such as the site's search or listing page for the goal; \
     later calls can then leave url empty.";

fn key_from_env_or_config() -> Option<String> {
    std::env::var("JEV_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| roder_config::provider_api_key("jev"))
}

pub(crate) fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// The text helper from the environment and Roder's providers (see
/// [`text_model::resolve`]). A malformed `JEV_TEXT_MODEL_REASONING`, or an
/// explicit model Roder cannot serve, fails the call.
pub(crate) async fn resolve_text_model(
    turn_model: Option<&ModelSelection>,
) -> anyhow::Result<Option<TextModel>> {
    let explicit = Explicit {
        key: env_value("JEV_TEXT_MODEL_API_KEY").or_else(|| env_value("OPENROUTER_API_KEY")),
        base_url: env_value("JEV_TEXT_MODEL_BASE_URL"),
        model: env_value("JEV_TEXT_MODEL"),
        reasoning: env_value("JEV_TEXT_MODEL_REASONING")
            .map(|raw| Effort::parse(&raw))
            .transpose()?,
    };
    text_model::resolve(explicit, turn_model, &RoderKeys, &RoderCodexSignIn).await
}

/// Roder's own models and browser: `JEV_API_KEY`, the text helper resolved
/// from the turn's model, and the Chrome [`chrome::ensure`] finds or starts.
struct RoderDeps<'a> {
    turn_model: Option<&'a ModelSelection>,
}

#[async_trait]
impl SessionDeps for RoderDeps<'_> {
    fn model_key(&self) -> String {
        self.turn_model
            .map(|model| format!("{}/{}", model.provider, model.model))
            .unwrap_or_default()
    }

    async fn models(&self) -> anyhow::Result<SessionModels> {
        let key = key_from_env_or_config().context("JEV_API_KEY is required for jev_browse")?;
        let text = resolve_text_model(self.turn_model).await?;
        let decision = Arc::new(JevTypeSafeDecisionClient::new(
            key,
            env_value("JEV_MODEL").unwrap_or_else(|| "jev-latest".into()),
        ));
        let helper = text.map(|text| Arc::new(TextHelper::new(text)));
        Ok(SessionModels {
            key: self.model_key(),
            decision,
            text: helper
                .clone()
                .map(|helper| helper as Arc<dyn JevTextValueResolver>),
            helper,
        })
    }

    async fn endpoint(&self) -> anyhow::Result<ChromeEndpoint> {
        chrome::ensure().await
    }
}

/// Run one call on `thread`'s Jev session.
pub(crate) async fn run(
    request: JevRequest,
    thread: &str,
    turn_model: Option<&ModelSelection>,
) -> anyhow::Result<Value> {
    let sessions = JevSessions::global();
    sessions.start_sweeper();
    sessions
        .call(thread, request, &RoderDeps { turn_model })
        .await
}

/// Record how the task reached Chrome and which model wrote field values, so a
/// caller can tell a missing text helper from a page Jev could not act on.
/// The endpoint is reported only as far as [`ChromeEndpoint::reported_url`]
/// allows, and not at all when the task timed out before finding one.
pub(crate) fn annotate(
    value: &mut Value,
    endpoint: Option<&ChromeEndpoint>,
    text: Option<&TextModel>,
    foreground: bool,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let mut browser = Map::new();
    browser.insert("foreground".into(), json!(foreground));
    browser.insert(
        "launched_by_roder".into(),
        json!(endpoint.is_some_and(ChromeEndpoint::launched)),
    );
    if let Some(endpoint) = endpoint {
        browser.insert("cdp_url".into(), json!(endpoint.reported_url()));
    }
    object.insert("browser".into(), Value::Object(browser));
    object.insert(
        "text_model".into(),
        match text {
            Some(text) => {
                let mut provenance = json!({
                    "model": text.model,
                    "effort": text.effort(),
                    "source": text.source,
                });
                if let Some(note) = text.note {
                    provenance["note"] = json!(note);
                }
                provenance
            }
            None => json!(null),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_reports_browser_and_text_model_provenance() {
        let mut value = json!({"status":"done"});
        let endpoint = ChromeEndpoint::new("http://127.0.0.1:9222", true);
        let text = TextModel {
            model: "deepseek-chat".into(),
            source: "turn-model",
            note: None,
            fallback: None,
            transport: crate::text_model::Transport::Chat {
                base_url: "https://api.deepseek.com/v1".into(),
                api_key: "sk-deepseek".into(),
                reasoning: None,
            },
        };
        annotate(&mut value, Some(&endpoint), Some(&text), true);
        assert_eq!(value["browser"]["launched_by_roder"], json!(true));
        assert_eq!(value["browser"]["cdp_url"], json!("http://127.0.0.1:9222"));
        assert_eq!(value["browser"]["foreground"], json!(true));
        assert_eq!(value["text_model"]["model"], json!("deepseek-chat"));
        assert_eq!(value["text_model"]["source"], json!("turn-model"));
        assert_eq!(value["text_model"]["effort"], json!("none"));
        let codex = TextModel {
            model: "gpt-6-sol".into(),
            source: "codex",
            note: None,
            fallback: None,
            transport: crate::text_model::Transport::Codex {
                effort: Effort::Low,
            },
        };
        annotate(&mut value, Some(&endpoint), Some(&codex), true);
        assert_eq!(
            value["text_model"],
            json!({"model": "gpt-6-sol", "effort": "low", "source": "codex"})
        );
        // A stand-in for an unusable sign-in names itself and says why.
        let stand_in = text.clone().standing_in();
        annotate(&mut value, Some(&endpoint), Some(&stand_in), true);
        assert_eq!(value["text_model"]["model"], json!("deepseek-chat"));
        assert_eq!(
            value["text_model"]["note"],
            json!(crate::text_model::CODEX_UNUSABLE)
        );
        assert!(!value.to_string().contains("sk-deepseek"));
    }

    #[test]
    fn missing_text_helper_is_reported_as_null() {
        let mut value = json!({"status":"blocked"});
        let endpoint = ChromeEndpoint::new("http://127.0.0.1:9222", false);
        annotate(&mut value, Some(&endpoint), None, false);
        assert_eq!(value["text_model"], json!(null));
        assert_eq!(value["browser"]["launched_by_roder"], json!(false));
    }

    #[test]
    fn a_remote_endpoint_is_reported_without_its_token() {
        let mut value = json!({"status":"done"});
        let endpoint = ChromeEndpoint::new(
            "wss://browser.example.com/devtools/browser?token=cdp-secret",
            false,
        );
        annotate(&mut value, Some(&endpoint), None, true);
        assert_eq!(
            value["browser"]["cdp_url"],
            json!("wss://browser.example.com")
        );
        assert!(!value.to_string().contains("cdp-secret"));
    }

    #[test]
    fn a_task_that_never_found_chrome_reports_no_endpoint() {
        let mut value = json!({"status":"timed_out"});
        annotate(&mut value, None, None, true);
        assert_eq!(value["browser"]["launched_by_roder"], json!(false));
        assert!(value["browser"].get("cdp_url").is_none());
    }
}
