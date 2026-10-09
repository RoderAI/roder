//! Model input preparation and billed decision accounting.
use super::*;
use crate::{JevBilled, JevDecisionRecord};
impl Agent {
    pub(super) async fn predict(&mut self) -> anyhow::Result<JevDecision> {
        if !self.browser.fresh(&self.observation, None).await? {
            self.observe().await?;
            self.check_scope()?;
        }
        if self.decisions >= self.config.max_actions.saturating_mul(2) {
            return Err(JevStop::new(
                JevStatus::BudgetExceeded,
                "Reached the demo's model-call budget",
            )
            .into());
        }
        let mut observed = self.observation.clone();
        if let Some(object) = observed.as_object_mut() {
            object.remove("_screenshot");
        }
        // Images cannot be scrubbed like text. Do not capture a recognized
        // secret field or a page that may echo a secret entered earlier.
        if self.config.decision.uses_images()
            && may_capture(&observed, self.secrets.len(), &self.history)
            && let Some(image) = self.browser.screenshot().await?
        {
            observed["_screenshot"] = json!(image);
        }
        let decision = self.config.decision.as_ref();
        let (observation, goal, history) = (&observed, &self.config.goal, &self.history);
        let chosen = if self.config.irreversible_gate {
            decision.choose_gated(observation, goal, history).await
        } else {
            decision.choose(observation, goal, history).await
        };
        let decision = match chosen {
            Ok(decision) => decision,
            Err(error) => {
                // An answer that failed validation was still billed.
                if let Some(usage) = JevBilled::usage_of(&error) {
                    self.decisions += 1;
                    self.unusable_decisions.push(usage.clone());
                }
                return Err(error);
            }
        };
        self.decisions += 1;
        self.decision_calls.push(JevDecisionRecord {
            choice: decision.choice.clone(),
            operation: decision.operation.clone(),
            target: decision.target.clone(),
            confidence: decision.confidence,
            target_confidence: decision.target_confidence,
            probabilities: decision.probabilities.clone(),
            latency_ms: decision.latency_ms,
            usage: decision.usage.clone(),
            model: decision.model.clone(),
            irreversible: decision.irreversible,
        });
        Ok(decision)
    }
}
fn may_capture(observation: &Value, secrets: usize, history: &[Value]) -> bool {
    secrets == 0
        && !history.iter().any(|h| h["text"] == SECRET)
        && !observation["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .any(secret::is_secret)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshots_are_suppressed_for_secret_fields_and_past_secret_input() {
        let public = json!({"actions":[{"kind":"fill","input_type":"email"}]});
        assert!(may_capture(&public, 0, &[]));
        assert!(!may_capture(&public, 1, &[]));
        assert!(!may_capture(&public, 0, &[json!({"text":SECRET})]));
        for kind in ["password", "one-time-code"] {
            assert!(!may_capture(
                &json!({"actions":[{"input_type":kind}]}),
                0,
                &[]
            ));
        }
    }
}
