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
                .filter(|tool| native_tool_supported(&tool.name, provider))
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
    fn computer_is_native_openai_only() {
        assert!(native_tool_supported("computer", "openai"));
        for provider in ["codex", "anthropic", "openrouter", "xai", "mock"] {
            assert!(!native_tool_supported("computer", provider));
            assert!(native_tool_supported("chrome_click", provider));
        }
    }
}
