//! Live OpenAI native computer protocol, real browser, full Roder/ACP loop.
//! Run with OPENAI_API_KEY or Roder's configured OpenAI provider key.
use agent_client_protocol_schema as acp;
use async_trait::async_trait;
use roder_api::{
    extension::ExtensionRegistryBuilder, inference::HostedWebSearchConfig, policy_mode::PolicyMode,
};
use roder_app_server::{
    AppServer, AppServerFeatureConfig, LocalAppClient,
    acp::{AcpAdapter, AcpClientPeer},
};
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_chrome::{ComputerCdpBinding, ComputerToolContributor};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use roder_protocol::{JsonRpcNotification, JsonRpcRequest};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
#[path = "../../roder-ext-chrome/examples/native_computer/support.rs"]
#[allow(dead_code)]
mod support;
#[derive(Clone, Default)]
struct Peer {
    notifications: Arc<Mutex<Vec<JsonRpcNotification>>>,
}
#[async_trait]
impl AcpClientPeer for Peer {
    async fn send_notification(&self, n: JsonRpcNotification) -> anyhow::Result<()> {
        self.notifications.lock().await.push(n);
        Ok(())
    }
    async fn request_permission(
        &self,
        _: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse> {
        anyhow::bail!("native eval runs in explicit bypass mode; unexpected permission request")
    }
}
fn request(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(method)),
        method: method.into(),
        params: Some(params),
    }
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = roder_config::load_config()?;
    let provider = config.providers.get("openai");
    let key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .or_else(|| provider.and_then(|p| p.api_key.clone()))
        .or_else(|| {
            provider
                .and_then(|p| p.api_key_env.as_ref())
                .and_then(|name| std::env::var(name).ok())
        })
        .filter(|key| !key.trim().is_empty());
    anyhow::ensure!(
        key.is_some(),
        "Live native computer eval requires OPENAI_API_KEY or an OpenAI provider key in Roder config. Codex account auth does not support this tool."
    );
    let model = std::env::var("RODER_NATIVE_EVAL_MODEL").unwrap_or_else(|_| "gpt-6.1-sol".into());
    let output = std::path::PathBuf::from(
        std::env::var("RODER_NATIVE_EVAL_OUTPUT")
            .unwrap_or_else(|_| "evals/reports/native-computer/live".into()),
    );
    std::fs::create_dir_all(&output)?;
    let browser = support::Browser::start()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Chrome required for the live eval"))?;
    let fixture = support::Fixture::start().await?;
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(OpenAiResponsesEngine::new_with_config(
        key,
        "openai",
        "https://api.openai.com/v1",
        Vec::new(),
    )));
    builder.tool_contributor(Arc::new(ComputerToolContributor::new(Arc::new(
        ComputerCdpBinding::new(&browser.endpoint, &fixture.url),
    ))));
    let runtime = Arc::new(Runtime::new(
        builder.build()?,
        RuntimeConfig {
            default_provider: "openai".into(),
            default_model: model.clone(),
            policy_mode: PolicyMode::Bypass,
            hosted_web_search: HostedWebSearchConfig::disabled(),
            tool_allowlist: vec!["computer".into()],
            auto_compact_token_limit: None,
            turn_deadline_seconds: Some(240),
            ..Default::default()
        },
    )?);
    let temp = tempfile::tempdir()?;
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig::default()
            .with_workspace_registry_path(temp.path().join("workspaces.json")),
    ));
    let adapter = AcpAdapter::new(LocalAppClient::new(server));
    let peer = Peer::default();
    let created = adapter
        .handle_request(
            request(
                "session/new",
                serde_json::to_value(acp::NewSessionRequest::new(std::env::current_dir()?))?,
            ),
            &peer,
        )
        .await?
        .unwrap();
    let session = created
        .result
        .ok_or_else(|| anyhow::anyhow!("session/new failed: {:?}", created.error))?["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let goal=std::env::var("RODER_NATIVE_EVAL_GOAL").unwrap_or_else(|_|"Use only the native computer tool for UI interaction. Capture the current screen, click Show filters, type penguin into Search, select all that text and replace it with orca, then press Shift+a to append an uppercase A and Enter to submit. Verify the page visibly says Filters open and Submitted: orcaA. Leave that state in the browser.".into());
    let started = Instant::now();
    let response = tokio::time::timeout(
        Duration::from_secs(260),
        adapter.handle_request(
            request(
                "session/prompt",
                serde_json::to_value(acp::PromptRequest::new(
                    session,
                    vec![acp::ContentBlock::Text(acp::TextContent::new(goal))],
                ))?,
            ),
            &peer,
        ),
    )
    .await;
    let grade = fixture.grade(false).await;
    let notifications = peer.notifications.lock().await;
    let mut calls = std::collections::BTreeMap::new();
    for n in notifications
        .iter()
        .filter(|n| n.method == "session/update")
    {
        let u = &n.params["update"];
        if u["sessionUpdate"] == "tool_call" && u["rawInput"]["actions"].is_array() {
            calls.insert(
                u["toolCallId"].as_str().unwrap_or_default().to_string(),
                u["rawInput"].clone(),
            );
        }
    }
    let result = match response {
        Ok(Ok(Some(response))) => serde_json::to_value(response)?,
        Ok(Ok(None)) => json!({"error":"ACP prompt returned no response"}),
        Ok(Err(error)) => json!({"error":error.to_string()}),
        Err(error) => json!({"error":format!("ACP prompt timed out: {error}")}),
    };
    let passed = grade["passed"] == true
        && !calls.is_empty()
        && result
            .pointer("/result/stopReason")
            .is_some_and(|reason| reason == "end_turn");
    let report = json!({"mode":"live_openai_native_computer_real_browser_acp","model":model,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "passed":passed,"computer_calls":calls.len(),"actions":calls.values().flat_map(|call|call["actions"].as_array().unwrap().iter().map(|action|action["type"].clone())).collect::<Vec<_>>(),"grade":grade,"prompt_response":result});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    anyhow::ensure!(
        passed,
        "Live native computer eval did not meet its independent browser grader; report saved to {}",
        output.display()
    );
    Ok(())
}
