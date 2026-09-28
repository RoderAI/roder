//! The `effect` eval variant: each recent action's effect in the decision
//! request, next to `page_changed`, for an A/B run against the production
//! request, which does not carry it.
//!
//! The request is built from four keys of each history entry, so the effects
//! travel beside it: the client puts the last ten entries' effects in a
//! task-local for the one decision, and the transport writes them into
//! `state.recent_actions` in the same order. A task-local keeps concurrent
//! episodes apart. Still one request per step.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::decide::JevTypeSafeDecisionClient;
use crate::engine::{JevDecision, JevDecisionClient, JevDecisionTransport};

tokio::task_local! {
    static EFFECTS: Vec<Value>;
}

/// The decision client for the variant, sending through `transport`.
pub(crate) fn client(
    model: String,
    transport: Arc<dyn JevDecisionTransport>,
) -> Arc<dyn JevDecisionClient> {
    let inner = JevTypeSafeDecisionClient::with_transport(model, Arc::new(Carry { transport }));
    Arc::new(Effects { inner })
}

struct Effects {
    inner: JevTypeSafeDecisionClient,
}

#[async_trait]
impl JevDecisionClient for Effects {
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        EFFECTS
            .scope(
                recent_effects(history),
                self.inner.choose(observation, goal, history),
            )
            .await
    }

    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        EFFECTS
            .scope(
                recent_effects(history),
                self.inner.choose_gated(observation, goal, history),
            )
            .await
    }
}

/// The effects of the same ten entries `decide::request_body` sends.
fn recent_effects(history: &[Value]) -> Vec<Value> {
    history
        .iter()
        .skip(history.len().saturating_sub(10))
        .map(|entry| entry["effect"].clone())
        .collect()
}

struct Carry {
    transport: Arc<dyn JevDecisionTransport>,
}

#[async_trait]
impl JevDecisionTransport for Carry {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        let mut request = request.clone();
        add_effects(
            &mut request,
            &EFFECTS.try_with(Clone::clone).unwrap_or_default(),
        );
        self.transport.decide(&request).await
    }
}

/// Put `effects[i]` on the i-th recent action, after `page_changed`.
fn add_effects(request: &mut Value, effects: &[Value]) {
    let Some(recent) = request["state"]["recent_actions"].as_array_mut() else {
        return;
    };
    for (entry, effect) in recent.iter_mut().zip(effects) {
        if let Some(entry) = entry.as_object_mut()
            && !effect.is_null()
        {
            entry.insert("effect".into(), effect.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn effects_land_on_their_own_recent_actions() {
        let mut request = json!({"state": {"recent_actions": [
            {"action": "Open", "kind": "click", "text": null, "page_changed": true},
            {"action": "Retry", "kind": "click", "text": null, "page_changed": false},
        ]}});
        add_effects(
            &mut request,
            &[
                json!("went to https://a.test/"),
                json!("nothing visible changed"),
            ],
        );
        assert_eq!(
            request["state"]["recent_actions"][1],
            json!({"action": "Retry", "kind": "click", "text": null, "page_changed": false,
                "effect": "nothing visible changed"})
        );
        // Keys keep their order: the effect comes after page_changed.
        let keys = request["state"]["recent_actions"][0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(keys.last().map(String::as_str), Some("effect"));
    }
}
