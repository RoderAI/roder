//! MCP client over stdio: newline-delimited JSON-RPC with a local server
//! process that this client owns.
//!
//! The client spawns the server (see [`crate::stdio_process`]), runs the
//! `initialize` handshake, and multiplexes concurrent requests by id. The
//! server's stderr is kept as a short, redacted tail for error messages. When
//! the client is shut down or dropped, the server's whole process group is
//! stopped.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Mutex, oneshot};

use crate::client::{
    MCP_PROTOCOL_VERSION, McpToolDescriptor, McpToolOutcome, render_content_text, rpc_result,
};
use crate::stdio_process::{self, McpStdioServerConfig, redact_secrets};

const STDERR_TAIL_LINES: usize = 40;

type Pending = Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>>;
type SharedStdin = Arc<Mutex<Option<ChildStdin>>>;

/// A running stdio MCP server and the client connected to it.
pub struct McpStdioClient {
    config: McpStdioServerConfig,
    stdin: SharedStdin,
    child: Mutex<Option<Child>>,
    pid: Option<u32>,
    next_id: AtomicI64,
    pending: Pending,
    stderr_tail: Arc<StdMutex<VecDeque<String>>>,
    exited: Arc<AtomicBool>,
}

impl std::fmt::Debug for McpStdioClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpStdioClient")
            .field("config", &self.config)
            .field("pid", &self.pid)
            .finish()
    }
}

