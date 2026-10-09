//! Which decision failures the loop may ask again about: a reply the service
//! gave that cannot be used, and nothing else.

use super::*;
use crate::engine::{JevStatus, JevStop};
use crate::http::tests::{MockServer, Reply, fast_policy};
use crate::usage::UnusableAnswer;

const INVALID: &str = "Invalid browser decision response; no action executed.";

struct Fixed(Value);

#[async_trait]
impl JevDecisionTransport for Fixed {
    async fn decide(&self, _request: &Value) -> anyhow::Result<Value> {
        Ok(self.0.clone())
    }
}

struct Failing(JevStatus);

#[async_trait]
impl JevDecisionTransport for Failing {
    async fn decide(&self, _request: &Value) -> anyhow::Result<Value> {
        Err(JevStop::new(self.0, "scripted").into())
    }
}

fn page() -> Value {
    json!({"url": "https://a.test/", "title": "A", "text": "A",
        "actions": [{"id": "e1", "node": 1, "kind": "click", "label": "Buy", "value": ""}]})
}

async fn choose_with(transport: &dyn JevDecisionTransport) -> anyhow::Error {
    let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    choose(&page(), "Buy", &[], "jev-test", transport, today, false)
        .await
        .expect_err("the choice should fail")
}

/// An operation answer that validates, choosing `operation`.
fn operation(choice: &str) -> Value {
    let (_, _, ids) = request_body(
        &page(),
        "Buy",
        &[],
        "jev-test",
        NaiveDate::from_ymd_opt(2026, 10, 9).unwrap(),
    );
    let probabilities = ids
        .iter()
        .map(|id| (id.clone(), json!(f64::from(id == choice))))
        .collect::<Map<_, _>>();
    json!({"choice": choice, "confidence": 0.9, "probabilities": probabilities})
}

#[tokio::test]
async fn a_reply_that_cannot_be_used_is_marked_and_keeps_its_usage() {
    let usage = json!({"input_tokens": 7, "output_tokens": 1});
    let replies = [
        // An operation the page never offered.
        json!({"answers": {"operation": {"choice": "NOT_OFFERED", "confidence": 0.9,
            "probabilities": {"NOT_OFFERED": 1.0}}}, "usage": usage}),
        // No answers at all.
        json!({"usage": usage}),
        // A refusal of the required question.
        json!({"answers": {"operation": {"type": "refusal"}}, "usage": usage}),
        // A valid operation whose targets do not validate.
        json!({"answers": {"operation": operation("CLICK"),
            "click_target": {"choice": "99", "confidence": 0.9, "probabilities": {"99": 1.0}}},
            "usage": usage}),
    ];
    for reply in replies {
        let error = choose_with(&Fixed(reply.clone())).await;
        assert!(UnusableAnswer::is_behind(&error), "{reply}");
        assert_eq!(JevBilled::usage_of(&error), Some(&usage), "{reply}");
        // Marked, not changed: the same message, the same status.
        assert_eq!(JevStop::status_of(&error), JevStatus::Error);
    }
    let error = choose_with(&Fixed(json!({"usage": usage}))).await;
    assert_eq!(error.to_string(), INVALID);
}

#[tokio::test]
async fn a_refused_gate_question_is_unusable_too() {
    let mut delete = page();
    delete["actions"][0]["label"] = json!("Delete");
    let reply = json!({
        "answers": {
            "operation": operation("CLICK"),
            "click_target": {"choice": "1", "confidence": 0.9, "probabilities": {"1": 1.0}},
            "irreversible_click_1": {"type": "refusal"},
        },
        "usage": {"input_tokens": 3},
    });
    let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    let error = choose(
        &delete,
        "Delete it",
        &[],
        "jev-test",
        &Fixed(reply),
        today,
        true,
    )
    .await
    .expect_err("a refused gate question should fail");
    assert_eq!(
        error.to_string(),
        "OpenAI Decisions refused required question irreversible_click_1; no action executed."
    );
    assert!(UnusableAnswer::is_behind(&error));
    assert_eq!(JevBilled::usage_of(&error).unwrap()["input_tokens"], 3);
}

#[tokio::test]
async fn a_call_that_failed_is_not_a_reply_to_ask_again_about() {
    for status in [JevStatus::Unavailable, JevStatus::Error] {
        let error = choose_with(&Failing(status)).await;
        assert!(!UnusableAnswer::is_behind(&error), "{status:?}");
        assert_eq!(JevBilled::usage_of(&error), None);
    }
    // A body that is not JSON is the transport's failure, retried there once.
    let garbled = MockServer::start(vec![Reply::ok("not json")]).await;
    let hosted = TypeSafeHttpTransport::new(&garbled.url, "sk-typesafe", fast_policy());
    let error = choose_with(&hosted).await;
    assert_eq!(
        error.to_string(),
        "Invalid TypeSafe response; no action executed."
    );
    assert!(!UnusableAnswer::is_behind(&error));
}
