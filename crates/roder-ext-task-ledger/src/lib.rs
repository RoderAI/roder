use std::sync::Arc;

use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension, ToolProviderId,
};
use roder_api::task_ledger::{TaskLedgerItem, TaskLedgerSnapshot};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

pub struct TaskLedgerExtension;

impl RoderExtension for TaskLedgerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-task-ledger".to_string(),
            name: "Task Ledger".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Model-visible task ledger tool".to_string()),
            provides: vec![ProvidedService::ToolProvider("task-ledger".to_string())],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(TaskLedgerToolContributor::default()));
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct TaskLedgerToolContributor {
    state: Arc<Mutex<TaskLedgerSnapshot>>,
}

impl ToolContributor for TaskLedgerToolContributor {
    fn id(&self) -> ToolProviderId {
        "task-ledger".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(TaskLedgerUpdateTool {
            state: self.state.clone(),
        }))
    }
}

#[derive(Debug)]
struct TaskLedgerUpdateTool {
    state: Arc<Mutex<TaskLedgerSnapshot>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskLedgerUpdateArgs {
    tasks: Vec<TaskLedgerItem>,
    #[serde(default)]
    require_completion_evidence: bool,
}

#[async_trait::async_trait]
impl ToolExecutor for TaskLedgerUpdateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "task_ledger.update".to_string(),
            description: "Update the durable task ledger for decomposed work.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "content": { "type": "string" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed", "blocked"]
                                },
                                "evidence": { "type": "string" }
                            },
                            "required": ["id", "content", "status"],
                            "additionalProperties": false
                        }
                    },
                    "requireCompletionEvidence": { "type": "boolean" }
                },
                "required": ["tasks"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: roder_api::tools::ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: TaskLedgerUpdateArgs = serde_json::from_value(call.arguments.clone())?;
        let snapshot = TaskLedgerSnapshot { tasks: args.tasks };
        if let Err(err) = snapshot.validate(args.require_completion_evidence) {
            return Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: err.to_string(),
                data: json!({ "error": { "kind": "task_ledger_validation", "message": err.to_string() } }),
                is_error: true,
            });
        }
        *self.state.lock().await = snapshot.clone();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format_task_ledger(&snapshot),
            data: json!({
                "taskLedger": snapshot,
            }),
            is_error: false,
        })
    }
}

fn format_task_ledger(snapshot: &TaskLedgerSnapshot) -> String {
    let mut lines = vec![format!(
        "Task ledger: {}/{} completed",
        snapshot.completed_count(),
        snapshot.tasks.len()
    )];
    for task in &snapshot.tasks {
        let evidence = task
            .evidence
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!(" evidence: {value}"))
            .unwrap_or_default();
        lines.push(format!(
            "- {}: {} [{}]{}",
            task.status.as_str(),
            task.content,
            task.id,
            evidence
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::task_ledger::TaskLedgerStatus;
    use roder_api::tools::ToolExecutionContext;

    #[tokio::test]
    async fn task_ledger_update_validates_and_returns_panel_friendly_text() {
        let contributor = TaskLedgerToolContributor::default();
        let mut registry = ToolRegistry::default();
        contributor.contribute(&mut registry).unwrap();
        let tool = registry.get("task_ledger.update").unwrap();
        let result = tool
            .execute(
                ToolExecutionContext::new(
                    "thread".to_string(),
                    "turn".to_string(),
                    roder_api::policy_mode::PolicyMode::Default,
                ),
                ToolCall {
                    id: "ledger-1".to_string(),
                    name: "task_ledger.update".to_string(),
                    raw_arguments: "{}".to_string(),
                    arguments: json!({
                        "tasks": [
                            { "id": "inspect", "content": "Inspect", "status": "completed", "evidence": "tests" },
                            { "id": "verify", "content": "Verify", "status": "in_progress" }
                        ],
                        "requireCompletionEvidence": true
                    }),
                    thread_id: "thread".to_string(),
                    turn_id: "turn".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.text.contains("- completed: Inspect [inspect]"));
        let snapshot: TaskLedgerSnapshot =
            serde_json::from_value(result.data["taskLedger"].clone()).unwrap();
        assert_eq!(snapshot.tasks[0].status, TaskLedgerStatus::Completed);
    }

    #[tokio::test]
    async fn task_ledger_update_rejects_completed_without_evidence_when_required() {
        let contributor = TaskLedgerToolContributor::default();
        let mut registry = ToolRegistry::default();
        contributor.contribute(&mut registry).unwrap();
        let tool = registry.get("task_ledger.update").unwrap();
        let result = tool
            .execute(
                ToolExecutionContext::new(
                    "thread".to_string(),
                    "turn".to_string(),
                    roder_api::policy_mode::PolicyMode::Default,
                ),
                ToolCall {
                    id: "ledger-1".to_string(),
                    name: "task_ledger.update".to_string(),
                    raw_arguments: "{}".to_string(),
                    arguments: json!({
                        "tasks": [
                            { "id": "done", "content": "Done", "status": "completed" }
                        ],
                        "requireCompletionEvidence": true
                    }),
                    thread_id: "thread".to_string(),
                    turn_id: "turn".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.text.contains("requires evidence"));
    }
}
