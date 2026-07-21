use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::RoderEvent;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerProviderId, RemoteRunnerSession,
    RemoteWorkspaceExecutionLease, RunnerCapabilities, RunnerCommandId, RunnerCommandRequest,
    RunnerCommandResult, RunnerDestination, RunnerFileReadRequest, RunnerFileReadResult,
    RunnerFileWriteRequest, RunnerManifest, RunnerPortRequest, RunnerPortResult,
    RunnerSessionState, RunnerSnapshotRef, RunnerWorkspaceExecutionLeaseRequest,
};
use roder_api::thread::ThreadStore;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::{JsonlThreadStore, JsonlThreadStoreFactory};
use roder_ext_runner_sprites::{
    LIVE_ENV as SPRITES_LIVE_ENV, PROVIDER_ID as SPRITES_PROVIDER_ID, SpritesRunnerProvider,
};
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
    commands: Mutex<Vec<RunnerCommandRequest>>,
    default_workspace: Mutex<Option<String>>,
    create_started: tokio::sync::Notify,
    create_delay_ms: AtomicU64,
    create_failures_remaining: Mutex<usize>,
    synchronize_reads: AtomicBool,
    reads_started: AtomicU64,
    first_read_started: tokio::sync::Notify,
    second_read_started: tokio::sync::Notify,
    workspace_leases_enabled: AtomicBool,
    workspace_lease_lock: Arc<tokio::sync::Mutex<()>>,
    workspace_lease_acquire_delay_ms: AtomicU64,
    workspace_lease_requests: Mutex<Vec<RunnerWorkspaceExecutionLeaseRequest>>,
    workspace_lease_acquisitions: AtomicU64,
    workspace_lease_releases: AtomicU64,
    workspace_lease_lost: AtomicBool,
    workspace_lease_lost_notify: tokio::sync::Notify,
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
            pausable: true,
            detachable: true,
        }
    }

    fn default_workspace(&self) -> Option<String> {
        self.state.default_workspace.lock().unwrap().clone()
    }

    async fn create_session(
        &self,
        destination: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        *self.state.created.lock().unwrap() += 1;
        self.state.create_started.notify_waiters();
        let delay_ms = self.state.create_delay_ms.load(Ordering::SeqCst);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let should_fail = {
            let mut remaining = self.state.create_failures_remaining.lock().unwrap();
            if *remaining == 0 {
                false
            } else {
                *remaining -= 1;
                true
            }
        };
        if should_fail {
            anyhow::bail!("mock runner creation failed");
        }
        Ok(Arc::new(MockRunnerSession {
            state: self.state.clone(),
            destination_id: destination.id,
            session_id: "mock-session".to_string(),
            paused: std::sync::atomic::AtomicBool::new(false),
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
            paused: std::sync::atomic::AtomicBool::new(false),
        }))
    }
}

struct MockRunnerSession {
    state: Arc<MockRunnerState>,
    destination_id: String,
    session_id: String,
    paused: std::sync::atomic::AtomicBool,
}

struct MockWorkspaceExecutionLease {
    state: Arc<MockRunnerState>,
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

#[async_trait::async_trait]
impl RemoteWorkspaceExecutionLease for MockWorkspaceExecutionLease {
    async fn wait_lost(&self) -> anyhow::Result<()> {
        loop {
            let lost = self.state.workspace_lease_lost_notify.notified();
            if self.state.workspace_lease_lost.load(Ordering::SeqCst) {
                anyhow::bail!("mock provider dropped the workspace execution lease");
            }
            lost.await;
        }
    }

