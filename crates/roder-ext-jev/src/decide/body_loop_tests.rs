//! The agent loop on a hosted decision service whose body cannot be decoded
//! ("Invalid TypeSafe response"): the real hosted transport against a local
//! server (which resends such a body once), the real hosted client and the
//! real loop, on a page that never changes. The body is the service's reply,
//! so the loop asks again; refused or unreachable calls it does not.

use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::engine::{
    JevActOutcome, JevBrowser, JevEngine, JevEngineConfig, JevRunResult, JevStatus, JevStopCause,
};
use crate::fallback::trigger::{Trigger, trigger};
use crate::http::tests::{MockServer, Reply, fast_policy};
use crate::usage::JevTokenCount;

/// One page with a button, which nothing changes.
struct StaticPage;

#[async_trait]
impl JevBrowser for StaticPage {
    async fn observe(&mut self) -> anyhow::Result<Value> {
        Ok(json!({
            "url": "https://a.test/", "title": "A", "text": "A", "fingerprint": "f",
            "actions": [{"id": "e1", "node": 1, "kind": "click", "role": "button",
                "label": "Buy", "value": ""}],
        }))
    }

    async fn fresh(&mut self, _: &Value, _: Option<&Value>) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn act(
        &mut self,
        _: &Value,
        _: &Value,
        _: Option<&str>,
        _: Duration,
    ) -> anyhow::Result<JevActOutcome> {
        Ok(JevActOutcome::done())
    }
}

/// A hosted service whose first `garbled` replies come back over real HTTP
/// with a body that is not JSON, and the rest as a valid DONE.
struct Service {
    garbled: usize,
    asked: Mutex<usize>,
    http: TypeSafeHttpTransport,
}

impl Service {
    fn new(server: &MockServer, garbled: usize) -> Self {
        Self {
            garbled,
            asked: Mutex::new(0),
            http: TypeSafeHttpTransport::new(&server.url, "sk-typesafe", fast_policy()),
        }
    }
}

#[async_trait]
impl JevDecisionTransport for Service {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let asked = {
            let mut asked = self.asked.lock().unwrap();
            *asked += 1;
            *asked
        };
        if asked <= self.garbled {
            return self.http.decide(request).await;
        }
        let operations = request["questions"]["operation"]["criteria"]
            .as_object()
            .unwrap();
        let probabilities = operations
            .keys()
            .map(|id| (id.clone(), json!(f64::from(id == "DONE"))))
            .collect::<Map<_, _>>();
        Ok(json!({
            "answers": {"operation": {"choice": "DONE", "confidence": 0.9,
                "probabilities": probabilities}},
            "usage": {"input_tokens": 50, "output_tokens": 1},
        }))
    }
}

async fn run_on(transport: impl JevDecisionTransport + 'static) -> JevRunResult {
    let client = JevTypeSafeDecisionClient::with_transport("jev-test", Arc::new(transport));
    let config = JevEngineConfig::new("Buy it", Arc::new(client)).with_wait(Duration::ZERO);
    let mut engine = JevEngine::start(Box::new(StaticPage), config)
        .await
        .unwrap();
    engine.run(Duration::from_secs(10)).await
}

#[tokio::test]
async fn an_undecodable_body_twice_then_a_valid_reply_ends_done() {
    let server = MockServer::start(vec![Reply::ok("<html>bad gateway</html>")]).await;
    let result = run_on(Service::new(&server, 2)).await;
    assert_eq!(
        result.status,
        JevStatus::Done,
        "{:?}",
        result.stopped_because
    );
    assert_eq!(result.stopped_because, None);
    // Three questions asked: two replies that could not be decoded, then one
    // that could. Each undecodable body was posted twice (the resend).
    assert_eq!(result.model_calls, 3);
    assert_eq!(server.hits(), 4);
    assert_eq!(result.decisions.len(), 1);
    // The usage of a reply that cannot be decoded is not known, so the sum
    // is not either: never a number that reads as free.
    assert_eq!(result.usage.decision.calls, 3);
    assert_eq!(result.usage.decision.input_tokens, JevTokenCount::Unknown);
    assert_eq!(result.usage.decision.output_tokens, JevTokenCount::Unknown);
}

#[tokio::test]
async fn a_body_that_stays_undecodable_ends_decision_unusable_and_falls_back() {
    let server = MockServer::start(vec![Reply::ok("not json")]).await;
    let result = run_on(Service::new(&server, usize::MAX)).await;
    assert_eq!(result.status, JevStatus::Error);
    assert_eq!(result.stop_cause, Some(JevStopCause::DecisionUnusable));
    assert_eq!(result.model_calls, 3);
    assert_eq!(server.hits(), 6);
    assert_eq!(
        result.stopped_because.as_deref(),
        Some(
            "The decision service gave 3 unusable replies in a row. The first: \
             Invalid TypeSafe response; no action executed."
        )
    );
    assert_eq!(result.usage.decision.calls, 3);
    assert_eq!(result.usage.decision.input_tokens, JevTokenCount::Unknown);
    assert!(result.actions.is_empty());
    assert_eq!(trigger(&result), Some(Trigger::DecisionUnusable));
}

#[tokio::test]
async fn a_refused_or_unreachable_service_is_not_asked_again_and_does_not_fall_back() {
    // A key, billing or access refusal fails at once; a rate limit is retried
    // by the transport and, still failing, leaves the provider unavailable.
    for (status, ends) in [
        (401, JevStatus::Error),
        (402, JevStatus::Error),
        (403, JevStatus::Error),
        (429, JevStatus::Unavailable),
    ] {
        let server = MockServer::start(vec![Reply::status(status)]).await;
        let hosted = TypeSafeHttpTransport::new(&server.url, "sk-typesafe", fast_policy());
        let result = run_on(hosted).await;
        assert_eq!(result.status, ends, "HTTP {status}");
        assert_eq!(
            result.stop_cause,
            Some(JevStopCause::Stopped),
            "HTTP {status}"
        );
        assert_eq!(result.model_calls, 0, "HTTP {status}");
        assert_eq!(trigger(&result), None, "HTTP {status}");
        if ends == JevStatus::Error {
            assert_eq!(server.hits(), 1, "HTTP {status}");
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let hosted = TypeSafeHttpTransport::new(&closed, "sk-typesafe", fast_policy());
    let result = run_on(hosted).await;
    assert_eq!(result.status, JevStatus::Unavailable);
    assert_eq!(result.stop_cause, Some(JevStopCause::Stopped));
    assert_eq!(trigger(&result), None);
}
