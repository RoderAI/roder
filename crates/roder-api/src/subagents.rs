use serde::{Deserialize, Serialize};

use crate::events::{ThreadId, TurnId};
use crate::extension::SubagentDispatcherId;
use crate::inference::TokenUsage;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentRequest {
    pub description: String,
    pub prompt: String,
    pub subagent_type: Option<String>,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<serde_json::Value>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentDefinition {
    pub agent_type: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub permission_mode: SubagentPermissionMode,
    pub max_turns: Option<u32>,
    pub max_result_chars: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentPermissionMode {
    ReadOnly,
    #[default]
    Default,
    AutoEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub agent_type: String,
    pub model: Option<String>,
    pub final_message: String,
    pub usage: Option<TokenUsage>,
    pub exit_reason: SubagentExitReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<serde_json::Value>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentExitReason {
    Completed,
    MaxTurns,
    Timeout,
    Cancelled,
    Failed,
}

#[async_trait::async_trait]
pub trait SubagentDispatcher: Send + Sync + 'static {
    fn id(&self) -> SubagentDispatcherId;

    fn definitions(&self) -> Vec<SubagentDefinition>;

    async fn dispatch(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct NoopDispatcher;

    #[async_trait::async_trait]
    impl SubagentDispatcher for NoopDispatcher {
        fn id(&self) -> SubagentDispatcherId {
            "noop".to_string()
        }

        fn definitions(&self) -> Vec<SubagentDefinition> {
            vec![SubagentDefinition {
                agent_type: "explore".to_string(),
                description: "Explore the workspace".to_string(),
                tools: vec!["Read".to_string()],
                model: Some("test-model".to_string()),
                system_prompt: Some("Report findings only".to_string()),
                permission_mode: SubagentPermissionMode::ReadOnly,
                max_turns: Some(4),
                max_result_chars: Some(4000),
            }]
        }

        async fn dispatch(
            &self,
            _parent_thread_id: ThreadId,
            _parent_turn_id: TurnId,
            request: SubagentRequest,
        ) -> anyhow::Result<SubagentResult> {
            Ok(SubagentResult {
                thread_id: "child-thread".to_string(),
                turn_id: "child-turn".to_string(),
                agent_type: request
                    .subagent_type
                    .unwrap_or_else(|| "explore".to_string()),
                model: request.model,
                final_message: "done".to_string(),
                usage: None,
                exit_reason: SubagentExitReason::Completed,
                transcript: None,
                metadata: serde_json::json!({}),
            })
        }
    }

    #[tokio::test]
    async fn subagent_dispatcher_trait_is_object_safe() {
        let dispatcher: Arc<dyn SubagentDispatcher> = Arc::new(NoopDispatcher);

        assert_eq!(dispatcher.id(), "noop");
        assert_eq!(dispatcher.definitions()[0].agent_type, "explore");

        let result = dispatcher
            .dispatch(
                "parent-thread".to_string(),
                "parent-turn".to_string(),
                SubagentRequest {
                    description: "Check files".to_string(),
                    prompt: "Find the API entrypoint".to_string(),
                    subagent_type: Some("explore".to_string()),
                    model: Some("test-model".to_string()),
                    tools: Some(vec!["Read".to_string()]),
                    inputs: None,
                    timeout_seconds: Some(10),
                },
            )
            .await
            .unwrap();

        assert_eq!(result.thread_id, "child-thread");
        assert_eq!(result.exit_reason, SubagentExitReason::Completed);
    }
}
