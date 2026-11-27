use std::path::PathBuf;

use anyhow::{Context, bail};
use async_trait::async_trait;
use roder_api::backend::{
    AgentBackend, AgentBackendSession, BackendModel, BackendStart, BackendThreadSnapshot,
    BackendThreadSummary,
};
use roder_api::policy_mode::PolicyMode;
use serde_json::json;

use crate::mapping;
use crate::transport::{LOGIN_HELP, Transport};

#[path = "session.rs"]
mod session;
pub(crate) use session::Session;

#[derive(Clone, Default)]
pub struct AntigravityBackend {
    program: Option<PathBuf>,
}

impl AntigravityBackend {
    pub fn with_program(program: PathBuf) -> Self {
        Self {
            program: Some(program),
        }
    }

    async fn initialize(
        &self,
        cwd: &std::path::Path,
        native_bypass: bool,
    ) -> anyhow::Result<Session> {
        let program = match &self.program {
            Some(program) => program.clone(),
            None => crate::runtime::find_runtime()?.context(
                "Antigravity runtime is missing. Open Roder with --backend antigravity to install it, or run roder auth login antigravity.",
            )?,
        };
        let mut session = Session::new(
            Transport::spawn(&program, cwd, native_bypass).await?,
            native_bypass,
        );
        session.capabilities = session
            .rpc("initialize", json!({
                "protocolVersion":1,
                "clientInfo":{"name":"roder","title":"Roder","version":env!("CARGO_PKG_VERSION")},
                "clientCapabilities":{}
            }))
            .await?;
        if session.capabilities["protocolVersion"] != 1 {
            bail!("Antigravity ACP protocol version is not supported");
        }
        Ok(session)
    }

    async fn authenticated(
        &self,
        cwd: &std::path::Path,
        native_bypass: bool,
    ) -> anyhow::Result<Session> {
        let mut session = self.initialize(cwd, native_bypass).await?;
        let oauth = session.capabilities["authMethods"]
            .as_array()
            .is_some_and(|methods| {
                methods
                    .iter()
                    .any(|method| method["id"] == "oauth-personal")
            });
        if !oauth {
            bail!(
                "Antigravity does not advertise Google subscription authentication. {LOGIN_HELP}"
            );
        }
        session
            .rpc("authenticate", json!({"methodId":"oauth-personal"}))
            .await
            .with_context(|| LOGIN_HELP)?;
        Ok(session)
    }

    /// Explicit logout only; never called as part of connection or status checks.
    pub async fn logout(&self, cwd: PathBuf) -> anyhow::Result<()> {
        let mut session = self.initialize(&cwd, false).await?;
        if !session
            .capabilities
            .pointer("/agentCapabilities/auth/logout")
            .is_some_and(|value| value == true || value.is_object())
        {
            bail!("Installed Antigravity CLI does not advertise ACP logout");
        }
        session.rpc("logout", json!({})).await?;
        session.transport.stop().await
    }
}

#[async_trait]
impl AgentBackend for AntigravityBackend {
    fn id(&self) -> &'static str {
        "antigravity"
    }

    async fn connect(&self, start: BackendStart) -> anyhow::Result<Box<dyn AgentBackendSession>> {
        let mut session = self
            .authenticated(&start.cwd, start.policy_mode == PolicyMode::Bypass)
            .await?;
        let state = if let Some(id) = start.resume_thread {
            if session
                .capabilities
                .pointer("/agentCapabilities/loadSession")
                != Some(&json!(true))
                && session
                    .capabilities
                    .pointer("/agentCapabilities/sessionCapabilities/load")
                    .is_none()
            {
                bail!("Installed Antigravity CLI does not support loading sessions");
            }
            session.id = id.clone();
            session
                .rpc(
                    "session/load",
                    json!({
                        "sessionId":id,"cwd":start.cwd,"mcpServers":[]
                    }),
                )
                .await?
        } else {
            session
                .rpc("session/new", json!({"cwd":start.cwd,"mcpServers":[]}))
                .await?
        };
        if session.id.is_empty() {
            session.id = state["sessionId"]
                .as_str()
                .context("Antigravity session response missing sessionId")?
                .into();
        }
        session.state = state;
        session.refresh_selection();
        session.snapshot.cwd = start.cwd.to_string_lossy().into_owned();
        session.configure_policy(start.policy_mode).await?;
        if let Some(model) = start.model.filter(|model| !model.is_empty()) {
            session.configure_model(&model).await?;
        }
        session.finish_setup();
        Ok(Box::new(session))
    }

    async fn list_models(&self, cwd: PathBuf) -> anyhow::Result<Vec<BackendModel>> {
        let mut session = self.authenticated(&cwd, false).await?;
        let state = session
            .rpc("session/new", json!({"cwd":cwd,"mcpServers":[]}))
            .await?;
        let models = mapping::models(&state);
        if models.is_empty() {
            bail!("Antigravity ACP session did not advertise any models");
        }
        session.transport.stop().await?;
        Ok(models)
    }

    async fn list_threads(
        &self,
        cwd: PathBuf,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<BackendThreadSummary>> {
        let mut session = self.authenticated(&cwd, false).await?;
        if session
            .capabilities
            .pointer("/agentCapabilities/sessionCapabilities/list")
            .is_none()
        {
            bail!("Installed Antigravity CLI does not advertise session listing");
        }
        let result = session.rpc("session/list", json!({"cwd":cwd})).await?;
        let threads = result["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .take(limit.unwrap_or(usize::MAX))
            .filter_map(|entry| {
                Some(BackendThreadSummary {
                    id: entry["sessionId"].as_str()?.into(),
                    model: String::new(),
                    snapshot: BackendThreadSnapshot {
                        preview: entry["title"].as_str().unwrap_or("").into(),
                        cwd: entry["cwd"].as_str().unwrap_or("").into(),
                        ..Default::default()
                    },
                })
            })
            .collect();
        session.transport.stop().await?;
        Ok(threads)
    }
}