    async fn release(mut self: Box<Self>) -> anyhow::Result<()> {
        self.guard.take();
        self.state
            .workspace_lease_releases
            .fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl RemoteRunnerSession for MockRunnerSession {
    fn state(&self) -> RunnerSessionState {
        RunnerSessionState {
            provider_id: "mock-hosted".to_string(),
            session_id: self.session_id.clone(),
            destination_id: self.destination_id.clone(),
            snapshot: None,
            metadata: serde_json::json!({
                "mock": true,
                "paused": self.paused.load(std::sync::atomic::Ordering::SeqCst),
            }),
        }
    }

    async fn acquire_workspace_execution_lease(
        &self,
        request: RunnerWorkspaceExecutionLeaseRequest,
    ) -> anyhow::Result<Option<Box<dyn RemoteWorkspaceExecutionLease>>> {
        if !self.state.workspace_leases_enabled.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let delay_ms = self
            .state
            .workspace_lease_acquire_delay_ms
            .load(Ordering::SeqCst);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let guard = self.state.workspace_lease_lock.clone().lock_owned().await;
        self.state
            .workspace_lease_requests
            .lock()
            .unwrap()
            .push(request);
        self.state
            .workspace_lease_acquisitions
            .fetch_add(1, Ordering::SeqCst);
        Ok(Some(Box::new(MockWorkspaceExecutionLease {
            state: self.state.clone(),
            guard: Some(guard),
        })))
    }

    async fn pause(&self) -> anyhow::Result<RunnerSessionState> {
        self.paused.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(self.state())
    }

    async fn resume(&self) -> anyhow::Result<RunnerSessionState> {
        self.paused
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(self.state())
    }

    async fn detach(&self) -> anyhow::Result<RunnerSessionState> {
        Ok(self.state())
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.state.commands.lock().unwrap().push(request.clone());
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
        if self.state.synchronize_reads.load(Ordering::SeqCst) {
            let second_read_started = self.state.second_read_started.notified();
            let read_index = self.state.reads_started.fetch_add(1, Ordering::SeqCst);
            self.state.first_read_started.notify_waiters();
            if read_index == 0 {
                let _ = tokio::time::timeout(Duration::from_millis(250), second_read_started).await;
            } else if read_index == 1 {
                self.state.second_read_started.notify_waiters();
            }
        }
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
async fn runner_bound_text_only_turn_does_not_provision_session() {
    let session_dir = temp_dir("remote-runner-sessions");
    let workspace = temp_dir("remote-runner-workspace");
    let runtime = runtime(session_dir.clone(), workspace.clone()).await;
    let mut events = runtime.subscribe_events();
    let metadata = runtime
        .create_thread_with(unix_runner_thread_request(&workspace))
        .await
        .unwrap();
    assert!(metadata.runner_destination.is_some());
    assert!(metadata.runner_state.is_none());

    start_and_wait(&runtime, &mut events, &metadata.thread_id, "text only").await;
    let state = runtime
        .load_thread(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state;

    assert!(state.is_none());

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn stale_remote_runner_state_falls_back_to_fresh_session() {
    let session_dir = temp_dir("remote-runner-stale-sessions");
    let workspace = temp_dir("remote-runner-stale-workspace");
    let runtime = runtime(session_dir.clone(), workspace.clone()).await;
    let metadata = runtime
        .create_thread_with(unix_runner_thread_request(&workspace))
        .await
        .unwrap();
    let destination_id = metadata
        .runner_binding
        .as_ref()
        .unwrap()
        .destination
        .id
        .clone();
    let store = JsonlThreadStore {
        base_path: session_dir.clone(),
    };
    let mut stale = metadata;
    stale.runner_state = Some(RunnerSessionState {
        provider_id: "unix-local".to_string(),
        session_id: "stale-session".to_string(),
        destination_id: destination_id.clone(),
        snapshot: None,
        metadata: serde_json::json!({ "root": "/definitely/missing/runner/root" }),
    });
    stale.updated_at = OffsetDateTime::now_utc();
    store.update_thread_metadata(stale.clone()).await.unwrap();

    let error = runtime
        .pause_thread_runner(&stale.thread_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not support pausing"));
    let recovered = runtime
        .load_thread(&stale.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state
        .unwrap();

    assert_ne!(recovered.session_id, "stale-session");
    assert_eq!(recovered.destination_id, destination_id);

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn mock_runner_e2e_tools_command_port_snapshot_resume_and_continue() {
    let session_dir = temp_dir("remote-runner-e2e-sessions");
    let workspace = temp_dir("remote-runner-e2e-workspace");
    let provider = MockRunnerProvider::default();
    *provider.state.default_workspace.lock().unwrap() = Some("/sandbox/workspace".to_string());
    let engine = Arc::new(ToolScriptEngine {
        requests: Mutex::new(0),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
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
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: Some(workspace.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::AcceptAll,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: Some(RunnerDestination {
                    id: "mock-hosted".to_string(),
                    provider_id: "mock-hosted".to_string(),
                    config: serde_json::Value::Null,
                    default_manifest: RunnerManifest::default(),
                }),
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();
    let metadata = runtime.create_thread(None).await.unwrap();

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
        .load_thread(&metadata.thread_id)
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
            timeout_ms: None,
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
    assert_eq!(
        *provider.state.resumed.lock().unwrap(),
        1,
        "the live runtime session is cached; only the explicit test resume should reattach"
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn runtime_destination_auto_binds_new_thread_when_provider_has_default_workspace() {
    let session_dir = temp_dir("remote-runner-autobind-sessions");
    let workspace = temp_dir("remote-runner-autobind-workspace");
    let provider = MockRunnerProvider::default();
    // Provider advertises a default workspace -> a runtime-level destination
    // (as set by the TUI runner picker / config default) should auto-bind.
    *provider.state.default_workspace.lock().unwrap() = Some("/runner/workspace".to_string());
    let engine = Arc::new(ToolScriptEngine {
        requests: Mutex::new(0),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: session_dir.clone(),
    }));
    builder.remote_runner_provider(Arc::new(provider.clone()));
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
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: Some(workspace.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::AcceptAll,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: Some(RunnerDestination {
                    id: "mock-hosted".to_string(),
                    provider_id: "mock-hosted".to_string(),
                    config: serde_json::Value::Null,
                    default_manifest: RunnerManifest::default(),
                }),
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );

    let metadata = runtime.create_thread(None).await.unwrap();
    let binding = metadata
        .runner_binding
        .expect("runtime-level destination should auto-bind a new thread");
    assert_eq!(binding.destination.provider_id, "mock-hosted");
    assert_eq!(
        binding.workspace,
        std::path::PathBuf::from("/runner/workspace")
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn runner_pause_resume_detach_and_rejoin_reuse_one_session() {
    let session_dir = temp_dir("remote-runner-lifecycle-sessions");
    let workspace = temp_dir("remote-runner-lifecycle-workspace");
    let provider = MockRunnerProvider::default();
    let engine = Arc::new(ToolScriptEngine {
        requests: Mutex::new(0),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: session_dir.clone(),
    }));
    builder.remote_runner_provider(Arc::new(provider.clone()));
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
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: Some(workspace.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::AcceptAll,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: Some(RunnerDestination {
                    id: "mock-hosted".to_string(),
                    provider_id: "mock-hosted".to_string(),
                    config: serde_json::Value::Null,
                    default_manifest: RunnerManifest::default(),
                }),
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let metadata = runtime.create_thread(None).await.unwrap();
    let thread_id = metadata.thread_id.clone();

    // Pause establishes (creates) the session, then marks standby intent.
    let paused = runtime.pause_thread_runner(&thread_id).await.unwrap();
    assert_eq!(paused.metadata["paused"].as_bool(), Some(true));
    assert_eq!(*provider.state.created.lock().unwrap(), 1);

    // Resume wakes the (resumed) session.
    let resumed = runtime.resume_thread_runner(&thread_id).await.unwrap();
    assert_eq!(resumed.metadata["paused"].as_bool(), Some(false));

    // Detach persists durable, rejoinable state without deleting the sandbox.
    let detached = runtime.detach_thread_runner(&thread_id).await.unwrap();
    assert_eq!(detached.provider_id, "mock-hosted");
    let persisted = runtime
        .load_thread(&thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state
        .unwrap();
    assert_eq!(persisted.session_id, detached.session_id);

    // Rejoin (simulating a fresh process) reuses the same sandbox via the
    // persisted state and never provisions a new one.
    let rejoined = runtime
        .rejoin_thread_runner(&thread_id, None)
        .await
        .unwrap();
    assert_eq!(rejoined.session_id, detached.session_id);
    assert_eq!(
        *provider.state.created.lock().unwrap(),
        1,
        "rejoin must not create a new sandbox"
    );
    assert_eq!(*provider.state.resumed.lock().unwrap(), 1);

    // Rejoin refreshes the live cache; a subsequent lifecycle operation uses
    // that handle instead of asking the provider to reattach again.
    runtime.resume_thread_runner(&thread_id).await.unwrap();
    assert_eq!(*provider.state.resumed.lock().unwrap(), 1);

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
#[ignore]
async fn live_sprites_runner_runtime_creates_session_and_offloads_operations() {
    if std::env::var(SPRITES_LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("set {SPRITES_LIVE_ENV}=1 to run the live Sprites runtime smoke");
        return;
    }

    let session_dir = temp_dir("remote-runner-sprites-live-sessions");
    let workspace = temp_dir("remote-runner-sprites-live-workspace");
    let provider = Arc::new(SpritesRunnerProvider::default());
    let engine = Arc::new(FinalEngine {
        requests: Mutex::new(0),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: session_dir.clone(),
    }));
    builder.remote_runner_provider(provider.clone());
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
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: Some(workspace.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::AcceptAll,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: Some(RunnerDestination {
                    id: "sprites-live-runtime".to_string(),
                    provider_id: SPRITES_PROVIDER_ID.to_string(),
                    config: serde_json::json!({
                        "sprite_name_prefix": "roder-core-live",
                        "cleanup": "delete-on-close",
                        "working_dir": "/home/sprite/roder-core-live"
                    }),
                    default_manifest: RunnerManifest::default(),
                }),
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();
    let metadata = runtime.create_thread(None).await.unwrap();

    start_and_wait(
        &runtime,
        &mut events,
        &metadata.thread_id,
        "create live sprites runner session",
    )
    .await;
    let runner_state = runtime
        .load_thread(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state
        .unwrap();
    assert_eq!(runner_state.provider_id, SPRITES_PROVIDER_ID);
    assert_eq!(runner_state.destination_id, "sprites-live-runtime");

    let session = provider.resume_session(runner_state).await.unwrap();
    let command = session
        .run_command(RunnerCommandRequest {
            command_id: "live-runtime-python".to_string(),
            program: "python3".to_string(),
            args: vec!["-c".to_string(), "print(2+2)".to_string()],
            cwd: None,
            env: Vec::new(),
            timeout_ms: None,
        })
        .await
        .unwrap();
    assert_eq!(command.exit_code, Some(0));
    assert_eq!(command.stdout.trim(), "4");
    session
        .write_file(RunnerFileWriteRequest {
            path: "runtime-proof.txt".into(),
            contents: b"hello from roder runtime\n".to_vec(),
        })
        .await
        .unwrap();
    let read = session
        .read_file(RunnerFileReadRequest {
            path: "runtime-proof.txt".into(),
        })
        .await
        .unwrap();
    assert_eq!(read.contents, b"hello from roder runtime\n");
    assert!(session.snapshot().await.unwrap().is_some());
    session.close().await.unwrap();

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(workspace);
}

/// Serves one scripted event batch per inference round, then a final message.
struct ScriptedEngine {
    rounds: Mutex<Vec<Vec<InferenceEvent>>>,
}

#[async_trait::async_trait]
impl InferenceEngine for ScriptedEngine {
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
        let mut rounds = self.rounds.lock().unwrap();
        let events = if rounds.is_empty() {
            vec![
                InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
                    phase: None,
                }),
                InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: Some("resp-final".to_string()),
                }),
            ]
        } else {
            rounds.remove(0)
        };
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

fn tool_call_round(calls: &[(&str, &str, &str)]) -> Vec<InferenceEvent> {
    let mut events = calls
        .iter()
        .map(|(id, name, arguments)| {
            InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: id.to_string(),
                name: name.to_string(),
                arguments: arguments.to_string(),
            })
        })
        .collect::<Vec<_>>();
    events.push(InferenceEvent::Completed(CompletionMetadata {
        stop_reason: Some("tool_calls".to_string()),
        provider_response_id: Some("resp-tools".to_string()),
    }));
    events
}

fn final_round() -> Vec<InferenceEvent> {
    vec![
        InferenceEvent::MessageDelta(MessageDelta {
            text: "done".to_string(),
            phase: None,
        }),
        InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("stop".to_string()),
            provider_response_id: Some("resp-final".to_string()),
        }),
    ]
}

fn coding_tools_runtime(
    session_dir: PathBuf,
    scratch: PathBuf,
    provider: MockRunnerProvider,
    rounds: Vec<Vec<InferenceEvent>>,
) -> Arc<Runtime> {
    coding_tools_runtime_with_deadline(session_dir, scratch, provider, rounds, None)
}

fn coding_tools_runtime_with_deadline(
    session_dir: PathBuf,
    scratch: PathBuf,
    provider: MockRunnerProvider,
    rounds: Vec<Vec<InferenceEvent>>,
    turn_deadline_seconds: Option<u64>,
) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ScriptedEngine {
        rounds: Mutex::new(rounds),
    }));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: session_dir,
    }));
    builder.remote_runner_provider(Arc::new(provider));
    builder
        .tool_contributor(roder_tools::builtin_coding_tools_contributor(scratch.clone()).unwrap());
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(scratch.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
                runtime_profile: if turn_deadline_seconds.is_some() {
                    roder_api::inference::RuntimeProfile::NonInteractive
                } else {
                    roder_api::inference::RuntimeProfile::Interactive
                },
                turn_deadline_seconds,
                model_parallel_tool_calls: HashMap::from([("mock".to_string(), true)]),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

#[derive(Debug, Clone)]
struct CompletedToolCall {
    tool_name: Option<String>,
    output: Option<String>,
    is_error: bool,
}

async fn run_turn_collecting_tool_calls(
    runtime: &Arc<Runtime>,
    thread_id: &str,
    workspace: &std::path::Path,
) -> Vec<CompletedToolCall> {
    let mut events = runtime.subscribe_events();
    runtime
        .start_turn(StartTurnRequest {
            thread_id: thread_id.to_string(),
            message: "run the scripted tools".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: workspace.display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();
    let mut completed = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event.event {
            RoderEvent::ToolCallCompleted(call) => completed.push(CompletedToolCall {
                tool_name: call.tool_name,
                output: call.output,
                is_error: call.is_error,
            }),
            RoderEvent::ExternalToolCallRequested(request) => {
                runtime
                    .resolve_external_tool_call(
                        &request.request_id,
                        roder_core::ExternalToolResolution {
                            output: "external MCP result".to_string(),
                            is_error: false,
                        },
                    )
                    .await
                    .unwrap();
            }
            RoderEvent::TurnCompleted(_) => break,
            RoderEvent::TurnFailed(failed) => panic!("turn failed: {}", failed.error),
            _ => {}
        }
    }
    completed
}

#[tokio::test]
async fn runner_bound_mcp_only_turn_does_not_provision_session() {
    let session_dir = temp_dir("runner-mcp-only-sessions");
    let scratch = temp_dir("runner-mcp-only-scratch");
    let provider = MockRunnerProvider::default();
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "mcp-list",
            "mcp__vex__repositories",
            r#"{"organization":"vex"}"#,
        )])],
    );
    let mut request = mock_runner_thread_request(&scratch);
    request.external_tools = vec![ToolSpec {
        name: "mcp__vex__repositories".to_string(),
        description: "List repositories through Vex MCP".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "organization": { "type": "string" } },
            "additionalProperties": false
        }),
    }];
    let metadata = runtime.create_thread_with(request).await.unwrap();

