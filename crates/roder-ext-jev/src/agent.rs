//! The bounded agent loop.
//!
//! A port of upstream Jev's `agent.py`: observe, ask for one operation, execute
//! it, observe again. The budgets, the statuses, and the rule that three
//! consecutive actions changing nothing mean `blocked` are upstream's.

use std::time::Instant;

use anyhow::bail;
use serde_json::{Value, json};

use crate::engine::{
    JevActionRecord, JevBrowser, JevDecision, JevDecisionRecord, JevEngineConfig, JevRunResult,
    JevStatus, JevTextValue, StaleObservation,
};
use crate::prompts::MAX_STEPS;
use crate::space::action_space;
use crate::text_helper;

pub(crate) struct Agent {
    browser: Box<dyn JevBrowser>,
    config: JevEngineConfig,
    observation: Value,
    history: Vec<Value>,
    text_calls: Vec<Value>,
    decisions: usize,
    decision_calls: Vec<JevDecisionRecord>,
    status: JevStatus,
    started_at: Instant,
    elapsed_ms: u64,
    /// A value already written for a field whose action had to be retried.
    pending_text: Option<(Value, JevTextValue)>,
}

impl Agent {
    pub(crate) async fn start(
        mut browser: Box<dyn JevBrowser>,
        config: JevEngineConfig,
    ) -> anyhow::Result<Self> {
        let observation = browser.observe().await?;
        Ok(Self {
            browser,
            config,
            observation,
            history: Vec::new(),
            text_calls: Vec::new(),
            decisions: 0,
            decision_calls: Vec::new(),
            status: JevStatus::Ready,
            started_at: Instant::now(),
            elapsed_ms: 0,
            pending_text: None,
        })
    }

    fn elapsed(&mut self) -> u64 {
        self.elapsed_ms = self.started_at.elapsed().as_millis() as u64;
        self.elapsed_ms
    }

    /// Run until the goal is met, nothing can progress, or a budget is reached.
    /// A budget or helper failure ends the run but keeps the trace, which is
    /// what upstream raises and callers want reported.
    pub(crate) async fn run(&mut self) -> Option<String> {
        while !self.status.stopped() {
            match self.tick().await {
                Ok(()) => {}
                Err(error) if error.is::<StaleObservation>() => {
                    // The page moved under the decision; observe and choose again.
                    self.status = JevStatus::Ready;
                    match self.browser.observe().await {
                        Ok(observation) => self.observation = observation,
                        Err(error) => return Some(describe(&error)),
                    }
                    self.elapsed();
                }
                Err(error) => return Some(describe(&error)),
            }
        }
        None
    }

    async fn tick(&mut self) -> anyhow::Result<()> {
        let decision = self.predict().await?;
        self.act(decision).await
    }

    async fn predict(&mut self) -> anyhow::Result<JevDecision> {
        if !self.browser.fresh(&self.observation, None).await? {
            self.observation = self.browser.observe().await?;
        }
        if self.decisions >= MAX_STEPS * 2 {
            bail!("Reached the demo's model-call budget");
        }
        let decision = self
            .config
            .decision
            .choose(&self.observation, &self.config.goal, &self.history)
            .await?;
        self.decisions += 1;
        self.decision_calls.push(JevDecisionRecord {
            choice: decision.choice.clone(),
            operation: decision.operation.clone(),
            target: decision.target.clone(),
            confidence: decision.confidence,
            probabilities: decision.probabilities.clone(),
            latency_ms: decision.latency_ms,
            usage: decision.usage.clone(),
        });
        Ok(decision)
    }

