use futures::stream;
use roder_api::catalog::{PROVIDER_MOCK, models_for_provider};
use roder_api::conversation::ConversationItem;
use roder_api::extension::InferenceEngineId;
use roder_api::inference::*;

pub struct FakeInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for FakeInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_MOCK, true))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        if should_request_user_input(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-user-input".to_string(),
                    name: "request_user_input".to_string(),
                    arguments: serde_json::json!({
                        "questions": [{
                            "header": "Choice",
                            "id": "choice",
                            "question": "Which option should be used?",
                            "options": [
                                { "label": "A", "description": "Use option A." },
                                { "label": "B", "description": "Use option B." }
                            ]
                        }]
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_update_task_ledger(&request) {
            let complete = prompt_contains(&request, "FAKE_TASK_LEDGER_COMPLETE");
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-task-ledger".to_string(),
                    name: "task_ledger.update".to_string(),
                    arguments: task_ledger_arguments(complete),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_write_file(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-write-file".to_string(),
                    name: "write_file".to_string(),
                    arguments: serde_json::json!({
                        "path": "src/lib.rs",
                        "content": "pub fn fake() -> &'static str { \"verified\" }\n"
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_grep(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-grep".to_string(),
                    name: "grep".to_string(),
                    arguments: serde_json::json!({
                        "query": "BUG_ROOT_CAUSE_TOKEN",
                        "path": ".",
                        "mode": "indexed",
                        "limit": 20
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_discovery_read(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-discovery-read".to_string(),
                    name: "discovery.read".to_string(),
                    arguments: serde_json::json!({
                        "item_id": "tool:builtin-coding-tools/grep",
                        "promote": true,
                        "limit": 20
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_discovery_search(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-discovery-search".to_string(),
                    name: "discovery.search".to_string(),
                    arguments: serde_json::json!({
                        "query": "grep",
                        "limit": 20
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_spawn_fake_agent(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-spawn-agent".to_string(),
                    name: "spawn_agent".to_string(),
                    arguments: serde_json::json!({
                        "task_name": "reviewer",
                        "message": "review the fake agent control smoke"
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_list_fake_agents(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-list-agents".to_string(),
                    name: "list_agents".to_string(),
                    arguments: "{}".to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_message_fake_agent(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-send-message".to_string(),
                    name: "send_message".to_string(),
                    arguments: serde_json::json!({
                        "target": "reviewer",
                        "message": "add one more fake smoke detail"
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_wait_fake_agent(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-wait-agent".to_string(),
                    name: "wait_agent".to_string(),
                    arguments: serde_json::json!({
                        "target": "reviewer",
                        "timeout_ms": 1000
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_close_fake_agent(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-close-agent".to_string(),
                    name: "close_agent".to_string(),
                    arguments: serde_json::json!({
                        "target": "reviewer"
                    })
                    .to_string(),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if should_complete_verification(&request) {
            let failed = prompt_contains(&request, "FAKE_VERIFICATION_FAILED");
            let stream = stream::iter(vec![Ok(InferenceEvent::ToolCallCompleted(
                ToolCallCompleted {
                    id: "fake-verification".to_string(),
                    name: "verification_review".to_string(),
                    arguments: verification_arguments(failed),
                },
            ))]);
            return Ok(Box::pin(stream));
        }
        if verification_failed(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::Failed(InferenceFailure {
                message: "verification gaps remain: tests not run".to_string(),
            }))]);
            return Ok(Box::pin(stream));
        }
        if user_input_unavailable(&request) {
            let stream = stream::iter(vec![Ok(InferenceEvent::Failed(InferenceFailure {
                message: "clarification unavailable in non-interactive runtime profile".to_string(),
            }))]);
            return Ok(Box::pin(stream));
        }
        let stream = stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " from".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " roder".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ]);

        Ok(Box::pin(stream))
    }
}

fn should_request_user_input(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_REQUEST_USER_INPUT")
        && !request.conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result)
                    if result.name.as_deref() == Some("request_user_input")
            )
        })
}

fn user_input_unavailable(request: &AgentInferenceRequest) -> bool {
    request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::ToolResult(result)
                if result.name.as_deref() == Some("request_user_input")
                    && result.is_error
                    && result.result.contains("User input is unavailable")
        )
    })
}

fn should_update_task_ledger(request: &AgentInferenceRequest) -> bool {
    (prompt_contains(request, "FAKE_TASK_LEDGER_UPDATE")
        || prompt_contains(request, "FAKE_TASK_LEDGER_COMPLETE"))
        && !request.conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result)
                    if result.name.as_deref() == Some("task_ledger.update")
            )
        })
}

fn should_write_file(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_WRITE_FILE")
        && !request.conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result)
                    if result.name.as_deref() == Some("write_file")
            )
        })
}

fn should_grep(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_GREP_INDEXED")
        && !request.conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result) if result.name.as_deref() == Some("grep")
            )
        })
}

fn should_discovery_search(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_DISCOVERY_SEARCH")
        && !request.conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result)
                    if result.name.as_deref() == Some("discovery.search")
            )
        })
}