    let completed = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;

    assert!(completed.iter().all(|call| !call.is_error));
    assert_eq!(*provider.state.created.lock().unwrap(), 0);
    let persisted = runtime
        .load_thread(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state;
    assert!(persisted.is_none());

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn concurrent_first_workspace_tools_initialize_one_runner_session() {
    let session_dir = temp_dir("runner-singleflight-sessions");
    let scratch = temp_dir("runner-singleflight-scratch");
    let provider = MockRunnerProvider::default();
    provider.state.create_delay_ms.store(25, Ordering::SeqCst);
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[
            ("write-a", "write_file", r#"{"path":"a.txt","content":"a"}"#),
            ("write-b", "write_file", r#"{"path":"b.txt","content":"b"}"#),
        ])],
    );
    let metadata = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();

    let completed = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;

    assert!(completed.iter().all(|call| !call.is_error));
    assert_eq!(*provider.state.created.lock().unwrap(), 1);
    let persisted = runtime
        .load_thread(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state;
    assert!(persisted.is_some());

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn runner_tool_timeout_uses_deadline_remaining_after_lazy_provisioning() {
    let session_dir = temp_dir("runner-deadline-sessions");
    let scratch = temp_dir("runner-deadline-scratch");
    let provider = MockRunnerProvider::default();
    provider
        .state
        .create_delay_ms
        .store(2_100, Ordering::SeqCst);
    let runtime = coding_tools_runtime_with_deadline(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "deadline-exec",
            "exec_command",
            r#"{"cmd":"true"}"#,
        )])],
        Some(34),
    );
    let metadata = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();

