use super::*;
use crate::http::tests::{MockServer, Reply, fast_policy};

pub(super) fn page() -> Value {
    json!({"url":"https://example.test", "title":"Checkout", "text":"Buy item",
        "actions":[{"id":"e1", "kind":"click", "label":"Buy item", "node":1}]})
}

pub(super) const ANSWER: &str = r#"{"answers":[
    {"type":"choice","name":"operation","choice":"CLICK","confidence":0.98,
     "probabilities":[{"value":"CLICK","probability":0.98},{"value":"DONE","probability":0.01},{"value":"BLOCKED","probability":0.01}]},
    {"type":"choice","name":"click_target","choice":"1","confidence":0.97,
     "probabilities":[{"value":"1","probability":1.0}]},
    {"type":"predicate","name":"irreversible_click_1","probability":0.95}
],"usage":{"input_tokens":42}}"#;

fn client(server: &MockServer) -> OpenAiDecisionsClient {
    OpenAiDecisionsClient::with_transport(Arc::new(DecisionsHttp {
        key: "test-openai-key".into(),
        url: server.url.replace("/systemone", "/decisions"),
        http: JsonPoster::new(fast_policy()),
    }))
}

#[tokio::test]
async fn native_http_contract_resolves_observed_action_and_gate() {
    let server = MockServer::start(vec![Reply::ok(ANSWER)]).await;
    let decision = client(&server)
        .choose_gated(&page(), "Buy item", &[])
        .await
        .unwrap();
    assert_eq!(decision.choice, "e1");
    assert_eq!(decision.operation, "CLICK");
    assert_eq!(decision.irreversible, Some(0.95));
    assert_eq!(decision.target_confidence, Some(0.97));
    assert_eq!(decision.model.as_deref(), Some(MODEL));
    assert_eq!(decision.usage["input_tokens"], 42);
    let requests = server.requests();
    assert_eq!(requests[0].0, "Bearer test-openai-key");
    assert!(server.heads()[0].starts_with("POST /v1/decisions "));
    let body = &requests[0].1;
    assert_eq!(body["model"], MODEL);
    assert!(body.get("state").is_none());
    let input: Value = serde_json::from_str(body["input"].as_str().unwrap()).unwrap();
    assert_eq!(input["page"]["url"], "https://example.test");
    let questions = body["questions"].as_array().unwrap();
    let operation = questions.iter().find(|q| q["name"] == "operation").unwrap();
    assert_eq!(operation["type"], "choice");
    assert_eq!(input["user_goal"], "Buy item");
    assert!(
        input.get("disabled_controls").is_none(),
        "Decisions must retain its own evidence profile"
    );
    assert!(!body.to_string().contains("form_scope"));
    assert!(!body.to_string().contains("Work through multi-step pages"));
    assert!(
        operation["choices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["value"] == "CLICK")
    );
    let gate = questions
        .iter()
        .find(|q| q["name"] == "irreversible_click_1")
        .unwrap();
    assert_eq!(gate["type"], "predicate");
    assert!(gate["instructions"].as_str().unwrap().contains("untrusted"));
}

pub(super) struct Fixed(pub(super) Value);
#[async_trait]
impl JevDecisionTransport for Fixed {
    async fn decide(&self, _: &Value) -> anyhow::Result<Value> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn malformed_responses_fail_closed_and_keep_billed_usage() {
    let good: Value = serde_json::from_str(ANSWER).unwrap();
    let mut cases = Vec::new();
    let mut unknown = good.clone();
    unknown["answers"][1]["choice"] = json!("999");
    cases.push(unknown);
    let mut duplicate = good.clone();
    duplicate["answers"]
        .as_array_mut()
        .unwrap()
        .push(good["answers"][0].clone());
    cases.push(duplicate);
    let mut duplicate_probability = good.clone();
    duplicate_probability["answers"][1]["probabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({"value":"1","probability":1.0}));
    cases.push(duplicate_probability);
    let mut invalid = good.clone();
    invalid["answers"][0]["confidence"] = json!(1.1);
    cases.push(invalid);
    let mut missing = good.clone();
    missing["answers"] = json!([]);
    cases.push(missing);
    for response in cases {
        let result = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(response)))
            .choose(&page(), "Buy item", &[])
            .await;
        let error = result.expect_err("invalid answer must not execute");
        assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 42);
        // A reply the loop may ask again about, whichever step found it wrong.
        assert!(crate::usage::UnusableAnswer::is_behind(&error));
    }
}

