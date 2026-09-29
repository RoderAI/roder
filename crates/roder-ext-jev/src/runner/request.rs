//! What one `jev_browse` call asks for.
//!
//! Roder sends every property of a tool's schema, so a caller that means to
//! leave an argument out sends its empty value instead: `null`, `""` or `[]`
//! all read as "not given", and every optional argument has one that is
//! harmless (`url: ""` continues on the current page, `tab: "current"` is
//! the default).

use std::time::Duration;

use anyhow::{Context, ensure};
use reqwest::Url;
use serde_json::Value;

use super::{Ceilings, env_value};
use crate::fallback::FallbackSettings;
use crate::scope::JevOriginScope;

/// Which of the thread's tabs a call acts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabChoice {
    /// The session's current tab, opened on the first call.
    Current,
    /// A new tab beside the session's others, made current.
    New,
    /// Close the session's tabs and start over.
    Reset,
    /// Close the session's tabs and stop, browsing nothing.
    Close,
}

impl TabChoice {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::New => "new",
            Self::Reset => "reset",
            Self::Close => "close",
        }
    }

    fn parse(raw: &str) -> anyhow::Result<Self> {
        Ok(match raw.trim().to_ascii_lowercase().as_str() {
            "current" => Self::Current,
            "new" => Self::New,
            "reset" => Self::Reset,
            "close" => Self::Close,
            _ => anyhow::bail!("tab must be \"current\", \"new\", \"reset\" or \"close\""),
        })
    }
}

#[derive(Debug)]
pub(crate) struct JevRequest {
    pub(crate) goal: String,
    /// Where to load, or `None` to go on from the page the tab shows.
    pub(crate) url: Option<String>,
    pub(crate) tab: TabChoice,
    pub(crate) timeout: Duration,
    pub(crate) foreground: bool,
    pub(crate) wait_ms: u64,
    /// `JEV_ALLOWED_ORIGINS`: any origin unless the operator set it.
    pub(crate) scope: JevOriginScope,
    /// `JEV_MAX_ACTIONS`, when the operator set it.
    pub(crate) max_actions: Option<usize>,
    /// `JEV_CONFIRM_IRREVERSIBLE`: stop before actions that cannot be undone.
    pub(crate) confirm_irreversible: bool,
    /// The call's `authorize_irreversible`.
    pub(crate) authorize_irreversible: bool,
    /// `JEV_REFUSE_COOKIE_BANNERS`, on unless set to 0.
    pub(crate) refuse_cookie_banners: bool,
    /// `JEV_FALLBACK` and its model and ceilings.
    pub(crate) fallback: FallbackSettings,
    /// What was left of the host's deadline when the call began, which the
    /// fallback runs within too.
    pub(crate) host_seconds: Option<u64>,
}

/// An argument, unless it is absent or empty: `null`, a blank string, an
/// empty list or `0`.
fn given<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    args.get(key).filter(|value| match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Number(number) => number.as_u64() != Some(0),
        _ => true,
    })
}

impl JevRequest {
    pub(crate) fn parse(args: &Value) -> anyhow::Result<Self> {
        Self::parse_with(args, &Ceilings::from_env()?)
    }

    pub(crate) fn parse_with(args: &Value, ceilings: &Ceilings) -> anyhow::Result<Self> {
        let tab = match given(args, "tab") {
            Some(tab) => TabChoice::parse(tab.as_str().context("tab must be a string")?)?,
            None => TabChoice::Current,
        };
        let url = given(args, "url")
            .map(|url| start_url(url.as_str().context("url must be a string")?))
            .transpose()?;
        let goal = match given(args, "goal") {
            Some(goal) => goal.as_str().context("goal must be a string")?.trim(),
            None => "",
        };
        ensure!(
            !goal.is_empty() || tab == TabChoice::Close,
            "jev_browse requires a nonempty goal"
        );
        ensure!(
            url.is_some() || !matches!(tab, TabChoice::New | TabChoice::Reset),
            "tab {:?} needs a url",
            tab.name()
        );
        let seconds = match given(args, "timeout_seconds") {
            Some(value) => value
                .as_u64()
                .context("timeout_seconds must be an integer")?,
            None => 120,
        };
        ensure!(
            (1..=300).contains(&seconds),
            "timeout_seconds must be 1–300"
        );
        let foreground = match given(args, "foreground") {
            Some(value) => value.as_bool().context("foreground must be a boolean")?,
            None => true,
        };
        let scope = ceilings.scope.clone();
        if let Some(url) = &url {
            ensure!(
                scope.allows(url),
                "the url is outside the operator's allowed origins ({scope})"
            );
        }
        let authorize_irreversible = match given(args, "authorize_irreversible") {
            Some(value) => value
                .as_bool()
                .context("authorize_irreversible must be a boolean")?,
            None => false,
        };
        let mut timeout = Duration::from_secs(seconds);
        if let Some(max) = ceilings.max_seconds {
            timeout = timeout.min(max);
        }
        Ok(Self {
            goal: goal.into(),
            url,
            tab,
            timeout,
            foreground,
            wait_ms: wait_ms(env_value("JEV_WAIT_MS").as_deref()),
            scope,
            max_actions: ceilings.max_actions,
            confirm_irreversible: ceilings.confirm_irreversible,
            authorize_irreversible,
            refuse_cookie_banners: ceilings.refuse_cookie_banners,
            fallback: ceilings.fallback.clone(),
            host_seconds: None,
        })
    }

