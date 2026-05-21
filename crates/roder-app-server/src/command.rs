use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use roder_api::artifacts::{ContextArtifact, ContextArtifactKind, format_artifact_reference};
use roder_api::context::PolicyGate;
use roder_api::events::{
    ContextArtifactAppended, ContextArtifactCapped, ContextArtifactCreated, RoderEvent,
};
use roder_api::policy_mode::PolicyDecision;
use roder_api::tools::{ToolCall, ToolExecutionContext};
use roder_core::artifacts::CreateArtifactRequest;
use roder_core::policy_gate::DefaultPolicyGate;
use roder_protocol::{
    CommandExecOutputDeltaNotification, CommandExecParams, CommandExecResponse, JsonRpcError,
    JsonRpcNotification,
};
use tokio::process::Command;

use crate::AppServer;

impl AppServer {
    pub(crate) async fn handle_command_exec(
        &self,
        params: CommandExecParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        validate_command_exec_params(&params)?;
        self.enforce_command_exec_policy(&params).await?;
        let mut command = Command::new(&params.command[0]);
        command.args(&params.command[1..]);
        if let Some(cwd) = params.cwd.as_deref() {
            command.current_dir(absolute_path(cwd)?);
        }
        if let Some(env) = params.env.as_ref() {
            for (key, value) in env {
                match value {
                    Some(value) => {
                        command.env(key, value);
                    }
                    None => {
                        command.env_remove(key);
                    }
                }
            }
        }

        let output_future = command.output();
        let output = if params.disable_timeout {
            output_future.await.map_err(internal_error)?
        } else {
            let timeout_ms = params.timeout_ms.unwrap_or(30_000);
            tokio::time::timeout(Duration::from_millis(timeout_ms), output_future)
                .await
                .map_err(|_| JsonRpcError {
                    code: -32000,
                    message: format!("command timed out after {timeout_ms}ms"),
                    data: None,
                })?
                .map_err(internal_error)?
        };

        let (stdout, stdout_truncated) = cap_output(&output.stdout, &params);
        let (stderr, stderr_truncated) = cap_output(&output.stderr, &params);
        let process_id = params
            .process_id
            .clone()
            .unwrap_or_else(|| format!("command-{}", uuid::Uuid::new_v4()));
        let stdout_artifact = self
            .write_command_artifact_if_capped(
                &process_id,
                "stdout",
                ContextArtifactKind::CommandStdout,
                &output.stdout,
                stdout.len(),
                stdout_truncated,
            )
            .await?;
        let stderr_artifact = self
            .write_command_artifact_if_capped(
                &process_id,
                "stderr",
                ContextArtifactKind::CommandStderr,
                &output.stderr,
                stderr.len(),
                stderr_truncated,
            )
            .await?;
        if params.stream_stdout_stderr {
            self.emit_command_output_delta(&process_id, "stdout", &stdout, stdout_truncated);
            self.emit_command_output_delta(&process_id, "stderr", &stderr, stderr_truncated);
            Ok(serde_json::to_value(CommandExecResponse {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: String::new(),
                stderr: String::new(),
                stdout_artifact: stdout_artifact.map(|artifact| artifact.descriptor()),
                stderr_artifact: stderr_artifact.map(|artifact| artifact.descriptor()),
            })
            .unwrap())
        } else {
            let stdout_text = command_output_text(&stdout, stdout_artifact.as_ref(), "stdout");
            let stderr_text = command_output_text(&stderr, stderr_artifact.as_ref(), "stderr");
            Ok(serde_json::to_value(CommandExecResponse {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: stdout_text,
                stderr: stderr_text,
                stdout_artifact: stdout_artifact.map(|artifact| artifact.descriptor()),
                stderr_artifact: stderr_artifact.map(|artifact| artifact.descriptor()),
            })
            .unwrap())
        }
    }