    let completed = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;

    assert!(completed.iter().all(|call| !call.is_error));
    let commands = provider.state.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].timeout_ms,
        Some(1_000),
        "the command lease must exclude time spent provisioning the runner"
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn runner_tool_timeout_uses_deadline_remaining_after_workspace_lease_wait() {
    let session_dir = temp_dir("runner-lease-deadline-sessions");
    let scratch = temp_dir("runner-lease-deadline-scratch");
    let provider = MockRunnerProvider::default();
    provider
        .state
        .workspace_leases_enabled
        .store(true, Ordering::SeqCst);
    provider
        .state
        .workspace_lease_acquire_delay_ms
        .store(2_100, Ordering::SeqCst);
    let runtime = coding_tools_runtime_with_deadline(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "lease-deadline-exec",
            "exec_command",
            r#"{"cmd":"true"}"#,
        )])],
        Some(34),
    );
    let metadata = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();

    let completed = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;

    assert!(completed.iter().all(|call| !call.is_error));
    let commands = provider.state.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].timeout_ms,
        Some(1_000),
        "the command lease must exclude time spent waiting for the workspace fence"
    );
    let lease_requests = provider.state.workspace_lease_requests.lock().unwrap();
    assert_eq!(lease_requests.len(), 1);
    assert!(lease_requests[0].acquire_timeout_ms > 0);
    assert!(lease_requests[0].lease_timeout_ms.is_some());
    assert_eq!(
        provider
            .state
            .workspace_lease_releases
            .load(Ordering::SeqCst),
        1
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn shared_runner_tool_calls_are_serialized_across_threads() {
    let session_dir = temp_dir("runner-tool-serialization-sessions");
    let scratch = temp_dir("runner-tool-serialization-scratch");
    let provider = MockRunnerProvider::default();
    provider
        .state
        .files
        .lock()
        .unwrap()
        .insert("shared.txt".to_string(), b"alpha beta\n".to_vec());
    provider
        .state
        .synchronize_reads
        .store(true, Ordering::SeqCst);
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![
            tool_call_round(&[(
                "edit-alpha",
                "edit",
                r#"{"path":"shared.txt","old_string":"alpha","new_string":"ALPHA"}"#,
            )]),
            tool_call_round(&[(
                "edit-beta",
                "edit",
                r#"{"path":"shared.txt","old_string":"beta","new_string":"BETA"}"#,
            )]),
            final_round(),
            final_round(),
        ],
    );
    let first = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();
    let second = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();
    let mut events = runtime.subscribe_events();

    for thread_id in [&first.thread_id, &second.thread_id] {
        runtime
            .start_turn(StartTurnRequest {
                thread_id: thread_id.clone(),
                message: "edit the shared runner workspace".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: scratch.display().to_string(),
                instructions: default_instructions(),
                developer_context: None,
                task_ledger_required: false,
            })
            .await
            .unwrap();
    }

    let target_threads = HashMap::from([
        (first.thread_id.clone(), false),
        (second.thread_id.clone(), false),
    ]);
    let mut completed_threads = target_threads;
    while completed_threads.values().any(|completed| !completed) {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event.event {
            RoderEvent::TurnCompleted(completed) => {
                if let Some(done) = completed_threads.get_mut(&completed.thread_id) {
                    *done = true;
                }
            }
            RoderEvent::TurnFailed(failed) if completed_threads.contains_key(&failed.thread_id) => {
                panic!("turn failed: {}", failed.error)
            }
            _ => {}
        }
    }

    assert_eq!(*provider.state.created.lock().unwrap(), 2);
    assert_eq!(
        provider
            .state
            .files
            .lock()
            .unwrap()
            .get("shared.txt")
            .cloned(),
        Some(b"ALPHA BETA\n".to_vec())
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn provider_workspace_lease_serializes_shared_session_across_runtimes() {
    let first_session_dir = temp_dir("runner-cross-runtime-first-sessions");
    let second_session_dir = temp_dir("runner-cross-runtime-second-sessions");
    let first_scratch = temp_dir("runner-cross-runtime-first-scratch");
    let second_scratch = temp_dir("runner-cross-runtime-second-scratch");
    let provider = MockRunnerProvider::default();
    provider
        .state
        .workspace_leases_enabled
        .store(true, Ordering::SeqCst);
    provider
        .state
        .files
        .lock()
        .unwrap()
        .insert("shared.txt".to_string(), b"alpha beta\n".to_vec());
    provider
        .state
        .synchronize_reads
        .store(true, Ordering::SeqCst);
    let first_runtime = coding_tools_runtime(
        first_session_dir.clone(),
        first_scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "edit-alpha",
            "edit",
            r#"{"path":"shared.txt","old_string":"alpha","new_string":"ALPHA"}"#,
        )])],
    );
    let second_runtime = coding_tools_runtime(
        second_session_dir.clone(),
        second_scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "edit-beta",
            "edit",
            r#"{"path":"shared.txt","old_string":"beta","new_string":"BETA"}"#,
        )])],
    );
    let first = first_runtime
        .create_thread_with(mock_runner_thread_request(&first_scratch))
        .await
        .unwrap();
    let second = second_runtime
        .create_thread_with(mock_runner_thread_request(&second_scratch))
        .await
        .unwrap();

    let (first_completed, second_completed) = tokio::join!(
        run_turn_collecting_tool_calls(&first_runtime, &first.thread_id, &first_scratch),
        run_turn_collecting_tool_calls(&second_runtime, &second.thread_id, &second_scratch),
    );

    assert!(first_completed.iter().all(|call| !call.is_error));
    assert!(second_completed.iter().all(|call| !call.is_error));
    assert_eq!(
        provider
            .state
            .files
            .lock()
            .unwrap()
            .get("shared.txt")
            .cloned(),
        Some(b"ALPHA BETA\n".to_vec())
    );
    assert_eq!(
        provider
            .state
            .workspace_lease_acquisitions
            .load(Ordering::SeqCst),
        2
    );
    assert_eq!(
        provider
            .state
            .workspace_lease_releases
            .load(Ordering::SeqCst),
        2
    );

    let _ = std::fs::remove_dir_all(first_session_dir);
    let _ = std::fs::remove_dir_all(second_session_dir);
    let _ = std::fs::remove_dir_all(first_scratch);
    let _ = std::fs::remove_dir_all(second_scratch);
}

