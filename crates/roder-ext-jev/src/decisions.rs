//! OpenAI Decisions wire adapter for the shared browser decision loop.
//!
//! Keeps action-space construction, validation and irreversible-action handling
//! identical across providers. Only this boundary knows the Decisions schema.

use std::sync::Arc;

use anyhow::{Context, bail};
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::decide::JevTypeSafeDecisionClient;
use crate::engine::{JevDecision, JevDecisionClient, JevDecisionTransport};
use crate::http::{JsonPoster, RetryPolicy};
use crate::usage::JevBilled;

const ENDPOINT: &str = "https://api.openai.com/v1/decisions";
pub(crate) const MODEL: &str = "gpt-6-luna";

/// Select browser actions with OpenAI's Decisions API, using an OpenAI API key.
/// Text generation continues to use the browser's configured text helper.
pub struct OpenAiDecisionsClient {
    inner: JevTypeSafeDecisionClient,
}

impl OpenAiDecisionsClient {
    pub fn new(key: impl Into<String>) -> Self {
        Self::with_transport(Arc::new(DecisionsHttp {
            key: key.into(),
            url: ENDPOINT.into(),
            http: JsonPoster::new(RetryPolicy::default()),
        }))
    }

    /// Inject a transport speaking the native OpenAI Decisions wire format.
    pub fn with_transport(transport: Arc<dyn JevDecisionTransport>) -> Self {
        Self {
            inner: JevTypeSafeDecisionClient::with_transport(
                MODEL,
                Arc::new(Adapter { transport }),
            ),
        }
    }
}

#[async_trait]
impl JevDecisionClient for OpenAiDecisionsClient {
    async fn choose(
        &self,
        page: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.inner.choose(page, goal, history).await
    }

    async fn choose_gated(
        &self,
        page: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.inner.choose_gated(page, goal, history).await
    }
}

struct DecisionsHttp {
    key: String,
    url: String,
    http: JsonPoster,
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
                    _ => "OpenAI Decisions request failed; no action executed.".into(),
                };
                failure.stop(message)
            })
    }
}

struct Adapter {
    transport: Arc<dyn JevDecisionTransport>,
}

#[async_trait]
impl JevDecisionTransport for Adapter {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let body = request_body(request)?;
        let response = self.transport.decide(&body).await?;
        normalize_response(&response).map_err(|error| {
            JevBilled::new(
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
                let choices = question["criteria"]
                    .as_object()
                    .context("Missing browser choices")?
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
            _ => bail!("Invalid OpenAI Decisions answer type"),
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
