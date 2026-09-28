//! Keyless stand-ins for the decision model and the text helper.
//!
//! Observed ids (`e1`, `e2`, …) depend on the fixture's DOM order, so the
//! scripted decider names its targets by kind and label and resolves them
//! against the observation it is shown. It picks step `history.len()`, which
//! keeps a plan in step even when a stale page makes the loop decide twice.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Context;
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::engine::{JevDecision, JevDecisionClient, JevTextValue, JevTextValueResolver};

/// One planned step: act on the observed action with this kind and label,
/// and this context when twins share the label.
#[derive(Debug, Clone)]
pub(crate) struct Pick {
    kind: &'static str,
    label: &'static str,
    context: Option<&'static str>,
}

pub(crate) fn pick(kind: &'static str, label: &'static str) -> Pick {
    Pick {
        kind,
        label,
        context: None,
    }
}

/// A step on one of several controls that share `label`.
pub(crate) fn pick_in(kind: &'static str, label: &'static str, context: &'static str) -> Pick {
    Pick {
        kind,
        label,
        context: Some(context),
    }
}

/// Plays a fixed plan, then answers `DONE`.
pub(crate) struct PlanDecider {
    plan: Vec<Pick>,
    /// The id chosen for each decision, in order, including `DONE`.
    pub(crate) chosen: Mutex<Vec<String>>,
}

impl PlanDecider {
    pub(crate) fn new(plan: Vec<Pick>) -> Self {
        Self {
            plan,
            chosen: Mutex::new(Vec::new()),
        }
    }
}

/// The observed action with this kind and exact label.
pub(crate) fn find<'a>(observation: &'a Value, kind: &str, label: &str) -> Option<&'a Value> {
    observation["actions"].as_array()?.iter().find(|action| {
        action["kind"].as_str() == Some(kind) && action["label"].as_str() == Some(label)
    })
}

/// The observed action with this kind, exact label and exact context.
pub(crate) fn find_in<'a>(
    observation: &'a Value,
    kind: &str,
    label: &str,
    context: &str,
) -> Option<&'a Value> {
    observation["actions"].as_array()?.iter().find(|action| {
        action["kind"].as_str() == Some(kind)
            && action["label"].as_str() == Some(label)
            && action["context"].as_str() == Some(context)
    })
}

#[async_trait]
impl JevDecisionClient for PlanDecider {
    async fn choose(
        &self,
        observation: &Value,
        _goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let (choice, operation) = match self.plan.get(history.len()) {
            Some(step) => {
                let action = match step.context {
                    Some(context) => find_in(observation, step.kind, step.label, context),
                    None => find(observation, step.kind, step.label),
                }
                .with_context(|| {
                    format!(
                        "scripted step {} ({} {:?} in {:?}) is not observed on {}",
                        history.len() + 1,
                        step.kind,
                        step.label,
                        step.context,
                        observation["url"]
                    )
                })?;
                let id = action["id"].as_str().unwrap_or_default().to_string();
                (id, step.kind.to_ascii_uppercase())
            }
            None => ("DONE".to_string(), "DONE".to_string()),
        };
        self.chosen.lock().unwrap().push(choice.clone());
        let mut probabilities = Map::new();
        probabilities.insert(choice.clone(), json!(1.0));
        Ok(JevDecision {
            choice,
            operation,
            target: None,
            confidence: 1.0,
            target_confidence: None,
            probabilities,
            latency_ms: 0,
            usage: json!({}),
            model: None,
            irreversible: None,
        })
    }
}

/// Answers each field by its observed label.
pub(crate) struct FieldValues(HashMap<&'static str, &'static str>);

impl FieldValues {
    pub(crate) fn new(values: &[(&'static str, &'static str)]) -> Self {
        Self(values.iter().copied().collect())
    }
}

#[async_trait]
impl JevTextValueResolver for FieldValues {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        let label = field_context["field"]["label"].as_str().unwrap_or_default();
        let value = self
            .0
            .get(label)
            .with_context(|| format!("no scripted value for field {label:?}"))?;
        Ok(JevTextValue {
            value: value.to_string(),
            model: "scripted".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }
}
