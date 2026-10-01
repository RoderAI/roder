use super::*;

impl Runtime {
    /**
     * Allowlists apply to built-in tools only; external tools are advertised with their
     * host-supplied schemas as given. An external tool shadows a built-in with the same name in
     * both advertisement and dispatch (see `route_tool_call`).
     */
    pub(super) fn filtered_tool_specs(
        &self,
        cfg: &RuntimeConfig,
        model: &str,
        provider: &str,
        profile: Option<&ModelHarnessProfile>,
        thread_allowlist: &[String],
        external_tools: &[roder_api::tools::ToolSpec],
    ) -> Vec<roder_api::tools::ToolSpec> {
        let mut specs = self
            .tool_registry
            .specs_for_edit_tool_with_schema_policy(
                edit_tool_for_model(cfg, model),
                schema_policy_for_model(profile),
            )
            .into_iter()
            .filter(|spec| {
                native_tool_supported(&spec.name, provider)
                    && allowlist_permits(&cfg.tool_allowlist, &spec.name)
                    && allowlist_permits(thread_allowlist, &spec.name)
                    && !external_tools.iter().any(|tool| tool.name == spec.name)
            })
            .collect::<Vec<_>>();
        specs.extend(
            external_tools
                .iter()
                .filter(|tool| tool.name != roder_api::computer::COMPUTER_TOOL_NAME)
                .cloned(),
        );
        specs
    }
}

fn native_tool_supported(name: &str, provider: &str) -> bool {
    name != roder_api::computer::COMPUTER_TOOL_NAME || provider == "openai"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_functions_cannot_shadow_the_native_computer_binding() {
        assert!(
            validate_external_tool_names(&[roder_api::computer::computer_tool_spec()]).is_err()
        );
        let mut external = roder_api::computer::computer_tool_spec();
        external.name = "host_computer".into();
        validate_external_tool_names(&[external]).unwrap();
    }
    #[test]
    fn computer_is_native_openai_only() {
        assert!(native_tool_supported("computer", "openai"));
        for provider in ["codex", "anthropic", "openrouter", "xai", "mock"] {
            assert!(!native_tool_supported("computer", provider));
            assert!(native_tool_supported("chrome_click", provider));
        }
    }
}

pub(super) fn validate_external_tool_names(
    tools: &[roder_api::tools::ToolSpec],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !tools
            .iter()
            .any(|tool| tool.name == roder_api::computer::COMPUTER_TOOL_NAME),
        "computer is reserved for the native browser binding; rename the external function tool"
    );
    Ok(())
}
