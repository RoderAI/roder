use async_trait::async_trait;
use claude_code_sdk_rust::internal::control::{
    control_request_payload, initialize_request, send_control_request_with_callbacks,
    ControlCallbacks,
};
use claude_code_sdk_rust::internal::transport::Transport;
use claude_code_sdk_rust::mcp::{MCPContent, MCPTool, SimpleMCPServer};
use claude_code_sdk_rust::{
    AgentDefinition, HookCallback, HookMatcher, PermissionMode, PermissionResult, Result,
    SettingSource, SkillsConfig, SystemPromptPreset,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MockState {
    writes: Vec<Vec<u8>>,
    reads: VecDeque<Vec<u8>>,
    auto_success_response: Option<serde_json::Value>,
    auto_success_sent: bool,
}

#[derive(Clone, Default)]
struct MockTransport {
    state: Arc<Mutex<MockState>>,
}

impl MockTransport {
    fn with_reads(reads: Vec<serde_json::Value>) -> Self {
        Self {
            state: Arc::new(Mutex::new(MockState {
                writes: Vec::new(),
                reads: reads
                    .into_iter()
                    .map(|value| serde_json::to_vec(&value).unwrap())
                    .collect(),
                auto_success_response: Some(serde_json::json!({})),
                auto_success_sent: false,
            })),
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.state.lock().unwrap().writes.push(data.to_vec());
        Ok(())
    }

    async fn close_input(&mut self) -> Result<()> {
        Ok(())
    }

    async fn read(&mut self) -> Result<Option<Vec<u8>>> {
        let mut state = self.state.lock().unwrap();
        if let Some(read) = state.reads.pop_front() {
            return Ok(Some(read));
        }
        if !state.auto_success_sent && !state.writes.is_empty() {
            if let Some(response) = state.auto_success_response.take() {
                let first_write: serde_json::Value = serde_json::from_slice(&state.writes[0])?;
                let request_id = first_write["request_id"].as_str().unwrap_or("");
                state.auto_success_sent = true;
                return Ok(Some(serde_json::to_vec(&serde_json::json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": request_id,
                        "response": response
                    }
                }))?));
            }
        }
        Ok(None)
    }

    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn builds_control_request_payload() {
    let payload = control_request_payload(
        "req_1",
        serde_json::json!({"subtype": "set_model", "model": "sonnet"}),
    );

    assert_eq!(payload["type"], "control_request");
    assert_eq!(payload["request_id"], "req_1");
    assert_eq!(payload["request"]["subtype"], "set_model");
}

#[test]
fn initialize_request_includes_agents_dynamic_prompt_and_skill_filter() {
    let options = claude_code_sdk_rust::ClaudeAgentOptions::builder()
        .system_prompt_preset(SystemPromptPreset {
            r#type: "preset".to_string(),
            preset: "claude_code".to_string(),
            append: None,
            exclude_dynamic_sections: Some(true),
        })
        .skills(Vec::new())
        .agent(
            "reviewer",
            AgentDefinition {
                description: "Review code".to_string(),
                prompt: "Be precise".to_string(),
                tools: Some(vec!["Read".to_string()]),
                disallowed_tools: Some(vec!["Bash".to_string()]),
                model: Some("sonnet".to_string()),
                skills: Some(vec!["rust".to_string()]),
                memory: Some(SettingSource::Project),
                mcp_servers: Some(vec![serde_json::Value::String("github".to_string())]),
                initial_prompt: Some("Start".to_string()),
                max_turns: Some(3),
                background: Some(false),
                effort: Some(serde_json::Value::String("high".to_string())),
                permission_mode: Some(PermissionMode::Plan),
            },
        )
        .build();
    let callbacks = ControlCallbacks::from_options(&options);
    let request = initialize_request(&callbacks);

    assert_eq!(request["subtype"], "initialize");
    assert_eq!(request["hooks"], serde_json::Value::Null);
    assert_eq!(request["excludeDynamicSections"], true);
    assert_eq!(request["skills"], serde_json::json!([]));
    assert_eq!(request["agents"]["reviewer"]["description"], "Review code");
    assert_eq!(request["agents"]["reviewer"]["disallowedTools"][0], "Bash");
    assert_eq!(request["agents"]["reviewer"]["mcpServers"][0], "github");
    assert_eq!(request["agents"]["reviewer"]["permissionMode"], "plan");

    let all_skills_options = claude_code_sdk_rust::ClaudeAgentOptions {
        skills: Some(SkillsConfig::All),
        ..Default::default()
    };
    let all_skills_request =
        initialize_request(&ControlCallbacks::from_options(&all_skills_options));
    assert!(all_skills_request.get("skills").is_none());
}

#[tokio::test]
async fn control_request_returns_matching_success_response() {
    let mut transport = MockTransport::with_reads(vec![serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": "req_wrong",
            "response": {"ignored": true}
        }
    })]);
    transport.state.lock().unwrap().auto_success_response = Some(serde_json::json!({"ok": true}));

    let result = send_control_request_with_callbacks(
        &mut transport,
        serde_json::json!({"subtype": "get_context_usage"}),
        &ControlCallbacks::default(),
    )
    .await
    .unwrap();

    assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn unsupported_inbound_control_request_gets_error_response() {
    let mut transport = MockTransport::with_reads(vec![serde_json::json!({
        "type": "control_request",
        "request_id": "incoming_1",
        "request": {"subtype": "hook_callback"}
    })]);

    let _ = send_control_request_with_callbacks(
        &mut transport,
        serde_json::json!({"subtype": "initialize"}),
        &ControlCallbacks::default(),
    )
    .await
    .unwrap();

    let response: serde_json::Value =
        serde_json::from_slice(&transport.state.lock().unwrap().writes[1]).unwrap();
    assert_eq!(response["type"], "control_response");
    assert_eq!(response["response"]["subtype"], "error");
    assert_eq!(response["response"]["request_id"], "incoming_1");
}