#[tokio::test]
async fn lost_provider_workspace_lease_cancels_tool_before_its_write() {
    let session_dir = temp_dir("runner-lost-lease-sessions");
    let scratch = temp_dir("runner-lost-lease-scratch");
    let provider = MockRunnerProvider::default();
    provider
        .state
        .workspace_leases_enabled
        .store(true, Ordering::SeqCst);
    provider
        .state
        .synchronize_reads
        .store(true, Ordering::SeqCst);
    provider
        .state
        .files
        .lock()
        .unwrap()
        .insert("shared.txt".to_string(), b"alpha\n".to_vec());
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "edit-alpha",
            "edit",
            r#"{"path":"shared.txt","old_string":"alpha","new_string":"ALPHA"}"#,
        )])],
    );
    let metadata = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();
    let runtime_for_turn = runtime.clone();
    let thread_id = metadata.thread_id.clone();
    let scratch_for_turn = scratch.clone();
    let turn = tokio::spawn(async move {
        run_turn_collecting_tool_calls(&runtime_for_turn, &thread_id, &scratch_for_turn).await
    });

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let started = provider.state.first_read_started.notified();
            if provider.state.reads_started.load(Ordering::SeqCst) > 0 {
                break;
            }
            started.await;
        }
    })
    .await
    .expect("the edit should begin its read while holding the provider lease");
    provider
        .state
        .workspace_lease_lost
        .store(true, Ordering::SeqCst);
    provider.state.workspace_lease_lost_notify.notify_waiters();

    let completed = turn.await.unwrap();
    let edit = completed
        .iter()
        .find(|call| call.tool_name.as_deref() == Some("edit"))
        .expect("the lost-lease edit should produce a tool result");
    assert!(edit.is_error);
    assert!(edit.output.as_deref().unwrap().contains("lease was lost"));
    assert_eq!(
        provider
            .state
            .files
            .lock()
            .unwrap()
            .get("shared.txt")
            .cloned(),
        Some(b"alpha\n".to_vec())
    );
    assert_eq!(
        provider
            .state
            .workspace_lease_releases
            .load(Ordering::SeqCst),
        1
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn runner_session_is_reused_by_workspace_tools_on_later_turns() {
    let session_dir = temp_dir("runner-reuse-sessions");
    let scratch = temp_dir("runner-reuse-scratch");
    let provider = MockRunnerProvider::default();
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![
            tool_call_round(&[(
                "write",
                "write_file",
                r#"{"path":"reuse.txt","content":"same session"}"#,
            )]),
            final_round(),
            tool_call_round(&[("read", "read_file", r#"{"path":"reuse.txt"}"#)]),
        ],
    );
    let metadata = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();

    run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;
    let second = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;

    assert_eq!(*provider.state.created.lock().unwrap(), 1);
    assert_eq!(*provider.state.resumed.lock().unwrap(), 0);
    let read = second
        .iter()
        .find(|call| call.tool_name.as_deref() == Some("read_file"))
        .unwrap();
    assert!(read.output.as_deref().unwrap().contains("same session"));

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn failed_runner_initialization_is_retryable_without_local_fallback() {
    let session_dir = temp_dir("runner-retry-sessions");
    let scratch = temp_dir("runner-retry-scratch");
    let provider = MockRunnerProvider::default();
    *provider.state.create_failures_remaining.lock().unwrap() = 1;
    provider.state.create_delay_ms.store(25, Ordering::SeqCst);
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![
            tool_call_round(&[
                (
                    "first-write-a",
                    "write_file",
                    r#"{"path":"retry-a.txt","content":"remote only"}"#,
                ),
                (
                    "first-write-b",
                    "write_file",
                    r#"{"path":"retry-b.txt","content":"remote only"}"#,
                ),
            ]),
            final_round(),
            tool_call_round(&[(
                "second-write",
                "write_file",
                r#"{"path":"retry.txt","content":"remote only"}"#,
            )]),
        ],
    );
    let metadata = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();

    let first = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;
    assert!(
        first.iter().all(|call| call.is_error),
        "expected every concurrent caller to share the initialization error: {first:?}"
    );
    assert_eq!(*provider.state.created.lock().unwrap(), 1);
    assert!(!scratch.join("retry-a.txt").exists());
    assert!(!scratch.join("retry-b.txt").exists());
    let first_state = runtime
        .load_thread(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state;
    assert!(first_state.is_none());

    let second = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;
    assert!(second.iter().all(|call| !call.is_error));
    assert_eq!(*provider.state.created.lock().unwrap(), 2);
    assert_eq!(
        provider
            .state
            .files
            .lock()
            .unwrap()
            .get("retry.txt")
            .cloned(),
        Some(b"remote only".to_vec())
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn interrupted_turn_does_not_strand_runner_initialization() {
    let session_dir = temp_dir("runner-interrupt-sessions");
    let scratch = temp_dir("runner-interrupt-scratch");
    let provider = MockRunnerProvider::default();
    *provider.state.create_failures_remaining.lock().unwrap() = 1;
    provider.state.create_delay_ms.store(100, Ordering::SeqCst);
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "interrupted-write",
            "write_file",
            r#"{"path":"never-written.txt","content":"interrupted"}"#,
        )])],
    );
    let metadata = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();
    let create_started = provider.state.create_started.notified();

    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: metadata.thread_id.clone(),
            message: "begin a runner-backed write".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: scratch.display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), create_started)
        .await
        .expect("runner initialization should start");
    runtime
        .interrupt_turn(metadata.thread_id.clone(), turn_id)
        .await
        .unwrap();

    // The detached singleflight finishes the failed attempt after the turn is
    // gone. A later caller observes that failure and can then retry instead of
    // waiting forever on an `Initializing` slot owned by the aborted turn.
    let state = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match runtime.pause_thread_runner(&metadata.thread_id).await {
                Ok(state) => break state,
                Err(error) if error.to_string().contains("mock runner creation failed") => {
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("unexpected runner retry error: {error:#}"),
            }
        }
    })
    .await
    .expect("runner initialization should recover after interrupt");

    assert_eq!(state.session_id, "mock-session");
    assert_eq!(*provider.state.created.lock().unwrap(), 2);
    assert!(!scratch.join("never-written.txt").exists());
    assert!(
        runtime
            .load_thread(&metadata.thread_id)
            .await
            .unwrap()
            .unwrap()
            .metadata
            .unwrap()
            .runner_state
            .is_some()
    );

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn runner_bound_thread_routes_coding_tools_through_remote_runner() {
    let session_dir = temp_dir("runner-bound-sessions");
    let scratch = temp_dir("runner-bound-scratch");
    let provider = MockRunnerProvider::default();
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![
            tool_call_round(&[
                (
                    "write",
                    "write_file",
                    r#"{"path":"remote-out.txt","content":"hello from runner"}"#,
                ),
                ("shell", "shell", r#"{"command":"echo hi"}"#),
                ("exec", "exec_command", r#"{"cmd":"echo nope"}"#),
            ]),
            tool_call_round(&[("read", "read_file", r#"{"path":"remote-out.txt"}"#)]),
        ],
    );

    let metadata = runtime
        .create_thread_with(roder_core::CreateThreadRequest {
            title: None,
            workspace: scratch.display().to_string(),
            workspace_id: None,
            root_id: None,
            provider: None,
            model: None,
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: Some(roder_core::ThreadRunnerSelection {
                provider_id: "mock-hosted".to_string(),
                config: serde_json::json!({ "space_id": "space-1" }),
                workspace: "/sandbox/workspace".to_string(),
                read_roots: Vec::new(),
            }),
        })
        .await
        .unwrap();
    let binding = metadata.runner_binding.clone().expect("runner binding");
    assert_eq!(binding.destination.provider_id, "mock-hosted");
    assert_eq!(
        binding.destination.id,
        format!("thread-{}", metadata.thread_id)
    );
    assert_eq!(binding.destination.config["space_id"], "space-1");
    assert_eq!(binding.workspace, PathBuf::from("/sandbox/workspace"));

    let completed = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;

    // write_file went to the runner, not local disk.
    assert_eq!(
        provider
            .state
            .files
            .lock()
            .unwrap()
            .get("remote-out.txt")
            .cloned(),
        Some(b"hello from runner".to_vec())
    );
    assert!(!scratch.join("remote-out.txt").exists());

    // shell executed through the runner session, scoped to the runner workspace root.
    let shell_command = provider
        .state
        .commands
        .lock()
        .unwrap()
        .iter()
        .find(|command| command.args.contains(&"echo hi".to_string()))
        .cloned()
        .expect("shell command routed to runner");
    assert_eq!(shell_command.program, "sh");
    assert_eq!(
        shell_command.cwd.as_deref(),
        Some(std::path::Path::new("/sandbox/workspace"))
    );

    // read_file returned the runner contents.
    let read = completed
        .iter()
        .find(|call| call.tool_name.as_deref() == Some("read_file"))
        .expect("read_file completed");
    assert!(!read.is_error);
    assert!(
        read.output
            .as_deref()
            .unwrap_or_default()
            .contains("hello from runner")
    );

    // Codex-shaped exec_command is a completed one-shot runner command.
    let exec = completed
        .iter()
        .find(|call| call.tool_name.as_deref() == Some("exec_command"))
        .expect("exec_command completed");
    assert!(!exec.is_error);
    assert!(
        exec.output
            .as_deref()
            .unwrap_or_default()
            .contains("Status: completed"),
        "unexpected exec output: {:?}",
        exec.output
    );
    assert_eq!(*provider.state.created.lock().unwrap(), 1);
    let runner_state = runtime
        .load_thread(&metadata.thread_id)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .runner_state;
    assert!(runner_state.is_some());

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn unbound_thread_keeps_local_coding_tools_on_a_runner_capable_server() {
    let session_dir = temp_dir("runner-unbound-sessions");
    let scratch = temp_dir("runner-unbound-scratch");
    let provider = MockRunnerProvider::default();
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "write",
            "write_file",
            r#"{"path":"local-out.txt","content":"hello locally"}"#,
        )])],
    );

    let metadata = runtime.create_thread(None).await.unwrap();
    assert!(metadata.runner_binding.is_none());

    let completed = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;
    let write = completed
        .iter()
        .find(|call| call.tool_name.as_deref() == Some("write_file"))
        .expect("write_file completed");
    assert!(!write.is_error);

    assert_eq!(
        std::fs::read_to_string(scratch.join("local-out.txt")).unwrap(),
        "hello locally"
    );
    assert!(provider.state.files.lock().unwrap().is_empty());
    assert_eq!(*provider.state.created.lock().unwrap(), 0);

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn unbound_thread_cannot_touch_host_when_local_workspaces_are_disabled() {
    let session_dir = temp_dir("runner-hosted-unbound-sessions");
    let scratch = temp_dir("runner-hosted-unbound-scratch");
    let provider = MockRunnerProvider::default();
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[(
            "write",
            "write_file",
            r#"{"path":"must-not-touch-host.txt","content":"host escape"}"#,
        )])],
    );
    runtime.set_allow_local_workspaces(false);
    let host_path = scratch.join("must-not-touch-host.txt");
    std::fs::write(&host_path, "original host contents").unwrap();
    let mut events = runtime.subscribe_events();

    let metadata = runtime.create_thread(None).await.unwrap();
    assert!(metadata.runner_binding.is_none());

    let completed = run_turn_collecting_tool_calls(&runtime, &metadata.thread_id, &scratch).await;
    let write = completed
        .iter()
        .find(|call| call.tool_name.as_deref() == Some("write_file"))
        .expect("write_file completed with an isolation error");
    assert!(write.is_error);
    assert!(
        write
            .output
            .as_deref()
            .unwrap_or_default()
            .contains("local workspace execution is disabled")
    );
    assert_eq!(
        std::fs::read_to_string(&host_path).unwrap(),
        "original host contents"
    );
    let mut preview_emitted = false;
    while let Ok(envelope) = events.try_recv() {
        if matches!(envelope.event, RoderEvent::FileChangePreviewReady(_)) {
            preview_emitted = true;
        }
    }
    assert!(
        !preview_emitted,
        "hosted calls must not read a host preview"
    );
    assert!(provider.state.files.lock().unwrap().is_empty());
    assert_eq!(*provider.state.created.lock().unwrap(), 0);

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

