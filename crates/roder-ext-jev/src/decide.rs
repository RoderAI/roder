//! Asking TypeSafe which operation to run and on which target.
//!
//! A port of upstream Jev's `model.choose` and `model.validate_choice`. The
//! request body and the validation rules are the contract with the service, so
//! both are reproduced exactly and pinned by fixtures recorded from upstream.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::engine::{JevDecision, JevDecisionClient, JevDecisionTransport};
use crate::prompts::{NEXT_ACTION, TARGET};
use crate::space::{ActionSpace, action_space};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const TIMEOUT: Duration = Duration::from_secs(25);
const RETRY_STATUSES: [u16; 3] = [429, 529, 503];

/// The same TypeSafe decision service used by the built-in `jev_browse` tool.
pub struct JevTypeSafeDecisionClient {
    model: String,
    transport: Arc<dyn JevDecisionTransport>,
}

impl JevTypeSafeDecisionClient {
    pub fn new(key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            transport: Arc::new(TypeSafeHttpTransport { key: key.into() }),
        }
    }

    pub fn with_transport(
        model: impl Into<String>,
        transport: Arc<dyn JevDecisionTransport>,
    ) -> Self {
        Self {
            model: model.into(),
            transport,
        }
    }
}

#[async_trait]
impl JevDecisionClient for JevTypeSafeDecisionClient {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        choose(
            observation,
            goal,
            history,
            &self.model,
            self.transport.as_ref(),
        )
        .await
        .map(|(decision, _)| decision)
    }
}

struct TypeSafeHttpTransport {
    key: String,
}

#[async_trait]
impl JevDecisionTransport for TypeSafeHttpTransport {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        post_json(ENDPOINT, &self.key, request).await
    }
}

/// Build the `questions` block: one operation choice, then one target choice
/// per operation that has targets.
fn questions(space: &ActionSpace, goal: &str) -> (Value, Vec<String>) {
    let mut operations = Map::new();
    for (operation, _) in &space.targets {
        let description = match operation.as_str() {
            "CLICK" => {
                "Click an element, button, menu option, autocomplete suggestion, or calendar day."
            }
            "TYPE_TEXT" => {
                "Enter or replace text in an editable field. A small LLM will supply the value from the goal."
            }
            "SELECT" => "Select an observed dropdown value.",
            _ => continue,
        };
        operations.insert(operation.clone(), json!(description));
    }
    for (name, action) in &space.controls {
        operations.insert(name.clone(), action["label"].clone());
    }
    operations.insert(
        "DONE".into(),
        json!("Every requirement is visibly satisfied."),
    );
    operations.insert(
        "BLOCKED".into(),
        json!("No supported operation can progress."),
    );
    let operation_ids = operations.keys().cloned().collect::<Vec<_>>();

    let mut questions = Map::new();
    questions.insert(
        "operation".into(),
        json!({
            "type": "choice",
            "criteria": Value::Object(operations),
            "instructions": {"goal": goal, "rules": NEXT_ACTION},
        }),
    );
    for (operation, candidates) in &space.targets {
        let mut criteria = Map::new();
        for (index, action) in candidates {
            let mut entry = Map::new();
            entry.insert(
                "element".into(),
                json!(format!(
                    "[{index}] {}",
                    action["label"].as_str().unwrap_or_default()
                )),
            );
            entry.insert(
                "current_value".into(),
                action
                    .get("current_value")
                    .or_else(|| action.get("value"))
                    .cloned()
                    .unwrap_or_else(|| json!("")),
            );
            for key in ["role", "checked", "selected", "expanded"] {
                if let Some(value) = action.get(key) {
                    entry.insert(key.into(), value.clone());
                }
            }
            criteria.insert(index.clone(), Value::Object(entry));
        }
        questions.insert(
            format!("{}_target", operation.to_lowercase()),
            json!({
                "type": "choice",
                "criteria": Value::Object(criteria),
                "instructions": {"goal": goal, "operation": operation, "rules": [NEXT_ACTION, TARGET]},
            }),
        );
    }
    (Value::Object(questions), operation_ids)
}

/// The exact body upstream posts to the decision service.
pub(crate) fn request_body(
    page: &Value,
    goal: &str,
    history: &[Value],
    model: &str,
) -> (Value, ActionSpace, Vec<String>) {
    let space = action_space(page["actions"].as_array().map_or(&[][..], Vec::as_slice));
    let (questions, operation_ids) = questions(&space, goal);
    let recent = history
        .iter()
        .skip(history.len().saturating_sub(10))
        .map(|entry| {
            let mut recorded = Map::new();
            for key in ["action", "kind", "text", "page_changed"] {
                recorded.insert(key.into(), entry.get(key).cloned().unwrap_or(Value::Null));
            }
            Value::Object(recorded)
        })
        .collect::<Vec<_>>();
    let mut page_view = Map::new();
    for key in ["url", "title", "text"] {
        page_view.insert(key.into(), page.get(key).cloned().unwrap_or(Value::Null));
    }
    let body = json!({
        "model": model,
        "state": {
            "page": Value::Object(page_view),
            "elements": space.elements.clone(),
            "recent_actions": recent,
        },
        "questions": questions,
    });
    (body, space, operation_ids)
}

