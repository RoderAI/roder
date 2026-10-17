//! Public decision-client and budget-aware provider boundaries.
use super::JevDecision;
use async_trait::async_trait;
use serde_json::Value;

/// Chooses the next action from an observation produced by [`JevBrowser`](crate::JevBrowser).
#[async_trait]
pub trait JevDecisionClient: Send + Sync {
    /// Whether the client can use a viewport image alongside observed actions.
    /// When available and allowed by secret handling, the engine supplies an
    /// inline data URL in `observation["_screenshot"]` for this decision only.
    /// Wrapping clients should forward this capability.
    fn uses_images(&self) -> bool {
        false
    }

    /// Custom clients making provider calls must use
    /// [`JevDecisionTransport::decide_budgeted`] for each component request.
    /// Aggregate component token usage, including [`crate::JevBilled`] usage
    /// if a later component fails. Built-in clients already do this.
    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision>;

    /// [`choose`](Self::choose) with the irreversible-action gate on: the
    /// same request also asks, for each offered action that may commit
    /// something, whether it would do what cannot be undone, and the decision
    /// carries the chosen action's answer in [`JevDecision::irreversible`].
    /// The loop calls this instead of `choose` when
    /// [`JevEngineConfig::with_irreversible_gate`](crate::JevEngineConfig::with_irreversible_gate) is set. The default asks
    /// nothing, so the gate treats every such action a client without it
    /// chooses as irreversible; a wrapping client should forward it.
    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.choose(observation, goal, history).await
    }
}

/// Sends a provider-native decision request and returns its JSON response.
/// The constructing client determines whether the wire format is TypeSafe or
/// OpenAI Decisions; relays must preserve that provider contract.
///
/// Hosted runners can use this seam to route decisions through an authenticated
/// control plane without moving provider credentials into the runner.
#[async_trait]
pub trait JevDecisionTransport: Send + Sync {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value>;

    /// Reserve one component request from the active browser decision budget
    /// before calling the provider. Custom decision clients should call this
    /// once per leaf request, including audits and independently batched heads.
    /// Built-in clients already reserve their leaf requests.
    ///
    /// Outside a browser engine decision there is no active budget, so this
    /// delegates directly. Arbitrary I/O that bypasses this boundary cannot be
    /// counted or prevented by the engine; the embedding host owns that I/O.
    async fn decide_budgeted(&self, request: &Value) -> anyhow::Result<Value> {
        crate::request_budget::reserve()?;
        self.decide(request).await
    }
}