fn should_discovery_read(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_DISCOVERY_PROMOTE")
        && request.conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result)
                    if result.name.as_deref() == Some("discovery.search")
            )
        })
        && !request.conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result)
                    if result.name.as_deref() == Some("discovery.read")
            )
        })
}

fn should_spawn_fake_agent(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_AGENT_CONTROL_SMOKE") && !has_tool_result(request, "spawn_agent")
}

fn should_list_fake_agents(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_AGENT_CONTROL_SMOKE")
        && has_tool_result(request, "spawn_agent")
        && !has_tool_result(request, "list_agents")
}

fn should_message_fake_agent(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_AGENT_CONTROL_SMOKE")
        && has_tool_result(request, "list_agents")
        && !has_tool_result(request, "send_message")
}

fn should_wait_fake_agent(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_AGENT_CONTROL_SMOKE")
        && has_tool_result(request, "send_message")
        && !has_tool_result(request, "wait_agent")
}

fn should_close_fake_agent(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_AGENT_CONTROL_SMOKE")
        && has_tool_result(request, "wait_agent")
        && !has_tool_result(request, "close_agent")
}

fn should_complete_verification(request: &AgentInferenceRequest) -> bool {
    request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::UserMessage(message)
                if message.text.contains("Verification gate blocked final completion")
        )
    }) && !request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::ToolResult(result)
                if result.name.as_deref() == Some("verification_review")
        )
    })
}

fn verification_failed(request: &AgentInferenceRequest) -> bool {
    request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::ToolResult(result)
                if result.name.as_deref() == Some("verification_review")
                    && result.result.contains("Verification failed")
        )
    })
}

fn has_tool_result(request: &AgentInferenceRequest, name: &str) -> bool {
    request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::ToolResult(result) if result.name.as_deref() == Some(name)
        )
    })
}

fn prompt_contains(request: &AgentInferenceRequest, needle: &str) -> bool {
    request.conversation.iter().any(|item| {
        matches!(
            item,
            ConversationItem::UserMessage(message) if message.text.contains(needle)
        )
    })
}

fn task_ledger_arguments(complete: bool) -> String {
    let second_status = if complete { "completed" } else { "in_progress" };
    let mut second = serde_json::json!({
        "id": "verify",
        "content": "Verify task",
        "status": second_status
    });
    if complete {
        second["evidence"] = serde_json::json!("fake-provider");
    }
    serde_json::json!({
        "tasks": [
            { "id": "inspect", "content": "Inspect task", "status": "completed", "evidence": "fake-provider" },
            second
        ],
        "requireCompletionEvidence": true
    })
    .to_string()
}

fn verification_arguments(failed: bool) -> String {
    let (status, open_gaps) = if failed {
        ("failed", serde_json::json!(["tests not run"]))
    } else {
        ("completed", serde_json::json!([]))
    };
    serde_json::json!({
        "originalTask": "fake verification eval",
        "changedFiles": ["src/lib.rs"],
        "toolEvidence": ["write_file wrote src/lib.rs"],
        "testsRun": if failed { serde_json::json!([]) } else { serde_json::json!(["cargo test -p roder-evals verification"]) },
        "openGaps": open_gaps,
        "status": status
    })
    .to_string()
}