#[tokio::test]
async fn thread_runner_binding_rejects_unknown_providers_and_relative_workspaces() {
    let session_dir = temp_dir("runner-validate-sessions");
    let scratch = temp_dir("runner-validate-scratch");
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        MockRunnerProvider::default(),
        Vec::new(),
    );

    let request = |provider_id: &str, workspace: &str| roder_core::CreateThreadRequest {
        title: None,
        workspace: scratch.display().to_string(),
        workspace_id: None,
        root_id: None,
        provider: None,
        model: None,
        selection_mode: None,
        tool_allowlist: Vec::new(),
        developer_instructions: None,
        external_tools: Vec::new(),
        runner: Some(roder_core::ThreadRunnerSelection {
            provider_id: provider_id.to_string(),
            config: serde_json::json!({}),
            workspace: workspace.to_string(),
            read_roots: Vec::new(),
        }),
    };

    let unknown = runtime
        .create_thread_with(request("missing-provider", "/sandbox/workspace"))
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("is not installed"));

    let relative = runtime
        .create_thread_with(request("mock-hosted", "sandbox/workspace"))
        .await
        .unwrap_err();
    assert!(relative.to_string().contains("absolute"));

    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}

async fn runtime(session_dir: PathBuf, workspace: PathBuf) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FinalEngine {
        requests: Mutex::new(0),
    }));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
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
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: Some(workspace.display().to_string()),
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: Some(RunnerDestination {
                    id: "unix-local".to_string(),
                    provider_id: "unix-local".to_string(),
                    config: serde_json::json!({ "root": workspace.display().to_string() }),
                    default_manifest: RunnerManifest::default(),
                }),
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

