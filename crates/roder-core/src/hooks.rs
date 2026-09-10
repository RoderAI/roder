//! Local Codex-compatible hook discovery and dispatch.
//!
//! `.roder/hooks.json` takes precedence over `.codex/hooks.json`. Hook failures
//! are diagnostic and fail open; only an explicit `PreToolUse` deny blocks a
//! tool call.

use std::path::{Path, PathBuf};
use std::time::Duration;

use roder_api::events::{HookRunRecorded, RoderEvent, ThreadId, TurnId};
use roder_api::tools::ToolCall;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use tokio::process::Command;

use crate::runtime::Runtime;

const DEFAULT_TIMEOUT_SECONDS: u64 = 10;
const MAX_OUTPUT_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct HookInspection {
    pub source: String,
    pub path: PathBuf,
    pub event_count: usize,
    pub handler_count: usize,
}

#[derive(Debug, Clone)]
pub enum PreToolUseResult {
    /// `Some` only when a hook rewrote the tool input; `None` leaves the call
    /// (and its original `raw_arguments`) exactly as the model produced it.
    Continue(Option<Value>),
    Denied(String),
}

/// Stand-in reason when a hook denies a call without explaining why. The denial
/// is the decision, not the prose: a blank reason must still block.
const DEFAULT_DENIAL_REASON: &str = "denied by PreToolUse hook";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HooksFile {
    #[serde(default)]
    hooks: HookEvents,
}

#[derive(Debug, Default, Deserialize)]
struct HookEvents {
    #[serde(rename = "PreToolUse", default)]
    pre_tool_use: Vec<MatcherGroup>,
    #[serde(rename = "PermissionRequest", default)]
    permission_request: Vec<MatcherGroup>,
    #[serde(rename = "PostToolUse", default)]
    post_tool_use: Vec<MatcherGroup>,
    #[serde(rename = "PreCompact", default)]
    pre_compact: Vec<MatcherGroup>,
    #[serde(rename = "PostCompact", default)]
    post_compact: Vec<MatcherGroup>,
    #[serde(rename = "SessionStart", default)]
    session_start: Vec<MatcherGroup>,
    #[serde(rename = "SessionEnd", default)]
    session_end: Vec<MatcherGroup>,
    #[serde(rename = "UserPromptSubmit", default)]
    user_prompt_submit: Vec<MatcherGroup>,
    #[serde(rename = "SubagentStart", default)]
    subagent_start: Vec<MatcherGroup>,
    #[serde(rename = "SubagentStop", default)]
    subagent_stop: Vec<MatcherGroup>,
    #[serde(rename = "Stop", default)]
    stop: Vec<MatcherGroup>,
}

impl HookEvents {
    fn groups_for(&self, event: &str) -> &[MatcherGroup] {
        match event {
            "PreToolUse" => &self.pre_tool_use,
            "PermissionRequest" => &self.permission_request,
            "PostToolUse" => &self.post_tool_use,
            "PreCompact" => &self.pre_compact,
            "PostCompact" => &self.post_compact,
            "SessionStart" => &self.session_start,
            "SessionEnd" => &self.session_end,
            "UserPromptSubmit" => &self.user_prompt_submit,
            "SubagentStart" => &self.subagent_start,
            "SubagentStop" => &self.subagent_stop,
            "Stop" => &self.stop,
            _ => &[],
        }
    }

    fn event_count(&self) -> usize {
        [
            &self.pre_tool_use,
            &self.permission_request,
            &self.post_tool_use,
            &self.pre_compact,
            &self.post_compact,
            &self.session_start,
            &self.session_end,
            &self.user_prompt_submit,
            &self.subagent_start,
            &self.subagent_stop,
            &self.stop,
        ]
        .into_iter()
        .filter(|groups| !groups.is_empty())
        .count()
    }

