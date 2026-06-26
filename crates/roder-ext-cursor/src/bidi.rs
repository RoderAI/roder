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
    TokenUsage, ToolCallCompleted, TurnToolExecutor,
};
use serde_json::json;

use crate::agentservice::AgentServiceConfig;
use crate::proto::{
    ConnectFrame, CursorExecRequest, CursorGrepMatch, decode_server_frame,
    encode_cli_stream_control_frames, encode_connect_frame, encode_exec_control,
    encode_exec_glob_result, encode_exec_grep_result, encode_exec_read_result,
    encode_exec_request_context_result, encode_exec_shell_results, encode_exec_unknown_result,
    encode_exec_write_result, encode_heartbeat, encode_kv_ack, take_connect_frame,
};

pub struct BidiRequest {
    pub access_token: String,
    /// Pre-encoded `AgentClientMessage{run_request}` payload (not connect-framed).
    pub run_request: Vec<u8>,
    /// Already connect-framed request-context frames (workspace context).
    pub context_frames: Vec<Vec<u8>>,
    pub workspace: PathBuf,
    pub tool_executor: Option<Arc<dyn TurnToolExecutor>>,
    pub usage_metadata: BidiUsageMetadata,
}

#[derive(Debug, Clone, Default)]
pub struct BidiUsageMetadata {
    pub prompt_tokens: u32,
    pub provider: String,
    pub transport: String,
    pub auth_source: String,
    pub thread_id: String,
    pub turn_id: String,
    pub model: String,
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
    let mut builder = reqwest::Client::builder().http2_adaptive_window(true);
    // Plaintext endpoints (local mock AgentService in tests / diagnostics)
    // cannot negotiate HTTP/2 via ALPN; force it so the same-stream bidi body
    // works. The production endpoint is always https and unaffected.
    if config.endpoint.starts_with("http://") {
        builder = builder.http2_prior_knowledge();
    }
    let client = builder.build()?;
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
    let usage_metadata = request.usage_metadata;
    // The model can think for a while between exec steps; allow generous idle.
    let idle = Duration::from_secs(120);
    let mut bytes_stream = response.bytes_stream();

    let events = async_stream::try_stream! {
        let mut buffer: Vec<u8> = Vec::new();
        let mut done = false;
        // Keep `tx` alive for the whole turn so the request body stays open.
        let outbound = tx;
        let mut visible_tokens = 0u32;
        let mut thinking_tokens = 0u32;
        let mut usage_fields = serde_json::Map::new();

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
                        for (field, value) in frame.usage_fields {
                            usage_fields.insert(format!("field_{field}"), json!(value));
                        }
                        if !frame.thinking.is_empty() {
                            last_progress = tokio::time::Instant::now();
                            thinking_tokens = thinking_tokens.saturating_add(estimate_text_tokens(&frame.thinking));
                            yield InferenceEvent::ReasoningDelta(ReasoningDelta { text: frame.thinking });
                        }
                        if !frame.text.is_empty() {
                            last_progress = tokio::time::Instant::now();
                            visible_tokens = visible_tokens.saturating_add(estimate_text_tokens(&frame.text));
                            yield InferenceEvent::MessageDelta(MessageDelta { text: frame.text, phase: None });
                        }
                        if let Some(exec) = frame.exec {
                            if let Some((result_frames, ack_seq)) =
                                service_exec(exec, &workspace, executor.as_deref()).await
                            {
                                for result_frame in &result_frames {
                                    crate::agentservice::capture_cursor_frame("send-result", 0, result_frame);
                                    let _ = outbound.send(Bytes::from(encode_connect_frame(result_frame)));
                                }
                                if let Some(seq) = ack_seq {
                                    let _ = outbound.send(Bytes::from(encode_connect_frame(&encode_exec_control(seq))));
                                }
                            }
                            // Servicing the exec (which can include a slow tool
                            // run or a user approval wait) is itself progress;
                            // without this reset a tool that takes longer than
                            // the no-progress cap ends the turn right after its
                            // result is sent, before the model can continue.
                            last_progress = tokio::time::Instant::now();
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
        let completion_tokens = visible_tokens.saturating_add(thinking_tokens);
        let total_tokens = usage_metadata
            .prompt_tokens
            .saturating_add(completion_tokens);
        yield InferenceEvent::Usage(TokenUsage::new(
            usage_metadata.prompt_tokens,
            completion_tokens,
            total_tokens,
        ));
        yield InferenceEvent::ProviderMetadata(json!({
            "provider": usage_metadata.provider,
            "transport": usage_metadata.transport,
            "authSource": usage_metadata.auth_source,
            "threadId": usage_metadata.thread_id,
            "turnId": usage_metadata.turn_id,
            "model": usage_metadata.model,
            "usage": {
                "input_tokens": usage_metadata.prompt_tokens,
                "output_tokens": completion_tokens,
                "total_tokens": total_tokens,
                "output_tokens_details": {
                    "reasoning_tokens": thinking_tokens,
                    "visible_output_tokens": visible_tokens
                }
            },
            "usageFields": usage_fields,
            "usageFieldsSource": "cursor-agentservice-turn-end",
            "usageEstimated": true,
            "usageSource": "chars_per_4",
        }));
        yield InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("turn_ended".to_string()),
            provider_response_id: None,
        });
    };

    Ok(Box::pin(events))
}

