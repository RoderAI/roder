//! The keyless tier's decider and the field values every tier types.
//!
//! A plan step names its target by kind and exact label, optionally by the
//! start of its `context` when twins share the label, or lists alternatives
//! (`any`) to take whichever is observed. The step after the actions the
//! model chose so far is played (a refused cookie banner is not one), so a
//! stale re-decision replays the same step; past the plan it answers
//! `DONE`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::engine::{
    JevDecision, JevDecisionClient, JevStatus, JevStop, JevTextValue, JevTextValueResolver,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    click: Option<String>,
    fill: Option<String>,
    select: Option<String>,
    /// A box that scrolls on its own, by label (down while it can go down).
    scroll: Option<String>,
    /// Enter in the focused field, by its label.
    enter: Option<String>,
    /// A key control such as Escape, by its label.
    key: Option<String>,
    /// Answer BLOCKED, as the model does when nothing on the page serves
    /// the goal.
    #[serde(default)]
    blocked: bool,
    #[serde(default)]
    done: bool,
    /// The start of the target's observed context.
    context: Option<String>,
    /// Alternatives, in preference order, instead of one target.
    #[serde(default)]
    any: Vec<Step>,
    /// Play this step this many times in a row.
    #[serde(default = "once")]
    repeat: usize,
    /// With the irreversible-action gate on, the answer the model gives
    /// about this step's target; absent is no valid answer, which the gate
    /// treats as irreversible.
    irreversible: Option<f64>,
    /// The call confidence the model reports for this step; absent is 1.0.
    /// Below 0.7, a repeat of the click just made ends the run `done`.
    confidence: Option<f64>,
}

fn once() -> usize {
    1
}

impl Step {
    /// The observed kind and label this step targets.
    fn target(&self) -> anyhow::Result<(&'static str, &str)> {
        let named = self.named();
        match named.as_slice() {
            [(kind, label)] => Ok((kind, label)),
            _ => bail!(
                "a step names exactly one of click, fill, select, scroll, enter or key: {self:?}"
            ),
        }
    }

    fn named(&self) -> Vec<(&'static str, &str)> {
        [
            ("click", &self.click),
            ("fill", &self.fill),
            ("select", &self.select),
            ("scroll", &self.scroll),
            ("enter", &self.enter),
            ("key", &self.key),
        ]
        .into_iter()
        .filter_map(|(kind, label)| label.as_deref().map(|label| (kind, label)))
        .collect()
    }

    pub(crate) fn check(&self) -> anyhow::Result<()> {
        if self.repeat == 0 {
            bail!("repeat must be at least 1");
        }
        if self
            .confidence
            .is_some_and(|confidence| !(0.0..=1.0).contains(&confidence))
        {
            bail!("confidence must be a probability");
        }
        if self.blocked || self.done {
            if !self.named().is_empty() || !self.any.is_empty() || (self.blocked && self.done) {
                bail!(
                    "a terminal step must name no target and choose exactly one of blocked or done"
                );
            }
            return Ok(());
        }
        if self
            .irreversible
            .is_some_and(|probability| !(0.0..=1.0).contains(&probability))
        {
            bail!("irreversible must be a probability");
        }
        if self.any.is_empty() {
            return self.target().map(|_| ());
        }
        if !self.named().is_empty() {
            bail!("a step with `any` names no target of its own");
        }
        for alternative in &self.any {
            if !alternative.any.is_empty()
                || alternative.repeat != 1
                || alternative.irreversible.is_some()
                || alternative.confidence.is_some()
            {
                bail!("`any` alternatives are single targets");
            }
            alternative.target()?;
        }
        Ok(())
    }

    fn alternatives(&self) -> Vec<&Step> {
        if self.any.is_empty() {
            vec![self]
        } else {
            self.any.iter().collect()
        }
    }

