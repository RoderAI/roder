//! Requested-model parameters differ between legacy and registry-backed models.
use super::*;

pub(super) fn encode_requested_model(
    model_id: &str,
    reasoning: Option<&ReasoningConfig>,
) -> Vec<u8> {
    // requested_model.f3 is a repeated {key, value} param list. cursor-agent
    // sends thinking/context/effort/fast; effort=high in particular drives the
    // model's full agentic thoroughness (without it the model does minimal work
    // and stops after a single read). Fast catalog ids such as
    // `composer-2.5-fast` map to the bare wire id plus `fast=true`.
    let (wire_model_id, fast) = cursor_run_model_params(model_id);
    let thinking = cursor_thinking_enabled(reasoning);
    let effort = cursor_effort_level(reasoning);
    let param = |key: &str, value: &str| {
        proto_field_bytes(
            3,
            proto_message(vec![
                proto_field_string(1, key),
                proto_field_string(2, value),
            ]),
        )
    };
    // Registry-backed Gemini 4 accepts reasoning_effort, not the older
    // thinking/context/effort/fast parameter set (AvailableModels contract).
    if wire_model_id == "gemini-4-argon" {
        let effort = if matches!(effort.as_str(), "low" | "medium" | "high") {
            effort.as_str()
        } else {
            "high"
        };
        return proto_message(vec![
            proto_field_string(1, &wire_model_id),
            param("reasoning_effort", effort),
        ]);
    }
    proto_message(vec![
        proto_field_string(1, &wire_model_id),
        param("thinking", if thinking { "true" } else { "false" }),
        param("context", "300k"),
        param("effort", &effort),
        param("fast", if fast { "true" } else { "false" }),
    ])
}

fn cursor_thinking_enabled(reasoning: Option<&ReasoningConfig>) -> bool {
    match reasoning {
        None => true,
        Some(cfg) if !cfg.enabled => false,
        Some(cfg) => !matches!(cfg.level.as_deref(), Some("none")),
    }
}

fn cursor_effort_level(reasoning: Option<&ReasoningConfig>) -> String {
    match reasoning {
        None => "high".to_string(),
        Some(cfg) if !cfg.enabled => "none".to_string(),
        Some(cfg) => cfg
            .level
            .as_deref()
            .filter(|level| !level.is_empty())
            .unwrap_or("high")
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gemini4_uses_only_registry_supported_parameters() {
        for level in ["low", "medium", "high"] {
            let model = encode_requested_model(
                "gemini-4-argon",
                Some(&ReasoningConfig {
                    enabled: true,
                    level: Some(level.into()),
                }),
            );
            let params = decode_fields_safe(&model)
                .into_iter()
                .filter_map(|field| match (field.no, field.value) {
                    (3, ProtoValue::Bytes(value)) => Some((
                        scalar_string(&value, 1).unwrap(),
                        scalar_string(&value, 2).unwrap(),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(params, vec![("reasoning_effort".into(), level.into())]);
        }
    }
}
