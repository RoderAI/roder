use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use roder_app_server::acp::{AcpAdapter, AcpClientPeer, parse_error, response_error};
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::{build_runtime_from_config, parse_cli_options};

pub(crate) async fn run_acp_cli(args: &[String]) -> anyhow::Result<()> {
    let cli_options = parse_cli_options(args)?;
    let (runtime, _) = build_runtime_from_config(cli_options).await?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    let client = LocalAppClient::new(app_server);
    let adapter = AcpAdapter::new(client);
    run_stdio_acp(adapter).await
}

async fn run_stdio_acp(adapter: AcpAdapter<LocalAppClient>) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = rx.recv().await {
            stdout
                .write_all(serde_json::to_string(&message)?.as_bytes())
                .await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        anyhow::Ok(())
    });

    let peer = StdioAcpPeer::new(tx.clone());
    let mut request_tasks = tokio::task::JoinSet::new();
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(value) => value,
            Err(err) => {
                let _ = tx.send(serde_json::to_value(response_error(
                    None,
                    parse_error(err),
                ))?);
                continue;
            }
        };
        if value.get("method").is_some() {
            let request = match serde_json::from_value::<JsonRpcRequest>(value) {
                Ok(request) => request,
                Err(err) => {
                    let _ = tx.send(serde_json::to_value(response_error(
                        None,
                        parse_error(err),
                    ))?);
                    continue;
                }
            };
            let adapter = adapter.clone();
            let peer = peer.clone();
            request_tasks.spawn(async move {
                let id = request.id.clone();
                match adapter.handle_request(request, &peer).await {
                    Ok(Some(response)) => {
                        let _ = peer.send_response(response);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        let _ = peer.send_response(response_error(
                            id,
                            JsonRpcError {
                                code: -32000,
                                message: err.to_string(),
                                data: Some(serde_json::json!({ "details": err.to_string() })),
                            },
                        ));
                    }
                }
            });
        } else {
            peer.resolve_response(value).await;
        }
    }
    request_tasks.abort_all();
    while request_tasks.join_next().await.is_some() {}
    drop(peer);
    drop(tx);
    writer.await??;
    Ok(())
}

#[derive(Clone)]
struct StdioAcpPeer {
    tx: mpsc::UnboundedSender<serde_json::Value>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<serde_json::Value, JsonRpcError>>>>>,
    next_id: Arc<AtomicU64>,
}

impl StdioAcpPeer {
    fn new(tx: mpsc::UnboundedSender<serde_json::Value>) -> Self {
        Self {
            tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn send_response(&self, response: JsonRpcResponse) -> anyhow::Result<()> {
        self.tx.send(serde_json::to_value(response)?)?;
        Ok(())
    }

    async fn resolve_response(&self, value: serde_json::Value) {
        let Some(id) = value.get("id").and_then(serde_json::Value::as_str) else {
            return;
        };
        let Some(tx) = self.pending.lock().await.remove(id) else {
            return;
        };
        let result = if let Some(error) = value.get("error") {
            serde_json::from_value::<JsonRpcError>(error.clone()).map_or_else(
                |err| {
                    Err(JsonRpcError {
                        code: -32603,
                        message: format!("invalid client error response: {err}"),
                        data: None,
                    })
                },
                Err,
            )
        } else {
            Ok(value
                .get("result")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})))
        };
        let _ = tx.send(result);
    }

    async fn request_client(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let id = format!("roder-acp-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        if let Err(err) = self.tx.send(request) {
            let _ = self.pending.lock().await.remove(&id);
            anyhow::bail!("failed to send ACP client request: {err}");
        }
        match rx.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => anyhow::bail!(
                "ACP client request failed: {} ({})",
                error.message,
                error.code
            ),
            Err(_) => anyhow::bail!("ACP client response channel closed"),
        }
    }
}

#[async_trait]
impl AcpClientPeer for StdioAcpPeer {
    async fn send_notification(
        &self,
        notification: roder_protocol::JsonRpcNotification,
    ) -> anyhow::Result<()> {
        self.tx.send(serde_json::to_value(notification)?)?;
        Ok(())
    }

    async fn request_permission(
        &self,
        request: agent_client_protocol_schema::RequestPermissionRequest,
    ) -> anyhow::Result<agent_client_protocol_schema::RequestPermissionResponse> {
        let result = self
            .request_client("session/request_permission", serde_json::to_value(request)?)
            .await?;
        Ok(serde_json::from_value(result)?)
    }
}
