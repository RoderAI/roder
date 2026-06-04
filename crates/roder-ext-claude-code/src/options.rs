use std::path::Path;
use std::sync::Arc;

use claude_agent_sdk::mcp::{MCPContent, SdkMcpTool, SimpleMCPServer};
use claude_agent_sdk::{ClaudeAgentOptions, PermissionMode, PermissionResult, SettingSource};
use roder_api::inference::{AgentInferenceRequest, ToolCallCompleted, TurnToolExecutor};
use roder_api::tools::ToolChoice;

use crate::provider::ClaudeCodeConfig;

pub fn build_options(
    config: &ClaudeCodeConfig,
    request: &AgentInferenceRequest,
    tool_executor: Option<Arc<dyn TurnToolExecutor>>,
    cwd: Option<&Path>,
) -> anyhow::Result<ClaudeAgentOptions> {
    let mut builder = ClaudeAgentOptions::builder()
        .model(request.model.model.clone())
        .include_partial_messages(true);
    if let Some(cli_path) = config
        .cli_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        builder = builder.cli_path(cli_path.to_string());
    }
    if let Some(cwd) = cwd {
        builder = builder.cwd(cwd.display().to_string());
    }
    if let Some(system) = merged_system_prompt(request) {
        builder = builder.system_prompt(system);
    }
    if let Some(mode) = config.permission_mode.as_deref() {
        builder = builder.permission_mode(parse_permission_mode(mode)?);
    }
    if let Some(setting_sources) = &config.setting_sources {
        builder = builder.setting_sources(parse_setting_sources(setting_sources)?);
    }
    if request.reasoning.enabled {
        if let Some(level) = request.reasoning.level.as_deref() {
            builder = builder.effort(level.to_string());
        }
    }
    if let Some(max_tokens) = request.output.max_tokens {
        let budget = i32::try_from(max_tokens).unwrap_or(i32::MAX);
        builder = builder.max_thinking_tokens(budget);
    }
    let roder_tool_names = roder_tool_names(request);
    if !request.tools.is_empty() && !matches!(request.tool_choice, ToolChoice::None) {
        let executor = tool_executor.ok_or_else(|| {
            anyhow::anyhow!("Claude Code provider requires a Roder tool executor")
        })?;
        let server = roder_sdk_mcp_server(request, executor);
        builder = builder.sdk_mcp_server("roder", server);
        builder = builder.allowed_tools(
            roder_tool_names
                .iter()
                .map(|name| format!("mcp__roder__{name}"))
                .collect(),
        );
    }
    builder = builder.can_use_tool(move |tool_name, _input, _context| {
        let roder_tool_names = roder_tool_names.clone();
        async move {
            let Some(roder_tool_name) = tool_name.strip_prefix("mcp__roder__") else {
                return Ok(PermissionResult::deny(format!(
                    "Claude Code tool {tool_name} is not managed by Roder"
                )));
            };
            if roder_tool_names.iter().any(|name| name == roder_tool_name) {
                Ok(PermissionResult::allow())
            } else {
                Ok(PermissionResult::deny(format!(
                    "Claude Code tool {tool_name} was not advertised for this Roder turn"
                )))
            }
        }
    });
    Ok(builder.build())
}

fn roder_tool_names(request: &AgentInferenceRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

fn roder_sdk_mcp_server(
    request: &AgentInferenceRequest,
    executor: Arc<dyn TurnToolExecutor>,
) -> SimpleMCPServer {
    let tools = request
        .tools
        .iter()
        .map(|spec| {
            let name = spec.name.clone();
            let description = spec.description.clone();
            let schema = spec.parameters.clone();
            let executor = Arc::clone(&executor);
            SdkMcpTool::new(name.clone(), description, schema, None, move |input| {
                let call = ToolCallCompleted {
                    id: format!("claude-code-{name}"),
                    name: name.clone(),
                    arguments: input.to_string(),
                };
                let handle = tokio::runtime::Handle::current();
                let outcome =
                    tokio::task::block_in_place(|| handle.block_on(executor.execute(call)));
                match outcome {
                    Ok(outcome) => {
                        let text = if outcome.is_error {
                            format!("Tool returned an error:\n{}", outcome.result)
                        } else {
                            outcome.result
                        };
                        Ok(vec![MCPContent::Text { text }])
                    }
                    Err(err) => Err(err.to_string()),
                }
            })
        })
        .collect();
    claude_agent_sdk::mcp::create_sdk_mcp_server("roder", tools)
}

fn merged_system_prompt(request: &AgentInferenceRequest) -> Option<String> {
    match (
        request.instructions.system.as_deref(),
        request.instructions.developer.as_deref(),
    ) {
        (Some(system), Some(developer)) => Some(format!("{system}\n\n{developer}")),
        (Some(system), None) => Some(system.to_string()),
        (None, Some(developer)) => Some(developer.to_string()),
        (None, None) => None,
    }
}

pub fn parse_permission_mode(value: &str) -> anyhow::Result<PermissionMode> {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "default" => Ok(PermissionMode::Default),
        "accept-edits" | "acceptedits" => Ok(PermissionMode::AcceptEdits),
        "plan" => Ok(PermissionMode::Plan),
        "bypass-permissions" | "bypasspermissions" => Ok(PermissionMode::BypassPermissions),
        "dont-ask" | "dontask" => Ok(PermissionMode::DontAsk),
        "auto" => Ok(PermissionMode::Auto),
        other => anyhow::bail!(
            "unsupported Claude Code permission mode {other:?}; expected default, accept-edits, plan, bypass-permissions, dont-ask, or auto"
        ),
    }
}

