//! Native computer executor and opt-in, per-thread CDP binding.
use crate::direct::{
    DirectBinding, DirectGuard, DirectLease, DirectSession, DirectStep, DirectTab, tool_result,
};
use async_trait::async_trait;
use roder_api::{
    computer::{ComputerActions, computer_tool_spec},
    extension::ToolProviderId,
    tools::{
        ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
        ToolSpec,
    },
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Register this tool only for a browser the host deliberately supplies.
/// Responses providers serialize it as the native `computer` tool.
pub struct ComputerTool {
    binding: Arc<dyn DirectBinding>,
    sessions: Mutex<HashMap<String, Arc<Mutex<Option<CachedSession>>>>>,
}
struct CachedSession {
    binding_target: String,
    session: DirectSession,
}
impl ComputerTool {
    pub fn new(binding: Arc<dyn DirectBinding>) -> Self {
        Self {
            binding,
            sessions: Mutex::new(HashMap::new()),
        }
    }
}
#[async_trait]
impl ToolExecutor for ComputerTool {
    fn spec(&self) -> ToolSpec {
        computer_tool_spec()
    }
    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let lease = self
            .binding
            .lease(&ctx, &call)
            .await
            .map_err(anyhow::Error::msg)?;
        let slot = {
            let mut sessions = self.sessions.lock().await;
            anyhow::ensure!(
                sessions.len() < 256 || sessions.contains_key(&ctx.thread_id),
                "computer session capacity reached"
            );
            sessions.entry(ctx.thread_id.clone()).or_default().clone()
        };
        let mut cached = slot.lock().await;
        let tab = lease.tab();
        if cached.as_ref().is_none_or(|state| {
            state.binding_target != tab.target_id() || state.session.target_id() != tab.target_id()
        }) {
            *cached = Some(CachedSession {
                binding_target: tab.target_id().into(),
                session: DirectSession::attach(&tab, lease.guard(), lease.may_authorize()).await?,
            });
        }
        let state = cached.as_mut().unwrap();
        state.session.guard = lease.guard();
        state.session.may_authorize = lease.may_authorize();
        let session = &mut state.session;
        let step = match serde_json::from_value::<ComputerActions>(call.arguments.clone()) {
            Ok(batch) => session.run_computer(&batch).await,
            Err(error) => {
                let mut observed = session.run("screenshot", &json!({})).await;
                observed.is_error = true;
                observed.text = format!(
                    "Invalid native computer actions; no input sent: {error}\n{}",
                    observed.text
                );
                observed
            }
        };
        let target = session.target_id().to_string();
        lease.finish(&step, &target).await;
        state.binding_target = target;
        Ok(tool_result(&call.id, &call.name, &step))
    }
}

pub struct ComputerToolContributor {
    binding: Arc<dyn DirectBinding>,
}
impl ComputerToolContributor {
    pub fn new(binding: Arc<dyn DirectBinding>) -> Self {
        Self { binding }
    }
}
impl ToolContributor for ComputerToolContributor {
    fn id(&self) -> ToolProviderId {
        "computer".into()
    }
    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(ComputerTool::new(self.binding.clone())))
    }
}

