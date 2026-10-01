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
const STACK_BYTES: usize = 32 * 1024 * 1024;

fn main() -> anyhow::Result<()> {
    // Match the CLI bootstrap: workspace dependencies enable both Rustls
    // providers, so HTTPS clients need an explicit process default.
    let _ = rustls::crypto::ring::default_provider().install_default();
    std::thread::Builder::new()
        .name("native-computer-eval".into())
        .stack_size(STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(STACK_BYTES)
                .enable_all()
                .build()?
                .block_on(run_eval())
        })?
        .join()
        .map_err(|panic| anyhow::anyhow!("Native computer eval panicked: {panic:?}"))?
}

async fn run_eval() -> anyhow::Result<()> {
    let config = roder_config::load_config()?;
    let provider = config.providers.get("openai");
    let nonblank = |value: String| (!value.trim().is_empty()).then(|| value.trim().to_string());
    let environment_key = std::env::var("OPENAI_API_KEY").ok().and_then(nonblank);
    let base_url = if environment_key.is_some() {
        "https://api.openai.com/v1".to_string()
    } else {
        provider
            .and_then(|provider| provider.base_url.clone())
            .and_then(nonblank)
            .unwrap_or_else(|| "https://api.openai.com/v1".into())
    };
    let key = environment_key
        .or_else(|| {
            provider
                .and_then(|provider| provider.api_key.clone())
                .and_then(nonblank)
        })
        .or_else(|| {
            provider
                .and_then(|provider| provider.api_key_env.as_ref())
                .and_then(|name| std::env::var(name).ok())
                .and_then(nonblank)
        });
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
    let visible = std::env::var("RODER_NATIVE_EVAL_VISIBLE").as_deref() == Ok("1");
    let initial_url = if visible {
        format!("{}start", fixture.url)
    } else {
        fixture.url.clone()
    };
    if visible {
        println!(
            "Visible Chrome demo ready; the model will navigate from the start page to the form."
        );
    }
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(OpenAiResponsesEngine::new_with_config(
        key,
        "openai",
        base_url,
        Vec::new(),
    )));
    let binding = Arc::new(ComputerCdpBinding::new(&browser.endpoint, initial_url));
    builder.tool_contributor(Arc::new(ComputerToolContributor::new(binding.clone())));
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
    if visible {
        use roder_api::tools::{ToolCall, ToolExecutionContext};
        use roder_ext_chrome::direct::{DirectBinding, DirectStep, DirectTab};
        let ctx = ToolExecutionContext::new(&session, "demo-setup", PolicyMode::Bypass);
        let setup = ToolCall {
            id: "demo-setup".into(),
            name: "computer".into(),
            arguments: json!({}),
            raw_arguments: String::new(),
            thread_id: session.clone(),
            turn_id: "demo-setup".into(),
        };
        let lease = binding
            .lease(&ctx, &setup)
            .await
            .map_err(anyhow::Error::msg)?;
        let DirectTab::Target { target_id, .. } = lease.tab() else {
            anyhow::bail!("Visible demo requires an owned page target");
        };
        browser.focus_tab(&target_id).await?;
        lease.finish(&DirectStep::default(), &target_id).await;
        println!("Visible Chrome window is in front; starting native actions in 10 seconds.");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    let goal=std::env::var("RODER_NATIVE_EVAL_GOAL").unwrap_or_else(|_|"Use only the native computer tool for UI interaction. Capture the current screen, click Show filters, type penguin into Search, select all that text and replace it with orca, then press Shift+a to append an uppercase A and Enter to submit. Verify the page visibly says Filters open and Submitted: orcaA. Leave that state in the browser.".into());
    let goal = if visible {
        format!(
            "This is Chrome on {}. First capture the screen and click Open the demo to navigate to the form. {goal}",
            std::env::consts::OS
        )
    } else {
        goal
    };
    println!("Native computer demo running with {model}.");
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
    let final_ui = final_browser_state(&browser, &fixture.url, &output)
        .await
        .unwrap_or_else(|error| json!({"passed":false,"error":error.to_string()}));
    let notifications = peer.notifications.lock().await;
    let mut calls = Vec::new();
    let mut seen_calls = std::collections::HashSet::new();
    let mut failed_calls = std::collections::HashSet::new();
    for n in notifications
        .iter()
        .filter(|n| n.method == "session/update")
    {
        let u = &n.params["update"];
        if u["sessionUpdate"] == "tool_call_update"
            && u["status"] == "failed"
            && let Some(id) = u["toolCallId"].as_str()
        {
            failed_calls.insert(id.to_string());
        }
        if u["sessionUpdate"] == "tool_call"
            && u["rawInput"]["actions"].is_array()
            && let Some(id) = u["toolCallId"].as_str().filter(|id| !id.is_empty())
            && seen_calls.insert(id.to_string())
        {
            // Preserve execution order; sorting random call ids changes the trace.
            calls.push(json!({"call_id":id,"actions":u["rawInput"]["actions"]}));
        }
    }
    let result = match response {
        Ok(Ok(Some(response))) => serde_json::to_value(response)?,
        Ok(Ok(None)) => json!({"error":"ACP prompt returned no response"}),
        Ok(Err(error)) => json!({"error":error.to_string()}),
        Err(error) => json!({"error":format!("ACP prompt timed out: {error}")}),
    };
    let passed = grade["passed"] == true
        && final_ui["passed"] == true
        && !calls.is_empty()
        && result
            .pointer("/result/stopReason")
            .is_some_and(|reason| reason == "end_turn");
    let report = json!({"mode":"live_openai_native_computer_real_browser_acp","model":model,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "visible":visible,
        "openai_api_validated":!calls.is_empty(),"passed":passed,"computer_calls":calls.len(),
        "failed_computer_calls":failed_calls.len(),
        "actions":calls.iter().flat_map(|call|call["actions"].as_array().unwrap().iter().map(|action|action["type"].clone())).collect::<Vec<_>>(),
        "call_trace":calls,"grade":grade,"final_ui":final_ui,"prompt_response":result});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!(
        "Native computer demo complete: passed={passed}; report={}",
        output.join("report.json").display()
    );
    let hold_seconds = std::env::var("RODER_NATIVE_EVAL_HOLD_SECONDS")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(if visible { 600 } else { 0 })
        .min(3600);
    if hold_seconds > 0 {
        println!("Visible Chrome page remains open for {hold_seconds} seconds.");
        tokio::time::sleep(Duration::from_secs(hold_seconds)).await;
    }
    anyhow::ensure!(
        passed,
        "Live native computer eval did not meet its independent browser grader; report saved to {}",
        output.display()
    );
    Ok(())
}

