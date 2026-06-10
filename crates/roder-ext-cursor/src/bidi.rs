//! Cursor `agent.v1.AgentService/Run` bidirectional agent-runtime client.
//!
//! Unlike the simple inference path (one request -> event stream), Cursor's
//! agent loop is *same-stream*: the server streams the model's output and emits
//! `exec_server_message` read/write/shell requests; the client executes them
//! locally and writes `exec_client_message` results back into the *same* open
//! request body, and the server continues. This client keeps the Run stream
//! open (channel-fed request body), services the exec channel, and surfaces the
//! model's text as Roder `MessageDelta`s. See
//! `docs/roder-cursor-agent-runtime-protocol.md`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use roder_api::inference::{
    CompletionMetadata, InferenceEvent, InferenceEventStream, MessageDelta, ReasoningDelta,
    ToolCallCompleted, TurnToolExecutor,
};
use serde_json::json;

use crate::agentservice::AgentServiceConfig;
use crate::proto::{
    ConnectFrame, CursorExecRequest, CursorGrepMatch, decode_server_frame,
    encode_cli_stream_control_frames, encode_connect_frame, encode_exec_control,
    encode_exec_glob_result, encode_exec_grep_result, encode_exec_read_result,
    encode_exec_request_context_result, encode_exec_shell_results, encode_exec_write_result,
    encode_heartbeat, encode_kv_ack, take_connect_frame,
};

pub struct BidiRequest {
    pub access_token: String,
    /// Pre-encoded `AgentClientMessage{run_request}` payload (not connect-framed).
    pub run_request: Vec<u8>,
    /// Already connect-framed request-context frames (workspace context).
    pub context_frames: Vec<Vec<u8>>,
    pub workspace: PathBuf,
    pub tool_executor: Option<Arc<dyn TurnToolExecutor>>,
}

