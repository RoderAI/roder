//! Running one bounded browser task.
//!
//! Resolves everything a task needs from Roder — the Chrome endpoint, the
//! decision key, the text model — then drives the ported agent loop and
//! reports the observed trace.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, ensure};
use roder_api::inference::ModelSelection;
use serde_json::{Map, Value, json};

use crate::cdp::Connection;
use crate::chrome::{self, ChromeEndpoint};
use crate::decide::JevTypeSafeDecisionClient;
use crate::engine::{JevEngine, JevEngineConfig, JevTextValueResolver};
use crate::page::Page;
use crate::text_model::{self, RoderKeys, TextModel};

pub(crate) struct JevRequest {
    url: String,
    goal: String,
    timeout: Duration,
    foreground: bool,
    wait_ms: u64,
}

impl JevRequest {
    pub(crate) fn parse(args: &Value) -> anyhow::Result<Self> {
        let url = args["url"].as_str().context("jev_browse requires url")?;
        ensure!(
            url.starts_with("https://") || url.starts_with("http://"),
            "jev_browse requires an http(s) URL"
        );
        let goal = args["goal"]
            .as_str()
            .context("jev_browse requires goal")?
            .trim();
        ensure!(!goal.is_empty(), "jev_browse requires a nonempty goal");
        let seconds = match args.get("timeout_seconds") {
            Some(value) => value
                .as_u64()
                .context("timeout_seconds must be an integer")?,
            None => 120,
        };
        ensure!(
            (1..=300).contains(&seconds),
            "timeout_seconds must be 1–300"
        );
        let foreground = match args.get("foreground") {
            Some(Value::Null) | None => true,
            Some(value) => value.as_bool().context("foreground must be a boolean")?,
        };
        Ok(Self {
            url: url.into(),
            goal: goal.into(),
            timeout: Duration::from_secs(seconds),
            foreground,
            wait_ms: wait_ms(env_value("JEV_WAIT_MS").as_deref()),
        })
    }
}