    /// Hold the whole task to what is left of the host's own deadline, when
    /// that is sooner than the requested timeout.
    pub(crate) fn within(mut self, deadline_remaining_seconds: Option<u64>) -> Self {
        if let Some(seconds) = deadline_remaining_seconds {
            self.timeout = self.timeout.min(Duration::from_secs(seconds));
        }
        self.host_seconds = deadline_remaining_seconds;
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

/// How long a `wait` action holds the page. Upstream sleeps 100ms, which is
/// shorter than most applications take to answer a click, so Roder waits longer
/// by default and lets `JEV_WAIT_MS` tune it.
pub(crate) fn wait_ms(raw: Option<&str>) -> u64 {
    const DEFAULT_WAIT_MS: u64 = 800;
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(100, 10_000))
        .unwrap_or(DEFAULT_WAIT_MS)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(args: Value) -> anyhow::Result<JevRequest> {
        JevRequest::parse_with(&args, &Ceilings::default())
    }

    #[test]
    fn validates_bounded_browser_request() {
        assert!(parse(json!({"url":"https://example.com","goal":"Find the title"})).is_ok());
        assert!(parse(json!({"url":"javascript:alert(1)","goal":"Click"})).is_err());
        for hostless in [
            "https://",
            "http://",
            "http:",
            "https://?q",
            "file:///etc/hosts",
        ] {
            let error = parse(json!({"url": hostless, "goal": "Read"}))
                .err()
                .unwrap_or_else(|| panic!("{hostless} was accepted"));
            assert!(
                error.to_string().contains("http(s) URL with a host"),
                "{error}"
            );
        }
        // The URL is loaded as parsed.
        let parsed = parse(json!({"url":" HTTPS://Example.com ","goal":"Read"})).unwrap();
        assert_eq!(parsed.url.as_deref(), Some("https://example.com/"));
        assert!(parse(json!({"url":"https://example.com","goal":" "})).is_err());
        assert!(
            parse(json!({"url":"https://example.com","goal":"Click","timeout_seconds":301}))
                .is_err()
        );
    }

    /// Roder sends every property, so each optional one has an empty value
    /// that means "not given".
    #[test]
    fn empty_optional_values_mean_absent() {
        for empty in [json!(null), json!(""), json!("  "), json!([])] {
            let request = parse(json!({
                "goal": "Read", "url": empty, "tab": empty, "foreground": empty,
                "timeout_seconds": empty, "authorize_irreversible": empty,
            }))
            .unwrap();
            assert_eq!(request.url, None, "{empty}");
            assert_eq!(request.tab, TabChoice::Current);
            assert!(request.foreground);
            assert_eq!(request.timeout, Duration::from_secs(120));
            assert!(!request.authorize_irreversible);
        }
        let zero = parse(json!({"goal": "Read", "timeout_seconds": 0})).unwrap();
        assert_eq!(zero.timeout, Duration::from_secs(120));
        let current = parse(json!({"goal": "Read", "url": "", "tab": "current"})).unwrap();
        assert_eq!((current.url, current.tab), (None, TabChoice::Current));
    }

    #[test]
    fn tab_choices_and_what_each_needs() {
        let tab = |args: Value| parse(args).map(|request| request.tab);
        assert_eq!(
            tab(json!({"goal":"Read","url":"https://a.test","tab":"new"})).unwrap(),
            TabChoice::New
        );
        assert_eq!(
            tab(json!({"goal":"Read","url":"https://a.test","tab":" Reset "})).unwrap(),
            TabChoice::Reset
        );
        // Closing browses nothing, so it needs no goal and ignores a url.
        assert_eq!(
            tab(json!({"goal":"","url":"","tab":"close"})).unwrap(),
            TabChoice::Close
        );
        for needs_url in ["new", "reset"] {
            let error = tab(json!({"goal":"Read","url":"","tab": needs_url}))
                .unwrap_err()
                .to_string();
            assert_eq!(error, format!("tab \"{needs_url}\" needs a url"));
        }
        assert!(tab(json!({"goal":"Read","tab":"t2"})).is_err());
        assert!(tab(json!({"goal":"Read","tab":true})).is_err());
        assert!(tab(json!({"goal":"","tab":"current"})).is_err());
    }

    #[test]
    fn the_hosts_deadline_caps_the_task_timeout() {
        let request = || {
            parse(json!({"url":"https://example.com","goal":"Read","timeout_seconds":60})).unwrap()
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
        // A continuing call has no url to check here; the loop checks the
        // page it is on.
        assert!(parse(json!({"url":"","goal":"Read"})).is_ok());
    }

    #[test]
    fn operator_scope_still_blocks_start_url() {
        let ceilings = Ceilings::parse(Some("https://*.example.com"), None, None).unwrap();
        let error =
            JevRequest::parse_with(&json!({"url":"https://evil.test","goal":"Read"}), &ceilings)
                .unwrap_err()
                .to_string();
        assert!(
            error.contains("outside the operator's allowed origins"),
            "{error}"
        );
        // Unset, any origin goes.
        assert!(parse(json!({"url":"https://evil.test","goal":"Read"})).is_ok());
    }

    /// There is no per-call origin list: the schema has no such argument,
    /// and only the operator's scope applies.
    #[test]
    fn a_call_names_no_origins() {
        let request = parse(json!({"url":"https://a.test","goal":"Read"})).unwrap();
        assert!(!request.scope.is_restricted());
        assert!(
            !crate::tools::jev_tool_spec().parameters["properties"]
                .as_object()
                .unwrap()
                .contains_key("allowed_origins")
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
        let default = parse(json!({"url":"https://example.com","goal":"Read"})).unwrap();
        assert!(default.foreground);
        let background =
            parse(json!({"url":"https://example.com","goal":"Read","foreground":false})).unwrap();
        assert!(!background.foreground);
        assert!(
            parse(json!({"url":"https://example.com","goal":"Read","foreground":"yes"})).is_err()
        );
    }
}
