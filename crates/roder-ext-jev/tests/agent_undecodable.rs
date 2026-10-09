//! A decision service whose body cannot be decoded ("Invalid TypeSafe
//! response"), as a hosted client reports it: an unusable reply of unknown
//! usage. The loop asks again up to twice, counts every call, and ends
//! `decision_unusable` (which falls back) when the body never decodes.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use roder_ext_jev::{
    JevBilled, JevDecision, JevDecisionClient, JevEngine, JevEngineConfig, JevRunResult, JevStatus,
    JevStop, JevStopCause,
};
use serde_json::{Value, json};
use support::{ScriptedBrowser, ScriptedDecider, page};

async fn run(browser: ScriptedBrowser, config: JevEngineConfig) -> JevRunResult {
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    engine.run(Duration::from_secs(5)).await
}

/// A hosted client whose service sometimes answers with a body it cannot
/// decode ("Invalid TypeSafe response"): it marks the reply unusable and, not
/// knowing what the call cost, reports an empty usage.
struct Garbling {
    bad: usize,
    asked: Mutex<usize>,
    good: ScriptedDecider,
}

impl Garbling {
    fn new(bad: usize) -> Self {
        Self {
            bad,
            asked: Mutex::new(0),
            good: ScriptedDecider::new(&["DONE"])
                .with_usage(json!({"input_tokens": 100, "output_tokens": 4})),
        }
    }
}

#[async_trait]
impl JevDecisionClient for Garbling {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let asked = {
            let mut asked = self.asked.lock().unwrap();
            *asked += 1;
            *asked
        };
        if asked <= self.bad {
            return Err(JevBilled::unusable(
                json!({}),
                JevStop::new(
                    JevStatus::Error,
                    "Invalid TypeSafe response; no action executed.",
                )
                .into(),
            )
            .into());
        }
        self.good.choose(observation, goal, history).await
    }
}

/// An undecodable body twice, then a valid reply, ends the run done: three
/// model calls, and a usage that says what it does not know.
#[tokio::test]
async fn an_undecodable_reply_twice_then_a_valid_one_ends_done() {
    let result = run(
        ScriptedBrowser::new(vec![page("one", &[("next", "click")])]),
        JevEngineConfig::new("Scripted goal", Arc::new(Garbling::new(2))).with_wait(Duration::ZERO),
    )
    .await;
    assert_eq!(result.status, JevStatus::Done);
    assert_eq!(result.stopped_because, None);
    assert_eq!(result.model_calls, 3);
    assert_eq!(result.decisions.len(), 1);
    // The calls are counted; their tokens are unknown, never a 0 that would
    // read as free.
    assert_eq!(
        serde_json::to_value(result.usage.decision).unwrap(),
        json!({"calls": 3, "input_tokens": "unknown", "output_tokens": "unknown"})
    );
}

/// A body that never decodes ends the run after three model calls with the
/// cause that routes it to the fallback.
#[tokio::test]
async fn an_always_undecodable_reply_ends_decision_unusable() {
    let result = run(
        ScriptedBrowser::new(vec![page("one", &[("next", "click")])]),
        JevEngineConfig::new("Scripted goal", Arc::new(Garbling::new(usize::MAX)))
            .with_wait(Duration::ZERO),
    )
    .await;
    assert_eq!(result.status, JevStatus::Error);
    assert_eq!(result.stop_cause, Some(JevStopCause::DecisionUnusable));
    assert_eq!(result.model_calls, 3);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some(
            "The decision service gave 3 unusable replies in a row. The first: \
             Invalid TypeSafe response; no action executed."
        )
    );
    assert_eq!(
        serde_json::to_value(result.usage.decision).unwrap(),
        json!({"calls": 3, "input_tokens": "unknown", "output_tokens": "unknown"})
    );
}
