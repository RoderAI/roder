//! Translate Cursor exec RPCs to the runtime's policy-approved tool results.
//! Never repeat a tool operation against the provider process's filesystem.
use crate::proto::*;
use roder_api::inference::{ToolCallCompleted, TurnToolExecutor, TurnToolOutcome};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct ExecutorFailure {
    kind: String,
    message: String,
}
impl std::fmt::Display for ExecutorFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ExecutorFailure {}

async fn execute(
    executor: Option<&dyn TurnToolExecutor>,
    id: String,
    name: &str,
    arguments: Value,
) -> anyhow::Result<TurnToolOutcome> {
    let executor = executor.ok_or_else(|| anyhow::anyhow!("Roder tool executor is unavailable"))?;
    let outcome = executor
        .execute(ToolCallCompleted {
            id,
            name: name.to_string(),
            arguments: arguments.to_string(),
        })
        .await?;
    if outcome.is_error {
        return Err(ExecutorFailure {
            kind: outcome.data["error"]["kind"]
                .as_str()
                .unwrap_or("tool_execution_failed")
                .into(),
            message: outcome.result,
        }
        .into());
    }
    Ok(outcome)
}

fn absolute_path(workspace: &Path, path: &str) -> PathBuf {
    if path.is_empty() {
        workspace.to_path_buf()
    } else if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        workspace.join(path)
    }
}

fn strings(data: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    data.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .ok_or_else(|| anyhow::anyhow!("Roder tool did not return structured {key}"))
}

pub(super) async fn service_exec(
    exec: CursorExecRequest,
    workspace: &Path,
    executor: Option<&dyn TurnToolExecutor>,
    tools: &[CursorMcpTool],
) -> Option<(Vec<Vec<u8>>, Option<u64>)> {
    let (seq, field_no, path) = match &exec {
        CursorExecRequest::Read {
            seq,
            field_no,
            path,
            ..
        } => (*seq, *field_no, path.clone()),
        CursorExecRequest::Write { seq, path, .. } => (*seq, 3, path.clone()),
        CursorExecRequest::Delete { seq, path, .. } => (*seq, 4, path.clone()),
        CursorExecRequest::Shell {
            seq, field_no, cwd, ..
        } => (*seq, *field_no, cwd.clone()),
        CursorExecRequest::Search { seq, path, .. } => (*seq, 5, path.clone()),
        CursorExecRequest::List { seq, path, .. } => (*seq, 8, path.clone()),
        CursorExecRequest::Diagnostics { seq, path, .. } => (*seq, 9, path.clone()),
        CursorExecRequest::Mcp { seq, .. } => (*seq, 11, String::new()),
        CursorExecRequest::McpState { seq, .. } => (*seq, 36, String::new()),
        CursorExecRequest::Unknown { seq, field_no, .. } => (*seq, *field_no, String::new()),
        CursorExecRequest::RequestContext { id } => {
            return Some((
                vec![encode_exec_request_context_result(
                    *id,
                    &workspace.to_string_lossy(),
                )],
                None,
            ));
        }
    };
    let outcome = execute_request(exec, workspace, executor, tools, seq, field_no).await;
    Some((
        outcome.unwrap_or_else(|error| {
            vec![encode_exec_error(
                seq,
                field_no,
                &path,
                &error.to_string(),
                error
                    .downcast_ref::<ExecutorFailure>()
                    .map(|error| error.kind.as_str())
                    .unwrap_or("tool_execution_failed"),
            )]
        }),
        Some(seq),
    ))
}

