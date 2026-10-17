use super::*;
use crate::{DecisionsBrowserProfile, JevDecisionClient, OpenAiDecisionsClient};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

struct Answering {
    operation: &'static str,
    performed: f64,
    refuse_gate: bool,
    requests: Mutex<Vec<Value>>,
}
#[async_trait]
impl JevDecisionTransport for Answering {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        self.requests.lock().unwrap().push(request.clone());
        let answers = request["questions"].as_array().unwrap().iter().map(|q| {
            let name = q["name"].as_str().unwrap();
            if q["type"] == "predicate" {
                if self.refuse_gate && name.starts_with("irreversible_") {
                    return json!({"name":name,"type":"refusal"});
                }
                let p = match name {"performed"=>self.performed,"ready"=>0.95,_=>0.99};
                return json!({"name":name,"type":"predicate","probability":p});
            }
            let choices = q["choices"].as_array().unwrap();
            let selected = if name=="operation" { self.operation } else { choices[0]["value"].as_str().unwrap() };
            let top = if name=="operation" {0.94} else {0.8};
            json!({"name":name,"type":"choice","choice":selected,"confidence":0.87,
                "probabilities":choices.iter().map(|c|json!({"value":c["value"],"probability":if c["value"]==selected {top} else {(1.0-top)/(choices.len()-1) as f64}})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        Ok(
            json!({"model":"gpt-6-luna","answers":answers,"usage":{"input_tokens":42,"output_tokens":3}}),
        )
    }
}
fn transport(operation: &'static str, performed: f64, refuse_gate: bool) -> Arc<Answering> {
    Arc::new(Answering {
        operation,
        performed,
        refuse_gate,
        requests: Mutex::new(Vec::new()),
    })
}
fn page(clipped: bool) -> Value {
    json!({"url":"https://fixture.test","title":"Publish","text":"Prepared title",
        "dom_snapshot":"- form Publish\n  - textbox Title [node=3] [value=Full inspection title]\n  - button Publish [node=1]",
        "_screenshot":"data:image/png;base64,YQ==",
        "actions":[{"id":"e1","node":1,"kind":"click","label":"Publish","form":"Publish","rect":{"x":5,"y":5,"width":50,"height":20}},
                   {"id":"e2","node":2,"kind":"click","label":"Preview","form":"Publish"},
                   {"id":"e3","node":3,"kind":"fill","label":"Title","value":"Full inspection title","value_clipped":clipped,"form":"Publish"}]})
}
fn input(request: &Value) -> Value {
    let text = if request["input"].is_string() {
        request["input"].as_str().unwrap()
    } else {
        request["input"][0]["content"][0]["text"].as_str().unwrap()
    };
    serde_json::from_str(text).unwrap()
}
#[tokio::test]
async fn semantic_dom_preserves_workflow_form_ownership_and_node_rectangles() {
    let wire = transport("CLICK", 0.99, false);
    let client = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::SemanticDom);
    client
        .choose(&page(false), "Publish title", &[])
        .await
        .unwrap();
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let state = input(&requests[0]);
    assert!(
        state["dom_snapshot"]
            .as_str()
            .unwrap()
            .contains("form Publish")
    );
    assert_eq!(state["candidate_rectangles"][0]["node"], 1);
    let operation = requests[0]["questions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["name"] == "operation")
        .unwrap();
    assert!(
        operation["instructions"]
            .as_str()
            .unwrap()
            .contains("form_scope")
    );
    assert!(
        operation["instructions"]
            .as_str()
            .unwrap()
            .contains("Work through multi-step pages")
    );
    assert!(operation.to_string().contains("available_targets"));
    assert_eq!(state["elements"][0]["form"], "Publish");
}
#[tokio::test]
async fn clipped_fields_split_operation_and_safety_from_visual_targets() {
    let wire = transport("CLICK", 0.99, false);
    let client = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited);
    let decision = client
        .choose_gated(&page(true), "Publish title", &[])
        .await
        .unwrap();
    assert_eq!(decision.choice, "e1");
    assert_eq!(decision.probabilities["e1"], 0.8);
    assert!((decision.probabilities["e2"].as_f64().unwrap() - 0.2).abs() < 1e-9);
    assert_eq!(decision.irreversible, Some(0.99));
    assert_eq!(
        decision.usage,
        json!({"calls":2,"input_tokens":84,"output_tokens":6})
    );
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let visual = requests.iter().find(|r| r["input"].is_array()).unwrap();
    let text = requests.iter().find(|r| r["input"].is_string()).unwrap();
    assert!(
        visual["questions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|q| q["type"] == "choice" && q["name"].as_str().unwrap().ends_with("_target"))
    );
    assert!(
        text["questions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["name"] == "operation")
    );
    assert!(
        text["questions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["type"] == "predicate")
    );
    assert!(!text.to_string().contains("data:image"));
    assert!(
        input(text)["dom_snapshot"]
            .as_str()
            .unwrap()
            .contains("Full inspection title")
    );
}
#[tokio::test]
async fn unclipped_fields_keep_one_shared_image_request() {
    let wire = transport("CLICK", 0.99, false);
    OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited)
        .choose(&page(false), "Publish title", &[])
        .await
        .unwrap();
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0]["input"].is_array());
}
#[tokio::test]
async fn text_only_is_preserved_in_both_builder_orders() {
    for before in [true, false] {
        let wire = transport("CLICK", 0.99, false);
        let client = OpenAiDecisionsClient::with_transport(wire.clone());
        let client = if before {
            client
                .text_only()
                .with_browser_profile(DecisionsBrowserProfile::Audited)
        } else {
            client
                .with_browser_profile(DecisionsBrowserProfile::Audited)
                .text_only()
        };
        assert!(!client.uses_images());
        client
            .choose(&page(true), "Publish title", &[])
            .await
            .unwrap();
        let requests = wire.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]["input"].is_string());
        assert!(!requests[0].to_string().contains("data:image"));
        assert!(input(&requests[0])["dom_snapshot"].is_string());
    }
}
#[tokio::test]
async fn native_predicate_audit_recovers_click_without_fabricating_target_probabilities() {
    let wire = transport("DONE", 0.3, false);
    let client = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited);
    let decision = client
        .choose_gated(&page(false), "Publish title", &[])
        .await
        .unwrap();
    assert_eq!(decision.operation, "CLICK");
    assert_eq!(decision.choice, "e1");
    assert!((decision.confidence - 0.64).abs() < 1e-9);
    assert_eq!(decision.target_confidence, Some(0.87));
    assert_eq!(decision.probabilities["e1"], 0.8);
    assert!((decision.probabilities["e2"].as_f64().unwrap() - 0.2).abs() < 1e-9);
    assert_eq!(decision.irreversible, Some(0.99));
    assert_eq!(decision.usage["calls"], 2);
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let audit = &requests[1];
    assert!(audit["input"].is_string());
    assert_eq!(audit["questions"].as_array().unwrap().len(), 3);
    assert!(
        audit["questions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|q| q["type"] == "predicate")
    );
    assert!(!audit.to_string().contains("data:image"));
}
#[tokio::test]
async fn audit_does_not_repeat_completed_actions_or_bypass_a_refused_gate() {
    let wire = transport("DONE", 0.99, false);
    let decision = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited)
        .choose(&page(false), "Publish title", &[])
        .await
        .unwrap();
    assert_eq!(decision.operation, "DONE");
    assert_eq!(wire.requests.lock().unwrap().len(), 2);
    let mut fields_only = page(false);
    fields_only["actions"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["kind"] == "fill");
    let wire = transport("DONE", 0.99, false);
    let decision = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited)
        .choose(&fields_only, "Keep the current title", &[])
        .await
        .unwrap();
    assert_eq!(decision.operation, "DONE");
    assert_eq!(wire.requests.lock().unwrap().len(), 1);
    let wire = transport("DONE", 0.3, true);
    let error = OpenAiDecisionsClient::with_transport(wire)
        .with_browser_profile(DecisionsBrowserProfile::Audited)
        .choose_gated(&page(false), "Publish title", &[])
        .await
        .unwrap_err();
    assert!(crate::usage::UnusableAnswer::is_behind(&error));
    assert!(error.to_string().contains("irreversible_click_1"));
    assert_eq!(JevBilled::usage_of(&error).unwrap()["calls"], 2);
}
#[test]
fn split_batch_failure_and_malformed_reply_keep_both_bills() {
    let good = || Ok(json!({"answers":[],"usage":{"input_tokens":42,"output_tokens":3}}));
    let error = combine(
        good(),
        Err(JevBilled::new(json!({"input_tokens":17}), anyhow::anyhow!("failed")).into()),
    )
    .unwrap_err();
    assert_eq!(
        JevBilled::usage_of(&error).unwrap(),
        &json!({"calls":2,"input_tokens":59,"output_tokens":"unknown"})
    );
    let error = combine(
        good(),
        Ok(json!({"answers":null,"usage":{"input_tokens":17,"output_tokens":1}})),
    )
    .unwrap_err();
    assert!(crate::usage::UnusableAnswer::is_behind(&error));
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 59);
}

