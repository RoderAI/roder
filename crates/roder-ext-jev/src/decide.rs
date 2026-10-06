//! Asking TypeSafe which operation to run and on which target.
//!
//! A port of upstream Jev's `model.choose` and `model.validate_choice`. The
//! request body and the validation rules are the contract with the service, so
//! both are reproduced exactly and pinned by fixtures recorded from upstream.
//! Jev's state also carries today's date (see [`crate::text_helper::today`]):
//! "next month" and "the next Monday" mean nothing without it.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, bail};
use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use crate::engine::{JevDecision, JevDecisionClient, JevDecisionTransport};
use crate::http::{JsonPoster, PostFailure, RetryPolicy};
use crate::irreversible;
use crate::prompts::{NEXT_ACTION, TARGET};
use crate::space::{ActionSpace, action_space};
use crate::usage::JevBilled;

pub(crate) const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

/// The same TypeSafe decision service used by the built-in `jev_browse` tool.
pub struct JevTypeSafeDecisionClient {
    model: String,
    transport: Arc<dyn JevDecisionTransport>,
}

impl JevTypeSafeDecisionClient {
    pub fn new(key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            transport: Arc::new(TypeSafeHttpTransport::new(
                ENDPOINT,
                key,
                RetryPolicy::default(),
            )),
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
            chrono::Local::now().date_naive(),
            false,
        )
        .await
        .map(|(decision, _)| decision)
    }

    /// The same request with the irreversible-action gate's questions added
    /// (see [`crate::irreversible`]), and the chosen action's answer read.
    async fn choose_gated(
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
            chrono::Local::now().date_naive(),
            true,
        )
        .await
        .map(|(decision, _)| decision)
    }
}

/// The hosted decision service, over one pooled client for the whole run.
pub(crate) struct TypeSafeHttpTransport {
    url: String,
    key: String,
    http: JsonPoster,
}

impl TypeSafeHttpTransport {
    pub(crate) fn new(url: impl Into<String>, key: impl Into<String>, policy: RetryPolicy) -> Self {
        Self {
            url: url.into(),
            key: key.into(),
            http: JsonPoster::new(policy),
        }
    }
}

