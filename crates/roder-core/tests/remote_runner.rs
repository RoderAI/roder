use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::RoderEvent;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession, RunnerCapabilities,
    RunnerCommandId, RunnerCommandRequest, RunnerCommandResult, RunnerDestination,
    RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest, RunnerManifest,
    RunnerPortRequest, RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};
use roder_api::session::SessionStore;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_session::store::{JsonlSessionStore, JsonlSessionStoreFactory};
use roder_ext_runner_unix_local::UnixLocalRunnerProvider;
use time::OffsetDateTime;

struct FinalEngine {
    requests: Mutex<usize>,
}

struct ToolScriptEngine {
    requests: Mutex<usize>,
}

#[derive(Default)]
struct MockRunnerState {
    files: Mutex<HashMap<String, Vec<u8>>>,
    created: Mutex<usize>,
    resumed: Mutex<usize>,
    commands: Mutex<Vec<String>>,
}

#[derive(Clone, Default)]
struct MockRunnerProvider {
    state: Arc<MockRunnerState>,
}

struct MockRunnerToolContributor {
    state: Arc<MockRunnerState>,
}

struct MockRunnerWriteTool {
    state: Arc<MockRunnerState>,
}

struct MockRunnerReadTool {
    state: Arc<MockRunnerState>,
}

#[async_trait::async_trait]
impl InferenceEngine for FinalEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
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
        *self.requests.lock().unwrap() += 1;
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "done".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: Some("resp".to_string()),
            })),
        ])))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ToolScriptEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
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
        let mut requests = self.requests.lock().unwrap();
        *requests += 1;
        let index = *requests;
        drop(requests);
        let events = match index {
            1 => vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "write".to_string(),
                    name: "write_file".to_string(),
                    arguments: r#"{"path":"out.txt","content":"hello from runner"}"#.to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp-write".to_string()),
                })),
            ],
            2 => vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "read".to_string(),
                    name: "read_file".to_string(),
                    arguments: r#"{"path":"out.txt"}"#.to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp-read".to_string()),
                })),
            ],
            _ => vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "runner workflow complete".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: Some("resp-final".to_string()),
                })),
            ],
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

#[async_trait::async_trait]
impl RemoteRunnerProvider for MockRunnerProvider {
    fn id(&self) -> RemoteRunnerProviderId {
        "mock-hosted".to_string()
    }

    fn capabilities(&self) -> RunnerCapabilities {
        RunnerCapabilities {
            command_exec: true,
            file_read: true,
            file_write: true,
            port_preview: true,
            snapshots: true,
            cancellation: true,
            artifact_export: true,
            mounts: Default::default(),
        }
    }

    async fn create_session(
        &self,
        destination: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        *self.state.created.lock().unwrap() += 1;
        Ok(Arc::new(MockRunnerSession {
            state: self.state.clone(),
            destination_id: destination.id,
            session_id: "mock-session".to_string(),
        }))
    }

    async fn resume_session(
        &self,
        state: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        *self.state.resumed.lock().unwrap() += 1;
        Ok(Arc::new(MockRunnerSession {
            state: self.state.clone(),
            destination_id: state.destination_id,
            session_id: state.session_id,
        }))
    }
}

struct MockRunnerSession {
    state: Arc<MockRunnerState>,
    destination_id: String,
    session_id: String,
}

