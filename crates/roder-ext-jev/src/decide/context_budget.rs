//! TypeSafe has TWO limits: shared state + all questions (64k), and shared
//! state + any single question (32k). https://docs.typesafe.ai/models
//! No tokenizer/count endpoint is published. Use deliberately conservative
//! UTF-8 bytes, not chars/4, plus image reserves, and leave framing headroom.
//! The provider remains authoritative; a rejected request is repacked once.
use crate::{JevBilled, JevDecisionTransport};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Mutex;

pub(super) const REQUEST_LIMIT: usize = 64_000;
pub(super) const QUESTION_LIMIT: usize = 32_000;
const REQUEST_RESERVE: usize = 4096;
const QUESTION_RESERVE: usize = 2048;

/// A request whose state plus largest question is estimated above this is
/// large enough that the service shedding it with 5xx answers is a reason to
/// ask again smaller. About 8k tokens at three bytes each: fastbrowse found
/// the gateway answered nearly every request under 10k tokens, but a 27k
/// one only four times in ten.
pub(super) const SHED_ABOVE: usize = 24_000;

#[derive(Debug)]
pub(super) struct TooLarge;
impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Jev context cannot fit the required goal, safety questions and minimal page view; no request sent for this oversized page and no browser action executed")
    }
}
impl std::error::Error for TooLarge {}

pub(super) fn with_usage(usages: &[Value], error: anyhow::Error) -> anyhow::Error {
    if usages.is_empty() {
        error
    } else {
        JevBilled::new(
            serde_json::to_value(crate::usage::JevCallUsage::sum(usages.iter())).unwrap(),
            error,
        )
        .into()
    }
}

fn cost(value: &Value, assets: &[(String, usize)]) -> usize {
    match value {
        Value::String(s) => s
            .len()
            .saturating_add(
                assets
                    .iter()
                    .map(|(key, reserve)| s.matches(key.as_str()).count().saturating_mul(*reserve))
                    .sum::<usize>(),
            )
            .saturating_add(2),
        Value::Array(items) => {
            items.iter().map(|v| cost(v, assets)).sum::<usize>() + items.len() + 2
        }
        Value::Object(items) => {
            items
                .iter()
                .map(|(key, v)| key.len() + 4 + cost(v, assets))
                .sum::<usize>()
                + 2
        }
        _ => value.to_string().len(),
    }
}

pub(super) fn estimate(request: &Value, image_cost: usize) -> (usize, usize) {
    let assets = request["assets"]
        .as_object()
        .map(|map| {
            map.keys()
                .map(|key| (key.clone(), image_cost))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let state = cost(&request["state"], &assets);
    let questions = request["questions"]
        .as_object()
        .into_iter()
        .flat_map(|map| map.values())
        .map(|q| cost(q, &assets))
        .collect::<Vec<_>>();
    (
        state + questions.iter().sum::<usize>() + REQUEST_RESERVE,
        state + questions.iter().max().copied().unwrap_or(0) + QUESTION_RESERVE,
    )
}

/// Whether the last request sent, as `reports` recorded it, was large enough
/// to ask again smaller (see [`SHED_ABOVE`]).
pub(super) fn sheddable(reports: &[Value]) -> bool {
    reports
        .last()
        .and_then(|report| report["estimated_max_question_tokens"].as_u64())
        .is_some_and(|tokens| tokens as usize > SHED_ABOVE)
}

pub(super) struct Bounded<'a> {
    pub inner: &'a dyn JevDecisionTransport,
    pub image_cost: usize,
    pub reports: &'a Mutex<Vec<Value>>,
}
#[async_trait]
impl JevDecisionTransport for Bounded<'_> {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let (total, question) = estimate(request, self.image_cost);
        if total > REQUEST_LIMIT || question > QUESTION_LIMIT {
            return Err(TooLarge.into());
        }
        self.reports.lock().unwrap().push(json!({"estimated_request_tokens":total,
            "estimated_max_question_tokens":question,"questions":request["questions"].as_object().map_or(0,|q|q.len())}));
        self.inner.decide(request).await
    }
}

#[cfg(test)]
mod tests;