    fn handler_count(&self) -> usize {
        [
            &self.pre_tool_use,
            &self.permission_request,
            &self.post_tool_use,
            &self.pre_compact,
            &self.post_compact,
            &self.session_start,
            &self.session_end,
            &self.user_prompt_submit,
            &self.subagent_start,
            &self.subagent_stop,
            &self.stop,
        ]
        .into_iter()
        .flatten()
        .map(|group| group.hooks.len())
        .sum()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatcherGroup {
    #[serde(default)]
    matcher: Option<String>,
    #[serde(default)]
    hooks: Vec<HookHandler>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum HookHandler {
    #[serde(rename = "command")]
    Command {
        command: String,
        #[serde(default, rename = "commandWindows", alias = "command_windows")]
        command_windows: Option<String>,
        #[serde(default, rename = "timeout")]
        timeout_seconds: Option<u64>,
        #[serde(default, rename = "statusMessage")]
        status_message: Option<String>,
        #[serde(default, rename = "async")]
        asynchronous: bool,
        #[serde(default, rename = "additionalContextLimit")]
        additional_context_limit: Option<usize>,
    },
    #[serde(rename = "mcp_tool")]
    McpTool {
        server: String,
        tool: String,
        #[serde(default)]
        input: Map<String, Value>,
        #[serde(default, rename = "timeout")]
        timeout_seconds: Option<u64>,
        #[serde(default, rename = "statusMessage")]
        status_message: Option<String>,
    },
    #[serde(rename = "prompt")]
    Prompt {
        /// Roder extension: textual guidance recorded with the hook execution.
        /// Unit-style Codex prompt hooks remain valid and produce diagnostics.
        #[serde(default)]
        prompt: Option<String>,
    },
    #[serde(rename = "agent")]
    Agent {
        /// Roder extension: task guidance recorded with the hook execution.
        /// Unit-style Codex agent hooks remain valid and produce diagnostics.
        #[serde(default)]
        prompt: Option<String>,
    },
}

pub fn inspect(workspace: impl AsRef<Path>) -> Option<HookInspection> {
    let (path, source) = hooks_path(workspace.as_ref())?;
    let file: HooksFile = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    Some(HookInspection {
        source: source.to_string(),
        path,
        event_count: file.hooks.event_count(),
        handler_count: file.hooks.handler_count(),
    })
}

pub async fn run_pre_tool_use(
    runtime: &Runtime,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    workspace: Option<&str>,
    call: &ToolCall,
) -> PreToolUseResult {
    let mut rewritten: Option<Value> = None;
    for handler in matching_handlers(workspace, "PreToolUse", Some(&call.name)) {
        // Rebuilt per handler: a chain of rewriting hooks must each see the
        // input as the previous hook left it, not the model's original.
        let payload = hook_payload(
            thread_id,
            turn_id,
            workspace,
            "PreToolUse",
            call,
            rewritten.clone().unwrap_or_else(|| call.arguments.clone()),
        );
        let outcome = run_handler(
            runtime,
            thread_id,
            turn_id,
            call,
            "PreToolUse",
            &handler,
            &payload,
        )
        .await;
        if let Some(denial) = outcome.denial {
            return PreToolUseResult::Denied(denial);
        }
        if let Some(updated) = outcome.updated_input {
            rewritten = Some(updated);
        }
    }
    PreToolUseResult::Continue(rewritten)
}

pub async fn run_post_tool_use(
    runtime: &Runtime,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    workspace: Option<&str>,
    call: &ToolCall,
    output: &str,
) {
    let payload = hook_payload(
        thread_id,
        turn_id,
        workspace,
        "PostToolUse",
        call,
        json!({"toolOutput": output}),
    );
    for handler in matching_handlers(workspace, "PostToolUse", Some(&call.name)) {
        let _ = run_handler(
            runtime,
            thread_id,
            turn_id,
            call,
            "PostToolUse",
            &handler,
            &payload,
        )
        .await;
    }
}

pub async fn run_lifecycle(
    runtime: &Runtime,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    workspace: Option<&str>,
    event: &str,
    matcher: Option<&str>,
    input: Value,
) {
    let call = ToolCall {
        id: format!("hook-{event}"),
        name: matcher.unwrap_or(event).to_string(),
        raw_arguments: input.to_string(),
        arguments: input.clone(),
        thread_id: thread_id.clone(),
        turn_id: turn_id.clone(),
    };
    let payload = hook_payload(thread_id, turn_id, workspace, event, &call, input);
    for handler in matching_handlers(workspace, event, matcher) {
        let _ = run_handler(
            runtime, thread_id, turn_id, &call, event, &handler, &payload,
        )
        .await;
    }
}

struct HookOutcome {
    denial: Option<String>,
    updated_input: Option<Value>,
}

async fn run_handler(
    runtime: &Runtime,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    call: &ToolCall,
    event: &str,
    handler: &HookHandler,
    payload: &Value,
) -> HookOutcome {
    let (handler_type, detail, timeout_seconds, output) = match handler {
        HookHandler::Command {
            command,
            command_windows,
            timeout_seconds,
            status_message,
            asynchronous,
            additional_context_limit,
        } => {
            let command = if cfg!(windows) {
                command_windows.as_deref().unwrap_or(command)
            } else {
                command.as_str()
            };
            let _ = additional_context_limit;
            if *asynchronous {
                let mut process = Command::new("sh");
                process
                    .arg("-lc")
                    .arg(command)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str)
                    && !cwd.is_empty()
                {
                    process.current_dir(cwd);
                }
                (
                    "command",
                    status_message
                        .clone()
                        .unwrap_or_else(|| command.to_string()),
                    *timeout_seconds,
                    process
                        .spawn()
                        .map(|_| "async hook started".to_string())
                        .map_err(|error| error.to_string()),
                )
            } else {
                let mut process = Command::new("sh");
                process
                    .arg("-lc")
                    .arg(command)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str)
                    && !cwd.is_empty()
                {
                    process.current_dir(cwd);
                }
                let result = async {
                    let mut child = process.spawn().map_err(|error| error.to_string())?;
                    use tokio::io::AsyncWriteExt;
                    if let Some(mut stdin) = child.stdin.take() {
                        stdin
                            .write_all(payload.to_string().as_bytes())
                            .await
                            .map_err(|error| error.to_string())?;
                    }
                    let output = child
                        .wait_with_output()
                        .await
                        .map_err(|error| error.to_string())?;
                    if output.status.success() {
                        Ok(String::from_utf8_lossy(&output.stdout).to_string())
                    } else {
                        Err(String::from_utf8_lossy(&output.stderr).to_string())
                    }
                };
                let output = match tokio::time::timeout(
                    Duration::from_secs(timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS)),
                    result,
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err("hook timed out".to_string()),
                };
                (
                    "command",
                    status_message
                        .clone()
                        .unwrap_or_else(|| command.to_string()),
                    *timeout_seconds,
                    output,
                )
            }
        }
        HookHandler::McpTool {
            server,
            tool,
            input,
            timeout_seconds,
            status_message,
        } => {
            let tool_name = format!("mcp__{server}__{tool}");
            let input = expand_input(&Value::Object(input.clone()), payload);
            let result = tokio::time::timeout(
                Duration::from_secs(timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS)),
                runtime.execute_workflow_tool(thread_id.clone(), &tool_name, input),
            )
            .await;
            let output = match result {
                Ok(Ok(result)) if !result.is_error => Ok(result.text),
                Ok(Ok(result)) => Err(result.text),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err("hook timed out".to_string()),
            };
            (
                "mcp_tool",
                status_message.clone().unwrap_or(tool_name),
                *timeout_seconds,
                output,
            )
        }
        HookHandler::Prompt { prompt } => {
            let output = prompt
                .as_ref()
                .map(|prompt| expand_string(prompt, payload))
                .ok_or_else(|| "Codex-compatible prompt handler has no prompt text".to_string());
            ("prompt", "prompt hook".to_string(), None, output)
        }
        HookHandler::Agent { prompt } => {
            let output = prompt
                .as_ref()
                .map(|prompt| expand_string(prompt, payload))
                .ok_or_else(|| "Codex-compatible agent handler has no prompt text".to_string());
            ("agent", "agent hook".to_string(), None, output)
        }
    };
    let _ = timeout_seconds;
    let (status, text) = match output {
        Ok(text) => ("success", text),
        Err(error) => ("failed", error),
    };
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let (denial, updated_input) = parse_pre_tool_use_output(&parsed);
    let output = truncate(&text);
    // Bypass Runtime::emit to avoid lifecycle hooks recursively observing their own diagnostics.
    runtime
        .bus
        .emit(RoderEvent::HookRunRecorded(HookRunRecorded {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            tool_id: call.id.clone(),
            tool_name: call.name.clone(),
            hook_event_name: event.to_string(),
            handler_type: handler_type.to_string(),
            status: if denial.is_some() {
                "blocked".to_string()
            } else {
                status.to_string()
            },
            detail,
            output: Some(output),
            timestamp: OffsetDateTime::now_utc(),
        }));
    HookOutcome {
        denial,
        updated_input,
    }
}

