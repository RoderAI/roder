use super::tests::page;
use super::tests::{ANSWER, Fixed};
use super::*;

#[tokio::test]
async fn refusal_on_unused_branch_does_not_abort_valid_action() {
    let mut response: Value = serde_json::from_str(ANSWER).unwrap();
    response["answers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"refusal","name":"type_text_target"}));
    response["answers"][0]["probabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({"value":"TYPE_TEXT","probability":0.0}));
    let mut observed = page();
    observed["actions"].as_array_mut().unwrap().extend([
        json!({"id":"e2","kind":"fill","label":"Email","node":2}),
        json!({"id":"e3","kind":"fill","label":"Name","node":3}),
    ]);
    let decision = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(response)))
        .choose(&observed, "Buy item", &[])
        .await
        .unwrap();
    assert_eq!(decision.operation, "CLICK");
}

#[tokio::test]
async fn required_refusal_names_question_and_keeps_usage() {
    let mut response: Value = serde_json::from_str(ANSWER).unwrap();
    response["answers"][0] = json!({"type":"refusal","name":"operation"});
    let error = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(response)))
        .choose(&page(), "Buy item", &[])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("required question operation"));
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 42);
}

#[tokio::test]
async fn refused_safety_question_remains_closed() {
    let mut response: Value = serde_json::from_str(ANSWER).unwrap();
    response["answers"][2] = json!({"type":"refusal","name":"irreversible_click_1"});
    let error = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(response)))
        .choose_gated(&page(), "Buy item", &[])
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("required question irreversible_click_1")
    );
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 42);
}

struct Answering {
    selected: &'static str,
    completion: f64,
    requests: std::sync::Mutex<Vec<Value>>,
}
#[async_trait]
impl JevDecisionTransport for Answering {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        self.requests.lock().unwrap().push(request.clone());
        let answers=request["questions"].as_array().unwrap().iter().map(|q| {
            if q["type"]=="predicate" { return json!({"type":"predicate","name":q["name"],"probability":self.completion}); }
            let choices=q["choices"].as_array().unwrap();
            let selected=choices.iter().find(|c|c["value"]==self.selected).unwrap_or(&choices[0])["value"].clone();
            json!({"type":"choice","name":q["name"],"choice":selected,"confidence":0.97,"probabilities":choices.iter().map(|c|json!({"value":c["value"],"probability":if c["value"]==selected {1.0} else {0.0}})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        Ok(json!({"answers":answers,"usage":{"input_tokens":40,"output_tokens":0}}))
    }
}
fn answering(selected: &'static str, completion: f64) -> Arc<Answering> {
    Arc::new(Answering {
        selected,
        completion,
        requests: std::sync::Mutex::new(Vec::new()),
    })
}
fn two_targets() -> Value {
    json!({"url":"https://example.test","title":"Shop","text":"Two cards",
        "actions":[{"id":"e1","kind":"click","label":"Add","context":"Lamp","node":1},
                   {"id":"e2","kind":"click","label":"Add","context":"Bottle","node":2}]})
}

#[tokio::test]
async fn sequential_asks_only_selected_target_and_sums_both_requests() {
    let transport = answering("CLICK", 0.0);
    let client = OpenAiDecisionsClient::configured(transport.clone(), Strategy::Sequential);
    let decision = client
        .choose(&two_targets(), "Add Lamp", &[])
        .await
        .unwrap();
    assert_eq!(decision.choice, "e1");
    assert_eq!(decision.usage["input_tokens"], 80);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["questions"].as_array().unwrap().len(), 1);
    assert_eq!(requests[1]["questions"][0]["name"], "click_target");
}

#[tokio::test]
async fn joint_choice_maps_to_exact_offered_target_and_retains_confidence() {
    let transport = answering("CLICK:2", 0.0);
    let decision = OpenAiDecisionsClient::configured(transport, Strategy::Joint)
        .choose(&two_targets(), "Add Bottle", &[])
        .await
        .unwrap();
    assert_eq!(decision.choice, "e2");
    assert_eq!(decision.confidence, 0.97);
    assert_eq!(decision.target_confidence, Some(0.97));
}

#[tokio::test]
async fn completion_check_rejects_false_done_with_usage() {
    let transport = answering("DONE", 0.2);
    let error = OpenAiDecisionsClient::configured(transport, Strategy::Verified)
        .choose(&two_targets(), "Add Bottle", &[])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Completion lacks"));
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 40);
}

#[tokio::test]
async fn effects_are_observed_evidence_and_do_not_leak_between_calls() {
    let transport = answering("DONE", 1.0);
    let client = OpenAiDecisionsClient::configured(transport.clone(), Strategy::Effects);
    let page = two_targets();
    let history = vec![
        json!({"action":"Add", "effect":"Cart now contains Lamp", "context":"Lamp", "text":"[secret]", "usage":{"input_tokens":9}}),
    ];
    let (a, b) = tokio::join!(
        client.choose(&page, "Add Lamp", &history),
        client.choose(&page, "Inspect Bottle", &[])
    );
    a.unwrap();
    b.unwrap();
    let requests = transport.requests.lock().unwrap();
    let inputs = requests
        .iter()
        .map(|r| serde_json::from_str::<Value>(r["input"].as_str().unwrap()).unwrap())
        .collect::<Vec<_>>();
    let lamp = inputs
        .iter()
        .find(|i| i["user_goal"] == "Add Lamp")
        .unwrap();
    assert_eq!(
        lamp["recent_actions"][0]["effect"],
        "Cart now contains Lamp"
    );
    assert!(lamp["recent_actions"][0].get("usage").is_none());
    let bottle = inputs
        .iter()
        .find(|i| i["user_goal"] == "Inspect Bottle")
        .unwrap();
    assert!(bottle["recent_actions"].as_array().unwrap().is_empty());
}

#[test]
fn production_requests_images_and_text_only_explicitly_disables_them() {
    let client = OpenAiDecisionsClient::with_transport(answering("DONE", 1.0));
    assert!(client.uses_images());
    assert!(!client.text_only().uses_images());
}

struct TwoReplies(std::sync::Mutex<std::collections::VecDeque<Value>>);
#[async_trait]
impl JevDecisionTransport for TwoReplies {
    async fn decide(&self, _: &Value) -> anyhow::Result<Value> {
        Ok(self.0.lock().unwrap().pop_front().unwrap())
    }
}
#[tokio::test]
async fn sequential_malformed_second_response_keeps_total_billed_usage() {
    let first = json!({"answers":[{"type":"choice","name":"operation","choice":"CLICK","confidence":1.0,"probabilities":[{"value":"CLICK","probability":1.0},{"value":"DONE","probability":0.0},{"value":"BLOCKED","probability":0.0}]}],"usage":{"input_tokens":40,"output_tokens":0}});
    let second = json!({"answers":null,"usage":{"input_tokens":25,"output_tokens":0}});
    let transport = Arc::new(TwoReplies(std::sync::Mutex::new(
        [first, second].into_iter().collect(),
    )));
    let error = OpenAiDecisionsClient::configured(transport, Strategy::Sequential)
        .choose(&two_targets(), "Add Lamp", &[])
        .await
        .unwrap_err();
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 65);
}