pub fn parse_setting_sources(values: &[String]) -> anyhow::Result<Vec<SettingSource>> {
    values
        .iter()
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "user" => Ok(SettingSource::User),
            "project" => Ok(SettingSource::Project),
            "local" => Ok(SettingSource::Local),
            other => anyhow::bail!(
                "unsupported Claude Code setting source {other:?}; expected user, project, or local"
            ),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use roder_api::inference::{
        AgentInferenceRequest, HostedWebSearchConfig, InstructionBundle, ModelSelection,
        OutputConfig, ReasoningConfig, RuntimeHints, TurnToolOutcome,
    };
    use roder_api::tools::{ToolChoice, ToolSpec};

    use super::*;

    #[test]
    fn parses_permission_modes() {
        assert!(matches!(
            parse_permission_mode("accept-edits").unwrap(),
            PermissionMode::AcceptEdits
        ));
        assert!(parse_permission_mode("root").is_err());
    }

    #[test]
    fn builds_options_without_running_claude() {
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "claude-code".to_string(),
                model: "sonnet".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("system".to_string()),
                developer: Some("developer".to_string()),
            },
            transcript: Vec::new(),
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig {
                max_tokens: Some(1024),
                temperature: None,
                top_p: None,
                response_format: None,
            },
            runtime: RuntimeHints {
                hosted_web_search: HostedWebSearchConfig::disabled(),
                ..RuntimeHints::default()
            },
            metadata: serde_json::json!({}),
        };
        let options = build_options(
            &ClaudeCodeConfig {
                cli_path: Some("/bin/claude".to_string()),
                permission_mode: Some("default".to_string()),
                setting_sources: Some(vec!["user".to_string(), "project".to_string()]),
                workspace: None,
            },
            &request,
            None,
            None,
        )
        .unwrap();
        assert_eq!(options.model.as_deref(), Some("sonnet"));
        assert_eq!(options.cli_path.as_deref(), Some("/bin/claude"));
        assert_eq!(
            options.system_prompt.as_deref(),
            Some("system\n\ndeveloper")
        );
        assert_eq!(options.effort.as_deref(), Some("medium"));
        assert!(options.can_use_tool.is_some());
    }

    struct FakeToolExecutor;

    #[async_trait::async_trait]
    impl TurnToolExecutor for FakeToolExecutor {
        async fn execute(&self, _call: ToolCallCompleted) -> anyhow::Result<TurnToolOutcome> {
            Ok(TurnToolOutcome {
                result: "ok".to_string(),
                is_error: false,
            })
        }
    }

    #[test]
    fn registers_roder_tools_as_sdk_mcp_server() {
        let mut request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "claude-code".to_string(),
                model: "sonnet".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: Vec::new(),
            tools: vec![ToolSpec {
                name: "grep".to_string(),
                description: "Search text".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                hosted_web_search: HostedWebSearchConfig::disabled(),
                ..RuntimeHints::default()
            },
            metadata: serde_json::json!({}),
        };

        let options = build_options(
            &ClaudeCodeConfig::default(),
            &request,
            Some(Arc::new(FakeToolExecutor)),
            None,
        )
        .unwrap();

        assert!(options.mcp_servers.contains_key("roder"));
        assert_eq!(options.allowed_tools, vec!["mcp__roder__grep"]);
        let server = options.sdk_mcp_servers.get("roder").unwrap();
        assert_eq!(server.list_tools()[0].name, "grep");

        request.tool_choice = ToolChoice::None;
        let options = build_options(&ClaudeCodeConfig::default(), &request, None, None).unwrap();
        assert!(options.sdk_mcp_servers.is_empty());
        assert!(options.allowed_tools.is_empty());
    }
}
