//! Child process lifecycle and newline-delimited JSON-RPC stdio plumbing.
//!
//! The host owns request ids. Child stdout carries JSON-RPC responses and
//! notifications (`inference/event`, `extension/event`); stderr is captured
//! as diagnostics. Children are spawned lazily on first use with an
//! explicit env allowlist and a startup timeout, and shut down gracefully
//! via `extension/shutdown` before being killed.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
use roder_api::inference::InferenceEvent;
use roder_api::process_extension::{
    METHOD_INITIALIZE, METHOD_SHUTDOWN, PROCESS_EXTENSION_PROTOCOL_VERSION,
    ProcessExtensionOwnedEvent, ProcessInferenceEventNotification, ProcessInitializeParams,
    ProcessInitializeResult, validate_initialize_echo,
};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::manifest::LoadedProcessExtension;

/// Maximum accepted child stdout line (defends against runaway children).
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
/// Cap on buffered extension-owned events awaiting collection.
const MAX_BUFFERED_EXTENSION_EVENTS: usize = 256;
/// Per-request timeout for child responses after initialization.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ProcessHost {
    loaded: LoadedProcessExtension,
    state: Mutex<Option<Arc<RunningChild>>>,
}

struct RunningChild {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>>,
    streams: Mutex<HashMap<String, mpsc::Sender<anyhow::Result<InferenceEvent>>>>,
    extension_events: Mutex<Vec<ProcessExtensionOwnedEvent>>,
    next_id: AtomicU64,
}

impl ProcessHost {
    pub fn new(loaded: LoadedProcessExtension) -> Self {
        Self {
            loaded,
            state: Mutex::new(None),
        }
    }

    pub fn loaded(&self) -> &LoadedProcessExtension {
        &self.loaded
    }

    /// Sends a request and decodes the typed result, spawning and
    /// initializing the child first if needed.
    pub async fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<T> {
        let child = self.ensure_started().await?;
        let value = tokio::time::timeout(
            REQUEST_TIMEOUT,
            request_on(&child, method, params, REQUEST_TIMEOUT),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "process extension {} timed out answering {method}",
                self.loaded.manifest.id
            )
        })??;
        Ok(serde_json::from_value(value).with_context(|| {
            format!(
                "process extension {} returned a malformed {method} result",
                self.loaded.manifest.id
            )
        })?)
    }

    /// Sends a notification (no response expected).
    pub async fn notify(&self, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
        let child = self.ensure_started().await?;
        write_message(
            &child,
            serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
        .await
    }

    /// Registers an inference stream receiver for `stream_id`.
    pub async fn register_stream(
        &self,
        stream_id: String,
    ) -> anyhow::Result<mpsc::Receiver<anyhow::Result<InferenceEvent>>> {
        let child = self.ensure_started().await?;
        let (tx, rx) = mpsc::channel(256);
        child.streams.lock().await.insert(stream_id, tx);
        Ok(rx)
    }

    /// Drains extension-owned events the child emitted since the last call.
    pub async fn drain_extension_events(&self) -> Vec<ProcessExtensionOwnedEvent> {
        let Some(child) = self.state.lock().await.clone() else {
            return Vec::new();
        };
        std::mem::take(&mut *child.extension_events.lock().await)
    }

    /// Gracefully shuts the child down (`extension/shutdown`, then kill).
    pub async fn shutdown(&self) {
        let Some(child) = self.state.lock().await.take() else {
            return;
        };
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            request_on(
                &child,
                METHOD_SHUTDOWN,
                serde_json::json!({}),
                Duration::from_secs(2),
            ),
        )
        .await;
        let _ = child.child.lock().await.kill().await;
    }

    async fn ensure_started(&self) -> anyhow::Result<Arc<RunningChild>> {
        let mut state = self.state.lock().await;
        if let Some(child) = state.as_ref() {
            return Ok(child.clone());
        }
        let startup = Duration::from_millis(self.loaded.config.startup_timeout_ms.max(1));
        let child = tokio::time::timeout(startup, self.spawn_and_initialize())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "process extension {} did not initialize within {}ms",
                    self.loaded.manifest.id,
                    startup.as_millis()
                )
            })??;
        *state = Some(child.clone());
        Ok(child)
    }

    async fn spawn_and_initialize(&self) -> anyhow::Result<Arc<RunningChild>> {
        let config = &self.loaded.config;
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Explicit allowlist only: never inherit the host environment
            // wholesale into extension children.
            .env_clear()
            .envs(&config.env);
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().with_context(|| {
            format!(
                "spawn process extension {} ({})",
                self.loaded.manifest.id, config.command
            )
        })?;

        let stdin = child.stdin.take().context("child stdin unavailable")?;
        let stdout = child.stdout.take().context("child stdout unavailable")?;
        let stderr = child.stderr.take().context("child stderr unavailable")?;

        let running = Arc::new(RunningChild {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            extension_events: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        });

        let extension_id = self.loaded.manifest.id.clone();
        tokio::spawn(read_stdout(running.clone(), stdout, extension_id.clone()));
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("[process-ext {extension_id}] {line}");
            }
        });

        let params = ProcessInitializeParams {
            protocol_version: PROCESS_EXTENSION_PROTOCOL_VERSION.to_string(),
            api_version: roder_api::extension::SUPPORTED_EXTENSION_API_VERSION.to_string(),
            extension_id: self.loaded.manifest.id.clone(),
            cwd: self
                .loaded
                .config
                .cwd
                .clone()
                .unwrap_or_else(|| ".".to_string()),
            granted_capabilities: self.loaded.manifest.required_capabilities.clone(),
            config: serde_json::json!({}),
            event_filter: self.loaded.config.event_filter.clone(),
        };
        let result = request_on(
            &running,
            METHOD_INITIALIZE,
            serde_json::to_value(&params)?,
            REQUEST_TIMEOUT,
        )
        .await?;
        let initialized: ProcessInitializeResult = serde_json::from_value(result)
            .context("process extension returned a malformed initialize result")?;
        validate_initialize_echo(
            &self.loaded.manifest,
            &self.loaded.manifest_toml,
            &initialized,
        )?;
        Ok(running)
    }
}

