use claude_code_sdk_rust::{query_messages, ClaudeAgentClient, ClaudeAgentOptions};

#[test]
fn can_use_tool_rejects_explicit_permission_prompt_tool_name() {
    let options = ClaudeAgentOptions::builder()
        .can_use_tool(|_, _, _| async { Ok(claude_code_sdk_rust::PermissionResult::allow()) })
        .permission_prompt_tool_name("custom")
        .build();

    let err = ClaudeAgentClient::new(options).expect_err("conflicting options should fail fast");
    assert!(err
        .to_string()
        .contains("can_use_tool callback cannot be used"));
}

#[tokio::test]
async fn query_messages_rejects_can_use_tool_with_string_prompt() {
    let options = ClaudeAgentOptions::builder()
        .can_use_tool(|_, _, _| async { Ok(claude_code_sdk_rust::PermissionResult::allow()) })
        .build();

    let err = query_messages("hello", Some(options))
        .await
        .expect_err("string prompt cannot service permission callbacks");
    assert!(err
        .to_string()
        .contains("can_use_tool callback requires streaming mode"));
}