async fn execute_request(
    exec: CursorExecRequest,
    workspace: &Path,
    executor: Option<&dyn TurnToolExecutor>,
    tools: &[CursorMcpTool],
    seq: u64,
    field_no: u32,
) -> anyhow::Result<Vec<Vec<u8>>> {
    Ok(match exec {
        CursorExecRequest::Read {
            path,
            tool_call_id,
            offset,
            limit,
            ..
        } => {
            let start = offset.unwrap_or(1).max(1);
            let mut args = json!({"path":path,"start_line":start});
            if let Some(limit) = limit {
                args["limit"] = json!(limit.max(1));
            }
            let result = execute(executor, tool_call_id, "read_file", args).await?;
            let content = result
                .data
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    anyhow::anyhow!("Roder read_file did not return structured content")
                })?;
            vec![encode_exec_read_result(
                seq,
                field_no,
                &path,
                content.as_bytes(),
                result.data["total_lines"].as_u64().unwrap_or(0),
                result.data["file_size"]
                    .as_u64()
                    .unwrap_or(content.len() as u64),
                result.data["truncated"].as_bool().unwrap_or(false),
                offset.is_some() || limit.is_some(),
            )]
        }
        CursorExecRequest::Write {
            path,
            content,
            tool_call_id,
            ..
        } => {
            let text = std::str::from_utf8(&content)?;
            execute(
                executor,
                tool_call_id,
                "write_file",
                json!({"path":path,"content":text}),
            )
            .await?;
            vec![encode_exec_write_result(
                seq,
                &path,
                text.lines().count() as u64,
                content.len() as u64,
                text,
            )]
        }
        CursorExecRequest::Delete {
            path, tool_call_id, ..
        } => {
            let quoted = format!("'{}'", path.replace('\'', "'\\''"));
            execute(
                executor,
                tool_call_id,
                "shell",
                json!({"command":format!("rm -f -- {quoted}")}),
            )
            .await?;
            vec![encode_exec_delete_result(seq, &path)]
        }
        CursorExecRequest::Shell {
            command,
            cwd,
            tool_call_id,
            timeout_ms,
            ..
        } => {
            let mut args = json!({"command":command});
            if !cwd.is_empty() {
                args["workdir"] = json!(cwd);
            }
            if let Some(timeout) = timeout_ms {
                args["timeout_seconds"] = json!(timeout.div_ceil(1000).max(1));
            }
            let executor =
                executor.ok_or_else(|| anyhow::anyhow!("Roder tool executor is unavailable"))?;
            let result = executor
                .execute(ToolCallCompleted {
                    id: tool_call_id,
                    name: "shell".to_string(),
                    arguments: args.to_string(),
                })
                .await?;
            // A denied call has no process exit status; do not claim it ran.
            let code = result.data["exit_code"].as_i64();
            if result.is_error && code.is_none() {
                anyhow::bail!("{}", result.result);
            }
            let code = code
                .map(|code| {
                    if code < 0 {
                        1
                    } else {
                        code.min(i32::MAX as i64) as i32
                    }
                })
                .unwrap_or(0);
            let output = result.data["aggregated_output"]
                .as_str()
                .unwrap_or(&result.result);
            let cwd = result.data["cwd"]
                .as_str()
                .filter(|cwd| !cwd.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    absolute_path(workspace, &cwd)
                        .to_string_lossy()
                        .into_owned()
                });
            let duration = result.data["duration_ms"].as_u64().unwrap_or(0);
            if field_no == 14 {
                encode_exec_shell_results(seq, &cwd, output, duration, code)
            } else {
                vec![encode_exec_shell_result(
                    seq, &command, &cwd, output, duration, code,
                )]
            }
        }
        CursorExecRequest::Search {
            pattern,
            path,
            glob,
            mode,
            tool_call_id,
            case_insensitive,
            multiline,
            offset,
            limit,
            ..
        } => {
            anyhow::ensure!(
                !multiline,
                "multiline Cursor searches are not supported by the Roder grep executor"
            );
            let needle = pattern.filter(|value| !value.is_empty());
            let mode = if mode.is_empty() {
                "content"
            } else {
                mode.as_str()
            };
            anyhow::ensure!(
                ["content", "files_with_matches", "count"].contains(&mode),
                "unsupported Cursor search output mode {mode}"
            );
            let glob = glob.filter(|value| !value.is_empty());
            let result = if let Some(needle) = &needle {
                let mut args = json!({"query":needle,"path":if path.is_empty(){"."}else{&path},
                        "regex":true,"case_sensitive":!case_insensitive,"offset":offset});
                if let Some(limit) = limit {
                    args["limit"] = json!(limit.max(1));
                }
                if let Some(glob) = &glob {
                    args["glob"] = json!(glob);
                }
                execute(executor, tool_call_id, "grep", args).await?
            } else {
                let pattern = absolute_path(workspace, &path)
                    .join(glob.as_deref().unwrap_or("**/*"))
                    .to_string_lossy()
                    .into_owned();
                let mut args = json!({"pattern":pattern,"offset":offset});
                if let Some(limit) = limit {
                    args["limit"] = json!(limit.max(1));
                }
                execute(executor, tool_call_id, "glob", args).await?
            };
            let truncated = result.data["truncated"].as_bool().unwrap_or(false);
            let lines = strings(&result.data, "matches")?;
            let root = workspace.to_string_lossy();
            if let Some(needle) = needle {
                let matches: Vec<_> = lines
                    .iter()
                    .filter_map(|line| parse_grep_match(line))
                    .collect();
                if mode == "files_with_matches" {
                    let files = matches
                        .iter()
                        .map(|item| item.path.clone())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>();
                    vec![encode_exec_glob_result(
                        seq, &path, &root, &files, truncated,
                    )]
                } else if mode == "count" {
                    vec![encode_exec_count_result(
                        seq, &needle, &path, &root, &matches, truncated,
                    )]
                } else {
                    vec![encode_exec_grep_result(
                        seq, &needle, &path, &root, &matches, truncated,
                    )]
                }
            } else {
                vec![encode_exec_glob_result(
                    seq, &path, &root, &lines, truncated,
                )]
            }
        }
        CursorExecRequest::List {
            path, tool_call_id, ..
        } => {
            let result =
                execute(executor, tool_call_id, "list_files", json!({"path":path})).await?;
            vec![encode_exec_list_result(
                seq,
                &absolute_path(workspace, &path).to_string_lossy(),
                &strings(&result.data, "entries")?,
            )]
        }
        CursorExecRequest::Diagnostics { tool_call_id, .. } => {
            anyhow::bail!(
                "Roder has no diagnostics executor for Cursor diagnostics request {tool_call_id}"
            );
        }
        CursorExecRequest::Mcp {
            name,
            tool_call_id,
            arguments,
            ..
        } => {
            anyhow::ensure!(!name.is_empty(), "Cursor MCP call is missing its tool name");
            let executor =
                executor.ok_or_else(|| anyhow::anyhow!("Roder tool executor is unavailable"))?;
            let result = executor
                .execute(ToolCallCompleted {
                    id: tool_call_id,
                    name,
                    arguments: arguments.to_string(),
                })
                .await?;
            vec![encode_exec_mcp_result(
                seq,
                &result.result,
                result.is_error,
                &result.data,
            )]
        }
        CursorExecRequest::McpState {
            server_identifiers, ..
        } => {
            anyhow::ensure!(
                server_identifiers.iter().all(|id| id == "roder"),
                "Cursor requested an unknown MCP server"
            );
            vec![encode_exec_mcp_state_result(seq, tools)]
        }
        CursorExecRequest::Unknown { field_no, .. } => {
            vec![encode_exec_unknown_result(seq, field_no)]
        }
        CursorExecRequest::RequestContext { .. } => unreachable!(),
    })
}

fn parse_grep_match(line: &str) -> Option<CursorGrepMatch> {
    // The grep backend's canonical format is path:line:content. Permit colons
    // in filenames and contents by looking for the numeric line separator.
    for (i, _) in line.match_indices(':') {
        let (number, text) = line[i + 1..].split_once(':')?;
        if let Ok(number) = number.parse::<u64>() {
            return Some(CursorGrepMatch {
                path: line[..i].to_string(),
                line: number,
                text: text.to_string(),
            });
        }
    }
    None
}

#[cfg(test)]
#[path = "bidi_exec_tests.rs"]
mod tests;
