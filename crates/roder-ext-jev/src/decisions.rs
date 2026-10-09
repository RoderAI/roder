//! OpenAI Decisions wire adapter for the shared browser decision loop.
//!
//! Keeps action-space construction, validation and irreversible-action handling
//! identical across providers. Only this boundary knows the Decisions schema.

mod computer;
#[cfg(test)]
mod experiment_tests;
pub(crate) mod planning;
pub use computer::{ComputerCandidate, ComputerDecision};
use planning::Strategy;

use std::sync::Arc;

use anyhow::{Context, bail};
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::decide::JevTypeSafeDecisionClient;
use crate::engine::{JevDecision, JevDecisionClient, JevDecisionTransport};
use crate::http::{JsonPoster, RetryPolicy};
use crate::usage::JevBilled;

tokio::task_local! { static EVIDENCE: Value; }
fn evidence(page: &Value, history: &[Value]) -> Value {
    let history = history
        .iter()
        .skip(history.len().saturating_sub(10))
        .map(|entry| {
            let mut kept = Map::new();
            for key in [
                "action",
                "kind",
                "text",
                "context",
                "covered",
                "page_changed",
                "effect",
                "text_added",
            ] {
                if let Some(value) = entry.get(key) {
                    kept.insert(key.into(), value.clone());
                }
            }
            Value::Object(kept)
        })
        .collect::<Vec<_>>();
    json!({"history":history,"screenshot":page["_screenshot"],"rectangles":page["actions"].as_array().into_iter().flatten().filter_map(|a|a.get("rect").map(|r|json!({"label":a["label"],"context":a["context"],"rect":r}))).collect::<Vec<_>>()})
}

const ENDPOINT: &str = "https://api.openai.com/v1/decisions";
pub(crate) const MODEL: &str = "gpt-6-luna";

/// Select browser actions with OpenAI's Decisions API, using an OpenAI API key.
/// Text generation continues to use the browser's configured text helper.
pub struct OpenAiDecisionsClient {
    inner: JevTypeSafeDecisionClient,
    strategy: Strategy,
    transport: Arc<dyn JevDecisionTransport>,
}

impl OpenAiDecisionsClient {
    pub fn new(key: impl Into<String>) -> Self {
        Self::with_transport(Arc::new(DecisionsHttp {
            key: key.into(),
            url: ENDPOINT.into(),
            http: JsonPoster::new(RetryPolicy::default()),
        }))
    }

    /// Use text observations and action effects without requesting screenshots.
    pub fn text_only(self) -> Self {
        Self::configured(self.transport, Strategy::Effects)
    }

    /// Inject a transport speaking the native OpenAI Decisions wire format.
    pub fn with_transport(transport: Arc<dyn JevDecisionTransport>) -> Self {
        Self::configured(transport, Strategy::Vision)
    }

    pub(crate) fn configured(transport: Arc<dyn JevDecisionTransport>, strategy: Strategy) -> Self {
        Self {
            strategy,
            inner: JevTypeSafeDecisionClient::with_transport(
                MODEL,
                Arc::new(Adapter {
                    transport: transport.clone(),
                    strategy,
                }),
            )
            .with_profile(crate::jev_prompt::Profile::Baseline),
            transport,
        }
    }
    fn finish(
        &self,
        decision: JevDecision,
        page: &Value,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        if self.strategy == Strategy::Grounded {
            let action = page["actions"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|a| a["id"] == decision.choice);
            let last = history
                .iter()
                .rev()
                .find(|entry| entry["kind"] != "cookie_banner");
            if let (Some(action), Some(last)) = (action, last)
                && action["kind"] == "click"
                && last["kind"] == "click"
                && last["choice"] == decision.choice
                && last["action"] == action["label"]
                && last["page_changed"] == true
                && decision.confidence < 0.75
            {
                return Err(JevBilled::new(
                    decision.usage,
                    anyhow::anyhow!(
                        "Uncertain repeated action without verified completion; no action executed."
                    ),
                )
                .into());
            }
        }
        Ok(decision)
    }
}

#[async_trait]
impl JevDecisionClient for OpenAiDecisionsClient {
    fn uses_images(&self) -> bool {
        self.strategy == Strategy::Vision
    }

    async fn choose(
        &self,
        page: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.finish(
            EVIDENCE
                .scope(
                    evidence(page, history),
                    self.inner.choose(page, goal, history),
                )
                .await?,
            page,
            history,
        )
    }

    async fn choose_gated(
        &self,
        page: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.finish(
            EVIDENCE
                .scope(
                    evidence(page, history),
                    self.inner.choose_gated(page, goal, history),
                )
                .await?,
            page,
            history,
        )
    }
}

pub(crate) struct DecisionsHttp {
    pub(crate) key: String,
    pub(crate) url: String,
    pub(crate) http: JsonPoster,
}