    async fn act(&mut self, decision: JevDecision) -> anyhow::Result<()> {
        let selected = decision.choice.clone();
        if selected == "DONE" || selected == "BLOCKED" {
            if !self.browser.fresh(&self.observation, None).await? {
                return Err(StaleObservation::new(
                    "Page changed since the decision. Choose again.",
                )
                .into());
            }
            self.status = if selected == "DONE" {
                JevStatus::Done
            } else {
                JevStatus::Blocked
            };
            self.elapsed();
            return Ok(());
        }
        let action = self
            .observation
            .get("actions")
            .and_then(Value::as_array)
            .and_then(|actions| {
                actions
                    .iter()
                    .find(|action| action["id"].as_str() == Some(selected.as_str()))
            })
            .cloned();
        let Some(action) = action else {
            return Err(StaleObservation::new("Chosen action is no longer observed").into());
        };
        if self.history.len() >= MAX_STEPS {
            self.status = JevStatus::Blocked;
            bail!("Stopped at the {MAX_STEPS}-action demo budget");
        }

        let written = self.write_text_if_needed(&action).await?;
        let text = written.as_ref().map(|written| written.value.clone());
        self.browser
            .act(
                &action,
                &self.observation,
                text.as_deref(),
                self.config.wait,
            )
            .await?;
        self.pending_text = None;
        self.elapsed();

        // Record execution before observing: a stale observation afterwards
        // must not erase the fact that the action ran.
        let previous_fingerprint = self.observation["fingerprint"].clone();
        self.history.push(json!({
            "step": self.history.len() + 1,
            "action": action["label"],
            "kind": action["kind"],
            "choice": selected,
            "probability": decision.probability_of(&selected),
            "confidence": decision.confidence,
            "latency_ms": decision.latency_ms,
            "text": text,
            "text_helper": written.as_ref().map(|written| written.model.clone()),
            "text_latency_ms": written.as_ref().map_or(0, |written| written.latency_ms),
            "operation": decision.operation,
            "target": decision.target,
            "page_changed": Value::Null,
            "url": self.observation["url"],
            "usage": decision.usage,
            "executed_ms": self.elapsed_ms,
            "elapsed_ms": self.elapsed_ms,
        }));

        self.observation = self.browser.observe().await?;
        let elapsed = self.elapsed();
        if let Some(entry) = self.history.last_mut() {
            entry["page_changed"] = json!(self.observation["fingerprint"] != previous_fingerprint);
            entry["url"] = self.observation["url"].clone();
            entry["elapsed_ms"] = json!(elapsed);
        }
        self.status = if stalled(&self.history) {
            JevStatus::Blocked
        } else {
            JevStatus::Ready
        };
        Ok(())
    }

    /// Typing needs a value, and only the text helper may supply one.
    async fn write_text_if_needed(
        &mut self,
        action: &Value,
    ) -> anyhow::Result<Option<JevTextValue>> {
        if action["kind"].as_str() != Some("fill") {
            return Ok(None);
        }
        if !self.browser.fresh(&self.observation, Some(action)).await? {
            return Err(StaleObservation::new(
                "Page changed before text generation. Choose again.",
            )
            .into());
        }
        let context =
            text_helper::field_context(&self.config.goal, action, &self.observation, &self.history);
        if let Some((pending, written)) = &self.pending_text
            && pending == &context
        {
            return Ok(Some(written.clone()));
        }
        let Some(text) = self.config.text.as_ref() else {
            bail!(
                "TYPE_TEXT needs a text model; configure a Roder chat-completions provider or \
                 set JEV_TEXT_MODEL_API_KEY. No text is guessed."
            );
        };
        let written = text.resolve(&context).await?;
        self.pending_text = Some((context, written.clone()));
        self.text_calls.push(json!({
            "model": written.model,
            "latency_ms": written.latency_ms,
            "usage": written.usage,
            "field": action["label"],
            "value": written.value,
        }));
        Ok(Some(written))
    }