#[async_trait]
impl JevDecisionTransport for TypeSafeHttpTransport {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        self.http
            .post(&self.url, &self.key, request)
            .await
            .map_err(|failure| {
                let message = match &failure {
                    PostFailure::Connection => {
                        "Model connection failed; no action executed.".to_string()
                    }
                    PostFailure::Status(status, None) => {
                        format!("Model provider returned HTTP {status}; no action executed.")
                    }
                    PostFailure::Status(status, Some(detail)) => format!(
                        "Model provider returned HTTP {status} ({detail}); no action executed."
                    ),
                    PostFailure::InvalidBody => {
                        "Invalid TypeSafe response; no action executed.".to_string()
                    }
                    PostFailure::Failed(detail) => {
                        format!("Model provider failed ({detail}); no action executed.")
                    }
                };
                failure.stop(message)
            })
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
            "SCROLL_REGION_DOWN" => {
                "Scroll down inside a box that scrolls on its own (a text area, list or panel) to reveal more of it."
            }
            "SCROLL_REGION_UP" => "Scroll up inside a box that scrolls on its own.",
            "PRESS_ENTER" => {
                "Press Enter in the focused text field to submit or search for what it already holds."
            }
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
            // Right after the label it qualifies, so the option itself tells
            // twins apart.
            if let Some(context) = action.get("context") {
                entry.insert("context".into(), context.clone());
            }
            entry.insert(
                "current_value".into(),
                action
                    .get("current_value")
                    .or_else(|| action.get("value"))
                    .cloned()
                    .unwrap_or_else(|| json!("")),
            );
            for key in [
                "role",
                "input_type",
                "checked",
                "selected",
                "expanded",
                "scrolled",
                "offscreen",
            ] {
                if let Some(value) = crate::space::shown(action, key) {
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

/// The exact body upstream posts to the decision service, with Jev's `date`
/// first in the state. `today` is passed in so the body is deterministic
/// under test.
pub(crate) fn request_body(
    page: &Value,
    goal: &str,
    history: &[Value],
    model: &str,
    today: NaiveDate,
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
            "date": crate::text_helper::today(today),
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
        bail!("Invalid browser decision response; no action executed.");
    };
    let Some(choice) = answer.get("choice").and_then(Value::as_str) else {
        bail!("Invalid browser decision response; no action executed.");
    };
    let Some(confidence) = answer.get("confidence").and_then(Value::as_f64) else {
        bail!("Invalid browser decision response; no action executed.");
    };
    let known = ids.iter().any(|id| id == choice)
        && probabilities.len() == ids.len()
        && ids.iter().all(|id| probabilities.contains_key(id));
    let mut numbers = Vec::with_capacity(probabilities.len() + 1);
    for value in probabilities.values() {
        match value.as_f64() {
            Some(number) => numbers.push(number),
            None => bail!("Invalid browser decision response; no action executed."),
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
        bail!("Invalid browser decision response; no action executed.");
    }
    Ok(())
}

/// Ask for the next operation and resolve it to an executable choice. With
/// `gate`, the request also carries the irreversible-action gate's questions
/// and the decision the chosen action's answer; without it the request is
/// exactly as before.
pub(crate) async fn choose(
    page: &Value,
    goal: &str,
    history: &[Value],
    model: &str,
    transport: &dyn JevDecisionTransport,
    today: NaiveDate,
    gate: bool,
) -> anyhow::Result<(JevDecision, ActionSpace)> {
    let (mut body, space, operation_ids) = request_body(page, goal, history, model, today);
    let asked = if gate {
        irreversible::add_questions(&mut body)
    } else {
        Vec::new()
    };
    let started = Instant::now();
    let result = transport.decide(&body).await?;
    let usage = result.get("usage").cloned().unwrap_or_else(|| json!({}));
    // The service has answered, so the call is billed even when its answer
    // cannot be used: that usage is kept.
    let answer = read_answer(&result, &space, &operation_ids)
        .map_err(|error| anyhow::Error::from(JevBilled::new(usage.clone(), error)))?;
    // A missing or invalid answer is `None`, which the loop treats as
    // irreversible: the gate fails closed without failing the decision.
    let irreversible = irreversible::chosen_probability(
        &result["answers"],
        &asked,
        &answer.operation,
        answer.target.as_deref(),
    );
    Ok((
        JevDecision {
            choice: answer.choice,
            operation: answer.operation,
            target: answer.target,
            confidence: answer.confidence,
            target_confidence: answer.target_confidence,
            probabilities: answer.probabilities,
            latency_ms: started.elapsed().as_millis() as u64,
            usage,
            model: result["model"].as_str().map(str::to_string),
            irreversible,
        },
        space,
    ))
}

/// A validated answer, resolved to an observed action.
struct Answer {
    choice: String,
    operation: String,
    target: Option<String>,
    confidence: f64,
    target_confidence: Option<f64>,
    probabilities: Map<String, Value>,
}

fn read_answer(
    result: &Value,
    space: &ActionSpace,
    operation_ids: &[String],
) -> anyhow::Result<Answer> {
    let answers = &result["answers"];
    let operation_answer = &answers["operation"];
    validate_choice(operation_answer, operation_ids)?;
    let operation = operation_answer["choice"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let mut probabilities = Map::new();
    let mut target = None;
    let mut target_confidence = None;
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
            .context("Decision service chose an unoffered target")?;
        choice = action["id"].as_str().unwrap_or_default().to_string();
        for (index, candidate) in candidates {
            let id = candidate["id"].as_str().unwrap_or_default().to_string();
            probabilities.insert(id, target_answer["probabilities"][index].clone());
        }
        target = Some(selected);
        target_confidence = target_answer["confidence"].as_f64();
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
    Ok(Answer {
        choice,
        operation,
        target,
        confidence: operation_answer["confidence"].as_f64().unwrap_or_default(),
        target_confidence,
        probabilities,
    })
}

#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod tests;
