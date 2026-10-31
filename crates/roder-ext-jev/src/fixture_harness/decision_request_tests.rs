//! What a hosted Jev decision sends for a real page, through the HTTP
//! transport to a local stand-in for the service: a target choice with one
//! option is answered without asking, and a dense step the service keeps
//! answering with a 503 is asked again smaller instead of ending the run.
//! Rows go to `target/jev-fixtures/decision_requests_*.jsonl`.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use super::write_jsonl;
use crate::decide::TypeSafeHttpTransport;
use crate::decide::shed_tests::choosing;
use crate::http::tests::{MockServer, Reply, fast_policy};
use crate::{JevDecisionClient, JevTypeSafeDecisionClient};

#[derive(Debug, Serialize)]
struct RequestRow {
    case: &'static str,
    ok: bool,
    requests: usize,
    bytes_sent: usize,
    largest_request_bytes: usize,
    offered_targets: usize,
    shed: Value,
}

fn client(server: &MockServer) -> JevTypeSafeDecisionClient {
    JevTypeSafeDecisionClient::with_transport(
        "jev-test",
        Arc::new(TypeSafeHttpTransport::new(&server.url, "k", fast_policy())),
    )
}

fn row(
    case: &'static str,
    server: &MockServer,
    decision: Option<&crate::JevDecision>,
) -> RequestRow {
    let requests = server.requests();
    let sizes = requests
        .iter()
        .map(|(_, request)| request.to_string().len())
        .collect::<Vec<_>>();
    RequestRow {
        case,
        ok: decision.is_some(),
        requests: requests.len(),
        bytes_sent: sizes.iter().sum(),
        largest_request_bytes: sizes.iter().copied().max().unwrap_or(0),
        offered_targets: requests.last().map_or(0, |(_, request)| {
            request["questions"]["click_target"]["criteria"]
                .as_object()
                .map_or(0, |criteria| criteria.len())
        }),
        shed: decision.map_or(Value::Null, |d| d.usage["context_budget"]["shed"].clone()),
    }
}

#[tokio::test]
async fn the_only_field_is_typed_into_without_asking_which() {
    let harness = harness_or_skip!();
    let mut page = harness.open("shelf.html?n=3").await.unwrap();
    let observation = page.observe().await.unwrap();
    let server = MockServer::start(vec![Reply::with(|request: &Value| {
        (200, choosing("TYPE_TEXT", request).to_string())
    })])
    .await;

    let decision = client(&server)
        .choose(&observation, "Search the shelf for mugs", &[])
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let asked = requests[0].1["questions"].as_object().unwrap();
    assert!(
        !asked.contains_key("type_text_target"),
        "{:?}",
        asked.keys()
    );
    assert!(asked.contains_key("click_target"));
    assert_eq!(decision.operation, "TYPE_TEXT");
    assert_eq!(decision.target_confidence, Some(1.0));
    let field = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["kind"] == "fill")
        .unwrap();
    assert_eq!(decision.choice, field["id"].as_str().unwrap());
    write_jsonl(
        "decision_requests_one_field",
        &[row("one_field", &server, Some(&decision))],
    )
    .unwrap();
}

#[tokio::test]
async fn a_dense_shelf_the_service_sheds_is_asked_again_on_screen_and_acted_on() {
    let harness = harness_or_skip!();
    let mut page = harness.open("shelf.html?n=60").await.unwrap();
    let observation = page.observe().await.unwrap();
    let actions = observation["actions"].as_array().unwrap();
    let offscreen = actions.iter().filter(|a| a["offscreen"] == true).count();
    assert!(
        offscreen > 40,
        "most products are below the fold: {offscreen}"
    );
    // Sheds the whole page's request and each of its questions alone, as the
    // gateway sheds a dense step, but answers the on-screen request.
    let server = MockServer::start(vec![Reply::with(|request: &Value| {
        match request.to_string().len() > SHED_OVER {
            true => (503, r#"{"error":"overloaded"}"#.to_string()),
            false => (200, adding("Add product 2 ", request).to_string()),
        }
    })])
    .await;

    let decision = client(&server)
        .choose(&observation, "Add product 2 to the basket", &[])
        .await;

    let result = row("dense_shelf", &server, decision.as_ref().ok());
    eprintln!("decision requests: {result:?}");
    write_jsonl("decision_requests_dense", &[result]).unwrap();
    let decision = decision.unwrap();
    assert_eq!(decision.usage["context_budget"]["shed"], 1);
    let requests = server.requests();
    let (_, answered) = requests.last().unwrap();
    assert!(
        answered["state"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|element| element.get("offscreen").is_none()),
        "only on-screen controls are offered after the service shed the step"
    );
    let chosen = actions
        .iter()
        .find(|action| action["id"] == decision.choice.as_str())
        .unwrap()
        .clone();
    assert_eq!(chosen["label"], "Add product 2 to the basket");
    page.act(&chosen, &observation, None, Duration::from_millis(100))
        .await
        .unwrap();
    assert_eq!(page.evaluate("window.added").await.unwrap(), json!([2]));
}

/// The body over which the stand-in service answers 503: below the whole
/// shelf's request and each of its questions asked alone, above the request
/// with only the on-screen products.
const SHED_OVER: usize = 12_000;

/// Clicks the offered target whose element names `label`.
fn adding(label: &str, request: &Value) -> Value {
    let mut reply = choosing("CLICK", request);
    let Some(criteria) = request["questions"]["click_target"]["criteria"].as_object() else {
        return reply;
    };
    let Some(target) = criteria
        .iter()
        .find(|(_, entry)| entry["element"].as_str().is_some_and(|e| e.contains(label)))
        .map(|(index, _)| index.clone())
    else {
        return reply;
    };
    let answer = &mut reply["answers"]["click_target"];
    answer["choice"] = json!(target);
    for (index, probability) in answer["probabilities"].as_object_mut().unwrap() {
        *probability = json!(f64::from(*index == target));
    }
    reply
}
