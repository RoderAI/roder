//! Asking TypeSafe which operation to run and on which target.
//!
//! A port of upstream Jev's `model.choose` and `model.validate_choice`. The
//! base request body and validation rules are pinned by upstream fixtures. The
//! hosted Jev client adds measured browser evidence and workflow guidance through
//! `jev_prompt`; other backends can explicitly retain the base request.
//! Jev's state also carries today's date (see [`crate::text_helper::today`]):
//! "next month" and "the next Monday" mean nothing without it.

use std::sync::Arc;
use std::time::Instant;

use anyhow::bail;
use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use crate::engine::{JevDecision, JevDecisionClient, JevDecisionTransport};
use crate::http::{JsonPoster, PostFailure, RetryPolicy};
use crate::irreversible;
use crate::prompts::{NEXT_ACTION, TARGET};
use crate::space::{ActionSpace, action_space};
use crate::usage::JevBilled;

mod activation;
mod answer;
mod context_budget;
mod context_pages;
mod hosted;
use answer::read_answer;
pub(crate) use answer::validate_choice;

pub(crate) struct DecisionOptions<'a> {
    pub gate: bool,
    pub activation_audit: Option<&'a dyn JevDecisionTransport>,
}

pub(crate) const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

/// The same TypeSafe decision service used by the built-in `jev_browse` tool.
pub struct JevTypeSafeDecisionClient {
    model: String,
    profile: crate::jev_prompt::Profile,
    transport: Arc<dyn JevDecisionTransport>,
    images: bool,
    composed_transport: bool,
}

impl JevTypeSafeDecisionClient {
    /// Native adapters reserve their own leaf provider requests.
    pub(crate) fn with_composed_transport(mut self) -> Self {
        self.composed_transport = true;
        self
    }
    pub(crate) fn with_profile(mut self, profile: crate::jev_prompt::Profile) -> Self {
        self.profile = profile;
        self
    }

    /// Send the current viewport screenshot with each decision (TypeSafe
    /// image input, off by default). The engine captures it only for clients
    /// that use images and never around secret fields; it is fitted to
    /// TypeSafe's limits and referenced from the request state. A screenshot
    /// that cannot be fitted is left out and the decision is made from text.
    pub fn with_images(mut self, images: bool) -> Self {
        self.images = images;
        self
    }
    pub fn new(key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            profile: crate::jev_prompt::Profile::Workflow,
            transport: Arc::new(TypeSafeHttpTransport::new(
                ENDPOINT,
                key,
                RetryPolicy::default(),
            )),
            images: false,
            composed_transport: false,
        }
    }

    pub fn with_transport(
        model: impl Into<String>,
        transport: Arc<dyn JevDecisionTransport>,
    ) -> Self {
        Self {
            model: model.into(),
            profile: crate::jev_prompt::Profile::Workflow,
            transport,
            images: false,
            composed_transport: false,
        }
    }
}

#[async_trait]
impl JevDecisionClient for JevTypeSafeDecisionClient {
    fn uses_images(&self) -> bool {
        self.images
    }

    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.decide(observation, goal, history, false).await
    }

    /// The same request with the irreversible-action gate's questions added
    /// (see [`crate::irreversible`]), and the chosen action's answer read.
    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.decide(observation, goal, history, true).await
    }
}

/// Adds the fitted viewport screenshot once the prompt profile has shaped
/// the request; without one the request passes through unchanged.
struct Imaged<'a> {
    inner: &'a dyn JevDecisionTransport,
    image: Option<images::Fitted>,
    observation: &'a Value,
    placement: images::Placement,
}

#[async_trait]
impl JevDecisionTransport for Imaged<'_> {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        match &self.image {
            None => self.inner.decide(request).await,
            Some(fitted) => {
                let mut request = request.clone();
                images::attach(&mut request, fitted, self.observation, self.placement);
                self.inner.decide(&request).await
            }
        }
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
        let headers = match images::has_assets(request) {
            true => vec![(images::IMAGES_HEADER, "true".to_string())],
            false => Vec::new(),
        };
        self.http
            .post_with(&self.url, &self.key, &headers, request)
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
                failure.stop_decision(message)
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
                "ref",
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
    for key in ["url", "title", "text", "context_window"] {
        if key == "context_window" && page.get(key).is_none() {
            continue;
        }
        page_view.insert(key.into(), page.get(key).cloned().unwrap_or(Value::Null));
    }
    let mut body = json!({
        "model": model,
        "state": {
            "date": crate::text_helper::today(today),
            "page": Value::Object(page_view),
            "elements": space.elements.clone(),
            "recent_actions": recent,
        },
        "questions": questions,
    });
    if let Some(dom) = page.get("dom_snapshot") {
        body["state"]["dom_snapshot"] = dom.clone();
    }
    for key in ["dialogs", "frames"] {
        if let Some(events) = page
            .get(key)
            .filter(|value| value.as_array().is_some_and(|v| !v.is_empty()))
        {
            body["state"][key] = events.clone();
        }
    }
    (body, space, operation_ids)
}

/// Upstream's `validate_choice`: the answer must pick a known id, cover exactly
/// the offered ids, stay inside [0, 1], sum to one within tolerance, and not
/// contradict its own argmax.
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
    options: DecisionOptions<'_>,
) -> anyhow::Result<(JevDecision, ActionSpace)> {
    let (mut body, space, operation_ids) = request_body(page, goal, history, model, today);
    let asked = if options.gate {
        irreversible::add_questions(&mut body)
    } else {
        Vec::new()
    };
    let started = Instant::now();
    let result = transport.decide(&body).await?;
    let usage = result.get("usage").cloned().unwrap_or_else(|| json!({}));
    // The service has answered, so the call is billed even when its answer
    // cannot be used: that usage is kept, and the loop may ask again.
    let answer = read_answer(&result, &space, &operation_ids)
        .map_err(|error| anyhow::Error::from(JevBilled::unusable(usage.clone(), error)))?;
    let (answer, usage) = if let Some(auditor) = options
        .activation_audit
        .filter(|_| answer.operation == "DONE")
    {
        let mut audit_body = body.clone();
        crate::jev_prompt::rewrite(
            &mut audit_body,
            crate::jev_prompt::Profile::DomTargets,
            history,
            page,
        );
        activation::repair(auditor, &audit_body, &result, &space, answer).await?
    } else {
        (answer, usage)
    };
    // A missing or invalid answer is `None`, which the loop treats as
    // irreversible: the gate fails closed without failing the decision.
    if let Some(required) = asked.iter().find(|q| {
        q.operation == answer.operation && Some(q.target.as_str()) == answer.target.as_deref()
    }) {
        reject_refusal(&result["answers"][&required.key], &required.key)
            .map_err(|error| anyhow::Error::from(JevBilled::unusable(usage.clone(), error)))?;
    }
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

#[cfg(test)]
mod body_loop_tests;
#[cfg(test)]
mod gate_tests;
pub(crate) mod images;
#[cfg(test)]
mod images_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unusable_tests;

fn reject_refusal(answer: &Value, name: &str) -> anyhow::Result<()> {
    if answer["type"] == "refusal" {
        bail!("OpenAI Decisions refused required question {name}; no action executed.");
    }
    Ok(())
}