/// Parses the Codex `PreToolUse` output contract. The legacy top-level form is
/// retained for older copied hook scripts, while hook-specific output follows
/// Codex's strict permission-decision rules.
fn parse_pre_tool_use_output(parsed: &Value) -> (Option<String>, Option<Value>) {
    if let Some(output) = parsed.get("hookSpecificOutput") {
        let decision = output.get("permissionDecision").and_then(Value::as_str);
        let reason = output
            .get("permissionDecisionReason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(str::to_string);
        let updated = output.get("updatedInput").cloned();
        return match decision {
            Some("deny") => (
                Some(reason.unwrap_or_else(|| DEFAULT_DENIAL_REASON.to_string())),
                None,
            ),
            Some("allow") => (None, updated),
            _ => (None, None),
        };
    }
    let denial = parsed
        .get("decision")
        .and_then(Value::as_str)
        .filter(|decision| *decision == "deny" || *decision == "block")
        .map(|_| {
            parsed
                .get("reason")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| DEFAULT_DENIAL_REASON.to_string())
        });
    (denial, parsed.get("updatedInput").cloned())
}

fn hook_payload(
    thread_id: &str,
    turn_id: &str,
    workspace: Option<&str>,
    event: &str,
    call: &ToolCall,
    input: Value,
) -> Value {
    json!({"hookEventName": event, "sessionId": thread_id, "turnId": turn_id, "cwd": workspace.unwrap_or(""), "workspaceRoot": workspace.unwrap_or(""), "toolName": call.name, "toolUseId": call.id, "toolInput": input})
}