fn estimate_text_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Execute one exec request and return `(frames_to_send, ack_seq)`.
/// Returns `None` to send nothing. `ack_seq` is the exec seq to echo in the
/// `exec_client_control_message` ack — required so the server knows the exec
/// finished and resumes the model (an ack without the seq leaves streamed
/// results, e.g. shell, looking unfinished). `None` for exec types that do not
/// follow the request/result/ack cycle (request-context).
async fn service_exec(
    exec: CursorExecRequest,
    workspace: &Path,
    executor: Option<&dyn TurnToolExecutor>,
) -> Option<(Vec<Vec<u8>>, Option<u64>)> {
    match exec {
        // Server requests workspace context (field 10 = request_context_args)
        // before it starts generating model output.  Respond with the workspace
        // path so the model knows where to look for files.  No exec_control ack
        // is sent for this type — the model continues streaming after it gets
        // the context.
        CursorExecRequest::RequestContext { id } => {
            let ws = workspace.to_string_lossy().to_string();
            Some((vec![encode_exec_request_context_result(id, &ws)], None))
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
            Some((
                vec![encode_exec_read_result(seq, &path, &content, total_lines)],
                Some(seq),
            ))
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
            Some((
                vec![encode_exec_write_result(seq, &path, lines, size)],
                Some(seq),
            ))
        }
        CursorExecRequest::Delete {
            seq,
            path,
            tool_call_id,
        } => {
            // Route through Roder's policy-gated shell tool so the deletion is
            // approval-gated and visible in the timeline; only fall back to a
            // direct removal when no executor is available.
            if let Some(exec) = executor {
                let quoted = format!("'{}'", path.replace('\'', "'\\''"));
                let _ = exec
                    .execute(ToolCallCompleted {
                        id: tool_call_id,
                        name: "shell".to_string(),
                        arguments: json!({ "command": format!("rm -f -- {quoted}") }).to_string(),
                    })
                    .await;
            } else {
                let _ = tokio::fs::remove_file(&path).await;
            }
            // Result body shape is unconfirmed; a mirrored empty result keeps
            // the stream healthy (verified live) and the model can verify the
            // deletion itself.
            Some((vec![encode_exec_unknown_result(seq, 4)], Some(seq)))
        }
        CursorExecRequest::Shell {
            seq,
            command,
            cwd,
            tool_call_id,
        } => {
            let started = std::time::Instant::now();
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
            let duration_ms = started.elapsed().as_millis() as u64;
            // cursor-agent always reports a real working directory in the exit
            // message; an empty cwd does not match the capture shape.
            let exit_cwd = if cwd.is_empty() {
                workspace.to_string_lossy().to_string()
            } else {
                cwd.clone()
            };
            Some((
                encode_exec_shell_results(seq, &exit_cwd, &stdout, duration_ms),
                Some(seq),
            ))
        }
        CursorExecRequest::Search {
            seq,
            pattern,
            path,
            glob,
            mode,
            tool_call_id,
        } => {
            // Cursor's unified search tool overloads one request shape, and the
            // intent must be read from *which* pattern field is set, not from
            // `mode`:
            //   - a content regex in `pattern` (f1) is a GREP (ripgrep). `mode`
            //     only selects the result shape: `content` = line matches,
            //     `files_with_matches` = the distinct files that matched (rg -l).
            //   - a path pattern in `glob` (f3), with no content pattern, is a
            //     GLOB (list files whose path matches).
            // Composer models send globs via `glob` + mode=files_with_matches;
            // Claude models send grep-for-filenames via `pattern` + mode=
            // files_with_matches. Routing on `mode` alone fed the grep regex
            // into the path-glob matcher, so it matched no files and every
            // Claude search returned empty.
            let content_needle = pattern
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let path_glob = glob
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let is_grep = content_needle.is_some();
            let want_files_only = mode == "files_with_matches";

            if let Some(exec) = executor {
                let (name, arguments) = if is_grep {
                    (
                        "grep",
                        json!({
                            "query": content_needle.clone().unwrap_or_default(),
                            "path": if path.is_empty() { "." } else { path.as_str() },
                            "regex": false,
                            "case_sensitive": true,
                            "word_boundary": false,
                            "mode": "auto",
                        }),
                    )
                } else {
                    (
                        "glob",
                        json!({
                            "pattern": path_glob.clone().unwrap_or_else(|| "**/*".to_string()),
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

            if is_grep {
                // Content grep. The result shape must match the mode the server
                // asked for so Cursor can parse it.
                let needle = content_needle.unwrap_or_default();
                let glob_filter = path_glob.clone();
                let root_c = root.clone();
                let search_dir_c = search_dir.clone();
                let matches = tokio::task::spawn_blocking(move || {
                    grep_files(&search_dir_c, &root_c, &needle, glob_filter.as_deref(), 300)
                })
                .await
                .unwrap_or_default();
                if want_files_only {
                    // rg -l: return the distinct files that matched, encoded in
                    // the files_with_matches result shape.
                    let mut files = Vec::new();
                    for m in &matches {
                        if !files.contains(&m.path) {
                            files.push(m.path.clone());
                        }
                    }
                    Some((
                        vec![encode_exec_glob_result(seq, &path, &root, &files)],
                        Some(seq),
                    ))
                } else {
                    Some((
                        vec![encode_exec_grep_result(
                            seq,
                            pattern.as_deref().unwrap_or_default(),
                            &path,
                            &root,
                            &matches,
                        )],
                        Some(seq),
                    ))
                }
            } else {
                // Pure path glob: list files under search_dir whose path matches.
                let glob_pat = path_glob.unwrap_or_else(|| "**/*".to_string());
                let root_c = root.clone();
                let search_dir_c = search_dir.clone();
                let rel = tokio::task::spawn_blocking(move || {
                    glob_files(&search_dir_c, &root_c, &glob_pat, 500)
                })
                .await
                .unwrap_or_default();
                Some((
                    vec![encode_exec_glob_result(seq, &path, &root, &rel)],
                    Some(seq),
                ))
            }
        }
        CursorExecRequest::Unknown {
            seq,
            field_no,
            payload,
        } => {
            // Surface the unimplemented exec in Roder's tool timeline so the
            // user sees *which* Cursor-native tool was requested instead of a
            // silently stalled turn (the registry rejects the unknown name and
            // records an error result; nothing blocks).
            if let Some(exec) = executor {
                let strings = crate::proto::collect_payload_strings(&payload);
                let _ = exec
                    .execute(ToolCallCompleted {
                        id: format!("cursor-exec-{seq}"),
                        name: "cursor_unsupported_tool".to_string(),
                        arguments: json!({
                            "reason": "unsupported_cursor_exec_request",
                            "exec_field_no": field_no,
                            "strings": strings,
                        })
                        .to_string(),
                    })
                    .await;
            }
            // Reply with a mirrored empty result so the server does not wait
            // forever on the call; an explicit (if empty) answer lets the model
            // move on rather than hanging the whole turn.
            Some((vec![encode_exec_unknown_result(seq, field_no)], Some(seq)))
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
#[path = "bidi_stream_tests.rs"]
mod bidi_stream_tests;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use roder_api::inference::{ToolCallCompleted, TurnToolExecutor, TurnToolOutcome};

    use super::{glob_match, service_exec};
    use crate::proto::CursorExecRequest;

    fn frames_contain(frames: &[Vec<u8>], needle: &[u8]) -> bool {
        frames
            .iter()
            .any(|frame| frame.windows(needle.len()).any(|w| w == needle))
    }

    #[derive(Default)]
    struct RecordingExecutor {
        calls: Mutex<Vec<ToolCallCompleted>>,
    }

    #[async_trait::async_trait]
    impl TurnToolExecutor for RecordingExecutor {
        async fn execute(&self, call: ToolCallCompleted) -> anyhow::Result<TurnToolOutcome> {
            self.calls.lock().unwrap().push(call);
            Ok(TurnToolOutcome {
                result: "unknown tool".to_string(),
                is_error: true,
            })
        }
    }

    /// Composer's native delete tool (exec field 4) must route through Roder's
    /// policy-gated shell tool, with the path shell-quoted, and mirror a
    /// result so the stream keeps moving.
    #[tokio::test]
    async fn delete_exec_routes_policy_gated_rm_and_replies() {
        let executor = Arc::new(RecordingExecutor::default());
        let (frames, ack) = service_exec(
            CursorExecRequest::Delete {
                seq: 9,
                path: "/tmp/it's a file.txt".to_string(),
                tool_call_id: "tool_del_1".to_string(),
            },
            std::path::Path::new("/tmp"),
            Some(executor.as_ref()),
        )
        .await
        .expect("delete exec must produce a reply");

        assert_eq!(ack, Some(9));
        assert_eq!(frames.len(), 1);
        let calls = executor.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
        assert_eq!(calls[0].id, "tool_del_1");
        let args: serde_json::Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["command"], r#"rm -f -- '/tmp/it'\''s a file.txt'"#);
    }

    /// An exec request Roder cannot service must still produce a mirrored
    /// result frame (so the server does not wait forever — the composer-2.5
    /// "stuck on tool calls" failure) and surface a `cursor_unsupported_tool`
    /// call in the timeline for visibility.
    #[tokio::test]
    async fn unknown_exec_request_gets_a_mirrored_reply_and_timeline_entry() {
        let executor = Arc::new(RecordingExecutor::default());
        let (frames, ack) = service_exec(
            CursorExecRequest::Unknown {
                seq: 51,
                field_no: 21,
                payload: crate::proto::proto_message(vec![crate::proto::proto_field_string(
                    1,
                    "mystery/arg",
                )]),
            },
            std::path::Path::new("/tmp"),
            Some(executor.as_ref()),
        )
        .await
        .expect("unknown exec must produce a reply");

        assert_eq!(
            ack,
            Some(51),
            "unknown exec follows the request/result/ack cycle"
        );
        assert_eq!(frames.len(), 1);
        let calls = executor.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "cursor_unsupported_tool");
        assert_eq!(calls[0].id, "cursor-exec-51");
        assert!(calls[0].arguments.contains("\"exec_field_no\":21"));
        assert!(calls[0].arguments.contains("mystery/arg"));
    }

    /// Claude/Opus searches send the ripgrep regex in `pattern` (f1) with
    /// `mode = files_with_matches` and no path `glob` (f3). The old router keyed
    /// on `mode` and treated the regex as a path glob, so every Claude search
    /// returned zero files. It must instead grep file *contents* and return the
    /// distinct matching files.
    #[tokio::test]
    async fn files_with_matches_greps_content_when_only_a_pattern_is_set() {
        let dir = std::env::temp_dir().join(format!("cursor-search-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/hit.rs"), "fn encode_exec_read_result() {}\n").unwrap();
        std::fs::write(dir.join("src/miss.rs"), "fn unrelated() {}\n").unwrap();

        let (frames, ack) = service_exec(
            CursorExecRequest::Search {
                seq: 7,
                pattern: Some("encode_exec_read_result".to_string()),
                path: String::new(),
                glob: None,
                mode: "files_with_matches".to_string(),
                tool_call_id: "toolu_x".to_string(),
            },
            &dir,
            None,
        )
        .await
        .expect("search produces a result frame");

        assert_eq!(ack, Some(7));
        assert!(
            frames_contain(&frames, b"src/hit.rs"),
            "files_with_matches must return the file that contains the pattern"
        );
        assert!(
            !frames_contain(&frames, b"src/miss.rs"),
            "non-matching files must not be returned"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Composer-style path globs (pattern in `glob`/f3, no content pattern) must
    /// still list files purely by path.
    #[tokio::test]
    async fn files_with_matches_globs_by_path_when_glob_is_set() {
        let dir = std::env::temp_dir().join(format!("cursor-glob-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/app.rs"), "// nothing relevant\n").unwrap();
        std::fs::write(dir.join("src/app.txt"), "// nothing relevant\n").unwrap();

        let (frames, _) = service_exec(
            CursorExecRequest::Search {
                seq: 9,
                pattern: None,
                path: String::new(),
                glob: Some("**/*.rs".to_string()),
                mode: "files_with_matches".to_string(),
                tool_call_id: "toolu_y".to_string(),
            },
            &dir,
            None,
        )
        .await
        .expect("glob produces a result frame");

        assert!(frames_contain(&frames, b"src/app.rs"));
        assert!(!frames_contain(&frames, b"src/app.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
