use super::cli_args::build_cli_args;
use super::transport::TransportOptions;
use crate::types::{
    ClaudeAgentOptions, PermissionMode, SandboxNetworkConfig, SandboxSettings, SdkBeta,
    ThinkingConfig, ThinkingConfigType,
};

#[test]
fn empty_tools_list_serializes_to_disable_all_tools_like_python_sdk() {
    let args = args_for(ClaudeAgentOptions::builder().tools(Vec::new()).build());

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--tools")
            .map(|window| window[1].as_str()),
        Some("")
    );
}

fn args_for(options: ClaudeAgentOptions) -> Vec<String> {
    build_cli_args(&TransportOptions::from(&options)).expect("args")
}

#[test]
fn serializes_permission_mode_and_betas_as_cli_wire_values() {
    let options = ClaudeAgentOptions::builder()
        .permission_mode(PermissionMode::AcceptEdits)
        .betas(vec![SdkBeta::Context1M20250807])
        .build();

    let args = args_for(options);

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--permission-mode")
            .map(|window| window[1].as_str()),
        Some("acceptEdits")
    );
    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--betas")
            .map(|window| window[1].as_str()),
        Some("context-1m-2025-08-07")
    );
}

#[test]
fn serializes_thinking_and_json_schema_like_python_sdk() {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".to_string()),
    );

    let mut output_format = serde_json::Map::new();
    output_format.insert(
        "type".to_string(),
        serde_json::Value::String("json_schema".to_string()),
    );
    output_format.insert(
        "schema".to_string(),
        serde_json::Value::Object(schema.clone()),
    );
    let expected_schema = serde_json::Value::Object(schema).to_string();

    let options = ClaudeAgentOptions::builder()
        .thinking(ThinkingConfig {
            r#type: ThinkingConfigType::Adaptive,
            budget_tokens: None,
            display: Some("full".to_string()),
        })
        .output_format(output_format)
        .build();

    let args = args_for(options);

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--thinking")
            .map(|window| window[1].as_str()),
        Some("adaptive")
    );
    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--thinking-display")
            .map(|window| window[1].as_str()),
        Some("full")
    );
    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--json-schema")
            .map(|window| window[1].as_str()),
        Some(expected_schema.as_str())
    );
}

#[test]
fn disabled_thinking_does_not_forward_display() {
    let options = ClaudeAgentOptions::builder()
        .thinking(ThinkingConfig {
            r#type: ThinkingConfigType::Disabled,
            budget_tokens: None,
            display: Some("omitted".to_string()),
        })
        .build();

    let args = args_for(options);

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--thinking")
            .map(|window| window[1].as_str()),
        Some("disabled")
    );
    assert!(!args.iter().any(|arg| arg == "--thinking-display"));
}

#[test]
fn serializes_current_python_sdk_session_and_control_flags() {
    let options = ClaudeAgentOptions::builder()
        .session_id("session-123")
        .task_budget_total(100_000)
        .include_hook_events(true)
        .strict_mcp_config(true)
        .setting_sources(vec![
            crate::types::SettingSource::User,
            crate::types::SettingSource::Project,
        ])
        .build();

    let args = args_for(options);

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--session-id")
            .map(|window| window[1].as_str()),
        Some("session-123")
    );
    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--task-budget")
            .map(|window| window[1].as_str()),
        Some("100000")
    );
    assert!(args.iter().any(|arg| arg == "--include-hook-events"));
    assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
    assert!(args
        .iter()
        .any(|arg| arg == "--setting-sources=user,project"));
}

#[test]
fn can_use_tool_defaults_permission_prompt_tool_to_stdio() {
    let options = ClaudeAgentOptions::builder()
        .can_use_tool(|_, _, _| async { Ok(crate::types::PermissionResult::allow()) })
        .build();

    let args = args_for(options);

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--permission-prompt-tool")
            .map(|window| window[1].as_str()),
        Some("stdio")
    );
}

