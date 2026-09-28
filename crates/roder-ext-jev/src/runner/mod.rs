//! Running one bounded browser task.
//!
//! Resolves everything a task needs from Roder — the Chrome endpoint, the
//! decision key, the text model — then drives the ported agent loop and
//! reports the observed trace. One timeout covers the whole task, from
//! finding Chrome to the last step (see [`drive`]).

mod ceilings;
mod drive;
#[cfg(test)]
mod secret_tests;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, ensure};
use reqwest::Url;
use roder_api::inference::ModelSelection;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use crate::chrome::{self, ChromeEndpoint};
use crate::decide::JevTypeSafeDecisionClient;
use crate::engine::{JevEngineConfig, JevRunResult, JevStatus, JevTextValueResolver};
use crate::scope::JevOriginScope;
use crate::text_helper::TextHelper;
use crate::text_model::{self, RoderKeys, TextModel};
pub(crate) use ceilings::Ceilings;
#[cfg(test)]
pub(crate) use ceilings::switch;
pub(crate) use drive::{Task, drive};

#[derive(Debug)]
pub(crate) struct JevRequest {
    url: String,
    goal: String,
    timeout: Duration,
    foreground: bool,
    wait_ms: u64,
    /// The operator's allowed origins, narrowed by the call's own list.
    scope: JevOriginScope,
    /// `JEV_MAX_ACTIONS`, when the operator set it.
    max_actions: Option<usize>,
    /// `JEV_CONFIRM_IRREVERSIBLE`: stop before actions that cannot be undone.
    confirm_irreversible: bool,
    /// The call's `authorize_irreversible`.
    authorize_irreversible: bool,
    /// `JEV_REFUSE_COOKIE_BANNERS`, on unless set to 0.
    refuse_cookie_banners: bool,
}

impl JevRequest {
    pub(crate) fn parse(args: &Value) -> anyhow::Result<Self> {
        Self::parse_with(args, &Ceilings::from_env()?)
    }

    fn parse_with(args: &Value, ceilings: &Ceilings) -> anyhow::Result<Self> {
        let url = start_url(args["url"].as_str().context("jev_browse requires url")?)?;
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
        let mut scope = ceilings.scope.clone();
        match args.get("allowed_origins") {
            Some(Value::Null) | None => {}
            Some(Value::Array(origins)) => {
                let origins = origins
                    .iter()
                    .map(|origin| origin.as_str().context("allowed_origins must be strings"))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                scope = scope.narrow(&origins)?;
            }
            Some(_) => anyhow::bail!("allowed_origins must be a list of origins"),
        }
        ensure!(
            scope.allows(&url),
            "the start URL is outside the allowed origins ({scope})"
        );
        let authorize_irreversible = match args.get("authorize_irreversible") {
            Some(Value::Null) | None => false,
            Some(value) => value
                .as_bool()
                .context("authorize_irreversible must be a boolean")?,
        };
        let mut timeout = Duration::from_secs(seconds);
        if let Some(max) = ceilings.max_seconds {
            timeout = timeout.min(max);
        }
        Ok(Self {
            url,
            goal: goal.into(),
            timeout,
            foreground,
            wait_ms: wait_ms(env_value("JEV_WAIT_MS").as_deref()),
            scope,
            max_actions: ceilings.max_actions,
            confirm_irreversible: ceilings.confirm_irreversible,
            authorize_irreversible,
            refuse_cookie_banners: ceilings.refuse_cookie_banners,
        })
    }

    /// Hold the whole task to what is left of the host's own deadline, when
    /// that is sooner than the requested timeout.
    pub(crate) fn within(mut self, deadline_remaining_seconds: Option<u64>) -> Self {
        if let Some(seconds) = deadline_remaining_seconds {
            self.timeout = self.timeout.min(Duration::from_secs(seconds));
        }
        self
    }
}

/// An http(s) URL with a host, as the browser will load it.
fn start_url(raw: &str) -> anyhow::Result<String> {
    let url = Url::parse(raw.trim()).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some_and(|host| !host.is_empty())
    });
    Ok(url
        .context("jev_browse requires an http(s) URL with a host")?
        .to_string())
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

pub(crate) fn resolve_text_model(turn_model: Option<&ModelSelection>) -> Option<TextModel> {
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
    let started = Instant::now();
    let deadline = started + request.timeout;
    let key = key_from_env_or_config().context("JEV_API_KEY is required for jev_browse")?;
    let text = resolve_text_model(turn_model);
    let decision = Arc::new(JevTypeSafeDecisionClient::new(
        key,
        env_value("JEV_MODEL").unwrap_or_else(|| "jev-latest".into()),
    ));
    let mut config = JevEngineConfig::new(request.goal.clone(), decision)
        .with_wait(Duration::from_millis(request.wait_ms))
        .with_scope(request.scope.clone());
    if let Some(actions) = request.max_actions {
        config = config.with_max_actions(actions);
    }
    if request.confirm_irreversible {
        config = config.with_irreversible_gate();
    }
    if request.authorize_irreversible {
        config = config.with_irreversible_authorized();
    }
    config = config.with_cookie_banner_refusal(request.refuse_cookie_banners);
    if let Some(text) = text.clone() {
        let resolver: Arc<dyn JevTextValueResolver> = Arc::new(TextHelper::new(text));
        config = config.with_text_resolver(resolver);
    }

    // Starting Chrome counts against the task's timeout like everything else.
    let (result, endpoint) = match tokio::time::timeout_at(deadline, chrome::ensure()).await {
        Ok(endpoint) => {
            let endpoint = endpoint?;
            let task = Task {
                url: request.url.clone(),
                foreground: request.foreground,
                started,
                deadline,
                config,
            };
            (drive(endpoint.url(), task).await?, Some(endpoint))
        }
        Err(_) => {
            let result = JevRunResult::before_start(
                JevStatus::TimedOut,
                &request.url,
                started.elapsed(),
                drive::timed_out("starting Chrome"),
            );
            (result, None)
        }
    };
    let mut value = serde_json::to_value(result).context("serialize the JEV result")?;
    annotate(
        &mut value,
        endpoint.as_ref(),
        text.as_ref(),
        request.foreground,
    );
    Ok(value)
}

