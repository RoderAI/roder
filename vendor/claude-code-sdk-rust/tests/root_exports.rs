use claude_code_sdk_rust::{
    create_sdk_mcp_server, create_sdk_mcp_server_with_version, delete_session,
    delete_session_via_store, fork_session, fork_session_via_store, get_session_info,
    get_session_info_from_store, get_session_messages, get_session_messages_from_store,
    get_subagent_messages, get_subagent_messages_from_store, initialize_server, list_sessions,
    list_sessions_from_store, list_subagents, list_subagents_from_store, query_messages,
    rename_session, rename_session_via_store, tag_session, tag_session_via_store,
    tool_with_annotations, ClaudeSDKClient, ContextUsageCategory, ContextUsageResponse,
    ForkSessionResult, ListSessionsOptions, LocalForkSessionResult, MCPToolAnnotations,
    McpServerInfo, McpServerStatus, McpStatusResponse, McpToolAnnotations, McpToolInfo,
    SDKSessionInfo, SDKSessionMessage, SdkMcpTool, SessionInfo, SessionMessage,
    SessionMutationOptions, SessionQueryOptions, SimpleMCPServer, StderrCallback, VERSION,
};

#[test]
fn session_helpers_are_exported_at_crate_root() {
    let _ = list_sessions;
    let _ = get_session_info;
    let _ = get_session_messages;
    let _ = list_subagents;
    let _ = get_subagent_messages;
    let _ = rename_session;
    let _ = tag_session;
    let _ = delete_session;
    let _ = fork_session;
    let _query_messages_future = query_messages("hello", None);
    let _server = initialize_server("server");
    let _annotated_tool = tool_with_annotations(
        "tool",
        "description",
        serde_json::json!({"type": "object"}),
        MCPToolAnnotations {
            title: None,
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
            open_world_hint: false,
            max_result_size_chars: None,
        },
        |_| Ok(Vec::<claude_code_sdk_rust::MCPContent>::new()),
    );
    let _sdk_server = create_sdk_mcp_server("sdk-server", Vec::new());
    let _versioned_sdk_server =
        create_sdk_mcp_server_with_version("sdk-server", "2.0.0", Vec::new());
    let http_config = claude_code_sdk_rust::mcp::MCPServerConfig::Http {
        url: "https://example.com/mcp".to_string(),
        headers: std::collections::HashMap::new(),
    };
    assert_eq!(serde_json::to_value(&http_config).unwrap()["type"], "http");
    let _ = list_sessions_from_store;
    let _ = get_session_info_from_store;
    let _ = get_session_messages_from_store;
    let _ = list_subagents_from_store;
    let _ = get_subagent_messages_from_store;
    let _ = rename_session_via_store;
    let _ = tag_session_via_store;
    let _ = delete_session_via_store;
    let _ = fork_session_via_store;
    let _ = std::mem::size_of::<ListSessionsOptions>();
    let _ = std::mem::size_of::<SessionQueryOptions>();
    let _ = std::mem::size_of::<SessionMutationOptions>();
    let _ = std::mem::size_of::<SessionInfo>();
    let _ = std::mem::size_of::<SessionMessage>();
    let _ = std::mem::size_of::<LocalForkSessionResult>();
    let _ = std::mem::size_of::<ForkSessionResult>();
    let _ = std::mem::size_of::<SDKSessionInfo>();
    let _ = std::mem::size_of::<SDKSessionMessage>();
    let _ = std::mem::size_of::<SimpleMCPServer>();
    let _ = std::mem::size_of::<SdkMcpTool>();
    let _ = std::mem::size_of::<StderrCallback>();
    let _ = std::mem::size_of::<ClaudeSDKClient>();
    let _ = std::mem::size_of::<McpServerStatus>();
    let _ = std::mem::size_of::<McpServerInfo>();
    let _ = std::mem::size_of::<McpStatusResponse>();
    let _ = std::mem::size_of::<McpToolInfo>();
    let status_annotations = McpToolAnnotations {
        read_only: Some(true),
        destructive: Some(false),
        open_world: Some(false),
    };
    let status_payload = serde_json::to_value(&status_annotations).unwrap();
    assert_eq!(status_payload["readOnly"], true);
    assert!(status_payload.get("read_only").is_none());
    let usage = ContextUsageResponse {
        categories: vec![ContextUsageCategory {
            name: "messages".to_string(),
            tokens: 12,
            color: "blue".to_string(),
            is_deferred: Some(false),
        }],
        total_tokens: 12,
        max_tokens: 200_000,
        raw_max_tokens: 200_000,
        percentage: 0.01,
        model: "claude-sonnet-4-5".to_string(),
        is_auto_compact_enabled: true,
        memory_files: Vec::new(),
        mcp_tools: Vec::new(),
        agents: Vec::new(),
        grid_rows: Vec::new(),
        auto_compact_threshold: Some(160_000),
        deferred_builtin_tools: Some(Vec::new()),
        system_tools: Some(Vec::new()),
        system_prompt_sections: Some(Vec::new()),
        slash_commands: Some(serde_json::json!({})),
        skills: Some(serde_json::json!({})),
    };
    assert_eq!(usage.categories[0].name, "messages");
    let payload = serde_json::to_value(&usage).unwrap();
    assert_eq!(payload["isAutoCompactEnabled"], true);
    assert_eq!(payload["deferredBuiltinTools"], serde_json::json!([]));
    assert!(!VERSION.is_empty());

    // task_updated lifecycle exports (upstream parity).
    let _ = std::mem::size_of::<claude_code_sdk_rust::TaskUpdatedMessage>();
    let _ = std::mem::size_of::<claude_code_sdk_rust::TaskUpdatedStatus>();
    assert!(claude_code_sdk_rust::is_terminal_task_status("completed"));
    assert!(!claude_code_sdk_rust::is_terminal_task_status("running"));
    assert!(claude_code_sdk_rust::TERMINAL_TASK_STATUSES.contains(&"killed"));
}