impl McpStdioClient {
    /// Spawns the server and completes the MCP `initialize` handshake.
    pub async fn start(config: McpStdioServerConfig) -> anyhow::Result<Self> {
        let mut child = stdio_process::spawn_server(&config)?;
        let pid = child.id();
        let stdin = child.stdin.take().context("MCP server stdin")?;
        let stdout = child.stdout.take().context("MCP server stdout")?;
        let stderr = child.stderr.take().context("MCP server stderr")?;

        let pending: Pending = Arc::default();
        let stderr_tail = Arc::new(StdMutex::new(VecDeque::new()));
        let exited = Arc::new(AtomicBool::new(false));
        let stdin: SharedStdin = Arc::new(Mutex::new(Some(stdin)));

        let stderr_task = tokio::spawn(read_stderr(stderr, stderr_tail.clone()));
        tokio::spawn(read_stdout(
            stdout,
            pending.clone(),
            exited.clone(),
            stdin.clone(),
        ));
        let client = Self {
            config,
            stdin,
            child: Mutex::new(Some(child)),
            pid,
            next_id: AtomicI64::new(1),
            pending: pending.clone(),
            stderr_tail,
            exited: exited.clone(),
        };
        let timeout = client.config.startup_timeout;
        let initialized = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "roder-ext-mcp", "version": env!("CARGO_PKG_VERSION") }
                }),
                timeout,
            )
            .await;
        if let Err(error) = initialized {
            client.shutdown().await;
            // Let the stderr reader drain so the error carries the server's
            // own explanation.
            let _ = tokio::time::timeout(Duration::from_secs(2), stderr_task).await;
            anyhow::bail!(client.failure(&format!("could not start: {error:#}")));
        }
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    pub fn config(&self) -> &McpStdioServerConfig {
        &self.config
    }

    /// Whether the server process has closed its stdout (exited or crashed).
    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub async fn list_tools(&self, timeout: Duration) -> anyhow::Result<Vec<McpToolDescriptor>> {
        let result = self.request("tools/list", json!({}), timeout).await?;
        let tools = result.get("tools").cloned().unwrap_or_else(|| json!([]));
        serde_json::from_value(tools).context("parse tools/list result")
    }

    /// Calls a tool and returns the raw MCP `CallToolResult`.
    pub async fn call_tool_raw(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
            timeout,
        )
        .await
        .with_context(|| format!("tools/call {name}"))
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> anyhow::Result<McpToolOutcome> {
        let result = self.call_tool_raw(name, arguments, timeout).await?;
        Ok(McpToolOutcome {
            text: redact_secrets(&render_content_text(&result), &self.config.redact),
            data: result
                .get("structuredContent")
                .cloned()
                .unwrap_or(Value::Null),
            is_error: result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// The last lines the server wrote to stderr, with secrets redacted.
    pub fn stderr_tail(&self) -> String {
        let lines = self
            .stderr_tail
            .lock()
            .map(|tail| tail.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        redact_secrets(&lines, &self.config.redact)
    }

    /// Closes stdin, gives the server a moment to exit on its own, then stops
    /// its whole process group. Safe to call more than once.
    pub async fn shutdown(&self) {
        self.stdin.lock().await.take();
        let child = self.child.lock().await.take();
        if let Some(mut child) = child {
            stdio_process::stop_child(&mut child, Duration::from_secs(2)).await;
        }
        self.exited.store(true, Ordering::SeqCst);
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        if self.has_exited() {
            anyhow::bail!(self.failure("has exited"));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("MCP pending-request table poisoned"))?
            .insert(id, tx);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(error) = self.write(&message).await {
            self.forget(id);
            return Err(error);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => rpc_result(response),
            Ok(Err(_)) => anyhow::bail!(self.failure(&format!("exited during {method}"))),
            Err(_) => {
                self.forget(id);
                anyhow::bail!(
                    "MCP server {} did not answer {method} within {}s",
                    self.config.name,
                    timeout.as_secs()
                )
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn write(&self, message: &Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        let Some(stdin) = stdin.as_mut() else {
            anyhow::bail!("MCP server {} is shut down", self.config.name);
        };
        stdin
            .write_all(&line)
            .await
            .with_context(|| format!("write to MCP server {}", self.config.name))?;
        stdin.flush().await?;
        Ok(())
    }

    fn forget(&self, id: i64) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
    }

    /// An error message naming the server, with its redacted stderr tail.
    fn failure(&self, what: &str) -> String {
        let tail = self.stderr_tail();
        if tail.trim().is_empty() {
            format!("MCP server {} {what}", self.config.name)
        } else {
            format!(
                "MCP server {} {what}\nserver stderr (last lines):\n{tail}",
                self.config.name
            )
        }
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        // Best effort when the async `shutdown` was not awaited: terminate the
        // group now and force-kill it shortly after. `kill_on_drop` covers the
        // direct child where process groups are unavailable.
        if let Some(pid) = self.pid
            && self.child.get_mut().is_some()
        {
            stdio_process::signal_group(pid, false);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1500));
                stdio_process::signal_group(pid, true);
            });
        }
    }
}

async fn read_stdout(
    stdout: tokio::process::ChildStdout,
    pending: Pending,
    exited: Arc<AtomicBool>,
    stdin: SharedStdin,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let is_response = message.get("result").is_some() || message.get("error").is_some();
        if !is_response {
            if let Some(method) = message.get("method").and_then(Value::as_str) {
                answer_server_request(&stdin, message.get("id").cloned(), method).await;
            }
            continue;
        }
        let Some(id) = message.get("id").and_then(Value::as_i64) else {
            continue;
        };
        let sender = pending.lock().ok().and_then(|mut map| map.remove(&id));
        if let Some(sender) = sender {
            let _ = sender.send(message);
        }
    }
    exited.store(true, Ordering::SeqCst);
    if let Ok(mut pending) = pending.lock() {
        pending.clear();
    }
}

/// Answers a server-initiated request: `ping` succeeds, anything else (such
/// as `roots/list` or `sampling/createMessage`) is declined, since this
/// client advertises no capabilities. Notifications get no reply.
async fn answer_server_request(stdin: &SharedStdin, id: Option<Value>, method: &str) {
    let Some(id) = id.filter(|id| !id.is_null()) else {
        return;
    };
    let reply = if method == "ping" {
        json!({ "jsonrpc": "2.0", "id": id, "result": {} })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method {method} is not supported") }
        })
    };
    let Ok(mut line) = serde_json::to_vec(&reply) else {
        return;
    };
    line.push(b'\n');
    if let Some(stdin) = stdin.lock().await.as_mut() {
        let _ = stdin.write_all(&line).await;
        let _ = stdin.flush().await;
    }
}

async fn read_stderr(stderr: tokio::process::ChildStderr, tail: Arc<StdMutex<VecDeque<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(mut tail) = tail.lock() {
            if tail.len() == STDERR_TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    }
}