    /// The result payload, in the shape the tool reports.
    pub(crate) fn result(&self, stopped_because: Option<String>) -> JevRunResult {
        let text = self.observation["text"].as_str().unwrap_or_default();
        let visible_text = text.chars().take(6000).collect::<String>();
        let actions = self
            .history
            .iter()
            .map(|entry| JevActionRecord {
                step: entry["step"].as_u64().unwrap_or_default() as usize,
                action: entry["action"].as_str().unwrap_or_default().to_string(),
                kind: entry["kind"].as_str().unwrap_or_default().to_string(),
                text: entry["text"].as_str().map(str::to_string),
                url: entry["url"].as_str().unwrap_or_default().to_string(),
                page_changed: entry["page_changed"].as_bool(),
                elapsed_ms: entry["elapsed_ms"].as_u64().unwrap_or_default(),
                choice: entry["choice"].as_str().unwrap_or_default().to_string(),
                probability: entry["probability"].clone(),
                confidence: entry["confidence"].as_f64().unwrap_or_default(),
                decision_latency_ms: entry["latency_ms"].as_u64().unwrap_or_default(),
                text_helper: entry["text_helper"].as_str().map(str::to_string),
                text_latency_ms: entry["text_latency_ms"].as_u64().unwrap_or_default(),
                operation: entry["operation"].as_str().unwrap_or_default().to_string(),
                target: entry["target"].as_str().map(str::to_string),
                usage: entry["usage"].clone(),
                executed_ms: entry["executed_ms"].as_u64().unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let observed = action_space(
            self.observation["actions"]
                .as_array()
                .map_or(&[][..], Vec::as_slice),
        );
        JevRunResult {
            status: self.status,
            url: self.observation["url"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            title: self.observation["title"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            visible_text,
            actions,
            elapsed_ms: self.elapsed_ms,
            observed_elements: observed.elements.len(),
            model_calls: self.decisions,
            text_calls: self.text_calls.len(),
            decisions: self.decision_calls.clone(),
            stopped_because,
            untrusted: true,
        }
    }

    pub(crate) async fn activate(&mut self) -> anyhow::Result<()> {
        self.browser.activate().await
    }

    pub(crate) async fn close(&mut self) -> anyhow::Result<()> {
        self.browser.close().await
    }
}

/// Upstream's stall rule: three consecutive executed actions, none of them a
/// wait, none of which changed the page.
fn stalled(history: &[Value]) -> bool {
    let recent = &history[history.len().saturating_sub(3)..];
    recent.len() == 3
        && recent.iter().all(|entry| {
            entry["page_changed"] == json!(false) && entry["kind"].as_str() != Some("wait")
        })
}

fn describe(error: &anyhow::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, changed: Option<bool>) -> Value {
        json!({"kind": kind, "page_changed": changed})
    }

    #[test]
    fn three_unchanged_actions_block_the_run() {
        let history = vec![
            entry("click", Some(false)),
            entry("click", Some(false)),
            entry("click", Some(false)),
        ];
        assert!(stalled(&history));
    }

    #[test]
    fn a_wait_or_a_change_keeps_the_run_going() {
        assert!(!stalled(&[
            entry("click", Some(false)),
            entry("wait", Some(false)),
            entry("click", Some(false)),
        ]));
        assert!(!stalled(&[
            entry("click", Some(false)),
            entry("click", Some(true)),
            entry("click", Some(false)),
        ]));
        // A run that has not executed three actions yet cannot stall.
        assert!(!stalled(&[entry("click", Some(false))]));
        assert!(!stalled(&[]));
    }

    #[test]
    fn only_the_last_three_actions_matter() {
        let history = vec![
            entry("click", Some(true)),
            entry("click", Some(false)),
            entry("click", Some(false)),
            entry("click", Some(false)),
        ];
        assert!(stalled(&history));
    }

    #[test]
    fn statuses_use_upstream_names() {
        assert!(JevStatus::Done.stopped() && JevStatus::Blocked.stopped());
        assert!(!JevStatus::Ready.stopped());
    }

    #[test]
    fn budgets_match_upstream() {
        assert_eq!(MAX_STEPS, 60);
        assert_eq!(MAX_STEPS * 2, 120);
    }
}