fn key_from_env_or_config() -> Option<String> {
    std::env::var("JEV_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| roder_config::provider_api_key("jev"))
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn resolve_text_model(turn_model: Option<&ModelSelection>) -> Option<TextModel> {
    text_model::resolve(
        env_value("JEV_TEXT_MODEL_API_KEY").or_else(|| env_value("OPENROUTER_API_KEY")),
        env_value("JEV_TEXT_MODEL_BASE_URL"),
        env_value("JEV_TEXT_MODEL"),
        turn_model,
        &RoderKeys,
    )
}

/// How long a `wait` action holds the page. Upstream sleeps 100ms, which is
/// shorter than most applications take to answer a click, so Roder waits longer
/// by default and lets `JEV_WAIT_MS` tune it.
fn wait_ms(raw: Option<&str>) -> u64 {
    const DEFAULT_WAIT_MS: u64 = 800;
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(100, 10_000))
        .unwrap_or(DEFAULT_WAIT_MS)
}

pub(crate) async fn run(
    request: JevRequest,
    turn_model: Option<&ModelSelection>,
) -> anyhow::Result<Value> {
    let key = key_from_env_or_config().context("JEV_API_KEY is required for jev_browse")?;
    let endpoint = chrome::ensure().await?;
    let text = resolve_text_model(turn_model);
    let connection = Connection::connect(
        endpoint
            .url()
            .context("no Chrome DevTools endpoint for jev_browse")?,
    )
    .await?;

    let page = Page::open(connection, &request.url).await?;
    let decision = Arc::new(JevTypeSafeDecisionClient::new(
        key,
        env_value("JEV_MODEL").unwrap_or_else(|| "jev-latest".into()),
    ));
    let mut config = JevEngineConfig::new(request.goal.clone(), decision)
        .with_wait(Duration::from_millis(request.wait_ms));
    if let Some(text) = text.clone() {
        let resolver: Arc<dyn JevTextValueResolver> = Arc::new(text);
        config = config.with_text_resolver(resolver);
    }
    let mut agent = JevEngine::start(Box::new(page), config).await?;
    if request.foreground {
        // Jev owns a background tab; show it so the work is watchable and the
        // final page stays on screen for verification.
        agent.activate().await?;
    }
    let result = agent.run(request.timeout).await;
    let mut value = serde_json::to_value(result).context("serialize the JEV result")?;
    if !request.foreground {
        agent.close().await.ok();
    }
    annotate(&mut value, &endpoint, text.as_ref(), request.foreground);
    Ok(value)
}

/// Record how the task reached Chrome and which model wrote field values, so a
/// caller can tell a missing text helper from a page Jev could not act on.
fn annotate(
    value: &mut Value,
    endpoint: &ChromeEndpoint,
    text: Option<&TextModel>,
    foreground: bool,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let mut browser = Map::new();
    browser.insert("foreground".into(), json!(foreground));
    browser.insert("launched_by_roder".into(), json!(endpoint.launched()));
    if let Some(url) = endpoint.url() {
        browser.insert("cdp_url".into(), json!(url));
    }
    object.insert("browser".into(), Value::Object(browser));
    object.insert(
        "text_model".into(),
        match text {
            Some(text) => json!({"model": text.model, "source": text.source}),
            None => json!(null),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bounded_browser_request() {
        assert!(
            JevRequest::parse(&json!({"url":"https://example.com","goal":"Find the title"}))
                .is_ok()
        );
        assert!(JevRequest::parse(&json!({"url":"javascript:alert(1)","goal":"Click"})).is_err());
        assert!(JevRequest::parse(&json!({"url":"https://example.com","goal":" "})).is_err());
        assert!(
            JevRequest::parse(
                &json!({"url":"https://example.com","goal":"Click","timeout_seconds":301})
            )
            .is_err()
        );
    }

    #[test]
    fn wait_action_holds_longer_than_upstreams_hundred_milliseconds() {
        assert_eq!(wait_ms(None), 800);
        assert_eq!(wait_ms(Some(" 1500 ")), 1500);
        assert_eq!(wait_ms(Some("10")), 100);
        assert_eq!(wait_ms(Some("60000")), 10_000);
        assert_eq!(wait_ms(Some("soon")), 800);
    }

    #[test]
    fn browser_runs_in_the_foreground_unless_asked_otherwise() {
        let default =
            JevRequest::parse(&json!({"url":"https://example.com","goal":"Read"})).unwrap();
        assert!(default.foreground);
        let background = JevRequest::parse(
            &json!({"url":"https://example.com","goal":"Read","foreground":false}),
        )
        .unwrap();
        assert!(!background.foreground);
        assert!(
            JevRequest::parse(
                &json!({"url":"https://example.com","goal":"Read","foreground":"yes"})
            )
            .is_err()
        );
    }

    #[test]
    fn annotation_reports_browser_and_text_model_provenance() {
        let mut value = json!({"status":"done"});
        let endpoint = ChromeEndpoint::Attached {
            url: "http://127.0.0.1:9222".into(),
            launched: true,
        };
        let text = TextModel {
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            api_key: "sk-deepseek".into(),
            source: "turn-model",
            reasoning_none: false,
        };
        annotate(&mut value, &endpoint, Some(&text), true);
        assert_eq!(value["browser"]["launched_by_roder"], json!(true));
        assert_eq!(value["browser"]["cdp_url"], json!("http://127.0.0.1:9222"));
        assert_eq!(value["browser"]["foreground"], json!(true));
        assert_eq!(value["text_model"]["model"], json!("deepseek-chat"));
        assert_eq!(value["text_model"]["source"], json!("turn-model"));
        assert!(!value.to_string().contains("sk-deepseek"));
    }

    #[test]
    fn missing_text_helper_is_reported_as_null() {
        let mut value = json!({"status":"blocked"});
        annotate(&mut value, &ChromeEndpoint::Inherited, None, false);
        assert_eq!(value["text_model"], json!(null));
        assert_eq!(value["browser"]["launched_by_roder"], json!(false));
        assert!(value["browser"].get("cdp_url").is_none());
    }

    #[test]
    fn secrets_never_reach_the_reported_result() {
        // The port makes its own HTTP calls, so a key can only leak through the
        // annotated result; it carries the model name, never the key.
        let mut value = json!({"status":"done"});
        let text = TextModel {
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            api_key: "sk-secret-value".into(),
            source: "roder-provider",
            reasoning_none: false,
        };
        annotate(&mut value, &ChromeEndpoint::Inherited, Some(&text), true);
        assert!(!value.to_string().contains("sk-secret-value"));
    }
}
