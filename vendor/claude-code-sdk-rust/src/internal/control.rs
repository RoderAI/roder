use crate::error::{CLIConnectionError, ClaudeSDKError, Result};
use crate::internal::sdk_mcp::answer_mcp_message;
use crate::internal::transport::Transport;
use crate::types::{
    CanUseToolCallback, ClaudeAgentOptions, HookCallback, HookContext, PermissionResult,
    PermissionUpdate, SkillsConfig, ToolPermissionContext,
};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct ControlCallbacks {
    pub can_use_tool: Option<CanUseToolCallback>,
    pub sdk_mcp_servers: HashMap<String, crate::mcp::SimpleMCPServer>,
    pub hook_callbacks: HashMap<String, HookCallback>,
    pub hooks_config: Option<serde_json::Value>,
    pub agents: Option<serde_json::Value>,
    pub exclude_dynamic_sections: Option<bool>,
    pub skills: Option<Vec<String>>,
}

impl ControlCallbacks {
    pub fn from_options(options: &ClaudeAgentOptions) -> Self {
        let (hooks_config, hook_callbacks) = build_hooks_config(options);
        Self {
            can_use_tool: options.can_use_tool.clone(),
            sdk_mcp_servers: options.sdk_mcp_servers.clone(),
            hook_callbacks,
            hooks_config,
            agents: agents_config(options),
            exclude_dynamic_sections: options
                .system_prompt_preset
                .as_ref()
                .and_then(|preset| preset.exclude_dynamic_sections),
            skills: match &options.skills {
                Some(SkillsConfig::Names(skills)) => Some(skills.clone()),
                Some(SkillsConfig::All) | None => None,
            },
        }
    }
}

pub fn initialize_request(callbacks: &ControlCallbacks) -> serde_json::Value {
    let mut request = serde_json::Map::new();
    request.insert(
        "subtype".to_string(),
        serde_json::Value::String("initialize".to_string()),
    );
    request.insert(
        "hooks".to_string(),
        callbacks
            .hooks_config
            .clone()
            .unwrap_or(serde_json::Value::Null),
    );
    if let Some(agents) = &callbacks.agents {
        request.insert("agents".to_string(), agents.clone());
    }
    if let Some(exclude_dynamic_sections) = callbacks.exclude_dynamic_sections {
        request.insert(
            "excludeDynamicSections".to_string(),
            serde_json::Value::Bool(exclude_dynamic_sections),
        );
    }
    if let Some(skills) = &callbacks.skills {
        request.insert("skills".to_string(), serde_json::json!(skills));
    }

    serde_json::Value::Object(request)
}

fn agents_config(options: &ClaudeAgentOptions) -> Option<serde_json::Value> {
    if options.agents.is_empty() {
        return None;
    }

    let mut agents = serde_json::Map::new();
    let mut names: Vec<_> = options.agents.keys().cloned().collect();
    names.sort();
    for name in names {
        let Some(agent) = options.agents.get(&name) else {
            continue;
        };
        agents.insert(name, serde_json::to_value(agent).ok()?);
    }
    Some(serde_json::Value::Object(agents))
}

pub fn control_request_payload(request_id: &str, request: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    })
}

pub fn control_error_response_payload(request_id: &str, error: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": error,
        },
    })
}

pub async fn send_control_request(
    transport: &mut dyn Transport,
    request: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    send_control_request_with_callbacks(transport, request, &ControlCallbacks::default()).await
}

pub async fn send_control_request_with_callbacks(
    transport: &mut dyn Transport,
    request: serde_json::Value,
    callbacks: &ControlCallbacks,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    send_control_request_with_callbacks_and_timeout(
        transport,
        request,
        callbacks,
        Duration::from_secs(60),
    )
    .await
}

pub(crate) async fn send_control_request_with_callbacks_and_timeout(
    transport: &mut dyn Transport,
    request: serde_json::Value,
    callbacks: &ControlCallbacks,
    timeout_duration: Duration,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let request_id = format!("req_{}", uuid::Uuid::new_v4().simple());
    let subtype = request
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    match tokio::time::timeout(
        timeout_duration,
        send_control_request_with_id(transport, &request_id, request, callbacks),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(ClaudeSDKError::Other(format!(
            "Control request timeout: {subtype}"
        ))),
    }
}

pub(crate) fn initialize_timeout_duration() -> Duration {
    initialize_timeout_from_millis_env_value(
        std::env::var("CLAUDE_CODE_STREAM_CLOSE_TIMEOUT")
            .ok()
            .as_deref(),
    )
}

fn initialize_timeout_from_millis_env_value(value: Option<&str>) -> Duration {
    let millis = value
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60_000)
        .max(60_000);
    Duration::from_millis(millis)
}

