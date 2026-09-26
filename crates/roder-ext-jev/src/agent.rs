//! The bounded agent loop.
//!
//! A port of upstream Jev's `agent.py`: observe, ask for one operation, execute
//! it, observe again. The budgets, the statuses, and the rule that three
//! consecutive actions changing nothing mean `blocked` are upstream's.

use std::time::{Duration, Instant};

use anyhow::bail;
use serde_json::{Value, json};

use crate::decide::{self, Decision};
use crate::page::{Page, StalePage};
use crate::prompts::MAX_STEPS;
use crate::space::action_space;
use crate::text_helper::{self, WrittenText};
use crate::text_model::TextModel;

pub(crate) struct AgentConfig {
    pub(crate) goal: String,
    pub(crate) typesafe_key: String,
    pub(crate) typesafe_model: String,
    pub(crate) text: Option<TextModel>,
    /// How long a `wait` action holds the page.
    pub(crate) wait: Duration,
}

/// Upstream's run statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ready,
    Done,
    Blocked,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Done => "done",
            Self::Blocked => "blocked",
        }
    }

    fn stopped(self) -> bool {
        matches!(self, Self::Done | Self::Blocked)
    }
}

pub(crate) struct Agent {
    page: Page,
    config: AgentConfig,
    observation: Value,
    history: Vec<Value>,
    text_calls: Vec<Value>,
    decisions: usize,
    status: Status,
    started_at: Instant,
    elapsed_ms: u64,
    /// A value already written for a field whose action had to be retried.
    pending_text: Option<(Value, WrittenText)>,
}

impl Agent {
    pub(crate) async fn start(mut page: Page, config: AgentConfig) -> anyhow::Result<Self> {
        let observation = page.observe().await?;
        Ok(Self {
            page,
            config,
            observation,
            history: Vec::new(),
            text_calls: Vec::new(),
            decisions: 0,
            status: Status::Ready,
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
                Err(error) if error.is::<StalePage>() => {
                    // The page moved under the decision; observe and choose again.
                    self.status = Status::Ready;
                    match self.page.observe().await {
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

    async fn predict(&mut self) -> anyhow::Result<Decision> {
        if !self.page.fresh(&self.observation, None).await? {
            self.observation = self.page.observe().await?;
        }
        if self.decisions >= MAX_STEPS * 2 {
            bail!("Reached the demo's model-call budget");
        }
        let (decision, _) = decide::choose(
            &self.observation,
            &self.config.goal,
            &self.history,
            &self.config.typesafe_model,
            &self.config.typesafe_key,
        )
        .await?;
        self.decisions += 1;
        Ok(decision)
    }

    async fn act(&mut self, decision: Decision) -> anyhow::Result<()> {
        let selected = decision.choice.clone();
        if selected == "DONE" || selected == "BLOCKED" {
            if !self.page.fresh(&self.observation, None).await? {
                return Err(
                    StalePage("Page changed since the decision. Choose again.".into()).into(),
                );
            }
            self.status = if selected == "DONE" {
                Status::Done
            } else {
                Status::Blocked
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
            return Err(StalePage("Chosen action is no longer observed".into()).into());
        };
        if self.history.len() >= MAX_STEPS {
            self.status = Status::Blocked;
            bail!("Stopped at the {MAX_STEPS}-action demo budget");
        }

        let written = self.write_text_if_needed(&action).await?;
        let text = written.as_ref().map(|written| written.value.clone());
        self.page
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

        self.observation = self.page.observe().await?;
        let elapsed = self.elapsed();
        if let Some(entry) = self.history.last_mut() {
            entry["page_changed"] = json!(self.observation["fingerprint"] != previous_fingerprint);
            entry["url"] = self.observation["url"].clone();
            entry["elapsed_ms"] = json!(elapsed);
        }
        self.status = if stalled(&self.history) {
            Status::Blocked
        } else {
            Status::Ready
        };
        Ok(())
    }

    /// Typing needs a value, and only the text helper may supply one.
    async fn write_text_if_needed(
        &mut self,
        action: &Value,
    ) -> anyhow::Result<Option<WrittenText>> {
        if action["kind"].as_str() != Some("fill") {
            return Ok(None);
        }
        if !self.page.fresh(&self.observation, Some(action)).await? {
            return Err(
                StalePage("Page changed before text generation. Choose again.".into()).into(),
            );
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
        let written = text_helper::field_text(&context, text).await?;
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
    pub(crate) fn result(&self, stopped_because: Option<String>) -> Value {
        let text = self.observation["text"].as_str().unwrap_or_default();
        let visible_text = text.chars().take(6000).collect::<String>();
        let actions = self
            .history
            .iter()
            .map(|entry| {
                let mut recorded = serde_json::Map::new();
                for key in [
                    "step",
                    "action",
                    "kind",
                    "text",
                    "url",
                    "page_changed",
                    "elapsed_ms",
                ] {
                    recorded.insert(key.into(), entry.get(key).cloned().unwrap_or(Value::Null));
                }
                Value::Object(recorded)
            })
            .collect::<Vec<_>>();
        let observed = action_space(
            self.observation["actions"]
                .as_array()
                .map_or(&[][..], Vec::as_slice),
        );
        json!({
            "status": self.status.as_str(),
            "url": self.observation["url"],
            "title": self.observation["title"],
            "visible_text": visible_text,
            "actions": actions,
            "elapsed_ms": self.elapsed_ms,
            "observed_elements": observed.elements.len(),
            "model_calls": self.decisions,
            "text_calls": self.text_calls.len(),
            "stopped_because": stopped_because,
            "untrusted": true,
        })
    }

    pub(crate) async fn activate(&mut self) -> anyhow::Result<()> {
        self.page.activate().await
    }

    pub(crate) async fn close(&mut self) -> anyhow::Result<()> {
        self.page.close().await
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
        assert_eq!(Status::Ready.as_str(), "ready");
        assert_eq!(Status::Done.as_str(), "done");
        assert_eq!(Status::Blocked.as_str(), "blocked");
        assert!(Status::Done.stopped() && Status::Blocked.stopped());
        assert!(!Status::Ready.stopped());
    }

    #[test]
    fn budgets_match_upstream() {
        assert_eq!(MAX_STEPS, 60);
        assert_eq!(MAX_STEPS * 2, 120);
    }
}
