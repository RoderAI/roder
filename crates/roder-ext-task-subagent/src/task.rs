use std::sync::Arc;

use anyhow::Context;
use roder_api::subagents::{SubagentDispatcher, SubagentLane, SubagentRequest};
use roder_api::tasks::{
    TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskOutputStream, TaskSpec,
};
use serde::Deserialize;

pub const SUBAGENT_TASK_EXECUTOR_ID: &str = "subagent";

#[derive(Debug, Clone, Deserialize)]
struct SubagentTaskInput {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    lane: Option<SubagentLane>,
    #[serde(default)]
    max_concurrent: Option<usize>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    parent_deadline_seconds: Option<u64>,
    #[serde(default)]
    inputs: Option<serde_json::Value>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Clone)]
pub struct SubagentTaskExecutor {
    dispatcher: Arc<dyn SubagentDispatcher>,
}

impl SubagentTaskExecutor {
    pub fn new(dispatcher: Arc<dyn SubagentDispatcher>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for SubagentTaskExecutor {
    fn id(&self) -> String {
        SUBAGENT_TASK_EXECUTOR_ID.to_string()
    }

    fn spec(&self) -> TaskSpec {
        TaskSpec {
            kind: SUBAGENT_TASK_EXECUTOR_ID.to_string(),
            description: "Run a subagent as a background task.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["description", "prompt"],
                "properties": {
                    "description": { "type": "string" },
                    "prompt": { "type": "string" },
                    "subagent_type": { "type": "string" },
                    "model": { "type": "string" },
                    "tools": { "type": "array", "items": { "type": "string" } },
                    "lane": {
                        "type": "string",
                        "enum": ["scout", "editor", "reviewer", "runner"]
                    },
                    "max_concurrent": { "type": "integer", "minimum": 1 },
                    "allowed_tools": { "type": "array", "items": { "type": "string" } },
                    "parent_deadline_seconds": { "type": "integer", "minimum": 1 },
                    "inputs": {
                        "type": "object",
                        "description": "Optional freeform structured context for the child task."
                    },
                    "timeout_seconds": { "type": "integer", "minimum": 1 }
                },
                "additionalProperties": false
            }),
            default_timeout_seconds: None,
            metadata: serde_json::json!({ "category": "subagent" }),
        }
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult> {
        let input: SubagentTaskInput =
            serde_json::from_value(input).context("deserialize subagent task input")?;
        let parent_thread_id = ctx.thread_id.unwrap_or_else(|| ctx.task_id.clone());
        let parent_turn_id = ctx.turn_id.unwrap_or_else(|| "background-task".to_string());
        let result = self
            .dispatcher
            .dispatch(
                parent_thread_id,
                parent_turn_id,
                SubagentRequest {
                    description: input.description,
                    prompt: input.prompt,
                    subagent_type: input.subagent_type,
                    model: input.model,
                    tools: input.tools,
                    lane: input.lane,
                    max_concurrent: input.max_concurrent,
                    allowed_tools: input.allowed_tools,
                    parent_deadline_seconds: input.parent_deadline_seconds,
                    inputs: input.inputs,
                    timeout_seconds: input.timeout_seconds,
                },
            )
            .await?;

        if let Some(transcript) = &result.transcript {
            ctx.output
                .write(TaskOutputStream::Log, transcript.to_string())
                .await?;
        }
        ctx.output
            .write(TaskOutputStream::Log, result.final_message.clone())
            .await?;

        Ok(TaskExecutionResult {
            exit_code: None,
            payload: serde_json::to_value(result)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use roder_api::events::{ThreadId, TurnId};
    use roder_api::subagents::{
        SubagentDefinition, SubagentExitReason, SubagentPermissionMode, SubagentResult,
    };

    use super::*;

    struct EmptyDispatcher;

    #[async_trait::async_trait]
    impl SubagentDispatcher for EmptyDispatcher {
        fn id(&self) -> String {
            "empty".to_string()
        }

        fn definitions(&self) -> Vec<SubagentDefinition> {
            vec![SubagentDefinition {
                agent_type: "explore".to_string(),
                description: "Explore the workspace".to_string(),
                tools: vec!["read_file".to_string()],
                model: None,
                system_prompt: None,
                permission_mode: SubagentPermissionMode::ReadOnly,
                max_turns: Some(2),
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
                metadata: serde_json::json!({ "lane": request.lane }),
            })
        }
    }

    #[test]
    fn speed_schema_snapshot_covers_subagent_task_input() {
        let executor = SubagentTaskExecutor::new(Arc::new(EmptyDispatcher));
        let spec = executor
            .spec()
            .normalized_for_model(roder_api::ToolSchemaPolicy::strict());
        let schema = serde_json::to_string(&spec.input_schema).unwrap();

        assert!(
            schema.starts_with(
                r#"{"type":"object","required":["description","prompt"],"properties":"#
            )
        );
        assert!(schema.contains(
            r#""inputs":{"type":"object","description":"Optional freeform structured context for the child task."}"#
        ));
        assert!(schema.contains(r#""lane":{"type":"string""#));
        assert!(schema.contains(r#""enum":["scout","editor","reviewer","runner"]"#));
        assert!(schema.contains(r#""max_concurrent":{"type":"integer""#));
        assert!(schema.contains(r#""minimum":1"#));
        assert!(schema.contains(r#""additionalProperties":false"#));
    }
}
