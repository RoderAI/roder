use super::*;
use crate::{JevDecisionClient, JevTypeSafeDecisionClient};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct Replies {
    requests: Mutex<Vec<Value>>,
    replies: Mutex<VecDeque<Result<Value, &'static str>>>,
}
#[async_trait]
impl JevDecisionTransport for Replies {
    async fn decide(&self, body: &Value) -> anyhow::Result<Value> {
        self.requests.lock().unwrap().push(body.clone());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap()
            .map_err(anyhow::Error::msg)
    }
}
fn original(operation: &str) -> Value {
    json!({"answers":{
        "operation":{"choice":operation,"confidence":0.98,"probabilities":{"CLICK":if operation=="CLICK" {0.99}else{0.01},"DONE":if operation=="DONE" {0.99}else{0.01},"BLOCKED":0.0}},
        "click_target":{"choice":"1","confidence":0.97,"probabilities":{"1":1.0}},
        "irreversible_click_1":{"type":"noul","noul":0.99}},
        "usage":{"input_tokens":10,"output_tokens":2}})
}
fn facts(required: f64, performed: f64, ready: f64) -> Value {
    json!({"answers":{"required":{"type":"noul","noul":required},"performed":{"type":"noul","noul":performed},"ready":{"type":"noul","noul":ready}},"usage":{"input_tokens":4,"output_tokens":3}})
}
fn setup(
    replies: Vec<Result<Value, &'static str>>,
) -> (JevTypeSafeDecisionClient, Arc<Replies>, Value) {
    let transport = Arc::new(Replies {
        requests: Mutex::new(Vec::new()),
        replies: Mutex::new(replies.into()),
    });
    let client = JevTypeSafeDecisionClient::with_transport("jev-1.14.0", transport.clone())
        .with_profile(crate::jev_prompt::Profile::Activation)
        .with_images(true);
    let screenshot = crate::decide::images::tests::data_url(image::ImageFormat::Jpeg, 1120, 780);
    let page = json!({"url":"https://fixture.test", "title":"A", "text":"Prepared", "dom_snapshot":"- button Submit [node=3]",
        "_screenshot":screenshot, "actions":[{"id":"e1","kind":"click","node":3,"role":"button","label":"Submit"}]});
    (client, transport, page)
}
#[tokio::test]
async fn activation_correction_keeps_target_probabilities_gate_and_both_usages() {
    let (client, transport, page) = setup(vec![Ok(original("DONE")), Ok(facts(0.99, 0.3, 0.95))]);
    let decision = client
        .choose_gated(&page, "Prepare and submit", &[])
        .await
        .unwrap();
    assert_eq!(decision.operation, "CLICK");
    assert_eq!(decision.choice, "e1");
    assert_eq!(decision.target.as_deref(), Some("1"));
    assert!((decision.confidence - 0.64).abs() < 1e-9);
    assert_eq!(decision.target_confidence, Some(0.97));
    assert_eq!(
        decision.probabilities,
        Map::from_iter([("e1".into(), json!(1.0))])
    );
    assert_eq!(decision.irreversible, Some(0.99));
    let mut usage = decision.usage.clone();
    assert_eq!(usage["context_budget"]["per_question_limit"], 32000);
    usage.as_object_mut().unwrap().remove("context_budget");
    assert_eq!(
        usage,
        json!({"calls":2,"input_tokens":14,"output_tokens":5})
    );
    let requests = transport.requests.lock().unwrap();
    assert!(requests[0]["assets"].is_object());
    assert!(requests[1].get("assets").is_none());
    assert!(requests[1]["state"].get("viewport_screenshot").is_none());
    assert_eq!(requests[1]["questions"].as_object().unwrap().len(), 3);
    assert!(
        requests[1]["questions"]
            .as_object()
            .unwrap()
            .values()
            .all(|q| q["type"] == "noul")
    );
}
#[tokio::test]
async fn activation_audit_does_not_repeat_a_completed_mutation() {
    let (client, transport, page) = setup(vec![Ok(original("DONE")), Ok(facts(0.99, 0.98, 0.99))]);
    let decision = client.choose(&page, "Submit", &[]).await.unwrap();
    assert_eq!(decision.operation, "DONE");
    assert_eq!(decision.confidence, 0.98);
    assert_eq!(decision.usage["calls"], 2);
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn activation_audit_rejects_completion_without_ready_prerequisites() {
    let (client, _, page) = setup(vec![Ok(original("DONE")), Ok(facts(1.0, 0.0, 0.3))]);
    let error = client
        .choose(&page, "Fill a field then submit", &[])
        .await
        .unwrap_err();
    assert!(crate::usage::UnusableAnswer::is_behind(&error));
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 14);
}
#[tokio::test]
async fn activation_audit_keeps_prior_bill_when_the_check_fails() {
    let (client, _, page) = setup(vec![Ok(original("DONE")), Err("offline")]);
    let error = client.choose(&page, "Submit", &[]).await.unwrap_err();
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 10);
    assert!(error.to_string().contains("offline"));
    let mut missing = facts(1.0, 0.0, 1.0);
    missing["answers"].as_object_mut().unwrap().remove("ready");
    missing["usage"]
        .as_object_mut()
        .unwrap()
        .remove("output_tokens");
    let (client, _, page) = setup(vec![Ok(original("DONE")), Ok(missing)]);
    let error = client.choose(&page, "Submit", &[]).await.unwrap_err();
    assert!(crate::usage::UnusableAnswer::is_behind(&error));
    assert_eq!(
        JevBilled::usage_of(&error).unwrap()["output_tokens"],
        "unknown"
    );
}
#[tokio::test]
async fn activation_audit_ignores_missing_unused_targets_and_non_done_operations() {
    let mut reply = original("DONE");
    reply["answers"]
        .as_object_mut()
        .unwrap()
        .remove("click_target");
    let (client, transport, page) = setup(vec![Ok(reply)]);
    assert_eq!(
        client.choose(&page, "Submit", &[]).await.unwrap().operation,
        "DONE"
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    let (client, transport, page) = setup(vec![Ok(original("CLICK"))]);
    assert_eq!(
        client.choose(&page, "Submit", &[]).await.unwrap().operation,
        "CLICK"
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}
#[test]
fn support_requires_a_majority_under_the_conservative_bound() {
    assert_eq!(support(1.0, 0.0, 1.0), 1.0);
    assert_eq!(support(0.5, 0.5, 0.5), 0.0);
    assert!((support(0.99, 0.37, 0.94) - 0.56).abs() < 1e-9);
    assert!(support(0.9, 0.3, 0.9) <= 0.5 + 1e-9);
}

#[tokio::test]
async fn activation_correction_cannot_bypass_a_refused_irreversible_gate() {
    let mut reply = original("DONE");
    reply["answers"]["irreversible_click_1"] = json!({"type":"refusal"});
    let (client, _, page) = setup(vec![Ok(reply), Ok(facts(1.0, 0.0, 1.0))]);
    let error = client.choose_gated(&page, "Submit", &[]).await.unwrap_err();
    assert!(crate::usage::UnusableAnswer::is_behind(&error));
    assert!(error.to_string().contains("irreversible_click_1"));
    assert_eq!(JevBilled::usage_of(&error).unwrap()["calls"], 2);
}