    /// The observed action this step picks, if any alternative is observed.
    fn resolve<'a>(&self, observation: &'a Value) -> Option<(&'static str, &'a Value)> {
        let actions = observation["actions"].as_array()?;
        self.alternatives().into_iter().find_map(|step| {
            let (kind, label) = step.target().ok()?;
            let action = actions.iter().find(|action| {
                action["kind"].as_str() == Some(kind)
                    && action["label"].as_str() == Some(label)
                    && step.context.as_deref().is_none_or(|context| {
                        action["context"]
                            .as_str()
                            .is_some_and(|actual| actual.starts_with(context))
                    })
            })?;
            Some((kind, action))
        })
    }
}

/// Plays an eval task's plan.
pub(crate) struct StepDecider {
    plan: Vec<Step>,
}

impl StepDecider {
    pub(crate) fn new(plan: &[Step]) -> Self {
        let plan = plan
            .iter()
            .flat_map(|step| std::iter::repeat_n(step.clone(), step.repeat))
            .collect();
        Self { plan }
    }
}

#[async_trait]
impl JevDecisionClient for StepDecider {
    async fn choose(
        &self,
        observation: &Value,
        _goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let played = chosen(history);
        let step = self.plan.get(played);
        let (choice, operation) = match step {
            Some(step) if step.done => ("DONE".to_string(), "DONE".to_string()),
            Some(step) if step.blocked => ("BLOCKED".to_string(), "BLOCKED".to_string()),
            Some(step) => {
                let (kind, action) = step.resolve(observation).with_context(|| {
                    format!(
                        "scripted step {} ({step:?}) is not observed on {}",
                        played + 1,
                        observation["url"]
                    )
                })?;
                let operation = match kind {
                    "fill" => "TYPE_TEXT",
                    "select" => "SELECT",
                    "scroll" if action["direction"] == "up" => "SCROLL_REGION_UP",
                    "scroll" => "SCROLL_REGION_DOWN",
                    "enter" => "PRESS_ENTER",
                    "key" => "PRESS_ESCAPE",
                    _ => "CLICK",
                };
                let id = action["id"].as_str().unwrap_or_default().to_string();
                (id, operation.to_string())
            }
            None => ("DONE".to_string(), "DONE".to_string()),
        };
        let mut probabilities = Map::new();
        probabilities.insert(choice.clone(), json!(1.0));
        Ok(JevDecision {
            choice,
            operation,
            target: None,
            confidence: step.and_then(|step| step.confidence).unwrap_or(1.0),
            target_confidence: None,
            probabilities,
            latency_ms: 0,
            usage: json!({}),
            model: None,
            irreversible: None,
        })
    }

    /// The plan step's own answer about its target, as the gate reads it.
    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let mut decision = self.choose(observation, goal, history).await?;
        decision.irreversible = self
            .plan
            .get(chosen(history))
            .and_then(|step| step.irreversible);
        Ok(decision)
    }
}

/// How many actions in `history` the model chose.
fn chosen(history: &[Value]) -> usize {
    history
        .iter()
        .filter(|entry| entry["kind"] != crate::agent::COOKIE_BANNER)
        .count()
}

/// Holds the first decision back, so a page that moves on its own timer
/// moves while the decision is in flight.
pub(crate) struct FirstDecisionDelay {
    inner: Arc<dyn JevDecisionClient>,
    delay: Duration,
    delayed: AtomicBool,
}

