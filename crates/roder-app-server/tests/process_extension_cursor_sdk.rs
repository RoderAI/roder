//! End-to-end proof for roadmap phase 93: the Cursor SDK TypeScript
//! extension registers through the process-extension host and drives
//! remote-cloud-agent dispatch and resume through the public app-server
//! JSON-RPC task surfaces. Offline — the child is the real compiled
//! TypeScript extension running with `CURSOR_SDK_FAKE=1` (the in-process
//! fake of `@cursor/sdk`); no network and no real key.
//!
//! Requires `node` and a built `dist/` (`npm ci && npm run build` in
//! `examples/non-rust-extensions/cursor-sdk-agents`); the test skips with
//! a notice when either is missing so toolchain-free CI stays green.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    ModelDescriptor,
};
use roder_api::process_extension::{ProcessEventFilter, ProcessExtensionConfig};
use roder_api::tasks::TaskState;
use roder_app_server::{AppServer, AppServerFeatureConfig, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_process_host::{ProcessHostExtension, load_process_extension};
use roder_protocol::{
    AgentsListResult, ExtensionsListResult, JsonRpcRequest, TasksGetParams, TasksGetResult,
    TasksSubmitParams, TasksSubmitResult,
};

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/non-rust-extensions/cursor-sdk-agents")
        .canonicalize()
        .unwrap()
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Inert engine so `Runtime::new` accepts a registry whose only real
/// services are the process-hosted dispatcher and task executor.
struct InertEngine;

#[async_trait::async_trait]
impl InferenceEngine for InertEngine {
    fn id(&self) -> String {
        "inert-test-engine".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            }),
        )])))
    }
}

fn cursor_sdk_config(example: &PathBuf, env: BTreeMap<String, String>) -> ProcessExtensionConfig {
    ProcessExtensionConfig {
        id: "cursor-sdk".to_string(),
        enabled: true,
        manifest: example.join("roder-extension.toml").display().to_string(),
        command: "node".to_string(),
        args: vec!["dist/src/main.js".to_string()],
        cwd: Some(example.display().to_string()),
        env,
        startup_timeout_ms: 20_000,
        event_filter: ProcessEventFilter::default(),
    }
}

fn fake_sdk_env() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("CURSOR_SDK_FAKE".to_string(), "1".to_string()),
        (
            "CURSOR_API_KEY".to_string(),
            "offline-e2e-placeholder-key".to_string(),
        ),
    ])
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: serde_json::Value,
) -> T {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    assert!(
        response.error.is_none(),
        "RPC error for {method}: {:?}",
        response.error
    );
    serde_json::from_value(response.result.unwrap()).unwrap()
}

fn client_for(env: BTreeMap<String, String>) -> LocalAppClient {
    let example = example_dir();
    let loaded = load_process_extension(cursor_sdk_config(&example, env), &example).unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(ProcessHostExtension::new(loaded)).unwrap();
    builder.inference_engine(Arc::new(InertEngine));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: "inert-test-engine".to_string(),
                default_model: "inert".to_string(),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let feature_config = AppServerFeatureConfig::default().with_workspace_registry_path(
        std::env::temp_dir()
            .join(format!("roder-cursor-sdk-e2e-{}", uuid::Uuid::new_v4()))
            .join("workspaces.json"),
    );
    LocalAppClient::new(Arc::new(AppServer::with_feature_config(
        runtime,
        feature_config,
    )))
}