/// Upstream's `validate_choice`: the answer must pick a known id, cover exactly
/// the offered ids, stay inside [0, 1], sum to one within tolerance, and not
/// contradict its own argmax.
pub(crate) fn validate_choice(answer: &Value, ids: &[String]) -> anyhow::Result<()> {
    let Some(probabilities) = answer.get("probabilities").and_then(Value::as_object) else {
        bail!("Invalid TypeSafe response; no action executed.");
    };
    let Some(choice) = answer.get("choice").and_then(Value::as_str) else {
        bail!("Invalid TypeSafe response; no action executed.");
    };
    let Some(confidence) = answer.get("confidence").and_then(Value::as_f64) else {
        bail!("Invalid TypeSafe response; no action executed.");
    };
    let known = ids.iter().any(|id| id == choice)
        && probabilities.len() == ids.len()
        && ids.iter().all(|id| probabilities.contains_key(id));
    let mut numbers = Vec::with_capacity(probabilities.len() + 1);
    for value in probabilities.values() {
        match value.as_f64() {
            Some(number) => numbers.push(number),
            None => bail!("Invalid TypeSafe response; no action executed."),
        }
    }
    numbers.push(confidence);
    let bounded = numbers
        .iter()
        .all(|number| number.is_finite() && (0.0..=1.0).contains(number));
    let total = probabilities
        .values()
        .filter_map(Value::as_f64)
        .sum::<f64>();
    let highest = probabilities
        .values()
        .filter_map(Value::as_f64)
        .fold(f64::NEG_INFINITY, f64::max);
    let chosen = probabilities
        .get(choice)
        .and_then(Value::as_f64)
        .unwrap_or(f64::NEG_INFINITY);
    if !(known && bounded && (total - 1.0).abs() < 0.02 && chosen >= highest - 1e-6) {
        bail!("Invalid TypeSafe response; no action executed.");
    }
    Ok(())
}

/// POST with upstream's retry policy: back off on 429/529/503, fail otherwise.
async fn post_json(url: &str, key: &str, body: &Value) -> anyhow::Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .context("build the decision client")?;
    for attempt in 0..3u32 {
        let response = client
            .post(url)
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Model connection failed; no action executed."))?;
        let status = response.status().as_u16();
        if RETRY_STATUSES.contains(&status) && attempt < 2 {
            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
            continue;
        }
        if status >= 400 {
            bail!("Model provider returned HTTP {status}; no action executed.");
        }
        return response
            .json::<Value>()
            .await
            .context("decode the decision response");
    }
    bail!("Model unavailable")
}

/// Ask for the next operation and resolve it to an executable choice.
pub(crate) async fn choose(
    page: &Value,
    goal: &str,
    history: &[Value],
    model: &str,
    transport: &dyn JevDecisionTransport,
) -> anyhow::Result<(JevDecision, ActionSpace)> {
    let (body, space, operation_ids) = request_body(page, goal, history, model);
    let started = Instant::now();
    let result = transport.decide(&body).await?;
    let answers = &result["answers"];
    let operation_answer = &answers["operation"];
    validate_choice(operation_answer, &operation_ids)?;
    let operation = operation_answer["choice"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let mut probabilities = Map::new();
    let mut target = None;
    let choice;
    if let Some(candidates) = space.targets_for(&operation) {
        let target_ids = candidates
            .iter()
            .map(|(index, _)| index.clone())
            .collect::<Vec<_>>();
        // Only the head the operation selected can cause an action.
        let target_answer = &answers[format!("{}_target", operation.to_lowercase())];
        validate_choice(target_answer, &target_ids)?;
        let selected = target_answer["choice"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let action = space
            .target_action(&operation, &selected)
            .context("TypeSafe chose an unoffered target")?;
        choice = action["id"].as_str().unwrap_or_default().to_string();
        for (index, candidate) in candidates {
            let id = candidate["id"].as_str().unwrap_or_default().to_string();
            probabilities.insert(id, target_answer["probabilities"][index].clone());
        }
        target = Some(selected);
    } else {
        choice = match space.control(&operation) {
            Some(action) => action["id"].as_str().unwrap_or_default().to_string(),
            None => operation.clone(),
        };
        probabilities.insert(
            choice.clone(),
            operation_answer["probabilities"][&operation].clone(),
        );
    }
    Ok((
        JevDecision {
            choice,
            operation,
            target,
            confidence: operation_answer["confidence"].as_f64().unwrap_or_default(),
            probabilities,
            latency_ms: started.elapsed().as_millis() as u64,
            usage: result.get("usage").cloned().unwrap_or_else(|| json!({})),
        },
        space,
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

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

    fn fixture(name: &str) -> Value {
        let raw = match name {
            "choose" => include_str!("../tests/fixtures/choose_request.json"),
            "validate" => include_str!("../tests/fixtures/validate_choice.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn request_body_matches_upstream_byte_for_byte() {
        let fixture = fixture("choose");
        let page: Value = serde_json::from_str(include_str!("../tests/fixtures/fingerprint.json"))
            .map(|value: Value| value["page"].clone())
            .unwrap();
        let history = fixture["history"].as_array().unwrap().clone();
        let (body, _, _) = request_body(
            &page,
            fixture["goal"].as_str().unwrap(),
            &history,
            "jev-latest",
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
    async fn injected_transport_keeps_request_and_response_parsing_upstream() {
        let page: Value = serde_json::from_str(include_str!("../tests/fixtures/fingerprint.json"))
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

        assert_eq!(decision.choice, "wait");
        assert_eq!(decision.operation, "WAIT");
        assert_eq!(decision.usage["input_tokens"], json!(10));
        assert_eq!(
            transport.request.lock().unwrap().as_ref().unwrap()["model"],
            json!("jev-hosted")
        );
    }
}
