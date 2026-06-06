use roder_api::inference::SpeedPolicyPhase;
use roder_api::inference_routing::{InferenceRoutingContext, InferenceRoutingSignal};

use crate::signals::{local_signal, text_signals};

#[derive(Debug, Clone)]
pub struct ProfiledTurn {
    pub signals: Vec<InferenceRoutingSignal>,
    pub risks: Vec<String>,
    pub intents: Vec<String>,
    pub routine: bool,
    pub recovery: bool,
    pub high_risk: bool,
}

pub fn profile_context(
    context: &InferenceRoutingContext,
    classifier_prompt: Option<&str>,
) -> ProfiledTurn {
    let mut signals = context.signals.clone();
    signals.push(local_signal(
        "runtime_profile",
        context.runtime_profile.as_str(),
        None,
    ));

    if context.transcript.has_image_input {
        signals.push(local_signal("input", "image", Some(1.0)));
    }
    if context.tools.has_file_tools {
        signals.push(local_signal("tools", "file", Some(0.3)));
    }
    if context.tools.has_shell_tools {
        signals.push(local_signal("tools", "shell", Some(0.5)));
    }
    if context.tools.has_network_tools {
        signals.push(local_signal("tools", "network", Some(0.4)));
    }
    if context.tools.requires_tool_calls {
        signals.push(local_signal("tools", "required", Some(0.4)));
    }
    if context.prior_failures > 0 {
        signals.push(local_signal("recovery", "prior_failure", Some(1.0)));
    }
    if context.prior_escalations > 0 {
        signals.push(local_signal("recovery", "prior_escalation", Some(1.0)));
    }
    if classifier_prompt.is_some_and(|prompt| !prompt.trim().is_empty()) {
        signals.push(local_signal("routing_prompt", "configured", None));
    }
    for tool_name in &context.transcript.recent_tool_names {
        signals.push(local_signal(
            "recent_tool",
            tool_family(tool_name),
            Some(0.4),
        ));
    }
    if let Some(preview) = context.transcript.latest_user_message_preview.as_deref() {
        signals.extend(text_signals(preview));
    }

    dedupe_signals(&mut signals);
    let risks = signal_values(&signals, "risk");
    let intents = signal_values(&signals, "intent");
    let recovery = context.prior_failures > 0
        || context.prior_escalations > 0
        || context.phase == Some(SpeedPolicyPhase::Recovery);
    let high_risk = recovery || !risks.is_empty() || context.transcript.has_image_input;
    let routine = !high_risk
        && intents.iter().any(|intent| {
            matches!(
                intent.as_str(),
                "file_lookup" | "small_edit" | "documentation"
            )
        });

    ProfiledTurn {
        signals,
        risks,
        intents,
        routine,
        recovery,
        high_risk,
    }
}

fn signal_values(signals: &[InferenceRoutingSignal], key: &str) -> Vec<String> {
    let mut values = Vec::new();
    for signal in signals {
        if signal.key == key && !values.contains(&signal.value) {
            values.push(signal.value.clone());
        }
    }
    values
}

fn dedupe_signals(signals: &mut Vec<InferenceRoutingSignal>) {
    let mut seen = Vec::<(String, String)>::new();
    signals.retain(|signal| {
        let key = (signal.key.clone(), signal.value.clone());
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
}

fn tool_family(name: &str) -> &'static str {
    if name.contains("shell") || name.contains("command") || name.contains("exec") {
        "shell"
    } else if name.contains("write") || name.contains("edit") || name.contains("patch") {
        "write"
    } else if name.contains("read") || name.contains("find") || name.contains("search") {
        "read"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use roder_api::inference::{ModelSelection, RuntimeProfile};
    use roder_api::inference_routing::{
        InferenceRoutingToolSummary, InferenceRoutingTranscriptSummary,
    };

    use super::*;

    #[test]
    fn profile_marks_security_recovery_as_high_risk() {
        let context = InferenceRoutingContext {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            round_index: 0,
            runtime_profile: RuntimeProfile::Interactive,
            default_selection: ModelSelection {
                provider: "mock".to_string(),
                model: "mock".to_string(),
            },
            requested_selection: None,
            phase: Some(SpeedPolicyPhase::Recovery),
            transcript: InferenceRoutingTranscriptSummary {
                latest_user_message_preview: Some("debug auth permission failure".to_string()),
                ..InferenceRoutingTranscriptSummary::default()
            },
            tools: InferenceRoutingToolSummary::default(),
            candidates: Vec::new(),
            signals: Vec::new(),
            prior_failures: 1,
            prior_escalations: 0,
            estimated_input_tokens: Some(100),
        };

        let profiled = profile_context(&context, None);

        assert!(profiled.high_risk);
        assert!(profiled.recovery);
        assert!(profiled.risks.contains(&"security".to_string()));
    }
}