/// Record how the task reached Chrome and which model wrote field values, so a
/// caller can tell a missing text helper from a page Jev could not act on.
/// The endpoint is reported only as far as [`ChromeEndpoint::reported_url`]
/// allows, and not at all when the task timed out before finding one.
fn annotate(
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
        for hostless in [
            "https://",
            "http://",
            "http:",
            "https://?q",
            "file:///etc/hosts",
        ] {
            let error = JevRequest::parse(&json!({"url": hostless, "goal": "Read"}))
                .err()
                .unwrap_or_else(|| panic!("{hostless} was accepted"));
            assert!(
                error.to_string().contains("http(s) URL with a host"),
                "{error}"
            );
        }
        // The URL is loaded as parsed.
        let parsed =
            JevRequest::parse(&json!({"url":" HTTPS://Example.com ","goal":"Read"})).unwrap();
        assert_eq!(parsed.url, "https://example.com/");
        assert!(JevRequest::parse(&json!({"url":"https://example.com","goal":" "})).is_err());
        assert!(
            JevRequest::parse(
                &json!({"url":"https://example.com","goal":"Click","timeout_seconds":301})
            )
            .is_err()
        );
    }

    #[test]
    fn the_hosts_deadline_caps_the_task_timeout() {
        let request = || {
            JevRequest::parse(
                &json!({"url":"https://example.com","goal":"Read","timeout_seconds":60}),
            )
            .unwrap()
        };
        assert_eq!(request().within(None).timeout, Duration::from_secs(60));
        assert_eq!(request().within(Some(15)).timeout, Duration::from_secs(15));
        assert_eq!(request().within(Some(600)).timeout, Duration::from_secs(60));
        assert_eq!(request().within(Some(0)).timeout, Duration::ZERO);
    }

    #[test]
    fn the_operators_ceilings_bound_every_call() {
        let ceilings =
            Ceilings::parse(Some("https://*.example.com"), Some("12"), Some("30")).unwrap();
        let parse = |args: Value| JevRequest::parse_with(&args, &ceilings);
        let request = parse(json!({"url":"https://shop.example.com","goal":"Read"})).unwrap();
        // The default 120 s is cut to the operator's 30 s.
        assert_eq!(request.timeout, Duration::from_secs(30));
        assert_eq!(request.max_actions, Some(12));
        assert!(request.scope.allows("https://docs.example.com/"));
        assert!(!request.scope.allows("https://evil.test/"));
        let short = parse(json!({"url":"https://example.com","goal":"Read","timeout_seconds":5}));
        assert_eq!(short.unwrap().timeout, Duration::from_secs(5));
        // A call narrows the scope and cannot widen it.
        let narrowed = parse(json!({"url":"https://docs.example.com","goal":"Read",
            "allowed_origins":["https://docs.example.com","https://evil.test"]}))
        .unwrap();
        assert!(!narrowed.scope.allows("https://shop.example.com/"));
        assert!(!narrowed.scope.allows("https://evil.test/"));
        let outside = parse(json!({"url":"https://evil.test","goal":"Read",
            "allowed_origins":["https://evil.test"]}))
        .unwrap_err()
        .to_string();
        assert!(outside.contains("outside the allowed origins"), "{outside}");
        assert!(
            parse(json!({"url":"https://example.com","goal":"Read","allowed_origins":"x"}))
                .is_err()
        );
        assert!(
            parse(json!({"url":"https://example.com","goal":"Read","allowed_origins":[]})).is_err()
        );
    }

    #[test]
    fn the_gate_and_banner_switches_come_from_the_operator() {
        let parse = |args: Value, ceilings: &Ceilings| JevRequest::parse_with(&args, ceilings);
        let defaults = Ceilings::parse(None, None, None).unwrap();
        let request = parse(
            json!({"url":"https://example.com","goal":"Read"}),
            &defaults,
        )
        .unwrap();
        assert!(!request.confirm_irreversible);
        assert!(!request.authorize_irreversible);
        assert!(request.refuse_cookie_banners);
        let off = Ceilings {
            refuse_cookie_banners: false,
            ..Ceilings::default()
        };
        let request = parse(json!({"url":"https://example.com","goal":"Read"}), &off).unwrap();
        assert!(!request.refuse_cookie_banners);
        let on = Ceilings {
            confirm_irreversible: true,
            refuse_cookie_banners: true,
            ..Ceilings::default()
        };
        let authorized = parse(
            json!({"url":"https://example.com","goal":"Pay","authorize_irreversible":true}),
            &on,
        )
        .unwrap();
        assert!(authorized.confirm_irreversible && authorized.refuse_cookie_banners);
        assert!(authorized.authorize_irreversible);
        let error = parse(
            json!({"url":"https://example.com","goal":"Pay","authorize_irreversible":"yes"}),
            &on,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("authorize_irreversible"), "{error}");
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
        let endpoint = ChromeEndpoint::new("http://127.0.0.1:9222", true);
        let text = TextModel {
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            api_key: "sk-deepseek".into(),
            source: "turn-model",
            reasoning_none: false,
        };
        annotate(&mut value, Some(&endpoint), Some(&text), true);
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
