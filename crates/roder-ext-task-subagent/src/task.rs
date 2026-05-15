use std::sync::Arc;

use anyhow::Context;
use roder_api::subagents::{SubagentDispatcher, SubagentRequest};
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
                "properties": {
                    "description": { "type": "string" },
                    "prompt": { "type": "string" },
                    "subagent_type": { "type": "string" },
                    "model": { "type": "string" },
                    "tools": { "type": "array", "items": { "type": "string" } },
                    "inputs": { "type": "object" },
                    "timeout_seconds": { "type": "integer", "minimum": 1 }
                },
                "required": ["description", "prompt"]
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
