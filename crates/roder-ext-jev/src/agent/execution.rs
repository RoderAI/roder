//! One bounded native decision and its observed execution.
use super::*;
impl Agent {
    pub(super) async fn tick(&mut self) -> anyhow::Result<()> {
        let decision = self.predict().await?;
        let previous = self.observation.clone();
        let steps = self.history.len();
        match self.act(decision.clone()).await {
            Err(error) if error.is::<StaleObservation>() && self.history.len() == steps => {
                self.recovery.record(
                    "stale_decision",
                    &self.secrets.scrub(&error.to_string()),
                    self.elapsed_ms,
                );
                if self.revalidate_navigation(&previous, &decision).await? {
                    self.act(decision).await
                } else {
                    Err(error)
                }
            }
            result => result,
        }
    }

    async fn act(&mut self, decision: JevDecision) -> anyhow::Result<()> {
        let selected = decision.choice.clone();
        // A page that showed nothing yet is read again once before a
        // BLOCKED about it is believed; the next decision sees what came.
        if selected == "BLOCKED" && !self.looked_again && self.shows_nothing() {
            self.looked_again = true;
            self.look_again_while_empty().await?;
            return Ok(());
        }
        if selected == "DONE" || selected == "BLOCKED" {
            let fresh = if selected == "DONE" && self.config.research {
                self.browser.fresh_document(&self.observation).await?
            } else {
                self.browser.fresh(&self.observation, None).await?
            };
            if !fresh {
                return Err(StaleObservation::new(
                    "Page changed since the decision. Choose again.",
                )
                .into());
            }
            if selected == "DONE"
                && !self
                    .last_step()
                    .is_some_and(|entry| entry["covered"] == true)
                && self.complete_requirement()
            {
                return Ok(());
            }
            // DONE straight after a covered attempt claims an effect that
            // never happened: nothing was dispatched.
            let covered = self
                .last_step()
                .is_some_and(|entry| entry["covered"] == json!(true));
            (self.status, self.cause) = match (selected.as_str(), covered) {
                ("DONE", false) => (JevStatus::Done, None),
                ("DONE", true) => (JevStatus::Blocked, Some(JevStopCause::DoneAfterCovered)),
                _ => (JevStatus::Blocked, Some(JevStopCause::ModelBlocked)),
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
        if let Some(held) = self.unsure_repeat(&action, &decision) {
            self.suppressed_click = Some(held);
            if self.retry_navigation(&action, &decision).await? {
                return Ok(());
            }
            self.cause = Some(JevStopCause::UncertainRepeat);
            return Err(JevStop::new(
                JevStatus::Blocked,
                "Stopped an uncertain repeated click; task completion has not been established",
            )
            .into());
        }
        let pair = loops::pair_of(&self.observation, &action);
        if let Some(stop) = self.pair_looped(&action, pair.as_ref()) {
            return Err(stop);
        }
        if self.history.iter().filter(chosen).count() >= self.config.max_actions {
            return Err(JevStop::new(
                JevStatus::BudgetExceeded,
                format!(
                    "Stopped at the {}-action demo budget",
                    self.config.max_actions
                ),
            )
            .into());
        }
        if action["kind"] == "read_document" {
            return self.read_document(&action).await;
        }
        self.gate(&action, &decision).await?;

        let written = self.write_text_if_needed(&action).await?;
        let text = written.as_ref().map(|written| written.value.clone());
        let secret = secret::is_secret(&action);
        if secret && let Some(text) = &text {
            self.secrets.remember(text);
        }
        let (covered, refused, uncovered, cover) = match self
            .browser
            .act(
                &action,
                &self.observation,
                text.as_deref(),
                self.config.wait,
            )
            .await
        {
            // A refusal is recorded and the run goes on: the next decision
            // sees the page as it stands.
            Ok(outcome) => (false, outcome.refused, outcome.uncovered, None),
            // Nothing was dispatched, but the attempt is a step that changed
            // nothing: a target that stays covered stalls the run instead of
            // paying for a decision per retry until the model-call budget.
            Err(error) if error.is::<Covered>() => {
                let cover = error
                    .downcast_ref::<Covered>()
                    .and_then(|covered| self.cover_name(covered));
                (true, None, None, cover)
            }
            Err(error) => return Err(error),
        };
        self.pending_text = None;
        self.elapsed();

        // Record execution before observing: a stale observation afterwards
        // must not erase the fact that the action ran.
        let previous = self.observation.clone();
        self.history.push(json!({
            "step": self.history.len() + 1,
            "action": action["label"],
            "kind": action["kind"],
            // Where it sat, for the caller's trace; not sent to the model.
            "context": action.get("context").or_else(|| action.get("section")),
            "choice": selected,
            "probability": decision.probability_of(&selected),
            "confidence": decision.confidence,
            "target_confidence": decision.target_confidence,
            "latency_ms": decision.latency_ms,
            // A covered fill typed nothing, and a secret is never recorded.
            "text": match (covered, secret) {
                (true, _) => None,
                (false, true) => text.map(|_| SECRET.to_string()),
                (false, false) => text,
            },
            "covered": covered,
            // What covered it, as the page names it (not sent to the model).
            "covered_by": cover,
            "refused": refused.map(|reason| self.secrets.scrub(&reason)),
            "uncovered": uncovered.map(|how| self.secrets.scrub(&how)),
            "text_helper": written.as_ref().map(|written| written.model.clone()),
            "text_latency_ms": written.as_ref().map_or(0, |written| written.latency_ms),
            "operation": decision.operation,
            "target": decision.target,
            "page_changed": Value::Null,
            "effect": Value::Null,
            "url": self.observation["url"],
            "usage": decision.usage,
            "executed_ms": self.elapsed_ms,
            "elapsed_ms": self.elapsed_ms,
        }));
        self.watch.record(pair);
        // A filter, tab, or field can change content without a new URL. A
        // subsequent document read must start from the new observed state.
        if !covered && !matches!(action["kind"].as_str(), Some("scroll" | "wait")) {
            self.research = research::Research::default();
        }

        // A refused cookie banner may be recorded after this step.
        let step = self.history.len() - 1;
        self.observe().await?;
        let elapsed = self.elapsed();
        if let Some(entry) = self.history.get_mut(step) {
            // A covered target may have been scrolled into view first; that
            // is not the action taking effect.
            entry["page_changed"] =
                json!(!covered && self.observation["fingerprint"] != previous["fingerprint"]);
            entry["effect"] = json!(effects::effect(&previous, &self.observation));
            entry["text_added"] = json!(effects::text_added(&previous, &self.observation));
            entry["url"] = self.observation["url"].clone();
            entry["elapsed_ms"] = json!(elapsed);
        }
        self.check_scope()?;
        self.status = match stalled(&self.history) {
            Some(cause) => {
                self.cause = Some(cause);
                JevStatus::Blocked
            }
            None => JevStatus::Ready,
        };
        // The stall rule has its say first.
        match self.status == JevStatus::Ready {
            true => self.waited_out().map_or(Ok(()), Err),
            false => Ok(()),
        }
    }
}
