use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use roder_app_server::acp::{
    AcpAdapter, AcpAuthentication, AcpClientPeer, parse_error, response_error,
};
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::{build_runtime_with_config, parse_cli_options};

pub(crate) async fn run_acp_cli(args: &[String]) -> anyhow::Result<()> {
    let cli_options = parse_cli_options(args)?;
    let config = acp_config(roder_config::load_config()?);
    let use_codex_auth = uses_codex(&config);
    let (runtime, _) = build_runtime_with_config(cli_options, config).await?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    let client = LocalAppClient::new(app_server);
    let mut adapter = AcpAdapter::new(client).with_agent_info(
        agent_client_protocol_schema::Implementation::new("roder", env!("CARGO_PKG_VERSION"))
            .title("Roder"),
    );
    if use_codex_auth {
        adapter = adapter.with_authentication(Arc::new(CodexAuthentication::default()));
    }
    run_stdio_acp(adapter).await
}

fn acp_config(mut config: roder_config::Config) -> roder_config::Config {
    // A fresh editor installation must use a real provider instead of mock inference.
    if config.provider.is_none() {
        config.provider = Some(roder_api::catalog::PROVIDER_CODEX.to_string());
    }
    config
}

fn uses_codex(config: &roder_config::Config) -> bool {
    let provider = config.provider.as_deref().unwrap_or_default();
    let provider = provider
        .split_once('/')
        .map_or(provider, |(provider, _)| provider);
    roder_api::catalog::normalize_provider_id(provider) == roder_api::catalog::PROVIDER_CODEX
}

#[derive(Default)]
struct CodexAuthentication {
    login: Mutex<()>,
}

#[async_trait]
impl AcpAuthentication for CodexAuthentication {
    fn methods(&self) -> Vec<agent_client_protocol_schema::AuthMethod> {
        use agent_client_protocol_schema::{AuthMethod, AuthMethodAgent};
        vec![AuthMethod::Agent(
            AuthMethodAgent::new("codex-login", "Sign in with Codex")
                .description("Sign in to your ChatGPT account in a browser."),
        )]
    }

    async fn is_authenticated(&self) -> anyhow::Result<bool> {
        Ok(roder_codex_auth::status().await?.is_some())
    }

    async fn authenticate(&self, _method_id: &str) -> anyhow::Result<()> {
        let _login = self.login.lock().await;
        // Keep OAuth's blocking callback listener outside the ACP process.
        let status = tokio::process::Command::new(std::env::current_exe()?)
            .args(["auth", "login", "codex"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .status()
            .await?;
        anyhow::ensure!(status.success(), "Codex sign-in failed ({status})");
        Ok(())
    }
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
    adapter
        .shutdown()
        .await
        .map_err(|error| anyhow::anyhow!("ACP shutdown failed: {}", error.message))?;
    drop(peer);
    drop(tx);
    writer.await??;
    Ok(())
}

#[derive(Clone)]
struct StdioAcpPeer {
    tx: mpsc::UnboundedSender<serde_json::Value>,
    pending:
        Arc<StdMutex<HashMap<String, oneshot::Sender<Result<serde_json::Value, JsonRpcError>>>>>,
    next_id: Arc<AtomicU64>,
}

impl StdioAcpPeer {
    fn new(tx: mpsc::UnboundedSender<serde_json::Value>) -> Self {
        Self {
            tx,
            pending: Arc::new(StdMutex::new(HashMap::new())),
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
        let Some(tx) = self
            .pending
            .lock()
            .expect("ACP pending requests lock")
            .remove(id)
        else {
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
        self.pending
            .lock()
            .expect("ACP pending requests lock")
            .insert(id.clone(), tx);
        let _pending = PendingRequest {
            peer: self.clone(),
            id: id.clone(),
        };
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        if let Err(err) = self.tx.send(request) {
            let _ = self
                .pending
                .lock()
                .expect("ACP pending requests lock")
                .remove(&id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_acp_config_uses_codex_and_exposes_browser_auth() {
        let config = acp_config(roder_config::Config::default());
        assert_eq!(config.provider.as_deref(), Some("codex"));
        let methods = serde_json::to_value(CodexAuthentication::default().methods()).unwrap();
        assert!(uses_codex(&config));
        assert!(methods[0].get("type").is_none());
        assert_eq!(methods[0]["id"], "codex-login");
        assert!(methods[0].get("args").is_none());
    }

    #[test]
    fn acp_preserves_configured_providers_and_models() {
        let mut config = roder_config::Config::default();
        config.provider = Some("anthropic".into());
        config.model = Some("configured-model".into());
        let config = acp_config(config);
        assert_eq!(config.provider.as_deref(), Some("anthropic"));
        assert_eq!(config.model.as_deref(), Some("configured-model"));
        assert!(!uses_codex(&config));
    }

    #[test]
    fn acp_codex_provider_with_inline_model_exposes_login() {
        let mut config = roder_config::Config::default();
        config.provider = Some(" CODEX /configured-model".into());
        assert!(uses_codex(&acp_config(config)));
    }
}

/// Dropping a cancelled permission future must also release its response slot.
struct PendingRequest {
    peer: StdioAcpPeer,
    id: String,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.peer
            .pending
            .lock()
            .expect("ACP pending requests lock")
            .remove(&self.id);
    }
}

#[cfg(test)]
mod peer_tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_permission_request_releases_pending_response() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let peer = StdioAcpPeer::new(tx);
        let task_peer = peer.clone();
        let task = tokio::spawn(async move {
            task_peer
                .request_client("session/request_permission", serde_json::json!({}))
                .await
        });
        rx.recv().await.unwrap();
        assert_eq!(peer.pending.lock().unwrap().len(), 1);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(peer.pending.lock().unwrap().is_empty());
    }
}
