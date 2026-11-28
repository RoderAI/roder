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
        .include_partial_messages(true)
        // Only load the MCP servers Roder passes in. Without this the CLI also
        // loads the user's own MCP servers and claude.ai connectors. Roder's
        // `can_use_tool` callback denies those tools, but their schemas still
        // go into every request: about 160K tokens for a one-line prompt with
        // a set of connectors. Claude-in-Chrome is wired by the CLI itself and
        // still connects under strict mode.
        .strict_mcp_config(true);
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
    // When the Claude-in-Chrome integration is enabled, force the CLI to wire
    // its browser MCP server even in the SDK's headless/streaming mode and blank
    // API-key auth so the CLI falls back to claude.ai subscription auth -- the
    // Chrome integration only connects under subscription auth, and an inherited
    // `ANTHROPIC_API_KEY` would otherwise take precedence and disable it.
    let chrome_enabled = claude_in_chrome_enabled(config);
    if chrome_enabled {
        builder = builder
            .env_var("CLAUDE_CODE_ENABLE_CFC", "1")
            .env_var("ANTHROPIC_API_KEY", "")
            .env_var("ANTHROPIC_AUTH_TOKEN", "");
    }
    let allowed_tool_names = allowed_claude_tool_names(request);
    if !request.tools.is_empty() && !matches!(request.tool_choice, ToolChoice::None) {
        // Register the Roder server only when some Roder tool survives the
        // Claude Code filter.
        if !allowed_tool_names.is_empty() {
            let executor = tool_executor.ok_or_else(|| {
                anyhow::anyhow!("Claude Code provider requires a Roder tool executor")
            })?;
            let server = roder_sdk_mcp_server(request, executor);
            builder = builder.sdk_mcp_server("roder", server);
        }
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
            // Let the CLI execute its own Claude-in-Chrome browser tools when the
            // integration is enabled; Roder does not mediate these (they drive
            // the user's real browser through the CLI's native host).
            if chrome_enabled && is_claude_in_chrome_tool(&tool_name) {
                return Ok(PermissionResult::allow());
            }
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

/// Tool-name prefixes for the Claude Code "Claude in Chrome" integration. The
/// CLI exposes the browser surface under two server spellings (the Chrome
/// extension and the desktop app), so both are recognized.
const CLAUDE_IN_CHROME_TOOL_PREFIXES: [&str; 2] =
    ["mcp__claude-in-chrome__", "mcp__Claude_in_Chrome__"];

/// Whether `name` is a Claude-in-Chrome browser tool the CLI executes itself.
fn is_claude_in_chrome_tool(name: &str) -> bool {
    CLAUDE_IN_CHROME_TOOL_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Resolve whether the Claude-in-Chrome integration should be enabled for this
/// turn: explicit config wins, then the `RODER_CLAUDE_CODE_ENABLE_CHROME` /
/// `CLAUDE_CODE_ENABLE_CHROME` env overrides, then auto-detection of a local
/// Claude-in-Chrome setup.
fn claude_in_chrome_enabled(config: &ClaudeCodeConfig) -> bool {
    if let Some(explicit) = config.enable_claude_in_chrome {
        return explicit;
    }
    if let Some(value) = env_flag("RODER_CLAUDE_CODE_ENABLE_CHROME")
        .or_else(|| env_flag("CLAUDE_CODE_ENABLE_CHROME"))
    {
        return value;
    }
    detect_claude_in_chrome_setup()
}

/// Parse a boolean-ish environment flag. Returns `None` when unset or
/// unrecognized so callers can fall back to the next signal.
fn env_flag(key: &str) -> Option<bool> {
    let raw = std::env::var(key).ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" | "" => Some(false),
        _ => None,
    }
}

/// Best-effort detection of a local Claude-in-Chrome setup by inspecting the
/// Claude Code config file (`$CLAUDE_CONFIG_DIR/.claude.json` or
/// `~/.claude.json`). Returns false when the file is missing or unreadable.
fn detect_claude_in_chrome_setup() -> bool {
    let Some(path) = claude_config_path() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let default_enabled = value
        .get("claudeInChromeDefaultEnabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let paired = value
        .get("chromeExtension")
        .and_then(|ext| ext.get("pairedDeviceId"))
        .map(|id| !id.is_null())
        .unwrap_or(false);
    let installed = value
        .get("cachedChromeExtensionInstalled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    default_enabled || paired || installed
}

/// Path to the Claude Code config JSON, honoring `CLAUDE_CONFIG_DIR`.
fn claude_config_path() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir).join(".claude.json"));
    }
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".claude.json"))
}