async fn send_control_request_with_id(
    transport: &mut dyn Transport,
    request_id: &str,
    request: serde_json::Value,
    callbacks: &ControlCallbacks,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let subtype = request
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let payload = control_request_payload(request_id, request);
    let mut encoded = serde_json::to_vec(&payload)?;
    encoded.push(b'\n');
    transport.write(&encoded).await?;

    while let Some(data) = transport.read().await? {
        let value: serde_json::Value = serde_json::from_slice(&data)?;
        match value.get("type").and_then(|v| v.as_str()) {
            Some("control_response") => {
                if let Some(response) = matching_control_response(&value, request_id) {
                    return parse_control_response(response, &subtype);
                }
            }
            Some("control_request") => {
                respond_to_control_request(transport, &value, callbacks).await?;
            }
            _ => {}
        }
    }

    Err(CLIConnectionError::new(format!("control request ended before response: {subtype}")).into())
}

fn matching_control_response<'a>(
    value: &'a serde_json::Value,
    request_id: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    let response = value.get("response")?.as_object()?;
    let response_id = response.get("request_id")?.as_str()?;
    (response_id == request_id).then_some(response)
}

fn parse_control_response(
    response: &serde_json::Map<String, serde_json::Value>,
    subtype: &str,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    match response.get("subtype").and_then(|v| v.as_str()) {
        Some("success") => Ok(response
            .get("response")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default()),
        Some("error") => Err(ClaudeSDKError::ControlRequest {
            subtype: subtype.to_string(),
            message: response
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown control request error")
                .to_string(),
        }),
        _ => Err(ClaudeSDKError::ControlRequest {
            subtype: subtype.to_string(),
            message: "malformed control response".to_string(),
        }),
    }
}

pub(crate) async fn respond_to_control_request(
    transport: &mut dyn Transport,
    value: &serde_json::Value,
    callbacks: &ControlCallbacks,
) -> Result<()> {
    let Some(request_id) = value.get("request_id").and_then(|v| v.as_str()) else {
        return Ok(());
    };
    let request = value
        .get("request")
        .and_then(|request| request.as_object())
        .cloned()
        .unwrap_or_default();
    let subtype = value
        .get("request")
        .and_then(|request| request.get("subtype"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if subtype == "can_use_tool" {
        let response = match answer_can_use_tool(&request, callbacks).await {
            Ok(response) => control_success_response_payload(request_id, response),
            Err(error) => control_error_response_payload(request_id, &error.to_string()),
        };
        let mut encoded = serde_json::to_vec(&response)?;
        encoded.push(b'\n');
        return transport.write(&encoded).await;
    }

    if subtype == "mcp_message" {
        let response = answer_mcp_control_request(&request, callbacks);
        let mut encoded =
            serde_json::to_vec(&control_success_response_payload(request_id, response))?;
        encoded.push(b'\n');
        return transport.write(&encoded).await;
    }

    if subtype == "hook_callback" {
        let response = match answer_hook_callback(&request, callbacks).await {
            Ok(response) => control_success_response_payload(request_id, response),
            Err(error) => control_error_response_payload(request_id, &error.to_string()),
        };
        let mut encoded = serde_json::to_vec(&response)?;
        encoded.push(b'\n');
        return transport.write(&encoded).await;
    }

    let response = control_error_response_payload(
        request_id,
        &format!("Unsupported control request subtype: {subtype}"),
    );
    let mut encoded = serde_json::to_vec(&response)?;
    encoded.push(b'\n');
    transport.write(&encoded).await
}

fn build_hooks_config(
    options: &ClaudeAgentOptions,
) -> (Option<serde_json::Value>, HashMap<String, HookCallback>) {
    if options.hooks.is_empty() {
        return (None, HashMap::new());
    }

    let mut callback_index = 0usize;
    let mut hook_callbacks = HashMap::new();
    let mut config = serde_json::Map::new();
    let mut events: Vec<_> = options.hooks.keys().cloned().collect();
    events.sort();

    for event in events {
        let Some(matchers) = options.hooks.get(&event) else {
            continue;
        };
        let mut matcher_values = Vec::new();
        for matcher in matchers {
            let mut callback_ids = Vec::new();
            for callback in &matcher.hooks {
                let callback_id = format!("hook_{callback_index}");
                callback_index += 1;
                hook_callbacks.insert(callback_id.clone(), callback.clone());
                callback_ids.push(serde_json::Value::String(callback_id));
            }
            let mut matcher_value = serde_json::Map::new();
            matcher_value.insert(
                "matcher".to_string(),
                matcher
                    .matcher
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
            );
            matcher_value.insert(
                "hookCallbackIds".to_string(),
                serde_json::Value::Array(callback_ids),
            );
            if let Some(timeout) = matcher.timeout {
                matcher_value.insert("timeout".to_string(), serde_json::json!(timeout));
            }
            matcher_values.push(serde_json::Value::Object(matcher_value));
        }
        config.insert(event, serde_json::Value::Array(matcher_values));
    }

    (Some(serde_json::Value::Object(config)), hook_callbacks)
}

async fn answer_hook_callback(
    request: &serde_json::Map<String, serde_json::Value>,
    callbacks: &ControlCallbacks,
) -> Result<serde_json::Value> {
    let callback_id =
        string_field(request, "callback_id").ok_or_else(|| ClaudeSDKError::ControlRequest {
            subtype: "hook_callback".to_string(),
            message: "missing callback_id".to_string(),
        })?;
    let callback = callbacks.hook_callbacks.get(&callback_id).ok_or_else(|| {
        ClaudeSDKError::ControlRequest {
            subtype: "hook_callback".to_string(),
            message: format!("No hook callback found for ID: {callback_id}"),
        }
    })?;
    let input = request
        .get("input")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let tool_use_id = string_field(request, "tool_use_id");
    let output = callback
        .call(input, tool_use_id, HookContext::default())
        .await?;
    Ok(convert_hook_output_for_cli(output))
}

fn convert_hook_output_for_cli(output: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(map) = output else {
        return output;
    };
    let mut converted = serde_json::Map::new();
    for (key, value) in map {
        let key = match key.as_str() {
            "async_" => "async".to_string(),
            "continue_" => "continue".to_string(),
            _ => key,
        };
        converted.insert(key, value);
    }
    serde_json::Value::Object(converted)
}

fn answer_mcp_control_request(
    request: &serde_json::Map<String, serde_json::Value>,
    callbacks: &ControlCallbacks,
) -> serde_json::Value {
    let server_name = request
        .get("server_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message = request.get("message").unwrap_or(&serde_json::Value::Null);
    serde_json::json!({
        "mcp_response": answer_mcp_message(&callbacks.sdk_mcp_servers, server_name, message)
    })
}

pub fn control_success_response_payload(
    request_id: &str,
    response: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        },
    })
}

