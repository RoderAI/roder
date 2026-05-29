use async_trait::async_trait;
use roder_api::extension::TaskExecutorId;
use roder_api::tasks::{TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskSpec};

use super::{WorkflowRunRequest, WorkflowRunner};

pub const WORKFLOW_TASK_EXECUTOR_ID: &str = "dynamic-workflow";

#[derive(Clone)]
pub struct WorkflowTaskExecutor {
    runner: WorkflowRunner,
}

impl WorkflowTaskExecutor {
    pub fn new(runner: WorkflowRunner) -> Self {
        Self { runner }
    }
}

#[async_trait]
impl TaskExecutor for WorkflowTaskExecutor {
    fn id(&self) -> TaskExecutorId {
        WORKFLOW_TASK_EXECUTOR_ID.to_string()
    }

    fn spec(&self) -> TaskSpec {
        TaskSpec {
            kind: "dynamic_workflow".to_string(),
            description: "Run a dynamic workflow in the background.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["runId", "script"],
                "properties": {
                    "runId": { "type": "string" },
                    "threadId": { "type": ["string", "null"] },
                    "turnId": { "type": ["string", "null"] },
                    "script": { "type": "object" },
                    "arguments": { "type": "object" },
                    "startPaused": { "type": "boolean" }
                }
            }),
            default_timeout_seconds: None,
            metadata: serde_json::json!({ "kind": "dynamic_workflow" }),
        }
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult> {
        let mut request: WorkflowRunRequest = serde_json::from_value(input)?;
        request.thread_id = request.thread_id.or(ctx.thread_id);
        request.turn_id = request.turn_id.or(ctx.turn_id);
        let handle = self.runner.start(request).await?;
        let snapshot = handle.wait().await?;
        Ok(TaskExecutionResult::success(serde_json::json!({
            "run": snapshot.run,
            "report": snapshot.report,
            "reusedAgentResults": snapshot.reused_agent_results
        })))
    }
}
