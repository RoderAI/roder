//! A large request the service keeps answering with a 5xx is asked again
//! smaller; a small one still refused is an outage, as before.

use std::sync::Arc;

use super::asking::Shed;
use super::*;
use crate::engine::{JevStatus, JevStop};
use crate::http::tests::{MockServer, Reply, fast_policy};

/// Picks `operation` when it is offered, and the first option of every other
/// choice: the first offered target.
pub(crate) fn choosing(operation: &str, request: &Value) -> Value {
    let answers = request["questions"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, question)| {
            let keys = question["criteria"]
                .as_object()
                .map(|criteria| criteria.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let choice = match name.as_str() {
                "operation" if keys.iter().any(|k| k == operation) => operation.to_string(),
                _ => keys.first().cloned().unwrap_or_default(),
            };
            let probabilities = keys
                .iter()
                .map(|key| (key.clone(), json!(f64::from(*key == choice))))
                .collect::<Map<_, _>>();
            let answer = json!({"type": "choice", "choice": choice,
                "probabilities": probabilities, "confidence": 0.9});
            (name.clone(), answer)
        })
        .collect::<Map<_, _>>();
    json!({"model": "jev-test", "answers": answers,
        "usage": {"input_tokens": 10, "output_tokens": 1, "cost": 0.0001}})
}

/// A provider that sheds every request body over `limit` bytes, and clicks
/// the first offered target otherwise.
pub(crate) fn shedding_over(limit: usize) -> Reply {
    Reply::with(
        move |request: &Value| match request.to_string().len() > limit {
            true => (503, r#"{"error":"overloaded"}"#.to_string()),
            false => (200, choosing("CLICK", request).to_string()),
        },
    )
}

/// `count` cards with long labels, the first `onscreen` of them in view. Fifty
/// make a request of about 31 kB: inside TypeSafe's limits, so it is sent
/// whole, but above [`context_budget::SHED_ABOVE`].
fn dense_page(count: usize, onscreen: usize) -> Value {
    let actions = (0..count)
        .map(|n| {
            let mut action = json!({"id": format!("e{n}"), "node": n, "kind": "click",
                "role": "button", "label": format!("Add item {n} {}", "detail ".repeat(25))});
            if n >= onscreen {
                action["offscreen"] = json!(true);
            }
            action
        })
        .collect::<Vec<_>>();
    json!({"url": "https://shop.test/", "title": "Shop", "text": "Products", "actions": actions})
}

fn client(server: &MockServer) -> JevTypeSafeDecisionClient {
    JevTypeSafeDecisionClient::with_transport(
        "jev-test",
        Arc::new(TypeSafeHttpTransport::new(&server.url, "k", fast_policy())),
    )
}

#[tokio::test]
async fn a_large_request_shed_by_the_service_is_asked_again_without_offscreen_targets() {
    let page = dense_page(50, 10);
    let server = MockServer::start(vec![shedding_over(10_000)]).await;

    let decision = client(&server)
        .choose(&page, "Add item 3 to the cart", &[])
        .await
        .unwrap();

    assert_eq!(decision.operation, "CLICK");
    assert_eq!(decision.choice, "e0");
    assert_eq!(decision.usage["context_budget"]["shed"], 1);
    let requests = server.requests();
    let (_, answered) = requests.last().unwrap();
    let offered = answered["questions"]["click_target"]["criteria"]
        .as_object()
        .unwrap();
    assert_eq!(offered.len(), 10, "only the on-screen targets are offered");
    assert_eq!(
        answered["state"]["page"]["context_window"]["offscreen_targets_omitted"],
        40
    );
    assert!(requests.len() < 2 * (fast_policy().delays.len() + 1));
}

#[tokio::test]
async fn a_request_shed_again_on_screen_is_paged_smaller() {
    let page = dense_page(50, 50);
    let server = MockServer::start(vec![shedding_over(18_500)]).await;

    let decision = client(&server)
        .choose(&page, "Add item 3 to the cart", &[])
        .await
        .unwrap();

    assert_eq!(decision.operation, "CLICK");
    assert_eq!(decision.usage["context_budget"]["shed"], 1);
    let requests = server.requests();
    let (_, answered) = requests.last().unwrap();
    assert_eq!(
        answered["state"]["page"]["context_window"]["targets_shown"],
        24
    );
}

#[tokio::test]
async fn a_small_request_still_refused_is_an_outage() {
    let page = dense_page(3, 3);
    let server = MockServer::start(vec![Reply::status(503)]).await;

    let error = client(&server)
        .choose(&page, "Add item 1 to the cart", &[])
        .await
        .unwrap_err();

    assert!(Shed::is(&error));
    assert_eq!(JevStop::status_of(&error), JevStatus::Unavailable);
    // The batch twice, then its first question alone within the retries
    // left, as many requests as one unsplit request; never a smaller one.
    assert_eq!(server.hits(), fast_policy().delays.len() + 1);
    assert!(
        server
            .requests()
            .iter()
            .all(|(_, request)| request["state"]["page"].get("context_window").is_none())
    );
}