async fn answer_can_use_tool(
    request: &serde_json::Map<String, serde_json::Value>,
    callbacks: &ControlCallbacks,
) -> Result<serde_json::Value> {
    let callback =
        callbacks
            .can_use_tool
            .as_ref()
            .ok_or_else(|| ClaudeSDKError::ControlRequest {
                subtype: "can_use_tool".to_string(),
                message: "can_use_tool callback is not provided".to_string(),
            })?;
    let tool_name = request
        .get("tool_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ClaudeSDKError::ControlRequest {
            subtype: "can_use_tool".to_string(),
            message: "missing tool_name".to_string(),
        })?
        .to_string();
    let input = request
        .get("input")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let context = ToolPermissionContext {
        suggestions: permission_suggestions(request),
        tool_use_id: string_field(request, "tool_use_id"),
        agent_id: string_field(request, "agent_id"),
        blocked_path: string_field(request, "blocked_path"),
        decision_reason: string_field(request, "decision_reason"),
        title: string_field(request, "title"),
        display_name: string_field(request, "display_name"),
        description: string_field(request, "description"),
    };
    let result = callback.call(tool_name, input.clone(), context).await?;
    Ok(permission_result_response(result, input))
}

fn string_field(request: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    request.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn permission_suggestions(
    request: &serde_json::Map<String, serde_json::Value>,
) -> Vec<PermissionUpdate> {
    request
        .get("permission_suggestions")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

fn permission_result_response(
    result: PermissionResult,
    original_input: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    match result {
        PermissionResult::Allow {
            updated_input,
            updated_permissions,
        } => {
            let mut response = serde_json::Map::new();
            response.insert(
                "behavior".to_string(),
                serde_json::Value::String("allow".to_string()),
            );
            response.insert(
                "updatedInput".to_string(),
                serde_json::Value::Object(updated_input.unwrap_or(original_input)),
            );
            if let Some(updated_permissions) = updated_permissions {
                response.insert(
                    "updatedPermissions".to_string(),
                    serde_json::to_value(updated_permissions).unwrap_or(serde_json::Value::Null),
                );
            }
            serde_json::Value::Object(response)
        }
        PermissionResult::Deny { message, interrupt } => {
            let mut response = serde_json::Map::new();
            response.insert(
                "behavior".to_string(),
                serde_json::Value::String("deny".to_string()),
            );
            response.insert("message".to_string(), serde_json::Value::String(message));
            if interrupt {
                response.insert("interrupt".to_string(), serde_json::Value::Bool(true));
            }
            serde_json::Value::Object(response)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::initialize_timeout_from_millis_env_value;
    use std::time::Duration;

    #[test]
    fn initialize_timeout_defaults_to_sixty_seconds() {
        assert_eq!(
            initialize_timeout_from_millis_env_value(None),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn initialize_timeout_uses_env_millis_when_above_minimum() {
        assert_eq!(
            initialize_timeout_from_millis_env_value(Some("120000")),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn initialize_timeout_keeps_sixty_second_minimum() {
        assert_eq!(
            initialize_timeout_from_millis_env_value(Some("1000")),
            Duration::from_secs(60)
        );
    }
}