pub async fn run_bidi_turn(
    config: AgentServiceConfig,
    request: BidiRequest,
) -> anyhow::Result<InferenceEventStream> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
    // Initial client frames: run request, request-context frames, then the CLI
    // stream-control frames the server expects to begin streaming.
    tx.send(Bytes::from(encode_connect_frame(&request.run_request)))
        .ok();
    for frame in &request.context_frames {
        tx.send(Bytes::from(frame.clone())).ok();
    }
    for frame in encode_cli_stream_control_frames() {
        tx.send(Bytes::from(frame)).ok();
    }

    let body_stream = async_stream::stream! {
        let mut rx = rx;
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, std::io::Error>(chunk);
        }
    };
    // No overall request timeout: a bidi agent turn can run for minutes. Stalls
    // are bounded by the per-read idle timeout in the response loop below.
    let client = reqwest::Client::builder()
        .http2_adaptive_window(true)
        .build()?;
    let traceparent = traceparent();
    let response = client
        .post(format!("{}{}", config.endpoint, config.path))
        .bearer_auth(&request.access_token)
        .header("backend-traceparent", &traceparent)
        .header("connect-accept-encoding", "identity")
        .header("connect-protocol-version", "1")
        .header("content-type", "application/connect+proto")
        .header("traceparent", traceparent)
        .header("user-agent", "connect-es/1.6.1")
        .header("x-cursor-client-type", "cli")
        .header("x-cursor-client-version", config.client_version)
        .header("x-ghost-mode", "true")
        .body(reqwest::Body::wrap_stream(body_stream))
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Cursor AgentService returned HTTP {status}: {text}");
    }

    let workspace = request.workspace;
    let executor = request.tool_executor;
    // The model can think for a while between exec steps; allow generous idle.
    let idle = Duration::from_secs(120);
    let mut bytes_stream = response.bytes_stream();

    let events = async_stream::try_stream! {
        let mut buffer: Vec<u8> = Vec::new();
        let mut done = false;
        // Keep `tx` alive for the whole turn so the request body stays open.
        let outbound = tx;
        let mut visible_tokens = 0u32;

        // Periodic heartbeat so the server doesn't reset long turns (the model
        // can think for tens of seconds between/after tool calls).
        let heartbeat = {
            let hb_tx = outbound.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(8));
                tick.tick().await; // consume immediate first tick
                loop {
                    tick.tick().await;
                    if hb_tx
                        .send(Bytes::from(encode_connect_frame(&encode_heartbeat())))
                        .is_err()
                    {
                        break;
                    }
                }
            })
        };

        // The server emits empty keepalive frames, so a stalled turn never goes
        // silent. Bound the turn on lack of *meaningful* progress (text, exec, or
        // a turn-end signal) instead of only on raw stream silence.
        let no_progress_limit = Duration::from_secs(90);
        let mut last_progress = tokio::time::Instant::now();

        while !done {
            if last_progress.elapsed() > no_progress_limit {
                break;
            }
            let chunk = tokio::time::timeout(idle, bytes_stream.next()).await;
            let chunk = match chunk {
                Ok(Some(Ok(c))) => c,
                Ok(Some(Err(err))) => { Err(anyhow::anyhow!(err))?; break; }
                Ok(None) => break,            // stream ended
                Err(_) => break,              // idle timeout
            };
            buffer.extend_from_slice(&chunk);

            loop {
                let frame = match take_connect_frame(&mut buffer) {
                    Ok(Some(f)) => f,
                    Ok(None) => break,
                    Err(err) => { Err(err)?; done = true; break; }
                };
                match frame {
                    ConnectFrame::EndStream(error) => {
                        if let Some(error) = error {
                            Err(anyhow::anyhow!("Cursor AgentService end-stream error: {error}"))?;
                        }
                        done = true;
                        break;
                    }
                    ConnectFrame::Payload(payload) => {
                        crate::agentservice::capture_cursor_frame("recv", 0, &payload);
                        let frame = decode_server_frame(&payload);
                        // Ack kv_server PUTs so the server persists conversation
                        // state and continues the multi-step turn.
                        if let Some(kv_seq) = frame.kv_seq {
                            let _ = outbound.send(Bytes::from(encode_connect_frame(&encode_kv_ack(kv_seq))));
                        }
                        if !frame.thinking.is_empty() {
                            last_progress = tokio::time::Instant::now();
                            yield InferenceEvent::ReasoningDelta(ReasoningDelta { text: frame.thinking });
                        }
                        if !frame.text.is_empty() {
                            last_progress = tokio::time::Instant::now();
                            visible_tokens = visible_tokens.saturating_add((frame.text.len() / 4) as u32);
                            yield InferenceEvent::MessageDelta(MessageDelta { text: frame.text, phase: None });
                        }
                        if let Some(exec) = frame.exec {
                            last_progress = tokio::time::Instant::now();
                            if let Some((result_frames, send_ctrl)) =
                                service_exec(exec, &workspace, executor.as_deref()).await
                            {
                                for result_frame in &result_frames {
                                    crate::agentservice::capture_cursor_frame("send-result", 0, result_frame);
                                    let _ = outbound.send(Bytes::from(encode_connect_frame(result_frame)));
                                }
                                if send_ctrl {
                                    let _ = outbound.send(Bytes::from(encode_connect_frame(&encode_exec_control())));
                                }
                            }
                        }
                        if frame.turn_ended {
                            done = true;
                            break;
                        }
                    }
                }
            }
        }

        heartbeat.abort();
        yield InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("turn_ended".to_string()),
            provider_response_id: None,
        });
    };

    Ok(Box::pin(events))
}

