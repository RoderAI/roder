use super::*;
use crate::{JevDecisionClient, JevTypeSafeDecisionClient};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Never(AtomicUsize);
#[async_trait]
impl JevDecisionTransport for Never {
    async fn decide(&self, _: &Value) -> anyhow::Result<Value> {
        self.0.fetch_add(1, Ordering::Relaxed);
        unreachable!()
    }
}

#[tokio::test]
async fn each_question_and_the_whole_request_are_checked_before_network_io() {
    let inner = Never(AtomicUsize::new(0));
    let reports = Mutex::new(Vec::new());
    let guard = Bounded {
        inner: &inner,
        image_cost: 0,
        reports: &reports,
    };
    let one =
        json!({"state":"x".repeat(20_000),"questions":{"q":{"instructions":"y".repeat(14_000)}}});
    assert!(guard.decide(&one).await.unwrap_err().is::<TooLarge>());
    let many = json!({"state":"x".repeat(10_000),"questions":(0..60).map(|n|(n.to_string(),json!({"instructions":"y".repeat(1000)}))).collect::<serde_json::Map<_,_>>()});
    assert!(guard.decide(&many).await.unwrap_err().is::<TooLarge>());
    assert_eq!(inner.0.load(Ordering::Relaxed), 0);
}

#[test]
fn image_references_expand_each_use_without_counting_base64_as_text() {
    let a = json!({"state":{"image":"asset-id"},"questions":{"q":{"instructions":"Look"}},
        "assets":{"asset-id":{"data_base64":"x".repeat(1_000_000)}}});
    let mut b = a.clone();
    b["questions"]["q"]["instructions"] = json!("asset-id asset-id");
    let (total, question) = estimate(&a, 5000);
    assert!(total < 10_000 && question < 10_000);
    let (more, larger) = estimate(&b, 5000);
    assert!(more >= total + 10_000 && larger >= question + 10_000);
    let unicode = json!({"state":"界".repeat(10_000),"questions":{"q":{}}});
    assert!(estimate(&unicode, 0).1 > QUESTION_LIMIT);
}

struct Reader {
    calls: Mutex<Vec<Value>>,
    read_text: bool,
}
#[async_trait]
impl JevDecisionTransport for Reader {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let mut calls = self.calls.lock().unwrap();
        calls.push(request.clone());
        let operations = request["questions"]["operation"]["criteria"]
            .as_object()
            .unwrap();
        let operation = if calls.len() == 1 {
            if self.read_text {
                "READ_MORE_TEXT"
            } else {
                "READ_MORE_TARGETS"
            }
        } else {
            "CLICK"
        };
        assert!(operations.contains_key(operation), "{operations:?}");
        let head = |ids: Vec<String>, chosen: &str| {
            json!({"choice":chosen,"confidence":1.0,
            "probabilities":ids.iter().map(|id|(id.clone(),json!(f64::from(id==chosen)))).collect::<serde_json::Map<_,_>>()})
        };
        let mut answers = json!({"operation":head(operations.keys().cloned().collect(),operation)});
        if operation == "CLICK" {
            let ids = request["questions"]["click_target"]["criteria"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            answers["click_target"] = head(ids.clone(), &ids[0]);
        }
        Ok(json!({"answers":answers,"usage":{"input_tokens":100,"output_tokens":0}}))
    }
}

#[tokio::test]
async fn the_model_can_request_more_targets_or_text_without_a_browser_action() {
    for read_text in [false, true] {
        let transport = Arc::new(Reader {
            calls: Mutex::new(Vec::new()),
            read_text,
        });
        let client = JevTypeSafeDecisionClient::with_transport("jev-latest", transport.clone());
        let observation = json!({"url":"https://a.test/","text":"hello ".repeat(20_000),"evidence":"Source: https://a.test/prices\n".to_string()+&"historical fact ".repeat(2000),
            "actions":(0..100).map(|n|json!({"id":format!("e{n}"),"node":n,"kind":"click","label":format!("button {n}")})).collect::<Vec<_>>()});
        let decision = client
            .choose(&observation, "Find the needed button", &[])
            .await
            .unwrap();
        assert!(decision.choice.starts_with('e'));
        if !read_text {
            assert_ne!(
                decision.choice, "e0",
                "must resolve an original ID on the second page"
            );
        }
        assert_eq!(decision.usage["calls"], 2);
        assert_eq!(decision.usage["input_tokens"], 200);
        assert_eq!(decision.usage["context_budget"]["pages_read"], 1);
        for body in transport.calls.lock().unwrap().iter() {
            let (a, b) = estimate(body, 0);
            assert!(a <= REQUEST_LIMIT && b <= QUESTION_LIMIT);
        }
    }
}

#[tokio::test]
async fn an_oversized_required_goal_fails_locally_instead_of_truncating_instructions() {
    let transport = Arc::new(Never(AtomicUsize::new(0)));
    let client = JevTypeSafeDecisionClient::with_transport("jev-latest", transport.clone());
    let error = client
        .choose(
            &json!({"actions":[],"text":""}),
            &"critical rule ".repeat(10_000),
            &[],
        )
        .await
        .unwrap_err();
    assert!(error.is::<TooLarge>());
    assert_eq!(transport.0.load(Ordering::Relaxed), 0);
}