/// Submits a `cursor-cloud-agent` task and waits for its terminal payload
/// through the public event stream.
async fn run_cloud_agent_task(
    client: &LocalAppClient,
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    input: serde_json::Value,
) -> (String, serde_json::Value) {
    let submitted: TasksSubmitResult = request(
        client,
        "tasks/submit",
        serde_json::to_value(TasksSubmitParams {
            executor_id: "cursor-cloud-agent".to_string(),
            input,
            thread_id: None,
            turn_id: None,
            workspace: None,
        })
        .unwrap(),
    )
    .await;
    assert_eq!(submitted.task.executor_id, "cursor-cloud-agent");

    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(30), events.recv())
            .await
            .expect("task completion event within 30s")
            .expect("event stream open");
        match &envelope.event {
            roder_api::events::RoderEvent::TaskCompleted(event)
                if event.task_id == submitted.task.task_id =>
            {
                return (submitted.task.task_id.clone(), event.payload.clone());
            }
            roder_api::events::RoderEvent::TaskFailed(event)
                if event.task_id == submitted.task.task_id =>
            {
                panic!("cloud agent task failed: {}", event.error);
            }
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cursor_sdk_extension_dispatches_and_resumes_cloud_agents() {
    if !node_available() {
        eprintln!("skipping: node is not installed");
        return;
    }
    let example = example_dir();
    if !example.join("dist/src/main.js").exists() {
        eprintln!(
            "skipping: build the extension first (npm ci && npm run build in {})",
            example.display()
        );
        return;
    }

    let client = client_for(fake_sdk_env());
    let mut events = client.subscribe_events();

    // The extension and its dispatcher/task services are visible through
    // the public surfaces, indistinguishable from native extensions.
    let extensions: ExtensionsListResult =
        request(&client, "extensions/list", serde_json::json!({})).await;
    let manifest = extensions
        .extensions
        .iter()
        .find(|extension| extension.id == "roder-ext-cursor-sdk")
        .unwrap_or_else(|| panic!("extension missing: {extensions:?}"));
    let provides = serde_json::to_string(&manifest.provides).unwrap();
    assert!(provides.contains("cursor-cloud"), "{provides}");
    assert!(provides.contains("cursor-cloud-agent"), "{provides}");

    // The dispatcher's child-declared definition surfaces through
    // `agents/list` once the background definitions fetch lands.
    let mut saw_cursor_cloud = false;
    for _ in 0..200 {
        let agents: AgentsListResult = request(&client, "agents/list", serde_json::json!({})).await;
        if agents
            .agents
            .iter()
            .any(|agent| agent.agent_type == "cursor-cloud")
        {
            saw_cursor_cloud = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        saw_cursor_cloud,
        "cursor-cloud must appear in agents/list from the child-declared definitions"
    );

    // Dispatch: create a cloud agent, stream progress into the task log,
    // and read the structured result with the bc- id and PR url.
    let (task_id, payload) = run_cloud_agent_task(
        &client,
        &mut events,
        serde_json::json!({
            "prompt": "Open a PR that fixes the flaky test",
            "repoUrl": "https://github.com/example-org/example-repo",
            "startingRef": "main",
            "autoCreatePr": true,
            "model": "composer-2.5",
        }),
    )
    .await;
    let agent_id = payload["agentId"].as_str().expect("agentId in payload");
    assert!(agent_id.starts_with("bc-"), "{payload}");
    assert_eq!(payload["status"], "finished");
    assert_eq!(payload["waited"], true);
    assert_eq!(payload["resumed"], false);
    assert_eq!(
        payload["prUrls"][0],
        "https://github.com/example-org/example-repo/pull/7"
    );
    assert!(
        payload["requestId"].as_str().is_some_and(|id| !id.is_empty()),
        "{payload}"
    );

    // The task record and its streamed log are readable afterwards; the
    // bc- id in the payload is what clients persist for later resume.
    let observed: TasksGetResult = request(
        &client,
        "tasks/get",
        serde_json::to_value(TasksGetParams {
            task_id: task_id.clone(),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(observed.task.state, TaskState::Completed);
    let log = observed
        .logs
        .iter()
        .map(|entry| entry.chunk.as_str())
        .collect::<String>();
    assert!(log.contains("created cloud agent bc-"), "{log}");
    assert!(log.contains("status: RUNNING"), "{log}");
    assert!(
        !log.contains("offline-e2e-placeholder-key"),
        "key material must never reach task logs: {log}"
    );

    // Resume: a second task targets the persisted bc- id and the child
    // reattaches through Agent.resume.
    let (_, resumed) = run_cloud_agent_task(
        &client,
        &mut events,
        serde_json::json!({
            "prompt": "Summarize what you did",
            "agentId": agent_id,
        }),
    )
    .await;
    assert_eq!(resumed["agentId"], agent_id);
    assert_eq!(resumed["resumed"], true);
    assert_eq!(resumed["status"], "finished");

    // Invalid input fails closed before any SDK work.
    let submitted: TasksSubmitResult = request(
        &client,
        "tasks/submit",
        serde_json::to_value(TasksSubmitParams {
            executor_id: "cursor-cloud-agent".to_string(),
            input: serde_json::json!({ "prompt": "no repo" }),
            thread_id: None,
            turn_id: None,
            workspace: None,
        })
        .unwrap(),
    )
    .await;
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(30), events.recv())
            .await
            .expect("task failure event within 30s")
            .expect("event stream open");
        if let roder_api::events::RoderEvent::TaskFailed(event) = &envelope.event
            && event.task_id == submitted.task.task_id
        {
            assert!(event.error.contains("repoUrl is required"), "{}", event.error);
            break;
        }
    }
}

/// Opt-in live check against the real `@cursor/sdk` and Cursor cloud:
///
/// ```sh
/// RODER_CURSOR_SDK_LIVE=1 \
/// CURSOR_API_KEY=... \
/// CURSOR_SDK_LIVE_REPO_URL="https://github.com/<org>/<disposable-repo>" \
/// cargo test -p roder-app-server --features e2e-tests \
///   --test process_extension_cursor_sdk -- --ignored --nocapture
/// ```
///
/// The dispatched cloud agent clones the configured repository and acts
/// with the key's Cursor account authority; point it at a disposable repo.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live Cursor cloud check; set RODER_CURSOR_SDK_LIVE=1, CURSOR_API_KEY, CURSOR_SDK_LIVE_REPO_URL"]
async fn cursor_sdk_extension_live_cloud_agent_dispatch_and_resume() {
    if std::env::var("RODER_CURSOR_SDK_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_CURSOR_SDK_LIVE=1 to run the live Cursor SDK check");
        return;
    }
    let api_key = std::env::var("CURSOR_API_KEY").expect("CURSOR_API_KEY env");
    let repo_url = std::env::var("CURSOR_SDK_LIVE_REPO_URL").expect("CURSOR_SDK_LIVE_REPO_URL env");
    assert!(node_available(), "node is required for the live check");
    let example = example_dir();
    assert!(
        example.join("dist/src/main.js").exists(),
        "build the extension first (npm ci && npm run build)"
    );

    let client = client_for(BTreeMap::from([("CURSOR_API_KEY".to_string(), api_key)]));
    let mut events = client.subscribe_events();

    // One real cloud agent dispatch with a trivial, non-destructive prompt.
    let (_, payload) = run_cloud_agent_task(
        &client,
        &mut events,
        serde_json::json!({
            "prompt": "Reply with a one-sentence summary of this repository. Do not modify any files.",
            "repoUrl": repo_url,
            "autoCreatePr": false,
        }),
    )
    .await;
    let agent_id = payload["agentId"].as_str().expect("agentId").to_string();
    assert!(agent_id.starts_with("bc-"), "{payload}");
    assert_eq!(payload["status"], "finished");
    let summary = payload["result"].as_str().unwrap_or_default();
    assert!(!summary.is_empty(), "live agent must return a summary");
    eprintln!("live cloud agent {agent_id} summarized the repo: {summary}");

    // Resume the same cloud agent by its persisted bc- id.
    let (_, resumed) = run_cloud_agent_task(
        &client,
        &mut events,
        serde_json::json!({
            "prompt": "Reply with exactly: RODER_CURSOR_SDK_RESUME_OK",
            "agentId": agent_id,
        }),
    )
    .await;
    assert_eq!(resumed["agentId"], agent_id);
    assert_eq!(resumed["resumed"], true);
    assert_eq!(resumed["status"], "finished");
    let resumed_text = resumed["result"].as_str().unwrap_or_default();
    assert!(
        resumed_text.contains("RODER_CURSOR_SDK_RESUME_OK"),
        "resumed agent must answer through the same conversation: {resumed_text}"
    );
    eprintln!("live cloud agent {agent_id} resume answered: {resumed_text}");
}