#[tokio::test]
async fn missing_or_out_of_range_predicate_preserves_closed_gate() {
    for probability in [Value::Null, json!(-0.1), json!(1.1)] {
        let mut response: Value = serde_json::from_str(ANSWER).unwrap();
        response["answers"][2]["probability"] = probability;
        let decision = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(response)))
            .choose_gated(&page(), "Buy item", &[])
            .await
            .unwrap();
        assert_eq!(decision.irreversible, None);
    }
}

#[tokio::test]
async fn authorization_error_is_redacted_and_not_retried() {
    let server = MockServer::start(vec![Reply::Status(
        401,
        vec![],
        r#"{"error":"test-openai-key"}"#,
    )])
    .await;
    let error = client(&server)
        .choose(&page(), "Buy item", &[])
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("401"));
    assert!(!error.to_string().contains("test-openai-key"));
    assert_eq!(server.hits(), 1);
}

#[tokio::test]
async fn computer_choices_can_block_and_reject_unoffered_actions() {
    use crate::ComputerCandidate;
    use roder_api::computer::{ComputerAction, ComputerActions};
    let candidates = vec![ComputerCandidate {
        id: "wait".into(),
        description: "Wait for loading".into(),
        actions: ComputerActions {
            actions: vec![ComputerAction::Wait],
        },
    }];
    let response = json!({"answers":[{"type":"choice","name":"computer_action", "choice":"blocked","confidence":0.99,
        "probabilities":[{"value":"blocked","probability":0.99},{"value":"wait","probability":0.01}]}],"usage":{"input_tokens":23}});
    let client = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(response.clone())));
    let result = client
        .choose_computer(
            "Do something useful",
            "data:image/png;base64,YQ==",
            &candidates,
        )
        .await
        .unwrap();
    assert!(result.actions.is_none());
    assert_eq!(result.choice, "blocked");
    assert!(
        client
            .choose_computer("goal", "https://example.test/screenshot.png", &candidates)
            .await
            .is_err()
    );
    assert!(
        client
            .choose_computer("goal", "data:image/png;base64,YQ==", &[])
            .await
            .is_err()
    );
    let mut invalid = response;
    invalid["answers"][0]["choice"] = json!("invented");
    let client = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(invalid)));
    let error = client
        .choose_computer("goal", "data:image/png;base64,YQ==", &candidates)
        .await
        .err()
        .unwrap();
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 23);
}

#[tokio::test]
async fn singleton_target_is_resolved_locally_without_invalid_api_question() {
    let response: Value = serde_json::from_str(ANSWER).unwrap();
    let mut response = response;
    response["answers"]
        .as_array_mut()
        .unwrap()
        .retain(|answer| answer["name"] != "click_target");
    let client = OpenAiDecisionsClient::with_transport(Arc::new(Fixed(response)));
    let decision = client.choose(&page(), "Buy item", &[]).await.unwrap();
    assert_eq!(decision.choice, "e1");
    assert_eq!(decision.target_confidence, Some(1.0));
    let (shared, _, _) = crate::decide::request_body(
        &page(),
        "Buy item",
        &[],
        MODEL,
        chrono::Local::now().date_naive(),
    );
    let wire = request_body(&shared).unwrap();
    assert!(
        wire["questions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|q| q["name"] != "click_target")
    );
    assert!(
        wire["questions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["name"] == "operation")
    );
}
