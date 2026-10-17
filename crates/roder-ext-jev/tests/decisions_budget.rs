mod support;
use async_trait::async_trait;
use roder_ext_jev::{
    DecisionsBrowserProfile, JevDecisionTransport, JevEngine, JevEngineConfig, JevStatus,
    OpenAiDecisionsClient,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use support::{ScriptedBrowser, labelled_page};

struct Replies {
    first_bad: bool,
    refused_gate: bool,
    calls: Mutex<usize>,
}
#[async_trait]
impl JevDecisionTransport for Replies {
    async fn decide(&self, body: &Value) -> anyhow::Result<Value> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if self.first_bad && *calls == 1 {
            return Ok(json!({"answers":[],"usage":{"input_tokens":42,"output_tokens":3}}));
        }
        let answers=body["questions"].as_array().unwrap().iter().map(|q|{
            let name=q["name"].as_str().unwrap();
            if q["type"]=="predicate" {
                if self.refused_gate && name.starts_with("irreversible_"){return json!({"name":name,"type":"refusal"});}
                return json!({"name":name,"type":"predicate","probability":if name=="performed"{0.3}else{0.99}});
            }
            let choices=q["choices"].as_array().unwrap();
            let choice=if name=="operation" {"DONE"}else{choices[0]["value"].as_str().unwrap()};
            json!({"name":name,"type":"choice","choice":choice,"confidence":0.97,
                "probabilities":choices.iter().map(|c|json!({"value":c["value"],"probability":if c["value"]==choice {0.95}else{0.05/(choices.len()-1) as f64}})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        Ok(json!({"answers":answers,"usage":{"input_tokens":42,"output_tokens":3}}))
    }
}
async fn run(first_bad: bool, refused_gate: bool) -> (roder_ext_jev::JevRunResult, usize, usize) {
    let wire = Arc::new(Replies {
        first_bad,
        refused_gate,
        calls: Mutex::new(0),
    });
    let client = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited)
        .text_only();
    let browser = ScriptedBrowser::new(vec![
        labelled_page(
            "one",
            &[("a", "click", "Publish"), ("b", "click", "Preview")],
        ),
        labelled_page(
            "two",
            &[("a", "click", "Publish"), ("b", "click", "Preview")],
        ),
    ]);
    let log = browser.log();
    let config = JevEngineConfig::new("Activate the first control", Arc::new(client))
        .with_max_actions(1)
        .with_wait(Duration::ZERO);
    let config = if refused_gate {
        config.with_irreversible_gate()
    } else {
        config
    };
    let mut engine = JevEngine::start(Box::new(browser), config).await.unwrap();
    let result = engine.run(Duration::from_secs(5)).await;
    let calls = *wire.calls.lock().unwrap();
    let acts = log.lock().unwrap().acts.len();
    (result, calls, acts)
}
#[tokio::test]
async fn a_two_request_decision_counts_both_calls_before_the_action_cap() {
    let (result, calls, acts) = run(false, false).await;
    assert_eq!(calls, 2);
    assert_eq!(acts, 1);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.usage.decision.calls, 2);
    assert_eq!(result.status, JevStatus::BudgetExceeded);
}
#[tokio::test]
async fn a_billed_retry_leaves_no_budget_for_a_third_provider_request() {
    let (result, calls, acts) = run(true, false).await;
    assert_eq!(calls, 2);
    assert_eq!(acts, 0);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.usage.decision.calls, 2);
    assert_eq!(result.status, JevStatus::BudgetExceeded);
}
#[tokio::test]
async fn a_refused_corrected_gate_counts_the_audit_and_cannot_execute() {
    let (result, calls, acts) = run(false, true).await;
    assert_eq!(calls, 2);
    assert_eq!(acts, 0);
    assert_eq!(result.model_calls, 2);
    assert_eq!(result.usage.decision.calls, 2);
    assert_eq!(result.status, JevStatus::BudgetExceeded);
}