#[async_trait::async_trait]
impl RemoteRunnerSession for MockRunnerSession {
    fn state(&self) -> RunnerSessionState {
        RunnerSessionState {
            provider_id: "mock-hosted".to_string(),
            session_id: self.session_id.clone(),
            destination_id: self.destination_id.clone(),
            snapshot: None,
            metadata: serde_json::json!({ "mock": true }),
        }
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.state.commands.lock().unwrap().push(request.program);
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: Some(0),
            stdout: "command ok\n".to_string(),
            stderr: String::new(),
        })
    }

    async fn cancel_command(&self, _command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let key = request.path.to_string_lossy().to_string();
        let contents = self
            .state
            .files
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or_default();
        Ok(RunnerFileReadResult {
            path: request.path,
            contents,
        })
    }

    async fn write_file(&self, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        self.state
            .files
            .lock()
            .unwrap()
            .insert(request.path.to_string_lossy().to_string(), request.contents);
        Ok(())
    }

    async fn expose_port(&self, request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        Ok(RunnerPortResult {
            port: request.port,
            url: Some(format!("https://mock.runner/{}", request.port)),
        })
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        Ok(Some(RunnerSnapshotRef {
            provider_id: "mock-hosted".to_string(),
            snapshot_id: "snap-1".to_string(),
            metadata: serde_json::json!({ "kind": "mock" }),
        }))
    }

    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

impl ToolContributor for MockRunnerToolContributor {
    fn id(&self) -> ToolProviderId {
        "mock-runner-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(MockRunnerWriteTool {
            state: self.state.clone(),
        }))?;
        registry.register(Arc::new(MockRunnerReadTool {
            state: self.state.clone(),
        }))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for MockRunnerWriteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".to_string(),
            description: "Write a file through the mock runner".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let path = call
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let content = call
            .arguments
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .as_bytes()
            .to_vec();
        self.state
            .files
            .lock()
            .unwrap()
            .insert(path.clone(), content);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: format!("wrote {path}"),
            data: serde_json::json!({ "path": path }),
            is_error: false,
        })
    }
}