/// Roder tools that are not offered to Claude Code. Every advertised tool
/// schema is sent on every request, so each extra tool costs tokens on every
/// turn.
///
/// - `exec_command`, `unified_exec` and `write_stdin` are Codex-style PTY shells.
///   Claude uses `Bash`.
/// - `echo` and `media_generate_video` are test fixtures.
///
/// `apply_patch` (the Codex patch format) is handled separately: it is hidden
/// only when `edit` is also offered, because in Roder's `patch` edit mode it is
/// the only editing tool.
const TOOLS_HIDDEN_FROM_CLAUDE: [&str; 5] = [
    "exec_command",
    "unified_exec",
    "write_stdin",
    "echo",
    "media_generate_video",
];

/// Prefixes of Roder tool families that are not offered to Claude Code.
/// Claude Code already has its own browser through Claude-in-Chrome, so the
/// Roder browser stacks (`chrome_*`, `jev_*`, `browser_use_*`, `webwright.*`)
/// would be extra copies. The other families are specialist tools.
const TOOL_PREFIXES_HIDDEN_FROM_CLAUDE: [&str; 8] = [
    "chrome_",
    "jev_",
    "browser_use_",
    "zerolang_",
    "design_",
    "knowledge_",
    "webwright.",
    "memory_",
];