    async fn write_command_artifact_if_capped(
        &self,
        process_id: &str,
        label: &str,
        kind: ContextArtifactKind,
        bytes: &[u8],
        inline_bytes: usize,
        capped: bool,
    ) -> Result<Option<ContextArtifact>, JsonRpcError> {
        if !capped {
            return Ok(None);
        }
        if !self.runtime.status().await.file_backed_dynamic_context {
            return Ok(None);
        }
        let thread_id = "app-server".to_string();
        let turn_id = process_id.to_string();
        let store = self.runtime.context_artifacts();
        let artifact = store
            .create(CreateArtifactRequest {
                kind,
                thread_id: &thread_id,
                turn_id: &turn_id,
                source_tool_id: Some(process_id),
                label: Some(label),
                bytes: &[],
            })
            .map_err(internal_error)?;
        let artifact = store
            .append(&thread_id, &artifact.id, bytes)
            .map_err(internal_error)?;
        self.runtime
            .emit(RoderEvent::ContextArtifactCreated(ContextArtifactCreated {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                artifact: artifact.clone(),
                timestamp: time::OffsetDateTime::now_utc(),
            }))
            .await;
        self.runtime
            .emit(RoderEvent::ContextArtifactAppended(
                ContextArtifactAppended {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    artifact_id: artifact.id.clone(),
                    appended_bytes: bytes.len() as u64,
                    byte_count: artifact.byte_count,
                    line_count: artifact.line_count,
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        self.runtime
            .emit(RoderEvent::ContextArtifactCapped(ContextArtifactCapped {
                thread_id,
                turn_id,
                artifact_id: artifact.id.clone(),
                inline_byte_count: inline_bytes as u64,
                original_byte_count: bytes.len() as u64,
                timestamp: time::OffsetDateTime::now_utc(),
            }))
            .await;
        Ok(Some(artifact))
    }

    async fn enforce_command_exec_policy(
        &self,
        params: &CommandExecParams,
    ) -> Result<(), JsonRpcError> {
        let mode = self.runtime.status().await.policy_mode;
        let tool_call = ToolCall {
            id: params
                .process_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: "shell".to_string(),
            arguments: serde_json::to_value(params).unwrap_or(serde_json::Value::Null),
            raw_arguments: serde_json::to_string(params).unwrap_or_default(),
            thread_id: "app-server".to_string(),
            turn_id: "command/exec".to_string(),
        };
        let ctx =
            ToolExecutionContext::new(tool_call.thread_id.clone(), tool_call.turn_id.clone(), mode);
        match DefaultPolicyGate::new().decide(&tool_call, mode, &ctx) {
            PolicyDecision::Allowed | PolicyDecision::AutoApproved { .. } => Ok(()),
            PolicyDecision::Denied { reason } => Err(JsonRpcError {
                code: -32004,
                message: format!("command/exec denied by policy: {reason}"),
                data: Some(serde_json::json!({ "kind": "policy_denied" })),
            }),
            PolicyDecision::RequiresApproval { reason } => Err(JsonRpcError {
                code: -32004,
                message: format!(
                    "command/exec requires approval{}",
                    reason
                        .as_deref()
                        .map(|reason| format!(": {reason}"))
                        .unwrap_or_default()
                ),
                data: Some(serde_json::json!({ "kind": "approval_required" })),
            }),
        }
    }

    fn emit_command_output_delta(
        &self,
        process_id: &str,
        stream: &str,
        chunk: &[u8],
        cap_reached: bool,
    ) {
        self.publish_notification(JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "command/exec/outputDelta".to_string(),
            params: serde_json::to_value(CommandExecOutputDeltaNotification {
                process_id: process_id.to_string(),
                stream: stream.to_string(),
                delta_base64: base64::engine::general_purpose::STANDARD.encode(chunk),
                cap_reached,
            })
            .unwrap(),
        });
    }
}

fn validate_command_exec_params(params: &CommandExecParams) -> Result<(), JsonRpcError> {
    if params.command.is_empty() {
        return Err(invalid_params("command must not be empty"));
    }
    if params.tty {
        return Err(unsupported("command/exec tty mode is not implemented"));
    }
    if params.stream_stdin {
        return Err(unsupported(
            "command/exec streaming stdin is not implemented",
        ));
    }
    if params.size.is_some() {
        return Err(unsupported("command/exec resize is not implemented"));
    }
    if params.stream_stdout_stderr && params.process_id.as_deref().unwrap_or_default().is_empty() {
        return Err(invalid_params(
            "processId is required when streamStdoutStderr is true",
        ));
    }
    if params.disable_timeout && params.timeout_ms.is_some() {
        return Err(invalid_params(
            "disableTimeout cannot be combined with timeoutMs",
        ));
    }
    if params.disable_output_cap && params.output_bytes_cap.is_some() {
        return Err(invalid_params(
            "disableOutputCap cannot be combined with outputBytesCap",
        ));
    }
    Ok(())
}

fn cap_output(output: &[u8], params: &CommandExecParams) -> (Vec<u8>, bool) {
    if params.disable_output_cap {
        return (output.to_vec(), false);
    }
    let cap = params.output_bytes_cap.unwrap_or(1_048_576);
    if output.len() > cap {
        (output[..cap].to_vec(), true)
    } else {
        (output.to_vec(), false)
    }
}

fn command_output_text(output: &[u8], artifact: Option<&ContextArtifact>, label: &str) -> String {
    let mut text = String::from_utf8_lossy(output).to_string();
    if let Some(artifact) = artifact {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format_artifact_reference(artifact, label));
    }
    text
}

fn absolute_path(path: &str) -> Result<PathBuf, JsonRpcError> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(invalid_params("cwd must be absolute"))
    }
}

fn unsupported(message: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32004,
        message: message.to_string(),
        data: Some(serde_json::json!({ "kind": "unsupported" })),
    }
}

fn invalid_params(message: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.to_string(),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}