/// One owned page target per Roder thread. A lease serializes the entire batch
/// and its screenshot; active-tab changes cannot retarget an in-flight call.
pub struct ComputerCdpBinding {
    endpoint: String,
    initial_url: String,
    threads: Mutex<HashMap<String, Arc<Mutex<Option<BoundTab>>>>>,
}
struct BoundTab {
    tab: DirectTab,
    guard: Arc<ComputerGuard>,
}
struct ComputerGuard {
    scope: crate::desktop_scope::DesktopScope,
    secrets: std::sync::Mutex<Vec<String>>,
}
impl DirectGuard for ComputerGuard {
    fn outside(&self, url: &str) -> Option<String> {
        self.scope.outside(url)
    }
    fn remember_secret(&self, value: &str) {
        if !value.is_empty() {
            self.secrets.lock().unwrap().push(value.to_string());
        }
    }
    fn scrub(&self, text: &str) -> String {
        let mut secrets = self.secrets.lock().unwrap().clone();
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.iter().fold(text.to_string(), |text, secret| {
            text.replace(secret, "[REDACTED]")
        })
    }
}
impl ComputerCdpBinding {
    pub fn new(endpoint: impl Into<String>, initial_url: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            initial_url: initial_url.into(),
            threads: Mutex::new(HashMap::new()),
        }
    }
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(endpoint) = std::env::var("RODER_COMPUTER_USE_CDP_URL").ok() else {
            return Ok(None);
        };
        let url = reqwest::Url::parse(&endpoint)?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https" | "ws" | "wss"),
            "computer CDP URL must be http(s) or ws(s)"
        );
        let initial = std::env::var("RODER_COMPUTER_USE_URL")
            .map_err(|_| anyhow::anyhow!("RODER_COMPUTER_USE_URL is required: native browser screenshots show the page viewport, not Chrome's address bar"))?;
        Ok(Some(Self::new(endpoint, initial)))
    }
}
struct ComputerLease {
    state: OwnedMutexGuard<Option<BoundTab>>,
}
#[async_trait]
impl DirectLease for ComputerLease {
    fn tab(&self) -> DirectTab {
        self.state.as_ref().unwrap().tab.clone()
    }
    fn guard(&self) -> Arc<dyn DirectGuard> {
        self.state.as_ref().unwrap().guard.clone()
    }
    fn may_authorize(&self) -> bool {
        false
    }
    async fn finish(mut self: Box<Self>, _step: &DirectStep, target_id: &str) {
        if let DirectTab::Target { target_id: id, .. } = &mut self.state.as_mut().unwrap().tab {
            *id = target_id.into();
        }
    }
}
#[async_trait]
impl DirectBinding for ComputerCdpBinding {
    async fn lease(
        &self,
        ctx: &ToolExecutionContext,
        _call: &ToolCall,
    ) -> Result<Box<dyn DirectLease>, String> {
        let slot = {
            let mut threads = self.threads.lock().await;
            if threads.len() >= 256 && !threads.contains_key(&ctx.thread_id) {
                return Err("computer thread capacity reached".into());
            }
            threads.entry(ctx.thread_id.clone()).or_default().clone()
        };
        let mut state = slot.lock_owned().await;
        if state.is_none() {
            let scope =
                crate::desktop_scope::DesktopScope::from_env().map_err(|e| e.to_string())?;
            if let Some(reason) = scope.outside(&self.initial_url) {
                return Err(reason);
            }
            let target_id = create_target(&self.endpoint, &self.initial_url)
                .await
                .map_err(|e| format!("create computer tab: {e:#}"))?;
            *state = Some(BoundTab {
                tab: DirectTab::Target {
                    endpoint: self.endpoint.clone(),
                    target_id,
                },
                guard: Arc::new(ComputerGuard {
                    scope,
                    secrets: Default::default(),
                }),
            });
        }
        Ok(Box::new(ComputerLease { state }))
    }
}

async fn create_target(endpoint: &str, url: &str) -> anyhow::Result<String> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let endpoint = crate::direct::devtools::browser_websocket(endpoint).await?;
        let (mut socket, _) = tokio_tungstenite::connect_async(endpoint).await?;
        socket
            .send(Message::Text(
                json!({"id":1,"method":"Target.createTarget","params":{"url":url}})
                    .to_string()
                    .into(),
            ))
            .await?;
        while let Some(message) = socket.next().await {
            if let Message::Text(text) = message? {
                let value: Value = serde_json::from_str(&text)?;
                if value["id"] == 1 {
                    anyhow::ensure!(
                        value.get("error").is_none(),
                        "CDP target creation failed: {}",
                        value["error"]
                    );
                    return value["result"]["targetId"]
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| anyhow::anyhow!("CDP omitted targetId"));
                }
            }
        }
        anyhow::bail!("CDP closed before target creation")
    })
    .await;
    result.map_err(|_| anyhow::anyhow!("CDP target creation timed out"))?
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    #[test]
    fn overlapping_secrets_are_scrubbed_longest_first() {
        let guard = ComputerGuard {
            scope: crate::desktop_scope::DesktopScope::unrestricted(),
            secrets: Default::default(),
        };
        guard.remember_secret("foo");
        guard.remember_secret("foobar");
        assert_eq!(guard.scrub("foobar foo"), "[REDACTED] [REDACTED]");
    }
}
