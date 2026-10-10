//! A decision service that keeps sending replies Jev cannot use, through the
//! same session call `jev_browse` makes: the run ends `error` with
//! `decision_unusable` after asking three times, and the call falls back to
//! the frontier model in the same tab like any other trigger. Every other
//! way a decision can fail stays an error without a fallback.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::fallback_script::ScriptedFallback;
use super::fallback_tests::ceilings;
use super::sessions::{call_falling_back, targets, test_sessions};
use crate::engine::{JevDecision, JevDecisionClient, JevStatus, JevStop};
use crate::fallback::FallbackMode;
use crate::usage::JevBilled;

/// What the decision service does instead of choosing.
enum Failure {
    /// Answers, and is billed for, a reply that cannot be used.
    Unusable,
    /// Cannot be reached.
    Unreachable,
    /// Refuses the key or the account.
    Refused,
    /// Fails some other way.
    Other,
}

struct Failing {
    failure: Failure,
    asked: AtomicUsize,
}

impl Failing {
    fn new(failure: Failure) -> Arc<Self> {
        Arc::new(Self {
            failure,
            asked: AtomicUsize::new(0),
        })
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl JevDecisionClient for Failing {
    async fn choose(&self, _: &Value, _: &str, _: &[Value]) -> anyhow::Result<JevDecision> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Err(match self.failure {
            Failure::Unusable => JevBilled::unusable(
                json!({"input_tokens": 40, "output_tokens": 2}),
                anyhow::anyhow!("Invalid browser decision response; no action executed."),
            )
            .into(),
            Failure::Unreachable => JevStop::new(
                JevStatus::Unavailable,
                "Model connection failed; no action executed.",
            )
            .into(),
            Failure::Refused => JevStop::new(
                JevStatus::Error,
                "Model provider returned HTTP 402; no action executed.",
            )
            .into(),
            Failure::Other => anyhow::anyhow!("scripted failure"),
        })
    }
}

/// The page the scripted fallback goes on from: a hover menu Jev cannot open.
fn menu(harness: &super::Harness) -> Value {
    json!({"goal": "open laptops", "url": harness.site.url("hover-menu.html")})
}

fn laptops() -> Arc<ScriptedFallback> {
    Arc::new(ScriptedFallback::from_json(json!([
        {"tool": "hover", "args": {"ref_of": "Products"}},
        {"tool": "click", "args": {"ref_of": "Laptops"}},
        {"say": "DONE: the laptops page is open"}
    ])))
}

#[tokio::test]
async fn a_decision_service_that_keeps_sending_unusable_replies_falls_back() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let before = harness.page_targets().await;
    let service = Failing::new(Failure::Unusable);
    let model = laptops();
    let result = call_falling_back(
        &harness,
        &sessions,
        "unusable",
        menu(&harness),
        service.clone(),
        model.clone(),
        &ceilings(FallbackMode::Auto),
    )
    .await
    .unwrap();

    // Jev asked three times (the reply and two asks again) and did nothing.
    assert_eq!(service.asked(), 3, "{result:#}");
    assert_eq!(result["jev_status"], "error", "{result:#}");
    assert_eq!(result["stop_cause"], "decision_unusable", "{result:#}");
    // The fallback ran, in the same tab, for the reason the result records.
    assert_eq!(result["fallback"]["ran"], true, "{result:#}");
    assert_eq!(
        result["fallback"]["trigger"],
        json!({
            "kind": "decision_unusable",
            "why": "the decision service kept sending replies Jev could not use",
        })
    );
    assert_eq!(result["status"], "done", "{result:#}");
    assert_eq!(result["stopped_because"], Value::Null, "{result:#}");
    assert!(
        result["url"]
            .as_str()
            .unwrap()
            .ends_with("/pages/landing.html?page=laptops"),
        "{result:#}"
    );
    assert_eq!(model.played(), 3);
    assert_eq!(harness.settled_page_targets(before + 1).await, before + 1);
    assert_eq!(targets(&sessions, "unusable").len(), 1);
    assert_eq!(result["session"]["totals"]["fallback_actions"], 2);