async fn final_browser_state(
    browser: &support::Browser,
    url: &str,
    output: &std::path::Path,
) -> anyhow::Result<Value> {
    use base64::Engine;
    use roder_ext_chrome::direct::{DirectSession, DirectTab, OpenGuard};
    let targets: Vec<Value> = reqwest::get(format!("{}/json/list", browser.endpoint))
        .await?
        .error_for_status()?
        .json()
        .await?;
    let target = targets
        .iter()
        .find(|target| target["type"] == "page" && target["url"] == url)
        .ok_or_else(|| anyhow::anyhow!("Fixture page is no longer open at its expected URL"))?;
    let tab = DirectTab::Target {
        endpoint: browser.endpoint.clone(),
        target_id: target["id"].as_str().unwrap_or_default().into(),
    };
    let mut session = DirectSession::attach(&tab, Arc::new(OpenGuard), false).await?;
    let look = session.run("look", &json!({})).await;
    anyhow::ensure!(!look.is_error, "Final browser read failed: {}", look.text);
    let text = look.data["page"]["text"].as_str().unwrap_or_default();
    let visible =
        text.contains("Filters open") && text.lines().any(|line| line.trim() == "Submitted: orcaA");
    let screenshot = session.run("screenshot", &json!({})).await;
    anyhow::ensure!(
        !screenshot.is_error,
        "Final screenshot failed: {}",
        screenshot.text
    );
    let image = screenshot
        .image
        .ok_or_else(|| anyhow::anyhow!("No final screenshot"))?;
    let (_, encoded) = image
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("Invalid screenshot data URL"))?;
    std::fs::write(
        output.join("final.jpg"),
        base64::engine::general_purpose::STANDARD.decode(encoded)?,
    )?;
    Ok(
        json!({"passed":visible,"text":text,"screenshot":"final.jpg",
        "width":screenshot.data["width"],"height":screenshot.data["height"],"masked_fields":screenshot.data["masked"]}),
    )
}