#[tokio::test]
async fn native_request_budget_counts_leaf_batches_and_stops_before_an_audit() {
    let wire = transport("DONE", 0.3, false);
    let client = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited);
    let (result, used) =
        crate::request_budget::scope(2, client.choose(&page(true), "Publish title", &[])).await;
    let error = result.unwrap_err();
    assert_eq!(
        crate::JevStop::status_of(&error),
        crate::JevStatus::BudgetExceeded
    );
    assert_eq!(used, 2);
    assert_eq!(wire.requests.lock().unwrap().len(), 2);
    assert_eq!(JevBilled::usage_of(&error).unwrap()["calls"], 2);
    let wire = transport("DONE", 0.3, false);
    let client = OpenAiDecisionsClient::with_transport(wire.clone())
        .with_browser_profile(DecisionsBrowserProfile::Audited);
    let (result, used) =
        crate::request_budget::scope(3, client.choose(&page(true), "Publish title", &[])).await;
    assert_eq!(result.unwrap().operation, "CLICK");
    assert_eq!(used, 3);
    assert_eq!(wire.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn secret_presence_replaces_empty_redaction_placeholders_in_state_and_choices() {
    for filled in [true, false] {
        let wire = transport("CLICK", 0.99, false);
        let mut page = page(false);
        page["actions"].as_array_mut().unwrap().push(json!({"id":"secret","node":4,"kind":"fill","role":"textbox","input_type":"password","label":"Password","filled":filled}));
        OpenAiDecisionsClient::with_transport(wire.clone())
            .with_browser_profile(DecisionsBrowserProfile::SemanticDom)
            .choose(&page, "Publish with the resolved password", &[])
            .await
            .unwrap();
        let requests = wire.requests.lock().unwrap();
        let body = &requests[0];
        let state = input(body);
        let secret = state["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["node"] == 4)
            .unwrap();
        assert_eq!(secret["filled"], filled);
        assert!(secret.get("value").is_none());
        let target = body["questions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["name"] == "type_text_target")
            .unwrap();
        let option = target["choices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["description"].as_str().unwrap().contains("Password"))
            .unwrap();
        let description: Value =
            serde_json::from_str(option["description"].as_str().unwrap()).unwrap();
        assert_eq!(description["filled"], filled);
        assert!(description.get("current_value").is_none());
        assert!(
            target["instructions"]
                .as_str()
                .unwrap()
                .contains("filled=true is populated")
        );
    }
}