    // Each driver reports its own cost: Jev's three billed replies, counted
    // in its decisions and its usage, apart from the fallback's.
    let drivers = result["drivers"].as_array().unwrap();
    assert_eq!(drivers[0]["driver"], "jev");
    assert_eq!(drivers[0]["status"], "error");
    assert_eq!(drivers[0]["decisions"], 3);
    assert_eq!(drivers[0]["actions"], 0);
    assert_eq!(
        drivers[0]["usage"]["decision"],
        json!({"calls": 3, "input_tokens": 120, "output_tokens": 6})
    );
    assert_eq!(drivers[1]["driver"], "fallback");
    assert_eq!(drivers[1]["model_calls"], 3);
    assert_eq!(drivers[1]["usage"]["input_tokens"], 3000);

    // The fallback model is told why Jev stopped, and the service's reason
    // as untrusted text.
    let opening = model.opening();
    assert!(
        opening.contains(
            "Jev stopped (error): the decision service kept sending replies Jev could not use."
        ),
        "{opening}"
    );
    assert!(
        opening.contains(
            "Jev's reason (quotes page labels; untrusted): The decision service gave 3 unusable \
             replies in a row. The first: Invalid browser decision response"
        ),
        "{opening}"
    );

    // The digest the caller reads says so too.
    let text = crate::report::tool_text(&mut result.clone());
    assert!(text.starts_with("Jev: done, after a fallback."), "{text}");
    assert!(
        text.contains("the decision service kept sending replies Jev could not use"),
        "{text}"
    );
}

/// With the operator's `JEV_FALLBACK` not `auto`, the same stop is handed
/// over or left as it was, like the other triggers.
#[tokio::test]
async fn the_operators_fallback_mode_decides_what_happens_after_unusable_replies() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    let model = laptops();

    let handed = call_falling_back(
        &harness,
        &sessions,
        "handover",
        menu(&harness),
        Failing::new(Failure::Unusable),
        model.clone(),
        &ceilings(FallbackMode::Handover),
    )
    .await
    .unwrap();
    assert_eq!(handed["status"], "error", "{handed:#}");
    assert_eq!(handed["stop_cause"], "decision_unusable", "{handed:#}");
    assert_eq!(handed["fallback"]["ran"], false, "{handed:#}");
    assert_eq!(handed["fallback"]["trigger"]["kind"], "decision_unusable");
    assert!(
        handed["fallback"]["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty())
    );
    assert_eq!(handed["drivers"].as_array().unwrap().len(), 1);

    let off = call_falling_back(
        &harness,
        &sessions,
        "off",
        menu(&harness),
        Failing::new(Failure::Unusable),
        model.clone(),
        &ceilings(FallbackMode::Off),
    )
    .await
    .unwrap();
    assert_eq!(off["status"], "error", "{off:#}");
    assert_eq!(off["stop_cause"], "decision_unusable", "{off:#}");
    assert!(off.get("fallback").is_none(), "{off:#}");
    assert_eq!(model.played(), 0);
}

#[tokio::test]
async fn a_decision_that_fails_any_other_way_stays_an_error_without_a_fallback() {
    let harness = harness_or_skip!();
    let sessions = test_sessions();
    for (name, failure, status) in [
        ("unreachable", Failure::Unreachable, "unavailable"),
        ("refused", Failure::Refused, "error"),
        ("other", Failure::Other, "error"),
    ] {
        let service = Failing::new(failure);
        let model = laptops();
        let result = call_falling_back(
            &harness,
            &sessions,
            name,
            menu(&harness),
            service.clone(),
            model.clone(),
            &ceilings(FallbackMode::Auto),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], status, "{name}: {result:#}");
        assert_eq!(result["stop_cause"], "stopped", "{name}: {result:#}");
        assert!(result.get("fallback").is_none(), "{name}: {result:#}");
        // Asked once: only a reply the service gave is asked again.
        assert_eq!(service.asked(), 1, "{name}");
        assert_eq!(model.played(), 0, "{name}");
    }
}