fn matching_handlers(
    workspace: Option<&str>,
    event: &str,
    matcher_input: Option<&str>,
) -> Vec<HookHandler> {
    let Some((path, _)) = workspace.and_then(|workspace| hooks_path(Path::new(workspace))) else {
        return Vec::new();
    };
    let Ok(file) = std::fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<HooksFile>(&raw).ok())
        .ok_or(())
    else {
        return Vec::new();
    };
    file.hooks
        .groups_for(event)
        .iter()
        .filter(|group| matcher_matches(group.matcher.as_deref(), matcher_input))
        .flat_map(|group| group.hooks.iter().cloned())
        .collect()
}

fn matcher_matches(matcher: Option<&str>, input: Option<&str>) -> bool {
    let Some(matcher) = matcher else {
        return true;
    };
    input.is_some_and(|input| regex::Regex::new(matcher).is_ok_and(|regex| regex.is_match(input)))
}

fn hooks_path(workspace: &Path) -> Option<(PathBuf, &'static str)> {
    let roder = workspace.join(".roder/hooks.json");
    if roder.is_file() {
        return Some((roder, "roder"));
    }
    let codex = workspace.join(".codex/hooks.json");
    codex.is_file().then_some((codex, "codex"))
}

fn expand_input(input: &Value, payload: &Value) -> Value {
    match input {
        Value::String(value) => Value::String(expand_string(value, payload)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| expand_input(value, payload))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), expand_input(value, payload)))
                .collect(),
        ),
        value => value.clone(),
    }
}

fn expand_string(value: &str, payload: &Value) -> String {
    let mut result = value.to_string();
    for (key, replacement) in [
        ("tool_input", payload.get("toolInput")),
        ("tool_name", payload.get("toolName")),
    ] {
        if let Some(replacement) = replacement {
            result = result.replace(
                &format!("${{{key}}}"),
                replacement.as_str().unwrap_or(&replacement.to_string()),
            );
        }
    }
    result
}

