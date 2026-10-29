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
        let mut observed =
            crate::semantic::for_model(&self.observation, self.model_snapshot.as_ref());
        observed["evidence"] = json!(self.evidence.render(&self.secrets));
        if let Some(object) = observed.as_object_mut() {
            object.remove("_screenshot");
        }
        // Images cannot be scrubbed like text. Do not capture a recognized
        // secret field or a page that may echo a secret entered earlier.
        if self.config.decision.uses_images()
            && may_capture(&observed, self.secrets.has_input(), &self.history)
            && let Some(image) = self.browser.screenshot().await.ok().flatten()
        {
            observed["_screenshot"] = json!(image);
        }
        let decision = self.config.decision.as_ref();
        let (observation, goal, history) = (&observed, &self.config.goal, &self.history);
        let remaining = self
            .config
            .max_actions
            .saturating_mul(2)
            .saturating_sub(self.decisions);
        let (chosen, requests) = crate::request_budget::scope(remaining, async {
            if self.config.irreversible_gate {
                decision.choose_gated(observation, goal, history).await
            } else {
                decision.choose(observation, goal, history).await
            }
        })
        .await;
        let mut decision = match chosen {
            Ok(decision) => decision,
            Err(error) => {
                // An answer that failed validation was still billed.
                if let Some(usage) = JevBilled::usage_of(&error) {
                    self.decisions = self.decisions.saturating_add(if requests > 0 {
                        requests
                    } else {
                        reported_calls(usage)
                    });
                    self.unusable_decisions.push(counted_usage(usage, requests));
                }
                return Err(error);
            }
        };
        self.model_snapshot = self.observation.get("snapshot").cloned();
        self.decisions = self.decisions.saturating_add(if requests > 0 {
            requests
        } else {
            reported_calls(&decision.usage)
        });
        decision.usage = counted_usage(&decision.usage, requests);
        self.unusable_streak.clear();
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
/// Count observed component requests even when a custom client omits usage.calls.
fn counted_usage(usage: &Value, requests: usize) -> Value {
    if requests == 0 {
        return usage.clone();
    }
    let mut usage = usage.as_object().cloned().unwrap_or_default();
    usage.insert("calls".into(), json!(requests));
    Value::Object(usage)
}
fn reported_calls(usage: &Value) -> usize {
    usage["calls"]
        .as_u64()
        .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
        .unwrap_or(1)
        .max(1)
}
fn may_capture(observation: &Value, secret_input: bool, history: &[Value]) -> bool {
    !secret_input
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
        assert!(may_capture(&public, false, &[]));
        assert!(!may_capture(&public, true, &[]));
        assert!(!may_capture(&public, false, &[json!({"text":SECRET})]));
        for kind in ["password", "one-time-code"] {
            assert!(!may_capture(
                &json!({"actions":[{"input_type":kind}]}),
                false,
                &[]
            ));
        }
    }
}
