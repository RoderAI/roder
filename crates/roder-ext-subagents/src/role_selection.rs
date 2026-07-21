use anyhow::bail;
use roder_api::subagents::SubagentDefinition;

/// Compact configured-role catalog for model-facing tool descriptions and
/// actionable validation errors.
pub(crate) fn configured_role_catalog(definitions: &[SubagentDefinition]) -> String {
    let mut roles = definitions
        .iter()
        .map(|definition| {
            let tools = if definition.tools.is_empty() {
                "no tools".to_string()
            } else {
                definition.tools.join(", ")
            };
            format!("{} [{tools}]", definition.agent_type)
        })
        .collect::<Vec<_>>();
    roles.sort();

    if roles.is_empty() {
        "none configured".to_string()
    } else {
        roles.join("; ")
    }
}

/// Reject a role name before a task or swarm has created child work.
///
/// Lanes are intentionally not aliases for roles: a lane only narrows the
/// selected role's existing whitelist and must never grant tools by fallback.
pub(crate) fn validate_configured_subagent_type(
    definitions: &[SubagentDefinition],
    subagent_type: Option<&str>,
) -> anyhow::Result<()> {
    let Some(subagent_type) = subagent_type else {
        return Ok(());
    };
    if definitions
        .iter()
        .any(|definition| definition.agent_type == subagent_type)
    {
        return Ok(());
    }

    let lane_hint = matches!(subagent_type, "scout" | "editor" | "reviewer" | "runner")
        .then_some(
            " Lane names are not roles: use a configured role whose declared tools fit the lane, or use spawn_agent for generic repository work.",
        )
        .unwrap_or_default();
    bail!(
        "unknown configured subagent type {subagent_type:?}. Available roles: {}.{lane_hint}",
        configured_role_catalog(definitions),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::subagents::SubagentPermissionMode;

    fn definition(name: &str, tools: &[&str]) -> SubagentDefinition {
        SubagentDefinition {
            agent_type: name.to_string(),
            description: name.to_string(),
            tools: tools.iter().map(|tool| (*tool).to_string()).collect(),
            model: None,
            system_prompt: None,
            permission_mode: SubagentPermissionMode::Default,
            max_turns: None,
            max_result_chars: None,
        }
    }

    #[test]
    fn catalog_is_sorted_and_includes_declared_tools() {
        assert_eq!(
            configured_role_catalog(&[
                definition("review", &["read_file"]),
                definition("market-analyst", &["echo"]),
            ]),
            "market-analyst [echo]; review [read_file]"
        );
    }

    #[test]
    fn lane_name_is_not_accepted_as_a_role_alias() {
        let error = validate_configured_subagent_type(
            &[definition("market-analyst", &["echo"])],
            Some("scout"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("Lane names are not roles"));
        assert!(error.to_string().contains("market-analyst [echo]"));
    }
}