#[async_trait::async_trait]
impl ToolExecutor for MockRunnerReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".to_string(),
            description: "Read a file through the mock runner".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let path = call
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let contents = self
            .state
            .files
            .lock()
            .unwrap()
            .get(&path)
            .cloned()
            .unwrap_or_default();
        let text = String::from_utf8(contents)?;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: text.clone(),
            data: serde_json::json!({ "path": path, "contents": text }),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn remote_runner_state_persists_and_resumes_between_turns() {
    let session_dir = temp_dir("remote-runner-sessions");
    let workspace = temp_dir("remote-runner-workspace");
    let runtime = runtime(session_dir.clone(), workspace.clone()).await;
    let mut events = runtime.subscribe_events();
    let metadata = runtime.create_session(None).await.unwrap();
    assert!(metadata.runner_destination.is_some());
    assert!(metadata.runner_state.is_none());

    start_and_wait(&runtime, &mut events, &metadata.thread_id, "first").await;
    let first_state = runtime
        .load_session(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state
        .unwrap();

    start_and_wait(&runtime, &mut events, &metadata.thread_id, "second").await;
    let second_state = runtime
        .load_session(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state
        .unwrap();

    assert_eq!(second_state.provider_id, "unix-local");
    assert_eq!(second_state.destination_id, "unix-local");
    assert_eq!(second_state.session_id, first_state.session_id);

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn stale_remote_runner_state_falls_back_to_fresh_session() {
    let session_dir = temp_dir("remote-runner-stale-sessions");
    let workspace = temp_dir("remote-runner-stale-workspace");
    let runtime = runtime(session_dir.clone(), workspace.clone()).await;
    let mut events = runtime.subscribe_events();
    let metadata = runtime.create_session(None).await.unwrap();
    let store = JsonlSessionStore {
        base_path: session_dir.clone(),
    };
    let mut stale = metadata;
    stale.runner_state = Some(RunnerSessionState {
        provider_id: "unix-local".to_string(),
        session_id: "stale-session".to_string(),
        destination_id: "unix-local".to_string(),
        snapshot: None,
        metadata: serde_json::json!({ "root": "/definitely/missing/runner/root" }),
    });
    stale.updated_at = OffsetDateTime::now_utc();
    store.update_session_metadata(stale.clone()).await.unwrap();

    start_and_wait(&runtime, &mut events, &stale.thread_id, "recover").await;
    let recovered = runtime
        .load_session(&stale.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state
        .unwrap();

    assert_ne!(recovered.session_id, "stale-session");
    assert_eq!(recovered.destination_id, "unix-local");

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn mock_runner_e2e_tools_command_port_snapshot_resume_and_continue() {
    let session_dir = temp_dir("remote-runner-e2e-sessions");
    let workspace = temp_dir("remote-runner-e2e-workspace");
    let provider = MockRunnerProvider::default();
    let engine = Arc::new(ToolScriptEngine {
        requests: Mutex::new(0),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.session_store_factory(Arc::new(JsonlSessionStoreFactory {
        base_path: session_dir.clone(),
    }));
    builder.remote_runner_provider(Arc::new(provider.clone()));
    builder.tool_contributor(Arc::new(MockRunnerToolContributor {
        state: provider.state.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: None,
                file_backed_dynamic_context: true,
                hosted_web_search: HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                workspace: Some(workspace.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::AcceptAll,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                remote_runner_destination: Some(RunnerDestination {
                    id: "mock-hosted".to_string(),
                    provider_id: "mock-hosted".to_string(),
                    config: serde_json::Value::Null,
                    default_manifest: RunnerManifest::default(),
                }),
                team_data_dir: None,
                roadmap_data_dir: None,
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();
    let metadata = runtime.create_session(None).await.unwrap();

    start_and_wait(&runtime, &mut events, &metadata.thread_id, "write and read").await;
    assert_eq!(
        provider
            .state
            .files
            .lock()
            .unwrap()
            .get("out.txt")
            .cloned()
            .unwrap(),
        b"hello from runner".to_vec()
    );

    let runner_state = runtime
        .load_session(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state
        .unwrap();
    let resumed = provider.resume_session(runner_state).await.unwrap();
    let command = resumed
        .run_command(RunnerCommandRequest {
            command_id: "cmd-1".to_string(),
            program: "echo".to_string(),
            args: vec!["ok".to_string()],
            cwd: None,
            env: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(command.stdout, "command ok\n");
    assert_eq!(
        resumed
            .expose_port(RunnerPortRequest {
                port: 3000,
                label: Some("web".to_string())
            })
            .await
            .unwrap()
            .url
            .as_deref(),
        Some("https://mock.runner/3000")
    );
    assert!(resumed.snapshot().await.unwrap().is_some());

    start_and_wait(&runtime, &mut events, &metadata.thread_id, "continue").await;
    assert_eq!(*provider.state.created.lock().unwrap(), 1);
    assert!(
        *provider.state.resumed.lock().unwrap() >= 2,
        "runtime and explicit resume should reuse the mock runner session"
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

async fn runtime(session_dir: PathBuf, workspace: PathBuf) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FinalEngine {
        requests: Mutex::new(0),
    }));
    builder.session_store_factory(Arc::new(JsonlSessionStoreFactory {
        base_path: session_dir,
    }));
    builder.remote_runner_provider(Arc::new(UnixLocalRunnerProvider::default()));
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: None,
                file_backed_dynamic_context: true,
                hosted_web_search: HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                workspace: Some(workspace.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                remote_runner_destination: Some(RunnerDestination {
                    id: "unix-local".to_string(),
                    provider_id: "unix-local".to_string(),
                    config: serde_json::json!({ "root": workspace.display().to_string() }),
                    default_manifest: RunnerManifest::default(),
                }),
                team_data_dir: None,
                roadmap_data_dir: None,
            },
        )
        .unwrap(),
    )
}

async fn start_and_wait(
    runtime: &Arc<Runtime>,
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &str,
    message: &str,
) {
    runtime
        .start_turn(StartTurnRequest {
            thread_id: thread_id.to_string(),
            message: message.to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            workspace: None,

            instructions: default_instructions(),
        })
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event.event {
            RoderEvent::TurnCompleted(_) => break,
            RoderEvent::TurnFailed(failed) => panic!("turn failed: {}", failed.error),
            _ => {}
        }
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}