#[async_trait]
impl JevDecisionTransport for DecisionsHttp {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        self.http
            .post(&self.url, &self.key, request)
            .await
            .map_err(|failure| {
                // Do not echo provider bodies containing page data or credentials.
                let message = match &failure {
                    crate::http::PostFailure::Status(status, _) => {
                        format!("OpenAI Decisions returned HTTP {status}; no action executed.")
                    }
                    crate::http::PostFailure::InvalidBody => {
                        "OpenAI Decisions returned a reply that could not be read; no action \
                         executed."
                            .into()
                    }
                    _ => "OpenAI Decisions request failed; no action executed.".into(),
                };
                failure.stop_decision(message)
            })
    }
}

struct Adapter {
    transport: Arc<dyn JevDecisionTransport>,
    strategy: Strategy,
}

#[async_trait]
impl JevDecisionTransport for Adapter {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let response = planning::evaluate(self.transport.as_ref(), request, self.strategy).await?;
        normalize_response(&response).map(|mut normalized| {
            // Decisions requires at least two choices. A single observed target
            // is deterministic once the operation has been chosen by the model.
            for (name, question) in request["questions"].as_object().into_iter().flatten() {
                if question["type"] == "choice"
                    && let Some(criteria) = question["criteria"].as_object()
                    && criteria.len() == 1
                    && let Some((value, _)) = criteria.iter().next()
                {
                    normalized["answers"].as_object_mut().unwrap().entry(name.clone()).or_insert_with(|| {
                        json!({"choice":value,"confidence":1.0,"probabilities":{value:1.0}})
                    });
                }
            }
            normalized
        }).map_err(|error| {
            JevBilled::unusable(
                response.get("usage").cloned().unwrap_or_else(|| json!({})),
                error,
            )
            .into()
        })
    }
}

fn description(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn request_body(request: &Value) -> anyhow::Result<Value> {
    let questions = request["questions"]
        .as_object()
        .context("Missing browser questions")?;
    let mut converted = Vec::with_capacity(questions.len());
    for (name, question) in questions {
        let mut instructions = description(&question["instructions"]);
        let converted_question = match question["type"].as_str() {
            Some("choice") => {
                let criteria = question["criteria"]
                    .as_object()
                    .context("Missing browser choices")?;
                if criteria.len() == 1 {
                    continue;
                }
                let choices = criteria
                    .iter()
                    .map(|(value, criterion)| {
                        json!({
                            "value": value, "description": description(criterion),
                        })
                    })
                    .collect::<Vec<_>>();
                json!({"type": "choice", "name": name, "instructions": instructions, "choices": choices})
            }
            Some("noul") => {
                instructions.push_str(&format!(
                    " True: {} False: {}",
                    description(&question["criteria"]["true"]),
                    description(&question["criteria"]["false"])
                ));
                json!({"type": "predicate", "name": name, "instructions": instructions})
            }
            _ => bail!("Unsupported browser decision question"),
        };
        converted.push(converted_question);
    }
    Ok(json!({"model": MODEL, "input": request["state"].to_string(), "questions": converted}))
}

fn normalize_response(response: &Value) -> anyhow::Result<Value> {
    let raw = response["answers"]
        .as_array()
        .context("Invalid OpenAI Decisions answers")?;
    let mut answers = Map::new();
    for answer in raw {
        let name = answer["name"]
            .as_str()
            .context("Missing OpenAI Decisions answer name")?;
        let converted = match answer["type"].as_str() {
            Some("choice") => {
                let mut probabilities = Map::new();
                for entry in answer["probabilities"]
                    .as_array()
                    .context("Invalid OpenAI Decisions probabilities")?
                {
                    let value = entry["value"]
                        .as_str()
                        .context("Invalid OpenAI Decisions choice value")?;
                    if probabilities
                        .insert(value.into(), entry["probability"].clone())
                        .is_some()
                    {
                        bail!("Duplicate OpenAI Decisions choice value");
                    }
                }
                json!({"choice": answer["choice"], "confidence": answer["confidence"], "probabilities": probabilities})
            }
            Some("predicate") => json!({"type": "noul", "noul": answer["probability"]}),
            Some("refusal") => json!({"type":"refusal", "name":name}),
            _ => bail!(
                "Invalid OpenAI Decisions answer type: {:?}",
                answer["type"].as_str()
            ),
        };
        if answers.insert(name.into(), converted).is_some() {
            bail!("Duplicate OpenAI Decisions answer name");
        }
    }
    Ok(
        json!({"answers": answers, "model": response.get("model").cloned().unwrap_or_else(|| json!(MODEL)),
        "usage": response.get("usage").cloned().unwrap_or_else(|| json!({}))}),
    )
}

#[cfg(test)]
mod tests;