fn truncate(value: &str) -> String {
    value.chars().take(MAX_OUTPUT_BYTES).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roder_path_has_precedence() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".roder")).unwrap();
        std::fs::create_dir_all(temp.path().join(".codex")).unwrap();
        std::fs::write(temp.path().join(".roder/hooks.json"), r#"{"hooks":{}}"#).unwrap();
        std::fs::write(temp.path().join(".codex/hooks.json"), r#"{"hooks":{}}"#).unwrap();
        assert_eq!(hooks_path(temp.path()).unwrap().1, "roder");
    }

    #[test]
    fn matcher_uses_codex_regex_semantics() {
        assert!(matcher_matches(Some("Read|Write"), Some("Write")));
        assert!(!matcher_matches(Some("Read"), Some("Write")));
        assert!(matcher_matches(None, Some("anything")));
    }

    #[test]
    fn prompt_and_agent_handlers_accept_unit_and_roder_prompt_forms() {
        let unit_prompt: HookHandler = serde_json::from_str(r#"{"type":"prompt"}"#).unwrap();
        let prompted_agent: HookHandler =
            serde_json::from_str(r#"{"type":"agent","prompt":"Review ${tool_name}"}"#).unwrap();
        assert!(matches!(unit_prompt, HookHandler::Prompt { prompt: None }));
        assert!(matches!(
            prompted_agent,
            HookHandler::Agent { prompt: Some(_) }
        ));
    }

    #[test]
    fn hook_output_parses_deny_and_rewrite() {
        let deny: Value = serde_json::from_str(r#"{"decision":"deny","reason":"no"}"#).unwrap();
        assert_eq!(deny.get("decision").and_then(Value::as_str), Some("deny"));
        let rewrite: Value =
            serde_json::from_str(r#"{"hookSpecificOutput":{"updatedInput":{"path":"safe"}}}"#)
                .unwrap();
        assert_eq!(
            rewrite
                .pointer("/hookSpecificOutput/updatedInput/path")
                .and_then(Value::as_str),
            Some("safe")
        );
    }

    #[test]
    fn copied_codex_configuration_accepts_all_event_and_command_fields() {
        let file: HooksFile = serde_json::from_str(
            r#"{
                "hooks": {
                    "PermissionRequest": [{"matcher":"shell","hooks":[{
                        "type":"command","command":"echo hook",
                        "commandWindows":"echo hook","timeout":2,
                        "async":false,"statusMessage":"checking",
                        "additionalContextLimit":2500
                    }]}]
                }
            }"#,
        )
        .expect("Codex hooks file parses");
        assert_eq!(file.hooks.event_count(), 1);
        assert_eq!(file.hooks.handler_count(), 1);
    }

    #[test]
    fn codex_pre_tool_use_output_honours_nested_allow_and_deny() {
        let deny: Value = serde_json::from_str(
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"do not run"}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_pre_tool_use_output(&deny).0.as_deref(),
            Some("do not run")
        );

        let rewrite: Value = serde_json::from_str(
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"command":"echo safe"}}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_pre_tool_use_output(&rewrite)
                .1
                .and_then(|value| value.get("command").cloned())
                .and_then(|value| value.as_str().map(str::to_string))
                .as_deref(),
            Some("echo safe")
        );

        // A deny with a blank reason is still a deny: the decision blocks the
        // call and a stand-in reason is supplied for the transcript.
        let blank_reason: Value = serde_json::from_str(
            r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":" "}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_pre_tool_use_output(&blank_reason).0.as_deref(),
            Some(DEFAULT_DENIAL_REASON)
        );

        let no_reason: Value =
            serde_json::from_str(r#"{"hookSpecificOutput":{"permissionDecision":"deny"}}"#)
                .unwrap();
        assert_eq!(
            parse_pre_tool_use_output(&no_reason).0.as_deref(),
            Some(DEFAULT_DENIAL_REASON)
        );

        // An unrecognised decision still falls through without blocking.
        let unknown: Value =
            serde_json::from_str(r#"{"hookSpecificOutput":{"permissionDecision":"ask"}}"#).unwrap();
        assert_eq!(parse_pre_tool_use_output(&unknown), (None, None));
    }

    #[test]
    fn legacy_top_level_deny_blocks_without_a_reason() {
        for raw in [
            r#"{"decision":"deny"}"#,
            r#"{"decision":"block","reason":"   "}"#,
        ] {
            let parsed: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(
                parse_pre_tool_use_output(&parsed).0.as_deref(),
                Some(DEFAULT_DENIAL_REASON),
                "{raw} must block"
            );
        }

        let allowed: Value = serde_json::from_str(r#"{"decision":"approve"}"#).unwrap();
        assert_eq!(parse_pre_tool_use_output(&allowed).0, None);
    }
}