impl FirstDecisionDelay {
    pub(crate) fn new(inner: Arc<dyn JevDecisionClient>, delay: Duration) -> Self {
        Self {
            inner,
            delay,
            delayed: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl JevDecisionClient for FirstDecisionDelay {
    fn uses_images(&self) -> bool {
        self.inner.uses_images()
    }
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let decision = self.inner.choose(observation, goal, history).await;
        self.hold().await;
        decision
    }

    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        let decision = self.inner.choose_gated(observation, goal, history).await;
        self.hold().await;
        decision
    }
}

impl FirstDecisionDelay {
    async fn hold(&self) {
        if !self.delayed.swap(true, Ordering::SeqCst) {
            tokio::time::sleep(self.delay).await;
        }
    }
}

/// The values a task's goal supplies, by field label. A field the goal does
/// not cover fails the fill, as the text helper does when a value is
/// missing, so nothing is guessed.
pub(crate) struct TaskValues(BTreeMap<String, String>);

impl TaskValues {
    pub(crate) fn new(values: &BTreeMap<String, String>) -> Self {
        Self(values.clone())
    }
}

#[async_trait]
impl JevTextValueResolver for TaskValues {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        let label = field_context["field"]["label"].as_str().unwrap_or_default();
        // What the text helper's `{"text": null}` means: the goal lacks it.
        let value = self.0.get(label).ok_or_else(|| {
            JevStop::new(
                JevStatus::NeedsInput,
                format!("the goal gives no value for field {label:?}"),
            )
        })?;
        Ok(JevTextValue {
            value: value.clone(),
            model: "task-values".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(raw: Value) -> Step {
        serde_json::from_value(raw).unwrap()
    }

    fn observation() -> Value {
        json!({"url": "http://x/", "actions": [
            {"id": "e1", "kind": "click", "label": "Add to cart", "context": "Trail Runner"},
            {"id": "e2", "kind": "click", "label": "Add to cart", "context": "Desk Lamp; column: x"},
            {"id": "e3", "kind": "fill", "label": "Name"},
        ]})
    }

    #[test]
    fn steps_resolve_by_kind_label_context_and_alternatives() {
        let page = observation();
        let id = |raw: Value| {
            step(raw)
                .resolve(&page)
                .map(|(_, action)| action["id"].as_str().unwrap().to_string())
        };
        assert_eq!(id(json!({"click": "Add to cart"})).as_deref(), Some("e1"));
        assert_eq!(
            id(json!({"click": "Add to cart", "context": "Desk Lamp"})).as_deref(),
            Some("e2")
        );
        assert_eq!(id(json!({"click": "Name"})), None);
        assert_eq!(
            id(json!({"any": [{"click": "Gone"}, {"fill": "Name"}]})).as_deref(),
            Some("e3")
        );
    }

    #[test]
    fn malformed_steps_are_rejected() {
        assert!(step(json!({"click": "A", "fill": "B"})).check().is_err());
        assert!(step(json!({})).check().is_err());
        assert!(step(json!({"click": "A", "repeat": 0})).check().is_err());
        assert!(
            step(json!({"click": "A", "any": [{"click": "B"}]}))
                .check()
                .is_err()
        );
        assert!(step(json!({"any": [{"click": "B"}]})).check().is_ok());
        assert!(serde_json::from_value::<Step>(json!({"tap": "A"})).is_err());
        assert!(
            step(json!({"click": "A", "confidence": 1.2}))
                .check()
                .is_err()
        );
        assert!(
            step(json!({"any": [{"click": "B", "confidence": 0.5}]}))
                .check()
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_step_reports_its_own_confidence() {
        let decider = StepDecider::new(&[
            step(json!({"click": "Add to cart"})),
            step(json!({"click": "Add to cart", "confidence": 0.6})),
        ]);
        let page = observation();
        let first = decider.choose(&page, "", &[]).await.unwrap();
        let second = decider.choose(&page, "", &[json!({})]).await.unwrap();
        let done = decider
            .choose(&page, "", &[json!({}), json!({})])
            .await
            .unwrap();
        assert_eq!(
            (first.confidence, second.confidence, done.confidence),
            (1.0, 0.6, 1.0)
        );
    }

    #[tokio::test]
    async fn the_plan_repeats_then_answers_done() {
        let decider = StepDecider::new(&[step(json!({"click": "Add to cart", "repeat": 2}))]);
        let page = observation();
        let mut history = Vec::new();
        for _ in 0..2 {
            let decision = decider.choose(&page, "", &history).await.unwrap();
            assert_eq!(
                (decision.choice.as_str(), decision.operation.as_str()),
                ("e1", "CLICK")
            );
            history.push(json!({}));
        }
        let done = decider.choose(&page, "", &history).await.unwrap();
        assert_eq!(done.choice, "DONE");
    }
}