async fn request_on(
    child: &Arc<RunningChild>,
    method: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let id = child.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    child.pending.lock().await.insert(id, tx);
    write_message(
        child,
        serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    )
    .await?;
    let response = tokio::time::timeout(timeout, rx)
        .await
        .map_err(|_| anyhow::anyhow!("process extension timed out answering {method}"))?
        .map_err(|_| anyhow::anyhow!("process extension exited while answering {method}"))?;
    response.map_err(|message| anyhow::anyhow!("{method} failed: {message}"))
}

async fn write_message(child: &Arc<RunningChild>, message: serde_json::Value) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(&message)?;
    line.push('\n');
    let mut stdin = child.stdin.lock().await;
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_stdout(
    child: Arc<RunningChild>,
    stdout: tokio::process::ChildStdout,
    extension_id: String,
) {
    let mut reader = BufReader::new(stdout);
    let mut buffer = String::new();
    loop {
        buffer.clear();
        // take() caps the line size so a runaway child cannot exhaust memory.
        let read = (&mut reader)
            .take(MAX_LINE_BYTES as u64)
            .read_line(&mut buffer)
            .await;
        match read {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                eprintln!("[process-ext {extension_id}] stdout read failed: {err}");
                break;
            }
        }
        let line = buffer.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            eprintln!("[process-ext {extension_id}] dropped non-JSON stdout line");
            continue;
        };
        dispatch_message(&child, message).await;
    }
    // Child stdout closed: fail any pending requests and active streams.
    for (_, tx) in child.pending.lock().await.drain() {
        let _ = tx.send(Err("process extension exited".to_string()));
    }
    for (_, tx) in child.streams.lock().await.drain() {
        let _ = tx
            .send(Err(anyhow::anyhow!("process extension exited mid-stream")))
            .await;
    }
}

async fn dispatch_message(child: &Arc<RunningChild>, message: serde_json::Value) {
    if let Some(id) = message.get("id").and_then(serde_json::Value::as_u64) {
        let outcome = if let Some(error) = message.get("error") {
            Err(error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown process extension error")
                .to_string())
        } else {
            Ok(message.get("result").cloned().unwrap_or(serde_json::Value::Null))
        };
        if let Some(tx) = child.pending.lock().await.remove(&id) {
            let _ = tx.send(outcome);
        }
        return;
    }
    let Some(method) = message.get("method").and_then(serde_json::Value::as_str) else {
        return;
    };
    let params = message.get("params").cloned().unwrap_or(serde_json::Value::Null);
    match method {
        roder_api::process_extension::METHOD_INFERENCE_EVENT => {
            let Ok(notification) =
                serde_json::from_value::<ProcessInferenceEventNotification>(params)
            else {
                return;
            };
            let terminal = matches!(
                notification.event,
                InferenceEvent::Completed(_) | InferenceEvent::Failed(_)
            );
            let mut streams = child.streams.lock().await;
            if let Some(tx) = streams.get(&notification.stream_id) {
                let _ = tx.send(Ok(notification.event)).await;
            }
            if terminal {
                streams.remove(&notification.stream_id);
            }
        }
        roder_api::process_extension::METHOD_EXTENSION_EVENT => {
            let Ok(event) = serde_json::from_value::<ProcessExtensionOwnedEvent>(params) else {
                return;
            };
            let mut events = child.extension_events.lock().await;
            if events.len() < MAX_BUFFERED_EXTENSION_EVENTS {
                events.push(event);
            }
        }
        _ => {}
    }
}
