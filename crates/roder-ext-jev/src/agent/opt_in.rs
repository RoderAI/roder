//! The two opt-in behaviours' part of the loop: the irreversible-action
//! gate's stop, and a refused cookie banner's step.

use serde_json::{Value, json};

use super::{Agent, COOKIE_BANNER};
use crate::engine::{JevDecision, JevStatus, JevStop, StaleObservation};
use crate::irreversible::{self, Verdict};

impl Agent {
    /// A refused banner as a step of its own. The label is page text, as
    /// untrusted as the rest.
    pub(super) fn record_cookie_banner(&mut self, label: &str) {
        let elapsed = self.elapsed();
        self.history.push(json!({
            "step": self.history.len() + 1,
            "action": format!("refused cookie banner: {label}"),
            "kind": COOKIE_BANNER,
            "choice": "",
            "text": null,
            "covered": false,
            "refused": null,
            "operation": "",
            "target": null,
            "page_changed": null,
            "effect": null,
            "url": self.observation["url"],
            "usage": {},
            "executed_ms": elapsed,
            "elapsed_ms": elapsed,
        }));
    }

    /// With the gate on, stop before an action it applies to that may not be
    /// undone, unless the run is authorized and the decision confident. A
    /// page that changed since the decision is decided again instead.
    pub(super) async fn gate(
        &mut self,
        action: &Value,
        decision: &JevDecision,
    ) -> anyhow::Result<()> {
        if !self.config.irreversible_gate || !irreversible::gated(action) {
            return Ok(());
        }
        let verdict = irreversible::verdict(
            action,
            decision.irreversible,
            self.config.irreversible_authorized,
            decision.call_confidence(),
        );
        let Verdict::Confirm(reason) = verdict else {
            return Ok(());
        };
        if !self.browser.fresh(&self.observation, Some(action)).await? {
            return Err(
                StaleObservation::new("Page changed since the decision. Choose again.").into(),
            );
        }
        Err(JevStop::new(JevStatus::NeedsConfirmation, reason).into())
    }
}