#[tokio::test]
async fn can_use_tool_control_request_can_allow() {
    let callback =
        claude_code_sdk_rust::CanUseToolCallback::new(|tool_name, input, context| async move {
            assert_eq!(tool_name, "Bash");
            assert_eq!(input.get("command").and_then(|v| v.as_str()), Some("pwd"));
            assert_eq!(context.tool_use_id.as_deref(), Some("toolu_1"));
            Ok(PermissionResult::allow())
        });
    let callbacks = ControlCallbacks {
        can_use_tool: Some(callback),
        ..ControlCallbacks::default()
    };
    let mut transport = MockTransport::with_reads(vec![serde_json::json!({
        "type": "control_request",
        "request_id": "permission_1",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "Bash",
            "input": {"command": "pwd"},
            "permission_suggestions": null,
            "blocked_path": null,
            "tool_use_id": "toolu_1"
        }
    })]);

    let _ = send_control_request_with_callbacks(
        &mut transport,
        serde_json::json!({"subtype": "initialize"}),
        &callbacks,
    )
    .await
    .unwrap();

    let response: serde_json::Value =
        serde_json::from_slice(&transport.state.lock().unwrap().writes[1]).unwrap();
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["request_id"], "permission_1");
    assert_eq!(response["response"]["response"]["behavior"], "allow");
    assert_eq!(
        response["response"]["response"]["updatedInput"]["command"],
        "pwd"
    );
}

#[tokio::test]
async fn can_use_tool_control_request_can_deny() {
    let callback = claude_code_sdk_rust::CanUseToolCallback::new(|_, _, _| async move {
        Ok(PermissionResult::Deny {
            message: "no".to_string(),
            interrupt: true,
        })
    });
    let callbacks = ControlCallbacks {
        can_use_tool: Some(callback),
        ..ControlCallbacks::default()
    };
    let mut transport = MockTransport::with_reads(vec![serde_json::json!({
        "type": "control_request",
        "request_id": "permission_1",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "Bash",
            "input": {},
            "permission_suggestions": null,
            "blocked_path": null,
            "tool_use_id": "toolu_1"
        }
    })]);

    let _ = send_control_request_with_callbacks(
        &mut transport,
        serde_json::json!({"subtype": "initialize"}),
        &callbacks,
    )
    .await
    .unwrap();

    let response: serde_json::Value =
        serde_json::from_slice(&transport.state.lock().unwrap().writes[1]).unwrap();
    assert_eq!(response["response"]["response"]["behavior"], "deny");
    assert_eq!(response["response"]["response"]["message"], "no");
    assert_eq!(response["response"]["response"]["interrupt"], true);
}

#[tokio::test]
async fn sdk_mcp_control_request_answers_tools_list() {
    let mut server = SimpleMCPServer::new("greeter");
    server.register_tool(
        MCPTool {
            name: "greet".to_string(),
            description: "Greet someone".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: None,
        },
        |_| {
            Ok(vec![MCPContent::Text {
                text: "hi".to_string(),
            }])
        },
    );
    let callbacks = ControlCallbacks {
        sdk_mcp_servers: HashMap::from([("greeter".to_string(), server)]),
        ..ControlCallbacks::default()
    };
    let mut transport = MockTransport::with_reads(vec![serde_json::json!({
        "type": "control_request",
        "request_id": "mcp_1",
        "request": {
            "subtype": "mcp_message",
            "server_name": "greeter",
            "message": {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}
        }
    })]);

    let _ = send_control_request_with_callbacks(
        &mut transport,
        serde_json::json!({"subtype": "initialize"}),
        &callbacks,
    )
    .await
    .unwrap();

    let response: serde_json::Value =
        serde_json::from_slice(&transport.state.lock().unwrap().writes[1]).unwrap();
    assert_eq!(
        response["response"]["response"]["mcp_response"]["result"]["tools"][0]["name"],
        "greet"
    );
}

#[tokio::test]
async fn hook_callback_request_invokes_registered_callback() {
    let options = claude_code_sdk_rust::ClaudeAgentOptions::builder()
        .hook(
            "PreToolUse",
            HookMatcher::new(HookCallback::new(|input, tool_use_id, _| async move {
                assert_eq!(input["tool_name"], "Bash");
                assert_eq!(tool_use_id.as_deref(), Some("toolu_1"));
                Ok(serde_json::json!({
                    "continue_": true,
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow"
                    }
                }))
            }))
            .matcher("Bash")
            .timeout(5.0),
        )
        .build();
    let callbacks = ControlCallbacks::from_options(&options);
    let hooks_config = callbacks.hooks_config.clone().unwrap();
    let callback_id = hooks_config["PreToolUse"][0]["hookCallbackIds"][0]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(hooks_config["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(hooks_config["PreToolUse"][0]["timeout"], 5.0);

    let mut transport = MockTransport::with_reads(vec![serde_json::json!({
        "type": "control_request",
        "request_id": "hook_1",
        "request": {
            "subtype": "hook_callback",
            "callback_id": callback_id,
            "input": {"tool_name": "Bash"},
            "tool_use_id": "toolu_1"
        }
    })]);

    let _ = send_control_request_with_callbacks(
        &mut transport,
        serde_json::json!({"subtype": "initialize"}),
        &callbacks,
    )
    .await
    .unwrap();

    let response: serde_json::Value =
        serde_json::from_slice(&transport.state.lock().unwrap().writes[1]).unwrap();
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["request_id"], "hook_1");
    assert_eq!(response["response"]["response"]["continue"], true);
    assert_eq!(
        response["response"]["response"]["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
}
