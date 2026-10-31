//! What a hosted Jev decision actually sends.
//!
//! A choice with a single option is never in doubt, so it is answered here
//! (that option, probability 1) and left out of the request; the service
//! would answer it the same way and bill for reading it. A batch the service
//! answers with a 5xx twice in a row is asked again one question at a time
//! within the retries it had left: a dense batch is shed far more often than
//! one question with the same state. Either way the caller sees one reply
//! with every question answered and the requests' usage summed.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::images;
use crate::engine::JevDecisionTransport;
use crate::http::{JsonPoster, PostFailure, Posted, RetryPolicy, Schedule};
use crate::usage::{JevBilled, JevCallUsage};

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

    /// The time this transport's requests lost to transient failures (see
    /// [`JsonPoster::transient`]).
    #[cfg(test)]
    pub(crate) fn transient(&self) -> std::time::Duration {
        self.http.transient()
    }

    async fn post(&self, request: &Value, schedule: Schedule) -> Result<(Value, usize), Posted> {
        let headers = match images::has_assets(request) {
            true => vec![(images::IMAGES_HEADER, "true".to_string())],
            false => Vec::new(),
        };
        self.http
            .post_scheduled(&self.url, &self.key, &headers, request, schedule)
            .await
    }

    /// Ask each question of `request` on its own, `spent` retries in, and
    /// merge the replies into one.
    async fn one_at_a_time(&self, request: &Value, mut spent: usize) -> anyhow::Result<Value> {
        let questions = request["questions"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut merged: Option<Value> = None;
        let mut answers = Map::new();
        let mut usages = Vec::new();
        for (name, question) in questions {
            let mut part = request.clone();
            part["questions"] = json!({ name: question });
            prune_assets(&mut part);
            let schedule = Schedule {
                spent,
                split: false,
            };
            let (reply, sent) = match self.post(&part, schedule).await {
                Ok(posted) => posted,
                Err(Posted::Failed(failure) | Posted::Split { failure, .. }) => {
                    let error = failed(failure);
                    return Err(match usages.is_empty() {
                        true => error,
                        false => JevBilled::new(summed(&usages), error).into(),
                    });
                }
            };
            // Each part's first request is its own; any retries it needed
            // come out of the batch's.
            spent += sent - 1;
            if let Some(answered) = reply["answers"].as_object() {
                answers.extend(answered.clone());
            }
            usages.push(reply.get("usage").cloned().unwrap_or_else(|| json!({})));
            merged.get_or_insert(reply);
        }
        let mut merged = merged.unwrap_or_else(|| json!({}));
        merged["answers"] = Value::Object(answers);
        merged["usage"] = summed(&usages);
        Ok(merged)
    }
}

#[async_trait]
impl JevDecisionTransport for TypeSafeHttpTransport {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let split = request["questions"]
            .as_object()
            .is_some_and(|questions| questions.len() > 1);
        match self.post(request, Schedule { spent: 0, split }).await {
            Ok((reply, _)) => Ok(reply),
            Err(Posted::Failed(failure)) => Err(failed(failure)),
            Err(Posted::Split { spent, .. }) => self.one_at_a_time(request, spent).await,
        }
    }
}

fn summed(usages: &[Value]) -> Value {
    serde_json::to_value(JevCallUsage::sum(usages.iter())).unwrap_or_else(|_| json!({}))
}

/// The run-ending error for a failed decision request. A 5xx is marked
/// [`Shed`], so the caller may ask again with a smaller request.
fn failed(failure: PostFailure) -> anyhow::Error {
    let message = match &failure {
        PostFailure::Connection => "Model connection failed; no action executed.".to_string(),
        PostFailure::Status(status, None) => {
            format!("Model provider returned HTTP {status}; no action executed.")
        }
        PostFailure::Status(status, Some(detail)) => {
            format!("Model provider returned HTTP {status} ({detail}); no action executed.")
        }
        PostFailure::InvalidBody => "Invalid TypeSafe response; no action executed.".to_string(),
        PostFailure::Failed(detail) => {
            format!("Model provider failed ({detail}); no action executed.")
        }
    };
    let stop = failure.stop_decision(message);
    match failure {
        PostFailure::Status(status, _) if status >= 500 => Shed(stop).into(),
        _ => stop,
    }
}

/// The service failed a request with a 5xx after every retry. It sheds large
/// requests far more often than small ones, so a large one may be asked again
/// smaller. Displays and chains as the stop it wraps.
#[derive(Debug)]
pub(crate) struct Shed(anyhow::Error);

impl Shed {
    pub(crate) fn is(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| cause.is::<Self>())
    }
}

impl std::fmt::Display for Shed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for Shed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// Answers each single-option choice itself and sends `inner` only the rest.
pub(super) struct Open<'a>(pub &'a dyn JevDecisionTransport);

#[async_trait]
impl JevDecisionTransport for Open<'_> {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let mut request = request.clone();
        let forced = take_forced(&mut request);
        if forced.is_empty() {
            return self.0.decide(&request).await;
        }
        prune_assets(&mut request);
        let mut reply = match request["questions"].as_object().is_some_and(Map::is_empty) {
            true => json!({
                "model": request["model"],
                "answers": {},
                "usage": {"calls": 0, "input_tokens": 0, "output_tokens": 0, "cost": 0.0},
            }),
            false => self.0.decide(&request).await?,
        };
        if let Some(answers) = reply["answers"].as_object_mut() {
            answers.extend(forced);
        }
        Ok(reply)
    }
}

/// Remove every choice with exactly one option from `request`, returning the
/// answer each one could only have had.
fn take_forced(request: &mut Value) -> Map<String, Value> {
    let Some(questions) = request["questions"].as_object_mut() else {
        return Map::new();
    };
    let forced = questions
        .iter()
        .filter(|(_, question)| question["type"] == "choice")
        .filter_map(|(name, question)| {
            let criteria = question["criteria"].as_object()?;
            let only = (criteria.len() == 1).then(|| criteria.keys().next())??;
            Some((
                name.clone(),
                json!({"type": "choice", "choice": only, "probabilities": {only: 1.0},
                    "confidence": 1.0, "answered": "locally"}),
            ))
        })
        .collect::<Map<_, _>>();
    for name in forced.keys() {
        questions.remove(name);
    }
    forced
}

/// Drop the assets no remaining question or state refers to: an image placed
/// on target questions only would otherwise ride along, unreferenced, with a
/// request whose target question was answered locally or split off.
fn prune_assets(request: &mut Value) {
    let Some(fields) = request.as_object_mut() else {
        return;
    };
    let Some(Value::Object(assets)) = fields.remove("assets") else {
        return;
    };
    let referenced = Value::Object(fields.clone()).to_string();
    let kept = assets
        .into_iter()
        .filter(|(key, _)| referenced.contains(key.as_str()))
        .collect::<Map<_, _>>();
    if !kept.is_empty() {
        fields.insert("assets".into(), Value::Object(kept));
    }
}

#[cfg(test)]
mod tests;