#[test]
fn serializes_mcp_servers_raw_config_like_python_sdk() {
    let json_config = r#"{"mcpServers":{"server":{"type":"stdio","command":"test"}}}"#;
    let args = args_for(
        ClaudeAgentOptions::builder()
            .mcp_servers_config(json_config)
            .build(),
    );

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--mcp-config")
            .map(|window| window[1].as_str()),
        Some(json_config)
    );

    let path = "/path/to/mcp-config.json";
    let args = args_for(
        ClaudeAgentOptions::builder()
            .mcp_servers_config(path)
            .build(),
    );
    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--mcp-config")
            .map(|window| window[1].as_str()),
        Some(path)
    );
}

#[test]
fn skills_config_adds_skill_tool_and_default_setting_sources() {
    let options = ClaudeAgentOptions::builder()
        .allowed_tools(vec!["Read".to_string()])
        .skills(vec!["reviewer".to_string(), "planner".to_string()])
        .build();

    let args = args_for(options);

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--allowedTools")
            .map(|window| window[1].as_str()),
        Some("Read,Skill(reviewer),Skill(planner)")
    );
    assert!(args
        .iter()
        .any(|arg| arg == "--setting-sources=user,project"));
}

#[test]
fn skills_all_deduplicates_skill_tool_and_respects_explicit_setting_sources() {
    let options = ClaudeAgentOptions::builder()
        .allowed_tools(vec!["Skill".to_string()])
        .skills_all()
        .setting_sources(Vec::new())
        .build();

    let args = args_for(options);

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--allowedTools")
            .map(|window| window[1].as_str()),
        Some("Skill")
    );
    assert!(args.iter().any(|arg| arg == "--setting-sources="));
}

#[test]
fn sandbox_settings_are_merged_into_settings_json() {
    let options = ClaudeAgentOptions::builder()
        .settings(r#"{"permissions":{"allow":["Read"]}}"#)
        .sandbox(SandboxSettings {
            enabled: Some(true),
            auto_allow_bash_if_sandboxed: Some(true),
            network: Some(SandboxNetworkConfig {
                allow_local_binding: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        })
        .build();

    let args = args_for(options);
    let settings = args
        .windows(2)
        .find(|window| window[0] == "--settings")
        .map(|window| window[1].as_str())
        .expect("settings argument");
    let settings: serde_json::Value = serde_json::from_str(settings).expect("settings json");

    assert_eq!(settings["permissions"]["allow"][0], "Read");
    assert_eq!(settings["sandbox"]["enabled"], true);
    assert_eq!(
        settings["sandbox"]["autoAllowBashIfSandboxed"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(settings["sandbox"]["network"]["allowLocalBinding"], true);
}

#[test]
fn unsupported_plugin_type_is_rejected() {
    let options = ClaudeAgentOptions::builder()
        .plugin(crate::types::SDKPluginConfig {
            r#type: "remote".to_string(),
            path: "/tmp/plugin".to_string(),
        })
        .build();

    let err = build_cli_args(&TransportOptions::from(&options))
        .expect_err("unsupported plugin type should fail");
    assert!(err.to_string().contains("Unsupported plugin type"));
}

#[test]
fn session_store_enables_session_mirror_flag() {
    let options = ClaudeAgentOptions::builder()
        .session_store(crate::session_store::InMemorySessionStore::new())
        .build();

    let args = args_for(options);

    assert!(args.iter().any(|arg| arg == "--session-mirror"));
}

#[test]
fn serializes_effort_level_as_cli_wire_value() {
    use crate::types::EffortLevel;

    let args = args_for(
        ClaudeAgentOptions::builder()
            .effort(EffortLevel::Xhigh)
            .build(),
    );

    assert_eq!(
        args.windows(2)
            .find(|window| window[0] == "--effort")
            .map(|window| window[1].as_str()),
        Some("xhigh")
    );
}
