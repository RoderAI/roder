//! Research requirements progress inside one delegation, without a planning LLM.
use super::*;
impl Agent {
    pub(super) fn decision_goal(&self) -> String {
        if !self.config.research || self.config.requirements.is_empty() {
            return self.config.goal.clone();
        }
        let index = self.requirement.min(self.config.requirements.len() - 1);
        format!(
            "CURRENT REQUIREMENT {}/{}: {}\nComplete only this requirement now. DONE means its required facts have been observed; code will advance the remaining checklist. Do not attempt later requirements yet. Preserve all constraints of the overall task below, including what must not be done. Previously observed source evidence may satisfy this requirement without repeating browsing.\nOverall task and constraints: {}",
            index + 1,
            self.config.requirements.len(),
            self.config.requirements[index],
            self.config.goal
        )
    }
    /// Record a driver claim with the sources available when it was made. This
    /// advances work; it is not an independent proof or an authorization.
    pub(super) fn complete_requirement(&mut self) -> bool {
        if !self.config.research || self.config.requirements.is_empty() {
            return false;
        }
        let index = self.requirement.min(self.config.requirements.len() - 1);
        let reason = self.secrets.scrub(&self.config.requirements[index]);
        let before = self.recovery.events.len();
        self.recovery
            .record("requirement_completed", &reason, self.elapsed_ms);
        if self.recovery.events.len() > before
            && let Some(record) = self.recovery.events.last_mut()
        {
            record["verification"] = json!("driver_claim");
            record["requirement"] = json!(index + 1);
            record["evidence_sources"] = json!(self.evidence.sources(&self.secrets));
        }
        if index + 1 < self.config.requirements.len() {
            self.requirement += 1;
            self.model_snapshot = None;
            self.status = JevStatus::Ready;
            return true;
        }
        false
    }
}
