use roder_api::remote_runner::RemoteRunnerSession;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec};
use serde::Deserialize;
use serde_json::json;

use crate::backend::{WorkspaceBackendHandle, backend_from_context_or_fallback};
use crate::files::{parse, result};
use crate::hunk_output;
use crate::workspace::Workspace;

pub(crate) struct ApplyPatchTool {
    pub(crate) workspace: Workspace,
    pub(crate) backend: WorkspaceBackendHandle,
}

#[async_trait::async_trait]
impl ToolExecutor for ApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "apply_patch".to_string(),
            description: "Edit files using the *** Begin Patch / *** End Patch format with Add File, Delete File, Update File, and optional Move to and @@ context markers.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Patch text to apply from the workspace root."
                    }
                },
                "required": ["patch"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let patch = match patch_text_from_call(&call) {
            Ok(patch) => patch,
            Err(err) => {
                return Ok(result(
                    call,
                    format!("failed to apply patch: {err}"),
                    json!({ "error": { "kind": "invalid_arguments", "message": err.to_string() } }),
                    true,
                ));
            }
        };
        let backend = backend_from_context_or_fallback(&ctx, &self.workspace, &self.backend)?;
        let outcome = backend.apply_patch(&patch).await;

        match outcome {
            Ok(outcome) => {
                let hunks = outcome
                    .hunks()
                    .into_iter()
                    .enumerate()
                    .map(|(index, hunk)| hunk_output::from_core(&ctx, &call, index, hunk))
                    .collect::<Vec<_>>();
                Ok(result(
                    call,
                    outcome.summary,
                    json!({ "hunks": hunks, "changesExact": outcome.exact }),
                    false,
                ))
            }
            Err(err) => {
                let partial = err
                    .downcast_ref::<roder_edit_core::patch::PatchFailure>()
                    .map(|failure| &failure.partial);
                let hunks = partial
                    .map(|outcome| {
                        outcome
                            .hunks()
                            .into_iter()
                            .enumerate()
                            .map(|(index, hunk)| hunk_output::from_core(&ctx, &call, index, hunk))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let prefix = if err
                    .downcast_ref::<roder_edit_core::patch::ParseError>()
                    .is_some()
                {
                    "apply_patch verification failed"
                } else {
                    "failed to apply patch"
                };
                Ok(result(
                    call,
                    format!("{prefix}: {err}"),
                    json!({ "hunks": hunks, "changesExact": partial.is_none_or(|outcome| outcome.exact), "error": { "kind": "apply_patch_failed", "message": err.to_string() } }),
                    true,
                ))
            }
        }
    }
}

#[derive(Deserialize)]
struct ApplyPatchArgs {
    patch: String,
}

// Providers normalize both function and custom calls to the canonical patch field.
fn patch_text_from_call(call: &ToolCall) -> anyhow::Result<String> {
    Ok(parse::<ApplyPatchArgs>(call)?.patch)
}

#[cfg(test)]
fn hunk_records_from_patch(
    ctx: &ToolExecutionContext,
    call: &ToolCall,
    patch: &str,
) -> anyhow::Result<Vec<roder_api::plan_review::HunkRecord>> {
    roder_edit_core::patch::codex_patch_hunks(patch)?
        .into_iter()
        .enumerate()
        .map(|(index, hunk)| Ok(hunk_output::from_core(ctx, call, index, hunk)))
        .collect()
}

pub(crate) async fn apply_patch_to_workspace(
    workspace: &Workspace,
    patch: &str,
) -> anyhow::Result<roder_edit_core::patch::PatchOutcome> {
    roder_edit_core::patch::apply_codex_patch_to_workspace_with_external_paths(
        workspace.root(),
        patch,
        workspace.path_scope().allows_external_paths(),
    )
}

#[path = "patch_runner.rs"]
mod runner;

pub(crate) async fn apply_patch_to_runner_workspace(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    patch: &str,
) -> anyhow::Result<roder_edit_core::patch::PatchOutcome> {
    runner::apply(workspace, session, patch).await
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
