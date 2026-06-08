use roder_api::catalog::{
    REASONING_HIGH, REASONING_LOW, REASONING_MEDIUM, REASONING_XHIGH,
    model_supports_reasoning_effort,
};
use roder_api::inference::{
    ReasoningConfig, RuntimeProfile, SpeedPolicyDecision, SpeedPolicyPhase,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSpeedPolicyConfig {
    pub enabled: bool,
    pub orientation_reasoning: String,
    pub execution_reasoning: String,
    pub verification_reasoning: String,
    pub recovery_reasoning: String,
    pub ultracode_reasoning: String,
}

impl Default for RuntimeSpeedPolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            orientation_reasoning: REASONING_HIGH.to_string(),
            execution_reasoning: REASONING_LOW.to_string(),
            verification_reasoning: REASONING_HIGH.to_string(),
            recovery_reasoning: REASONING_MEDIUM.to_string(),
            ultracode_reasoning: REASONING_XHIGH.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SpeedPolicyState {
    assistant_messages: u32,
    tool_rounds: u32,
    verification_required: bool,
    failure_seen: bool,
}

impl SpeedPolicyState {
    pub(crate) fn record_model_output(&mut self, assistant_message_seen: bool, tool_calls: usize) {
        if assistant_message_seen {
            self.assistant_messages = self.assistant_messages.saturating_add(1);
        }
        if tool_calls > 0 {
            self.tool_rounds = self.tool_rounds.saturating_add(1);
        }
    }

    pub(crate) fn record_verification_required(&mut self) {
        self.verification_required = true;
    }

    pub(crate) fn record_failure(&mut self) {
        self.failure_seen = true;
    }

    pub(crate) fn decision(
        &self,
        runtime_profile: RuntimeProfile,
        model: &str,
        config: &RuntimeSpeedPolicyConfig,
    ) -> Option<SpeedPolicyDecision> {
        if !config.enabled {
            return None;
        }
        if runtime_profile != RuntimeProfile::Eval {
            return None;
        }

        let phase = self.phase();
        let desired_reasoning = config.reasoning_for_phase(phase).to_string();
        let supported = model_supports_reasoning_effort(model, &desired_reasoning);
        Some(SpeedPolicyDecision {
            phase,
            applied_reasoning: supported.then(|| desired_reasoning.clone()),
            desired_reasoning,
            supported,
        })
    }

    pub(crate) fn phase(&self) -> SpeedPolicyPhase {
        if self.failure_seen {
            SpeedPolicyPhase::Recovery
        } else if self.verification_required {
            SpeedPolicyPhase::Verification
        } else if self.assistant_messages == 0 && self.tool_rounds == 0 {
            SpeedPolicyPhase::Orientation
        } else {
            SpeedPolicyPhase::Execution
        }
    }
}

impl RuntimeSpeedPolicyConfig {
    fn reasoning_for_phase(&self, phase: SpeedPolicyPhase) -> &str {
        match phase {
            SpeedPolicyPhase::Orientation => &self.orientation_reasoning,
            SpeedPolicyPhase::Execution => &self.execution_reasoning,
            SpeedPolicyPhase::Verification => &self.verification_reasoning,
            SpeedPolicyPhase::Recovery => &self.recovery_reasoning,
        }
    }
}

pub(crate) fn reasoning_from_decision(
    decision: Option<&SpeedPolicyDecision>,
    fallback: ReasoningConfig,
) -> ReasoningConfig {
    let Some(level) = decision.and_then(|decision| decision.applied_reasoning.as_deref()) else {
        return fallback;
    };
    ReasoningConfig {
        enabled: true,
        level: Some(level.to_string()),
    }
}

pub(crate) fn reasoning_for_supported_effort(
    model: &str,
    desired_reasoning: &str,
    fallback: ReasoningConfig,
) -> (ReasoningConfig, bool) {
    if model_supports_reasoning_effort(model, desired_reasoning) {
        (
            ReasoningConfig {
                enabled: true,
                level: Some(desired_reasoning.to_string()),
            },
            true,
        )
    } else {
        (fallback, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::catalog::{REASONING_NONE, REASONING_XHIGH};

    #[test]
    fn speed_policy_moves_high_to_low_to_high_and_recovery_medium() {
        let mut state = SpeedPolicyState::default();
        let config = RuntimeSpeedPolicyConfig::default();

        let orientation = state
            .decision(RuntimeProfile::Eval, "gpt-5.5", &config)
            .unwrap();
        assert_eq!(orientation.phase, SpeedPolicyPhase::Orientation);
        assert_eq!(
            orientation.applied_reasoning.as_deref(),
            Some(REASONING_HIGH)
        );

        state.record_model_output(true, 1);
        let execution = state
            .decision(RuntimeProfile::Eval, "gpt-5.5", &config)
            .unwrap();
        assert_eq!(execution.phase, SpeedPolicyPhase::Execution);
        assert_eq!(execution.applied_reasoning.as_deref(), Some(REASONING_LOW));

        state.record_verification_required();
        let verification = state
            .decision(RuntimeProfile::Eval, "gpt-5.5", &config)
            .unwrap();
        assert_eq!(verification.phase, SpeedPolicyPhase::Verification);
        assert_eq!(
            verification.applied_reasoning.as_deref(),
            Some(REASONING_HIGH)
        );

        state.record_failure();
        let recovery = state
            .decision(RuntimeProfile::Eval, "gpt-5.5", &config)
            .unwrap();
        assert_eq!(recovery.phase, SpeedPolicyPhase::Recovery);
        assert_eq!(
            recovery.applied_reasoning.as_deref(),
            Some(REASONING_MEDIUM)
        );
    }

    #[test]
    fn unsupported_model_degrades_to_fallback_reasoning() {
        let state = SpeedPolicyState::default();
        let config = RuntimeSpeedPolicyConfig::default();
        let decision = state
            .decision(RuntimeProfile::Eval, "mock", &config)
            .unwrap();
        let fallback = ReasoningConfig {
            enabled: true,
            level: Some(REASONING_XHIGH.to_string()),
        };

        assert_eq!(decision.phase, SpeedPolicyPhase::Orientation);
        assert_eq!(decision.desired_reasoning, REASONING_HIGH);
        assert_eq!(decision.applied_reasoning, None);
        assert!(!decision.supported);
        assert_eq!(
            reasoning_from_decision(Some(&decision), fallback.clone()),
            fallback
        );
    }

    #[test]
    fn supported_effort_override_degrades_to_fallback_reasoning() {
        let fallback = ReasoningConfig {
            enabled: true,
            level: Some(REASONING_MEDIUM.to_string()),
        };

        let (reasoning, supported) =
            reasoning_for_supported_effort("mock", REASONING_XHIGH, fallback.clone());
        assert!(!supported);
        assert_eq!(reasoning, fallback);

        let (reasoning, supported) =
            reasoning_for_supported_effort("gpt-5.5", REASONING_XHIGH, fallback);
        assert!(supported);
        assert_eq!(reasoning.level.as_deref(), Some(REASONING_XHIGH));
    }

    #[test]
    fn non_eval_profiles_do_not_apply_speed_policy() {
        let state = SpeedPolicyState::default();
        let config = RuntimeSpeedPolicyConfig::default();
        assert_eq!(
            state.decision(RuntimeProfile::Interactive, "gpt-5.5", &config),
            None
        );
        assert_eq!(
            reasoning_from_decision(
                None,
                ReasoningConfig {
                    enabled: false,
                    level: Some(REASONING_NONE.to_string())
                }
            ),
            ReasoningConfig {
                enabled: false,
                level: Some(REASONING_NONE.to_string())
            }
        );
    }
}
