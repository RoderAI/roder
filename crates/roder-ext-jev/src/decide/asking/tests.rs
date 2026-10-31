use std::sync::{Arc, Mutex};

use super::*;
use crate::engine::{JevStatus, JevStop};
use crate::http::tests::{MockServer, Reply, fast_policy};

/// Answers every choice it is asked with its first option.
fn first_options(request: &Value) -> Value {
    let answers = request["questions"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, question)| {
            let keys = question["criteria"]
                .as_object()
                .map(|criteria| criteria.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let probabilities = keys
                .iter()
                .enumerate()
                .map(|(n, key)| (key.clone(), json!(f64::from(n == 0))))
                .collect::<Map<_, _>>();
            let answer = json!({"type": "choice", "choice": keys.first(),
                "probabilities": probabilities, "confidence": 0.9});
            (name.clone(), answer)
        })
        .collect::<Map<_, _>>();
    json!({"model": "jev-test", "answers": answers,
        "usage": {"input_tokens": 100, "output_tokens": 10, "cost": 0.001}})
}

struct Recording(Mutex<Vec<Value>>);

#[async_trait]
impl JevDecisionTransport for Recording {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        self.0.lock().unwrap().push(request.clone());
        Ok(first_options(request))
    }
}

fn batch() -> Value {
    json!({"model": "jev-test", "state": {"page": {"url": "https://a.test/"}},
    "questions": {
        "operation": {"type": "choice", "criteria": {"CLICK": "c", "DONE": "d", "BLOCKED": "b"}},
        "click_target": {"type": "choice", "criteria": {"1": {"element": "[1] Buy"}}},
        "type_text_target": {"type": "choice", "criteria": {"2": {"element": "[2] Name"}, "3": {"element": "[3] Mail"}}},
        "irreversible_click_1": {"type": "noul", "instructions": "Is it irreversible?"},
    }})
}

#[tokio::test]
async fn a_single_option_choice_is_answered_without_asking() {
    let inner = Recording(Mutex::new(Vec::new()));

    let reply = Open(&inner).decide(&batch()).await.unwrap();

    let sent = inner.0.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let asked = sent[0]["questions"].as_object().unwrap();
    assert!(!asked.contains_key("click_target"));
    for kept in ["operation", "type_text_target", "irreversible_click_1"] {
        assert!(asked.contains_key(kept), "{kept} is still asked");
    }
    let forced = &reply["answers"]["click_target"];
    assert_eq!(forced["choice"], "1");
    assert_eq!(forced["probabilities"], json!({"1": 1.0}));
    assert_eq!(forced["confidence"], 1.0);
    super::super::validate_choice(forced, &["1".to_string()]).unwrap();
    assert_eq!(reply["answers"]["operation"]["choice"], "CLICK");
    assert_eq!(reply["usage"]["input_tokens"], 100);
}

#[tokio::test]
async fn nothing_is_sent_when_every_choice_has_one_option() {
    let inner = Recording(Mutex::new(Vec::new()));
    let request = json!({"model": "jev-test", "questions": {
        "click_target": {"type": "choice", "criteria": {"4": {"element": "[4] Go"}}}}});

    let reply = Open(&inner).decide(&request).await.unwrap();

    assert!(inner.0.lock().unwrap().is_empty());
    assert_eq!(reply["answers"]["click_target"]["choice"], "4");
    let usage = JevCallUsage::sum([&reply["usage"]]);
    assert_eq!(usage.calls, 0);
}

/// A provider that sheds every request carrying more than one question.
fn shedding_batches(request: &Value) -> (u16, String) {
    match request["questions"].as_object().map_or(0, Map::len) {
        0 | 1 => (200, first_options(request).to_string()),
        _ => (503, r#"{"error":"overloaded"}"#.to_string()),
    }
}

#[tokio::test]
async fn a_batch_shed_twice_is_asked_one_question_at_a_time() {
    let server = MockServer::start(vec![Reply::with(shedding_batches)]).await;
    let transport = TypeSafeHttpTransport::new(&server.url, "k", fast_policy());
    let request = batch();

    let reply = transport.decide(&request).await.unwrap();

    let questions = request["questions"].as_object().unwrap();
    // Two refusals of the batch, then each question once.
    assert_eq!(server.hits(), 2 + questions.len());
    for (name, _) in questions {
        assert!(reply["answers"].get(name).is_some(), "{name} answered");
    }
    let usage = JevCallUsage::sum([&reply["usage"]]);
    assert_eq!(usage.calls, questions.len());
    assert_eq!(
        usage.input_tokens,
        crate::usage::JevTokenCount::Known(100 * questions.len() as u64)
    );
}

#[tokio::test]
async fn a_part_that_still_fails_keeps_the_answered_parts_usage() {
    let calls = Arc::new(Mutex::new(0));
    let counted = calls.clone();
    let server = MockServer::start(vec![Reply::with(move |request: &Value| {
        let mut calls = counted.lock().unwrap();
        *calls += 1;
        // The batch twice, then the first part; the second part never clears.
        match *calls {
            3 => (200, first_options(request).to_string()),
            _ => (503, "{}".to_string()),
        }
    })])
    .await;
    let transport = TypeSafeHttpTransport::new(&server.url, "k", fast_policy());

    let error = transport.decide(&batch()).await.unwrap_err();

    assert!(Shed::is(&error));
    assert_eq!(JevStop::status_of(&error), JevStatus::Unavailable);
    assert_eq!(
        error.to_string(),
        "Model provider returned HTTP 503; no action executed."
    );
    let billed = JevBilled::usage_of(&error).expect("the answered part was billed");
    assert_eq!(billed["input_tokens"], 100);
    // The batch's two, the first part's own one, then the second part's own
    // one and the three retries the batch left.
    assert_eq!(server.hits(), 2 + 1 + 1 + (fast_policy().delays.len() - 2));
}

#[tokio::test]
async fn only_a_server_error_is_marked_shed() {
    for (status, shed) in [(503, true), (500, true), (400, false), (401, false)] {
        let server = MockServer::start(vec![Reply::status(status)]).await;
        let transport = TypeSafeHttpTransport::new(&server.url, "k", fast_policy());
        let request = json!({"questions": {"operation": {"type": "choice"}}});

        let error = transport.decide(&request).await.unwrap_err();

        assert_eq!(Shed::is(&error), shed, "HTTP {status}");
    }
}

#[tokio::test]
async fn an_image_only_a_local_answer_used_is_not_sent() {
    let inner = Recording(Mutex::new(Vec::new()));
    let mut request = batch();
    request["questions"]["click_target"]["instructions"] =
        json!({"viewport_screenshot": {"image": "img-1"}});
    request["assets"] = json!({"img-1": {"type": "image", "data_base64": "AAAA"}});

    Open(&inner).decide(&request).await.unwrap();

    let sent = inner.0.lock().unwrap().clone();
    assert!(
        !images::has_assets(&sent[0]),
        "the unreferenced image is dropped"
    );

    let inner = Recording(Mutex::new(Vec::new()));
    request["questions"]["type_text_target"]["instructions"] =
        json!({"viewport_screenshot": {"image": "img-1"}});
    Open(&inner).decide(&request).await.unwrap();
    let sent = inner.0.lock().unwrap().clone();
    assert!(
        images::has_assets(&sent[0]),
        "a still-referenced image is kept"
    );
}
