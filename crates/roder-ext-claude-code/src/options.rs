use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use claude_code_sdk_rust::mcp::{MCPContent, SdkMcpTool, SimpleMCPServer};
use claude_code_sdk_rust::{
    ClaudeAgentOptions, EffortLevel, PermissionMode, PermissionResult, SettingSource,
};
use roder_api::inference::{AgentInferenceRequest, ToolCallCompleted, TurnToolExecutor};
use roder_api::tools::ToolChoice;

use crate::provider::ClaudeCodeConfig;

pub fn build_options(
    config: &ClaudeCodeConfig,
    request: &AgentInferenceRequest,
    tool_executor: Option<Arc<dyn TurnToolExecutor>>,
    cwd: Option<&Path>,
    resume_session_id: Option<&str>,
) -> anyhow::Result<ClaudeAgentOptions> {
    let mut builder = ClaudeAgentOptions::builder()
        .model(request.model.model.clone())
        .include_partial_messages(true);
    // Resume the persisted CLI session so the `claude` process keeps the prior
    // conversation server-side and applies its own auto-compaction. When set,
    // the provider only sends the new transcript tail as the prompt instead of
    // replaying the whole transcript every turn (which is what overflowed the
    // 1M context window with "Prompt is too long").
    if let Some(session_id) = resume_session_id.filter(|value| !value.trim().is_empty()) {
        builder = builder.resume(session_id.to_string());
    }
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
    if request.reasoning.enabled
        && let Some(level) = request.reasoning.level.as_deref()
        && let Some(effort) = parse_effort_level(level)
    {
        builder = builder.effort(effort);
    }
    if let Some(max_tokens) = request.output.max_tokens {
        let budget = i32::try_from(max_tokens).unwrap_or(i32::MAX);
        builder = builder.max_thinking_tokens(budget);
    }
    let allowed_tool_names = allowed_claude_tool_names(request);
    if !request.tools.is_empty() && !matches!(request.tool_choice, ToolChoice::None) {
        let executor = tool_executor.ok_or_else(|| {
            anyhow::anyhow!("Claude Code provider requires a Roder tool executor")
        })?;
        let server = roder_sdk_mcp_server(request, executor);
        builder = builder.sdk_mcp_server("roder", server);
        // Disable every built-in Claude Code tool (Read/Bash/Edit/Glob/...).
        // Roder mediates all tool access through its own executor, so the
        // model must use the `mcp__roder__*` tools we advertise. Leaving the
        // built-ins enabled lets the model call e.g. bare `Read`, which our
        // `can_use_tool` callback then denies -- burning retries until the
        // turn trips the consecutive-tool-failure reliability limit.
        builder = builder.tools(Vec::new());
        builder = builder.allowed_tools(
            allowed_tool_names
                .iter()
                .map(|name| format!("mcp__roder__{name}"))
                .collect(),
        );
    }
    builder = builder.can_use_tool(move |tool_name, _input, _context| {
        let allowed_tool_names = allowed_tool_names.clone();
        async move {
            let Some(claude_tool_name) = tool_name.strip_prefix("mcp__roder__") else {
                return Ok(PermissionResult::deny(format!(
                    "Claude Code tool {tool_name} is not managed by Roder"
                )));
            };
            if allowed_tool_names
                .iter()
                .any(|name| name == claude_tool_name)
            {
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

fn allowed_claude_tool_names(request: &AgentInferenceRequest) -> Vec<String> {
    request
        .tools
        .iter()
        .flat_map(|tool| {
            let mut names = vec![tool.name.clone()];
            names.extend(claude_aliases_for_roder_tool(&tool.name).map(str::to_string));
            names
        })
        .collect()
}

fn roder_sdk_mcp_server(
    request: &AgentInferenceRequest,
    executor: Arc<dyn TurnToolExecutor>,
) -> SimpleMCPServer {
    let tools = request
        .tools
        .iter()
        .flat_map(|spec| {
            let mut names = vec![spec.name.clone()];
            names.extend(claude_aliases_for_roder_tool(&spec.name).map(str::to_string));
            names.into_iter().map(|claude_name| {
                sdk_tool_for_spec(
                    claude_name,
                    spec.name.clone(),
                    spec.description.clone(),
                    spec.parameters.clone(),
                    Arc::clone(&executor),
                )
            })
        })
        .collect();
    claude_code_sdk_rust::mcp::create_sdk_mcp_server("roder", tools)
}

/// Monotonic counter making every claude-code tool-call id unique across the
/// process, so repeated calls of the same tool render as distinct rows.
static NEXT_TOOL_CALL_SEQ: AtomicU64 = AtomicU64::new(0);

fn sdk_tool_for_spec(
    claude_name: String,
    roder_name: String,
    description: String,
    schema: serde_json::Value,
    executor: Arc<dyn TurnToolExecutor>,
) -> SdkMcpTool {
    let call_schema = schema.clone();
    SdkMcpTool::new(
        claude_name.clone(),
        description,
        schema,
        None,
        move |input| {
            let input =
                repair_sdk_mcp_input_for_tool(input, &call_schema, &claude_name, &roder_name);
            // Each invocation must carry a unique tool-call id. The TUI and
            // runtime key tool-call rows by id, so reusing a name-derived id
            // (e.g. `claude-code-Bash`) collapses every later call of the same
            // tool into the first row and only the first one ever renders.
            let seq = NEXT_TOOL_CALL_SEQ.fetch_add(1, Ordering::Relaxed);
            let call = ToolCallCompleted {
                id: format!("claude-code-{claude_name}-{seq}"),
                name: roder_name.clone(),
                arguments: input.to_string(),
            };
            let handle = tokio::runtime::Handle::current();
            let outcome = tokio::task::block_in_place(|| handle.block_on(executor.execute(call)));
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
        },
    )
}

fn claude_aliases_for_roder_tool(name: &str) -> impl Iterator<Item = &'static str> {
    match name {
        "shell" => ["Bash"].as_slice().iter().copied(),
        "read_file" => ["Read"].as_slice().iter().copied(),
        "list_files" => ["LS"].as_slice().iter().copied(),
        "grep" => ["Grep"].as_slice().iter().copied(),
        "glob" => ["Glob"].as_slice().iter().copied(),
        "write_file" => ["Write"].as_slice().iter().copied(),
        "edit" => ["Edit"].as_slice().iter().copied(),
        "multi_edit" => ["MultiEdit"].as_slice().iter().copied(),
        _ => [].as_slice().iter().copied(),
    }
}

fn retain_schema_properties(
    object: &mut serde_json::Map<String, serde_json::Value>,
    schema: &serde_json::Value,
) {
    let Some(properties) = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    object.retain(|key, _| properties.contains_key(key));
}

fn repair_sdk_mcp_input_for_tool(
    input: serde_json::Value,
    schema: &serde_json::Value,
    claude_name: &str,
    roder_name: &str,
) -> serde_json::Value {
    if let Some(mut object) = input.as_object().cloned() {
        normalize_claude_tool_input_aliases(&mut object, claude_name, roder_name);
        normalize_sdk_mcp_aliases(&mut object, schema);
        retain_schema_properties(&mut object, schema);
        return serde_json::Value::Object(object);
    }
    let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) else {
        return input;
    };
    let [only_required] = required.as_slice() else {
        return input;
    };
    let Some(property) = only_required.as_str() else {
        return input;
    };
    let value = if property == "command" {
        command_value_from_sdk_mcp_input(input)
    } else {
        input
    };
    serde_json::json!({ property: value })
}

fn normalize_claude_tool_input_aliases(
    object: &mut serde_json::Map<String, serde_json::Value>,
    claude_name: &str,
    roder_name: &str,
) {
    match (claude_name, roder_name) {
        ("Bash", "shell") => move_key_if_missing(object, "command", "command"),
        ("Read", "read_file") => move_key_if_missing(object, "file_path", "path"),
        ("LS", "list_files") => move_key_if_missing(object, "path", "path"),
        ("Grep", "grep") => {
            move_key_if_missing(object, "pattern", "query");
            move_key_if_missing(object, "path", "path");
            move_key_if_missing(object, "glob", "glob");
        }
        ("Glob", "glob") => move_key_if_missing(object, "pattern", "pattern"),
        ("Write", "write_file") => move_key_if_missing(object, "file_path", "path"),
        ("Edit", "edit") | ("MultiEdit", "multi_edit") => {
            move_key_if_missing(object, "file_path", "path")
        }
        _ => {}
    }
}

fn move_key_if_missing(
    object: &mut serde_json::Map<String, serde_json::Value>,
    from: &str,
    to: &str,
) {
    if !object.contains_key(to)
        && let Some(value) = object.remove(from)
    {
        object.insert(to.to_string(), value);
    }
}

fn command_value_from_sdk_mcp_input(input: serde_json::Value) -> serde_json::Value {
    let Some(items) = input.as_array() else {
        return input;
    };
    let command = items
        .iter()
        .map(|item| match item {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    serde_json::Value::String(command)
}

fn normalize_sdk_mcp_aliases(
    object: &mut serde_json::Map<String, serde_json::Value>,
    schema: &serde_json::Value,
) {
    if schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|properties| properties.contains_key("path"))
        && !object.contains_key("path")
        && let Some(value) = object.remove("file_path")
    {
        object.insert("path".to_string(), value);
    }
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

/// Maps a Roder reasoning level to the SDK's `EffortLevel`, returning `None`
/// for unrecognized values so the `--effort` flag is simply omitted.
fn parse_effort_level(value: &str) -> Option<EffortLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" | "minimal" => Some(EffortLevel::Low),
        "medium" => Some(EffortLevel::Medium),
        "high" => Some(EffortLevel::High),
        "xhigh" | "very-high" | "veryhigh" => Some(EffortLevel::Xhigh),
        "max" | "maximum" => Some(EffortLevel::Max),
        _ => None,
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
                developer_context: None,
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
                reuse_cli_session: None,
            },
            &request,
            None,
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
        assert_eq!(options.effort.map(|effort| effort.as_cli()), Some("medium"));
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
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"],
                    "additionalProperties": false
                }),
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
            None,
        )
        .unwrap();

        assert!(options.mcp_servers.contains_key("roder"));
        assert_eq!(
            options.allowed_tools,
            vec!["mcp__roder__grep", "mcp__roder__Grep"]
        );
        let server = options.sdk_mcp_servers.get("roder").unwrap();
        let mut names = server
            .list_tools()
            .into_iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["Grep", "grep"]);

        // Built-in Claude Code tools must be disabled so the model uses the
        // Roder MCP tools instead of bare built-ins (which would be denied).
        assert!(options.tools_set);
        assert!(options.tools.is_empty());

        request.tool_choice = ToolChoice::None;
        let options =
            build_options(&ClaudeCodeConfig::default(), &request, None, None, None).unwrap();
        assert!(options.sdk_mcp_servers.is_empty());
        assert!(options.allowed_tools.is_empty());
        // With no Roder tools advertised we leave the built-in tool set alone.
        assert!(!options.tools_set);
    }

    #[test]
    fn resume_session_id_is_passed_through_to_options() {
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "claude-code".to_string(),
                model: "sonnet".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: Vec::new(),
            tools: Vec::new(),
            tool_choice: ToolChoice::None,
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
            None,
            None,
            Some("session-123"),
        )
        .unwrap();
        assert_eq!(options.resume.as_deref(), Some("session-123"));

        // Blank/whitespace ids never resume a session.
        let options = build_options(
            &ClaudeCodeConfig::default(),
            &request,
            None,
            None,
            Some("  "),
        )
        .unwrap();
        assert!(options.resume.is_none());
    }

    #[test]
    fn repairs_raw_string_sdk_mcp_input_for_single_required_property() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"],
            "additionalProperties": false
        });

        assert_eq!(
            repair_sdk_mcp_input_for_tool(
                serde_json::json!("crates/roder-ext-claude-code"),
                &schema,
                "",
                "",
            ),
            serde_json::json!({"path": "crates/roder-ext-claude-code"})
        );
        assert_eq!(
            repair_sdk_mcp_input_for_tool(
                serde_json::json!({"path": "Cargo.toml"}),
                &schema,
                "",
                "",
            ),
            serde_json::json!({"path": "Cargo.toml"})
        );
        assert_eq!(
            repair_sdk_mcp_input_for_tool(
                serde_json::json!({"file_path": "README.md"}),
                &schema,
                "",
                "",
            ),
            serde_json::json!({"path": "README.md"})
        );
    }

    #[test]
    fn repairs_array_sdk_mcp_input_for_command_property() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" }
            },
            "required": ["command"],
            "additionalProperties": false
        });

        assert_eq!(
            repair_sdk_mcp_input_for_tool(serde_json::json!(["ls", "-la"]), &schema, "", ""),
            serde_json::json!({"command": "ls -la"})
        );
    }

    #[test]
    fn maps_claude_native_alias_inputs_to_roder_arguments() {
        let grep_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "path": { "type": "string" }
            },
            "required": ["query"],
            "additionalProperties": false
        });
        assert_eq!(
            repair_sdk_mcp_input_for_tool(
                serde_json::json!({"pattern": "TokenUsage", "path": "crates", "include": "*.rs"}),
                &grep_schema,
                "Grep",
                "grep",
            ),
            serde_json::json!({"query": "TokenUsage", "path": "crates"})
        );

        let read_schema = serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
            "additionalProperties": false
        });
        assert_eq!(
            repair_sdk_mcp_input_for_tool(
                serde_json::json!({"file_path": "README.md"}),
                &read_schema,
                "Read",
                "read_file",
            ),
            serde_json::json!({"path": "README.md"})
        );
    }
}