/// Execute one exec request and return `(frames_to_send, send_exec_control)`.
/// Returns `None` to send nothing.  `send_exec_control` should be `true` only
/// for exec types that follow the normal request/result/ack cycle.
async fn service_exec(
    exec: CursorExecRequest,
    workspace: &Path,
    executor: Option<&dyn TurnToolExecutor>,
) -> Option<(Vec<Vec<u8>>, bool)> {
    match exec {
        // Server requests workspace context (field 10 = request_context_args)
        // before it starts generating model output.  Respond with the workspace
        // path so the model knows where to look for files.  No exec_control ack
        // is sent for this type — the model continues streaming after it gets
        // the context.
        CursorExecRequest::RequestContext { id } => {
            let ws = workspace.to_string_lossy().to_string();
            Some((vec![encode_exec_request_context_result(id, &ws)], false))
        }
        CursorExecRequest::Read {
            seq,
            path,
            tool_call_id,
        } => {
            // Surface the read in Roder's tool timeline/transcript via the
            // executor (result ignored), but answer Cursor with the raw file
            // bytes it expects.
            if let Some(exec) = executor {
                let _ = exec
                    .execute(ToolCallCompleted {
                        id: tool_call_id,
                        name: "read_file".to_string(),
                        arguments: json!({ "path": &path }).to_string(),
                    })
                    .await;
            }
            let content = tokio::fs::read(&path).await.unwrap_or_default();
            let total_lines = content.iter().filter(|&&b| b == b'\n').count() as u64 + 1;
            Some((vec![encode_exec_read_result(
                seq,
                &path,
                &content,
                total_lines,
            )], true))
        }
        CursorExecRequest::Write {
            seq,
            path,
            content,
            tool_call_id,
        } => {
            let text = String::from_utf8_lossy(&content).to_string();
            let lines = content.iter().filter(|&&b| b == b'\n').count() as u64;
            let size = content.len() as u64;
            // Prefer routing through Roder's policy/registry; fall back to direct write.
            let applied = if let Some(exec) = executor {
                exec.execute(ToolCallCompleted {
                    id: tool_call_id,
                    name: "write_file".to_string(),
                    arguments: json!({ "path": &path, "content": text }).to_string(),
                })
                .await
                .map(|o| !o.is_error)
                .unwrap_or(false)
            } else {
                tokio::fs::write(&path, &content).await.is_ok()
            };
            if !applied {
                let _ = tokio::fs::write(&path, &content).await;
            }
            Some((vec![encode_exec_write_result(seq, &path, lines, size)], true))
        }
        CursorExecRequest::Shell {
            seq,
            command,
            cwd,
            tool_call_id,
        } => {
            let stdout = if let Some(exec) = executor {
                match exec
                    .execute(ToolCallCompleted {
                        id: tool_call_id,
                        name: "shell".to_string(),
                        arguments: json!({ "command": &command, "workdir": &cwd }).to_string(),
                    })
                    .await
                {
                    Ok(outcome) => outcome.result,
                    Err(err) => format!("error: {err}"),
                }
            } else {
                run_shell(&command, &cwd, workspace).await
            };
            Some((encode_exec_shell_results(seq, &cwd, &stdout), true))
        }
        CursorExecRequest::Search {
            seq,
            pattern,
            path,
            glob,
            mode,
            tool_call_id,
        } => {
            if let Some(exec) = executor {
                let (name, arguments) = if mode == "files_with_matches" {
                    (
                        "glob",
                        json!({
                            "pattern": glob.as_ref().or(pattern.as_ref()).map(String::as_str).unwrap_or("**/*"),
                        }),
                    )
                } else {
                    (
                        "grep",
                        json!({
                            "query": pattern.as_deref().unwrap_or_default(),
                            "path": if path.is_empty() { "." } else { path.as_str() },
                            "regex": false,
                            "case_sensitive": true,
                            "word_boundary": false,
                            "mode": "auto",
                        }),
                    )
                };
                let _ = exec
                    .execute(ToolCallCompleted {
                        id: tool_call_id,
                        name: name.to_string(),
                        arguments: arguments.to_string(),
                    })
                    .await;
            }
            let root = workspace.to_string_lossy().to_string();
            let search_dir = if path.is_empty() {
                root.clone()
            } else {
                path.clone()
            };
            if mode == "files_with_matches" {
                // glob: list files under search_dir matching the glob pattern.
                let glob_pat = glob
                    .or_else(|| pattern.clone())
                    .unwrap_or_else(|| "**/*".to_string());
                let rel = tokio::task::spawn_blocking(move || {
                    glob_files(&search_dir, &root, &glob_pat, 500)
                })
                .await
                .unwrap_or_default();
                let root2 = workspace.to_string_lossy().to_string();
                Some((vec![encode_exec_glob_result(seq, &path, &root2, &rel)], true))
            } else {
                // content (grep): search files under search_dir for the pattern.
                let needle = pattern.clone().unwrap_or_default();
                let glob_pat = glob.clone();
                let root_c = root.clone();
                let matches = tokio::task::spawn_blocking(move || {
                    grep_files(&search_dir, &root_c, &needle, glob_pat.as_deref(), 300)
                })
                .await
                .unwrap_or_default();
                Some((vec![encode_exec_grep_result(
                    seq,
                    &pattern.unwrap_or_default(),
                    &path,
                    &root,
                    &matches,
                )], true))
            }
        }
    }
}

