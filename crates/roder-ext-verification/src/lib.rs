use std::sync::Arc;

use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension, ToolProviderId,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

pub const VERIFICATION_TOOL_NAME: &str = "verification.review";

pub struct VerificationExtension;

impl RoderExtension for VerificationExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-verification".to_string(),
            name: "Verification".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Model-visible verification review tool".to_string()),
            provides: vec![ProvidedService::ToolProvider("verification".to_string())],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(VerificationToolContributor));
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct VerificationToolContributor;

impl ToolContributor for VerificationToolContributor {
    fn id(&self) -> ToolProviderId {
        "verification".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(VerificationReviewTool))
    }
}

#[derive(Debug)]
struct VerificationReviewTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerificationReviewArgs {
    original_task: String,
    #[serde(default)]
    changed_files: Vec<String>,
    #[serde(default)]
    tool_evidence: Vec<String>,
    #[serde(default)]
    tests_run: Vec<String>,
    #[serde(default)]
    open_gaps: Vec<String>,
    status: VerificationStatus,
    #[serde(default)]
    skip_reason: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum VerificationStatus {
    Completed,
    Failed,
    Skipped,
}

#[async_trait::async_trait]
impl ToolExecutor for VerificationReviewTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: VERIFICATION_TOOL_NAME.to_string(),
            description: "Review a coding turn before final completion.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "originalTask": { "type": "string" },
                    "changedFiles": { "type": "array", "items": { "type": "string" } },
                    "toolEvidence": { "type": "array", "items": { "type": "string" } },
                    "testsRun": { "type": "array", "items": { "type": "string" } },
                    "openGaps": { "type": "array", "items": { "type": "string" } },
                    "status": {
                        "type": "string",
                        "enum": ["completed", "failed", "skipped"]
                    },
                    "skipReason": { "type": "string" }
                },
                "required": ["originalTask", "status"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args: VerificationReviewArgs = serde_json::from_value(call.arguments.clone())?;
        if args.original_task.trim().is_empty() {
            return Ok(error_result(
                call,
                "verification originalTask must not be empty",
            ));
        }
        if args.status == VerificationStatus::Completed && !args.open_gaps.is_empty() {
            return Ok(error_result(
                call,
                "completed verification cannot include open gaps",
            ));
        }
        if args.status == VerificationStatus::Failed && args.open_gaps.is_empty() {
            return Ok(error_result(
                call,
                "failed verification requires at least one open gap",
            ));
        }
        if args.status == VerificationStatus::Skipped
            && args
                .skip_reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
        {
            return Ok(error_result(
                call,
                "skipped verification requires skipReason",
            ));
        }

        let status = match args.status {
            VerificationStatus::Completed => "completed",
            VerificationStatus::Failed => "failed",
            VerificationStatus::Skipped => "skipped",
        };
        let text = match status {
            "completed" => format!(
                "Verification completed: {} changed files, {} tests recorded",
                args.changed_files.len(),
                args.tests_run.len()
            ),
            "failed" => format!("Verification failed: {}", args.open_gaps.join("; ")),
            _ => format!(
                "Verification skipped: {}",
                args.skip_reason.clone().unwrap_or_default()
            ),
        };
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data: json!({
                "verification": {
                    "status": status,
                    "originalTask": args.original_task,
                    "changedFiles": args.changed_files,
                    "toolEvidence": args.tool_evidence,
                    "testsRun": args.tests_run,
                    "openGaps": args.open_gaps,
                    "skipReason": args.skip_reason
                }
            }),
            is_error: false,
        })
    }
}

fn error_result(call: ToolCall, message: impl Into<String>) -> ToolResult {
    let message = message.into();
    ToolResult {
        id: call.id,
        name: call.name,
        text: message.clone(),
        data: json!({ "error": { "kind": "verification_validation", "message": message } }),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verification_review_records_completed_evidence() {
        let contributor = VerificationToolContributor;
        let mut registry = ToolRegistry::default();
        contributor.contribute(&mut registry).unwrap();
        let tool = registry.get(VERIFICATION_TOOL_NAME).unwrap();
        let result = tool
            .execute(
                ToolExecutionContext::new(
                    "thread",
                    "turn",
                    roder_api::policy_mode::PolicyMode::Default,
                ),
                ToolCall {
                    id: "verify-1".to_string(),
                    name: VERIFICATION_TOOL_NAME.to_string(),
                    raw_arguments: "{}".to_string(),
                    arguments: json!({
                        "originalTask": "edit a file",
                        "changedFiles": ["src/lib.rs"],
                        "toolEvidence": ["write_file wrote src/lib.rs"],
                        "testsRun": ["cargo test"],
                        "openGaps": [],
                        "status": "completed"
                    }),
                    thread_id: "thread".to_string(),
                    turn_id: "turn".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["verification"]["status"], "completed");
        assert!(result.text.contains("Verification completed"));
    }

    #[tokio::test]
    async fn verification_review_rejects_failed_without_gaps() {
        let contributor = VerificationToolContributor;
        let mut registry = ToolRegistry::default();
        contributor.contribute(&mut registry).unwrap();
        let tool = registry.get(VERIFICATION_TOOL_NAME).unwrap();
        let result = tool
            .execute(
                ToolExecutionContext::new(
                    "thread",
                    "turn",
                    roder_api::policy_mode::PolicyMode::Default,
                ),
                ToolCall {
                    id: "verify-1".to_string(),
                    name: VERIFICATION_TOOL_NAME.to_string(),
                    raw_arguments: "{}".to_string(),
                    arguments: json!({
                        "originalTask": "edit a file",
                        "status": "failed"
                    }),
                    thread_id: "thread".to_string(),
                    turn_id: "turn".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.text.contains("open gap"));
    }
}
