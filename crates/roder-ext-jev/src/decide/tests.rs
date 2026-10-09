//! Contract tests for the decision request and its validation.

use std::sync::Mutex;

use super::*;
use crate::engine::{JevStatus, JevStop};
use crate::http::tests::{MockServer, Reply, fast_policy};

struct FixtureTransport {
    request: Mutex<Option<Value>>,
    response: Value,
}

#[async_trait]
impl JevDecisionTransport for FixtureTransport {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        *self.request.lock().unwrap() = Some(request.clone());
        Ok(self.response.clone())
    }
}

/// The day the request fixture was recorded on.
fn fixture_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
}

fn fixture(name: &str) -> Value {
    let raw = match name {
        "choose" => include_str!("../../tests/fixtures/choose_request.json"),
        "validate" => include_str!("../../tests/fixtures/validate_choice.json"),
        _ => unreachable!(),
    };
    serde_json::from_str(raw).unwrap()
}

#[test]
fn request_body_matches_upstream_byte_for_byte() {
    let fixture = fixture("choose");
    let page: Value = serde_json::from_str(include_str!("../../tests/fixtures/fingerprint.json"))
        .map(|value: Value| value["page"].clone())
        .unwrap();
    let history = fixture["history"].as_array().unwrap().clone();
    let (body, _, _) = request_body(
        &page,
        fixture["goal"].as_str().unwrap(),
        &history,
        "jev-latest",
        fixture_day(),
    );
    // Compact serialization, which is what httpx sends: this pins content
    // and key order together.
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        fixture["serialized"].as_str().unwrap()
    );
    assert_eq!(body, fixture["body"]);
}

#[test]
fn validation_verdicts_match_upstream() {
    let fixture = fixture("validate");
    let ids = fixture["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    for case in fixture["cases"].as_array().unwrap() {
        let accepted = validate_choice(&case["answer"], &ids).is_ok();
        assert_eq!(
            accepted,
            case["accepted"].as_bool().unwrap(),
            "case {} disagreed with upstream",
            case["name"]
        );
    }
}

#[tokio::test]
async fn injected_transport_uses_workflow_evidence_and_upstream_response_parsing() {
    let page: Value = serde_json::from_str(include_str!("../../tests/fixtures/fingerprint.json"))
        .map(|value: Value| value["page"].clone())
        .unwrap();
    let transport = Arc::new(FixtureTransport {
        request: Mutex::new(None),
        response: json!({
            "answers": {
                "operation": {
                    "choice": "WAIT",
                    "confidence": 1.0,
                    "probabilities": {
                        "CLICK": 0.0,
                        "TYPE_TEXT": 0.0,
                        "SELECT": 0.0,
                        "SCROLL_DOWN": 0.0,
                        "WAIT": 1.0,
                        "DONE": 0.0,
                        "BLOCKED": 0.0
                    }
                }
            },
            "usage": {"input_tokens": 10}
        }),
    });
    let client = JevTypeSafeDecisionClient::with_transport("jev-hosted", transport.clone());

    let decision = client.choose(&page, "Wait once", &[]).await.unwrap();

    {
        let request = transport.request.lock().unwrap();
        let request = request.as_ref().unwrap();
        assert!(
            request["questions"]["operation"]["instructions"]["workflow"]
                .as_str()
                .unwrap()
                .contains("multi-step")
        );
        assert!(request["questions"]["operation"]["instructions"]["form_scope"].is_string());
    }
    assert_eq!(decision.choice, "wait");
    assert_eq!(decision.operation, "WAIT");
    assert_eq!(decision.usage["input_tokens"], json!(10));
    assert_eq!(decision.target_confidence, None);
    assert_eq!(decision.model, None);
    assert_eq!(
        transport.request.lock().unwrap().as_ref().unwrap()["model"],
        json!("jev-hosted")
    );
}

fn hosted(url: &str) -> TypeSafeHttpTransport {
    TypeSafeHttpTransport::new(url, "sk-typesafe", fast_policy())
}