/// Whether a Roder tool is offered to Claude Code.
fn exposed_to_claude(name: &str) -> bool {
    !TOOLS_HIDDEN_FROM_CLAUDE.contains(&name)
        && !TOOL_PREFIXES_HIDDEN_FROM_CLAUDE
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn claude_tool_specs(
    request: &AgentInferenceRequest,
) -> impl Iterator<Item = &roder_api::tools::ToolSpec> {
    let has_edit = request.tools.iter().any(|tool| tool.name == "edit");
    request.tools.iter().filter(move |tool| {
        exposed_to_claude(&tool.name) && !(has_edit && tool.name == "apply_patch")
    })
}

fn allowed_claude_tool_names(request: &AgentInferenceRequest) -> Vec<String> {
    claude_tool_specs(request)
        .map(|tool| claude_name_for_roder_tool(&tool.name).to_string())
        .collect()
}

fn roder_sdk_mcp_server(
    request: &AgentInferenceRequest,
    executor: Arc<dyn TurnToolExecutor>,
) -> SimpleMCPServer {
    let tools = claude_tool_specs(request)
        .map(|spec| {
            sdk_tool_for_spec(
                claude_name_for_roder_tool(&spec.name).to_string(),
                spec.name.clone(),
                spec.description.clone(),
                spec.parameters.clone(),
                Arc::clone(&executor),
            )
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
    let handler_name = claude_name.clone();
    SdkMcpTool::new_async(claude_name, description, schema, None, move |input| {
        let input = repair_sdk_mcp_input_for_tool(input, &call_schema, &handler_name, &roder_name);
        // Each invocation must carry a unique tool-call id. The TUI and
        // runtime key tool-call rows by id, so reusing a name-derived id
        // (e.g. `claude-code-Bash`) collapses every later call of the same
        // tool into the first row and only the first one ever renders.
        let seq = NEXT_TOOL_CALL_SEQ.fetch_add(1, Ordering::Relaxed);
        let call = ToolCallCompleted {
            id: format!("claude-code-{handler_name}-{seq}"),
            name: roder_name.clone(),
            arguments: input.to_string(),
        };
        let executor = Arc::clone(&executor);
        async move {
            match executor.execute(call).await {
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
        }
    })
    // Claude calls the aliases with its native argument shapes (`file_path`,
    // argv arrays), which `repair_sdk_mcp_input_for_tool` maps onto Roder's
    // schema, so the SDK must not reject them against that schema first.
    .validate_input(false)
}

/// The name Claude Code sees for a Roder tool. Core tools use Claude Code's
/// native names, which the model is trained on. Calls still run the Roder tool.
fn claude_name_for_roder_tool(name: &str) -> &str {
    match name {
        "shell" => "Bash",
        "read_file" => "Read",
        "list_files" => "LS",
        "grep" => "Grep",
        "glob" => "Glob",
        "write_file" => "Write",
        "edit" => "Edit",
        "multi_edit" => "MultiEdit",
        other => other,
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
    // Core adds browser routing guidance for Roder's browser tool families.
    // Claude Code is offered none of them (it has Claude-in-Chrome), so the
    // guidance would name tools the model cannot call.
    let developer = request
        .instructions
        .developer
        .as_deref()
        .map(roder_api::browser_routing::strip_browser_routing)
        .filter(|text| !text.trim().is_empty());
    match (request.instructions.system.as_deref(), developer.as_deref()) {
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
                enable_claude_in_chrome: Some(false),
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
                data: serde_json::Value::Null,
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
        assert_eq!(options.allowed_tools, vec!["mcp__roder__Grep"]);
        let server = options.sdk_mcp_servers.get("roder").unwrap();
        let mut names = server
            .list_tools()
            .into_iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["Grep"]);

        // Built-in Claude Code tools must be disabled so the model uses the
        // Roder MCP tools instead of bare built-ins (which would be denied).
        assert!(options.tools_set);
        assert!(options.tools.is_empty());
        // Only the Roder MCP server is loaded, not the user's own servers.
        assert!(options.strict_mcp_config);

        request.tool_choice = ToolChoice::None;
        let options =
            build_options(&ClaudeCodeConfig::default(), &request, None, None, None).unwrap();
        assert!(options.sdk_mcp_servers.is_empty());
        assert!(options.allowed_tools.is_empty());
        // With no Roder tools advertised we leave the built-in tool set alone.
        assert!(!options.tools_set);
        assert!(options.strict_mcp_config);
    }

    fn options_for_tools(names: &[&str]) -> ClaudeAgentOptions {
        let mut request = chrome_request();
        request.tool_choice = ToolChoice::Auto;
        request.tools = names
            .iter()
            .map(|name| ToolSpec {
                name: name.to_string(),
                description: format!("{name} tool"),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            })
            .collect();
        build_options(
            &ClaudeCodeConfig::default(),
            &request,
            Some(Arc::new(FakeToolExecutor)),
            None,
            None,
        )
        .unwrap()
    }

    fn roder_server_tool_names(options: &ClaudeAgentOptions) -> Vec<String> {
        let mut names = options.sdk_mcp_servers["roder"]
            .list_tools()
            .into_iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn offers_claude_code_one_copy_of_each_needed_tool() {
        let options = options_for_tools(&[
            "read_file",
            "edit",
            "spawn_agent",
            "webwright_helper",
            "exec_command",
            "unified_exec",
            "write_stdin",
            "apply_patch",
            "echo",
            "media_generate_video",
            "chrome_click",
            "jev_browse",
            "browser_use_navigate",
            "zerolang_edit",
            "design_patch",
            "knowledge_search",
            "webwright.run_script",
            "memory_query",
        ]);

        assert_eq!(
            options.allowed_tools,
            vec![
                "mcp__roder__Read",
                "mcp__roder__Edit",
                "mcp__roder__spawn_agent",
                "mcp__roder__webwright_helper"
            ]
        );
        assert_eq!(
            roder_server_tool_names(&options),
            vec!["Edit", "Read", "spawn_agent", "webwright_helper"]
        );
    }

    #[test]
    fn keeps_apply_patch_when_it_is_the_only_editing_tool() {
        // Roder's `patch` edit mode offers `apply_patch` without `edit`.
        let options = options_for_tools(&["read_file", "apply_patch"]);
        assert_eq!(
            roder_server_tool_names(&options),
            vec!["Read", "apply_patch"]
        );
    }

    #[test]
    fn built_in_tools_stay_disabled_when_every_roder_tool_is_hidden() {
        let options = options_for_tools(&["chrome_click", "jev_browse"]);
        assert!(options.tools_set);
        assert!(options.tools.is_empty());
        assert!(options.allowed_tools.is_empty());
        assert!(options.sdk_mcp_servers.is_empty());
    }

    #[test]
    fn system_prompt_drops_browser_routing_for_hidden_browser_tools() {
        let mut request = chrome_request();
        request.instructions.system = Some("System.".to_string());
        request.instructions.developer = Some("Developer.".to_string());
        request.instructions = roder_api::browser_routing::apply_browser_routing(
            request.instructions,
            ["jev_browse", "chrome_click", "read_file"],
        );
        assert!(
            request
                .instructions
                .developer
                .as_deref()
                .is_some_and(|text| text.contains("## Browser Tool Routing"))
        );
        assert_eq!(
            merged_system_prompt(&request).as_deref(),
            Some("System.\n\nDeveloper.")
        );
    }

    fn chrome_request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "claude-code".to_string(),
                model: "opus".to_string(),
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
        }
    }

    #[test]
    fn classifies_claude_in_chrome_tools() {
        assert!(is_claude_in_chrome_tool("mcp__claude-in-chrome__navigate"));
        assert!(is_claude_in_chrome_tool("mcp__Claude_in_Chrome__read_page"));
        assert!(!is_claude_in_chrome_tool("mcp__roder__shell"));
        assert!(!is_claude_in_chrome_tool("Bash"));
    }

    #[test]
    fn chrome_disabled_sets_no_chrome_env() {
        let options = build_options(
            &ClaudeCodeConfig {
                enable_claude_in_chrome: Some(false),
                ..ClaudeCodeConfig::default()
            },
            &chrome_request(),
            None,
            None,
            None,
        )
        .unwrap();
        assert!(!options.env.contains_key("CLAUDE_CODE_ENABLE_CFC"));
        assert!(!options.env.contains_key("ANTHROPIC_API_KEY"));
        assert!(!options.env.contains_key("ANTHROPIC_AUTH_TOKEN"));
    }

    #[tokio::test]
    async fn chrome_enabled_wires_cfc_env_and_authorizes_browser_tools() {
        let options = build_options(
            &ClaudeCodeConfig {
                enable_claude_in_chrome: Some(true),
                ..ClaudeCodeConfig::default()
            },
            &chrome_request(),
            None,
            None,
            None,
        )
        .unwrap();
        // The CLI is told to wire the Claude-in-Chrome MCP server and to ignore
        // API-key auth so it falls back to subscription auth.
        assert_eq!(
            options
                .env
                .get("CLAUDE_CODE_ENABLE_CFC")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            options.env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("")
        );
        assert_eq!(
            options.env.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str),
            Some("")
        );

        let can_use_tool = options.can_use_tool.expect("permission callback set");
        let allowed = can_use_tool
            .call(
                "mcp__claude-in-chrome__navigate".to_string(),
                serde_json::Map::new(),
                claude_code_sdk_rust::ToolPermissionContext::default(),
            )
            .await
            .unwrap();
        assert!(matches!(allowed, PermissionResult::Allow { .. }));
    }

    #[tokio::test]
    async fn chrome_disabled_callback_denies_browser_tools() {
        let options = build_options(
            &ClaudeCodeConfig {
                enable_claude_in_chrome: Some(false),
                ..ClaudeCodeConfig::default()
            },
            &chrome_request(),
            None,
            None,
            None,
        )
        .unwrap();
        let can_use_tool = options.can_use_tool.expect("permission callback set");
        let denied = can_use_tool
            .call(
                "mcp__claude-in-chrome__navigate".to_string(),
                serde_json::Map::new(),
                claude_code_sdk_rust::ToolPermissionContext::default(),
            )
            .await
            .unwrap();
        assert!(matches!(denied, PermissionResult::Deny { .. }));
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