/// Recursively collect files under `dir`, returning paths relative to `root`,
/// matching `glob` (supports `*` and `**`). Skips noisy dirs; caps at `limit`.
fn glob_files(dir: &str, root: &str, glob: &str, limit: usize) -> Vec<String> {
    let root_path = std::path::Path::new(root);
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if matches!(name.as_str(), ".git" | "target" | "node_modules" | ".jj") {
                    continue;
                }
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(root_path) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if glob_match(&rel, glob) {
                    out.push(rel);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Search files under `dir` for the literal `needle`, returning matches with
/// paths relative to `root`. Caps at `limit`.
fn grep_files(
    dir: &str,
    root: &str,
    needle: &str,
    glob: Option<&str>,
    limit: usize,
) -> Vec<CursorGrepMatch> {
    if needle.is_empty() {
        return Vec::new();
    }
    let root_path = std::path::Path::new(root);
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if matches!(name.as_str(), ".git" | "target" | "node_modules" | ".jj") {
                    continue;
                }
                stack.push(p);
                continue;
            }
            let Ok(rel) = p.strip_prefix(root_path) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if let Some(g) = glob
                && !glob_match(&rel, g)
            {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&p) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if line.contains(needle) {
                    out.push(CursorGrepMatch {
                        path: rel.clone(),
                        line: (i + 1) as u64,
                        text: line.chars().take(400).collect(),
                    });
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// Minimal glob matcher supporting `*` (within a path segment) and `**`
/// (across segments).
fn glob_match(path: &str, pat: &str) -> bool {
    let p: Vec<&str> = path.split('/').collect();
    let g: Vec<&str> = pat.split('/').collect();
    fn seg(s: &str, pat: &str) -> bool {
        let parts: Vec<&str> = pat.split('*').collect();
        if parts.len() == 1 {
            return s == pat;
        }
        let mut idx = 0usize;
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if i == 0 {
                if !s[idx..].starts_with(part) {
                    return false;
                }
                idx += part.len();
            } else if i == parts.len() - 1 {
                if !s[idx..].ends_with(part) {
                    return false;
                }
            } else if let Some(pos) = s[idx..].find(part) {
                idx += pos + part.len();
            } else {
                return false;
            }
        }
        true
    }
    fn m(p: &[&str], g: &[&str]) -> bool {
        if g.is_empty() {
            return p.is_empty();
        }
        if g[0] == "**" {
            for i in 0..=p.len() {
                if m(&p[i..], &g[1..]) {
                    return true;
                }
            }
            return false;
        }
        if p.is_empty() {
            return false;
        }
        if seg(p[0], g[0]) {
            return m(&p[1..], &g[1..]);
        }
        false
    }
    m(&p, &g)
}

async fn run_shell(command: &str, cwd: &str, workspace: &Path) -> String {
    let dir = if cwd.is_empty() {
        workspace.to_path_buf()
    } else {
        PathBuf::from(cwd)
    };
    match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&dir)
        .output()
        .await
    {
        Ok(out) => {
            let mut s = String::from_utf8_lossy(&out.stdout).to_string();
            if !out.stderr.is_empty() {
                s.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            s
        }
        Err(err) => format!("shell error: {err}"),
    }
}

fn traceparent() -> String {
    let trace_id = uuid::Uuid::new_v4().simple().to_string();
    let span_id = &uuid::Uuid::new_v4().simple().to_string()[..16];
    format!("00-{trace_id}-{span_id}-01")
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn glob_match_handles_double_star_and_extensions() {
        assert!(glob_match("src/a.rs", "**/*.rs"));
        assert!(glob_match("src/nested/deep/b.rs", "**/*.rs"));
        assert!(glob_match("a.rs", "**/*.rs"));
        assert!(!glob_match("src/a.txt", "**/*.rs"));
        assert!(glob_match("src/main.rs", "src/*.rs"));
        assert!(!glob_match("src/nested/main.rs", "src/*.rs"));
        assert!(glob_match("crates/roder-tui/src/app.rs", "**/app.rs"));
        assert!(glob_match("foo/bar_test.rs", "**/*_test.rs"));
        assert!(!glob_match("foo/bar.rs", "**/*_test.rs"));
    }
}