fn unix_runner_thread_request(workspace: &std::path::Path) -> roder_core::CreateThreadRequest {
    roder_core::CreateThreadRequest {
        title: None,
        workspace: workspace.display().to_string(),
        workspace_id: None,
        root_id: None,
        provider: None,
        model: None,
        selection_mode: None,
        tool_allowlist: Vec::new(),
        developer_instructions: None,
        external_tools: Vec::new(),
        runner: Some(roder_core::ThreadRunnerSelection {
            provider_id: "unix-local".to_string(),
            config: serde_json::json!({ "root": workspace.display().to_string() }),
            workspace: workspace.display().to_string(),
            read_roots: Vec::new(),
        }),
    }
}

fn mock_runner_thread_request(workspace: &std::path::Path) -> roder_core::CreateThreadRequest {
    roder_core::CreateThreadRequest {
        title: None,
        workspace: workspace.display().to_string(),
        workspace_id: None,
        root_id: None,
        provider: None,
        model: None,
        selection_mode: None,
        tool_allowlist: Vec::new(),
        developer_instructions: None,
        external_tools: Vec::new(),
        runner: Some(roder_core::ThreadRunnerSelection {
            provider_id: "mock-hosted".to_string(),
            config: serde_json::json!({}),
            workspace: "/sandbox/workspace".to_string(),
            read_roots: Vec::new(),
        }),
    }
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
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
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