#[tokio::test]
async fn hosted_transport_retries_a_cloudflare_error_then_answers() {
    let server = MockServer::start(vec![Reply::status(520), Reply::ok(r#"{"answers":{}}"#)]).await;

    let answer = hosted(&server.url).decide(&json!({"model": "m"})).await;

    assert_eq!(answer.unwrap(), json!({"answers": {}}));
    assert_eq!(server.hits(), 2);
    assert_eq!(server.requests()[1].0, "Bearer sk-typesafe");
}

#[tokio::test]
async fn hosted_transport_keeps_the_no_action_messages() {
    let refused = MockServer::start(vec![Reply::status(401)]).await;
    let error = hosted(&refused.url).decide(&json!({})).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model provider returned HTTP 401; no action executed."
    );
    assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    assert_eq!(refused.hits(), 1);

    // A validation error names the offending field, so its body comes along.
    let rejected = MockServer::start(vec![Reply::Status(
        422,
        Vec::new(),
        "{\"detail\": \"questions.click_target.criteria:\n  at most 255 options\"}",
    )])
    .await;
    let error = hosted(&rejected.url).decide(&json!({})).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model provider returned HTTP 422 ({\"detail\": \"questions.click_target.criteria: at most 255 options\"}); no action executed."
    );
    assert_eq!(rejected.hits(), 1);

    let overloaded = MockServer::start(vec![Reply::status(529)]).await;
    let error = hosted(&overloaded.url)
        .decide(&json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model provider returned HTTP 529; no action executed."
    );
    // Retries exhausted on an overloaded provider: unavailable, not an error.
    assert_eq!(JevStop::status_of(&error), JevStatus::Unavailable);
    assert_eq!(overloaded.hits(), 6);

    let garbled = MockServer::start(vec![Reply::ok("not json")]).await;
    let error = hosted(&garbled.url).decide(&json!({})).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid TypeSafe response; no action executed."
    );
    assert_eq!(JevStop::status_of(&error), JevStatus::Error);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let error = hosted(&closed).decide(&json!({})).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model connection failed; no action executed."
    );
    assert_eq!(JevStop::status_of(&error), JevStatus::Unavailable);
}

#[test]
fn an_offscreen_control_is_flagged_to_the_model() {
    let page = json!({
        "url": "https://example.test/list",
        "title": "List",
        "text": "Results",
        "actions": [
            {"id": "e1", "node": 1, "kind": "click", "role": "link", "label": "Result one",
             "value": ""},
            {"id": "e2", "node": 2, "kind": "click", "role": "link", "label": "Next page",
             "value": "", "offscreen": true},
        ],
    });
    let (body, _, _) = request_body(&page, "Open page two", &[], "jev-latest", fixture_day());

    let elements = body["state"]["elements"].as_array().unwrap();
    assert_eq!(elements[0].get("offscreen"), None);
    assert_eq!(elements[1]["offscreen"], json!(true));
    let criteria = &body["questions"]["click_target"]["criteria"];
    assert_eq!(criteria["1"].get("offscreen"), None);
    assert_eq!(
        serde_json::to_string(&criteria["2"]).unwrap(),
        r#"{"element":"[2] Next page","current_value":"","role":"link","offscreen":true}"#
    );
    let rules = &body["questions"]["click_target"]["instructions"]["rules"];
    assert!(rules[1].as_str().unwrap().contains(
        "An element marked offscreen can be targeted directly; do not scroll just to reach it.\n"
    ));
}

#[test]
fn twins_are_told_apart_by_their_context() {
    let page = json!({
        "url": "https://example.test/shop",
        "title": "Shop",
        "text": "Products",
        "actions": [
            {"id": "e1", "node": 1, "kind": "click", "role": "button", "label": "Add to cart",
             "value": "", "context": "Trail Runner"},
            {"id": "e2", "node": 2, "kind": "click", "role": "button", "label": "Add to cart",
             "value": "", "offscreen": true, "context": "Canvas Tote"},
            {"id": "e3", "node": 3, "kind": "click", "role": "link", "label": "Checkout",
             "value": ""},
        ],
    });
    let (body, _, _) = request_body(
        &page,
        "Add the tote to the cart",
        &[],
        "jev-latest",
        fixture_day(),
    );

    let elements = body["state"]["elements"].as_array().unwrap();
    assert_eq!(
        serde_json::to_string(&elements[1]).unwrap(),
        r#"{"role":"button","value":"","offscreen":true,"index":"2","label":"Add to cart","operations":["CLICK"],"context":"Canvas Tote"}"#
    );
    assert_eq!(elements[2].get("context"), None);
    let criteria = &body["questions"]["click_target"]["criteria"];
    assert_eq!(
        serde_json::to_string(&criteria["1"]).unwrap(),
        r#"{"element":"[1] Add to cart","context":"Trail Runner","current_value":"","role":"button"}"#
    );
    assert_eq!(
        serde_json::to_string(&criteria["2"]).unwrap(),
        r#"{"element":"[2] Add to cart","context":"Canvas Tote","current_value":"","role":"button","offscreen":true}"#
    );
    assert_eq!(
        serde_json::to_string(&criteria["3"]).unwrap(),
        r#"{"element":"[3] Checkout","current_value":"","role":"link"}"#
    );
    let rules = &body["questions"]["click_target"]["instructions"]["rules"];
    assert!(rules[1].as_str().unwrap().ends_with(
        "Use an element's context, the card, row, section or table column it belongs to, to tell apart elements with the same label."
    ));
}

#[tokio::test]
async fn each_heads_confidence_and_the_answering_model_are_kept() {
    let page = json!({
        "url": "https://example.test/shop",
        "title": "Shop",
        "text": "Products",
        "actions": [
            {"id": "e1", "node": 1, "kind": "click", "role": "button", "label": "Add to cart",
             "value": "", "context": "Trail Runner"},
            {"id": "e2", "node": 2, "kind": "click", "role": "button", "label": "Add to cart",
             "value": "", "context": "Canvas Tote"},
            {"id": "wait", "kind": "wait", "label": "Wait for the page to update"},
        ],
    });
    let transport = Arc::new(FixtureTransport {
        request: Mutex::new(None),
        response: json!({
            "model": "jev-1.13",
            "answers": {
                "operation": {
                    "choice": "CLICK",
                    "confidence": 0.9,
                    "probabilities": {"CLICK": 0.9, "WAIT": 0.05, "DONE": 0.03, "BLOCKED": 0.02}
                },
                "click_target": {
                    "choice": "2",
                    "confidence": 0.6,
                    "probabilities": {"1": 0.4, "2": 0.6}
                }
            }
        }),
    });
    let client = JevTypeSafeDecisionClient::with_transport("jev-latest", transport);

    let decision = client.choose(&page, "Add the tote", &[]).await.unwrap();

    assert_eq!(decision.choice, "e2");
    assert_eq!(decision.confidence, 0.9);
    assert_eq!(decision.target_confidence, Some(0.6));
    assert_eq!(decision.call_confidence(), 0.6);
    assert_eq!(decision.probability_of("e2"), json!(0.6));
    assert_eq!(decision.model.as_deref(), Some("jev-1.13"));
}
