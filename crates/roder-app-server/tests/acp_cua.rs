// Public ACP -> Roder runtime -> thread-bound runner command -> Cua result.
use super::*;
use roder_api::inference::{CompletionMetadata, InferenceEvent, ToolCallCompleted};
use roder_api::policy_mode::PolicyMode;
use roder_api::remote_runner::*;
use roder_api::transcript::TranscriptItem;
use roder_ext_cua::{CuaConfig, CuaExtension};
use serde_json::{Value, json};

struct DecisionEngine;
#[async_trait]
impl InferenceEngine for DecisionEngine {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![])
    }
    async fn stream_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let results: Vec<_> = request
            .transcript
            .iter()
            .filter_map(|item| {
                if let TranscriptItem::ToolResult(result) = item {
                    Some(result)
                } else {
                    None
                }
            })
            .collect();
        let call = match results.len() {
            0 => Some(("cua_get_window_state", json!({"pid":10,"window_id":20}))),
            1 => {
                let observed: Value = serde_json::from_str(&results[0].result)?;
                Some((
                    "cua_click",
                    json!({"pid":10,"window_id":20,"x":1,"y":1,"capture_id":observed["observation"]["capture_id"]}),
                ))
            }
            2 => Some((
                "cua_press_key",
                json!({"pid":10,"window_id":20,"key":"7","delivery_mode":"foreground"}),
            )),
            _ => None,
        };
        let mut events = vec![];
        if let Some((name, args)) = call {
            events.push(Ok(InferenceEvent::ToolCallStarted(
                roder_api::inference::ToolCallStarted {
                    id: format!("cua-call-{}", results.len()),
                    name: name.into(),
                },
            )));
            events.push(Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: format!("cua-call-{}", results.len()),
                name: name.into(),
                arguments: args.to_string(),
            })));
        }
        events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some(
                if results.len() < 3 {
                    "tool_use"
                } else {
                    "end_turn"
                }
                .into(),
            ),
            provider_response_id: None,
        })));
        Ok(Box::pin(stream::iter(events)))
    }
}
#[derive(Default)]
struct Desktop {
    refuse_click: bool,
    commands: Mutex<Vec<RunnerCommandRequest>>,
    files: Mutex<std::collections::HashMap<std::path::PathBuf, Vec<u8>>>,
}
#[async_trait]
impl RemoteRunnerProvider for Desktop {
    fn id(&self) -> String {
        "desktop-fixture".into()
    }
    fn default_workspace(&self) -> Option<String> {
        Some("/workspace".into())
    }
    fn capabilities(&self) -> RunnerCapabilities {
        RunnerCapabilities {
            command_exec: true,
            file_read: true,
            file_write: true,
            port_preview: false,
            snapshots: false,
            cancellation: false,
            artifact_export: false,
            mounts: Default::default(),
            pausable: false,
            detachable: false,
        }
    }
    async fn create_session(
        &self,
        _: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        // Provider/session share the record so the test can prove exact routing.
        unreachable!("use SharedDesktop")
    }
    async fn resume_session(
        &self,
        _: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        anyhow::bail!("unused")
    }
}
struct SharedDesktop(Arc<Desktop>);
#[async_trait]
impl RemoteRunnerProvider for SharedDesktop {
    fn id(&self) -> String {
        self.0.id()
    }
    fn default_workspace(&self) -> Option<String> {
        self.0.default_workspace()
    }
    fn capabilities(&self) -> RunnerCapabilities {
        self.0.capabilities()
    }
    async fn create_session(
        &self,
        _: RunnerDestination,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        Ok(self.0.clone())
    }
    async fn resume_session(
        &self,
        _: RunnerSessionState,
    ) -> anyhow::Result<Arc<dyn RemoteRunnerSession>> {
        Ok(self.0.clone())
    }
}
#[async_trait]
impl RemoteRunnerSession for Desktop {
    fn state(&self) -> RunnerSessionState {
        RunnerSessionState {
            provider_id: "desktop-fixture".into(),
            session_id: "owned-desktop".into(),
            destination_id: "desktop".into(),
            snapshot: None,
            metadata: json!({}),
        }
    }
    async fn run_command(
        &self,
        command: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        if command.program == "/bin/rm" {
            return Ok(RunnerCommandResult {
                command_id: command.command_id,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        let mut commands = self.commands.lock().await;
        let tool = command.args[0].clone();
        assert_eq!(command.program, "/opt/roder-cua/bin/cua-call");
        let id = command.command_id.clone();
        let path = command.args[2].clone();
        commands.push(command);
        let observation = if tool == "get_window_state" {
            json!({"capture_id":format!("capture-{}",commands.len()),"snapshot_id":format!("s{:08x}",commands.len()),
                   "screenshot_width":2,"screenshot_height":2,
                   "screenshot_png_b64":include_str!("../../roder-ext-cua/tests/capture.b64")})
        } else {
            json!({"delivered":true})
        };
        self.files.lock().await.insert(
            path.clone().into(),
            json!({"driver_version":"0.34.0","is_error":self.refuse_click && tool == "click","observation":observation})
                .to_string()
                .into_bytes(),
        );
        Ok(RunnerCommandResult {
            command_id: id,
            exit_code: Some(0),
            stdout: json!({"driver_version":"0.34.0","result_file":path}).to_string(),
            stderr: String::new(),
        })
    }
    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        Ok(RunnerFileReadResult {
            contents: self.files.lock().await.get(&request.path).unwrap().clone(),
            path: request.path,
        })
    }
    async fn write_file(&self, _: RunnerFileWriteRequest) -> anyhow::Result<()> {
        Ok(())
    }
    async fn expose_port(&self, _: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        anyhow::bail!("unused")
    }
    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        Ok(None)
    }
    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
async fn exercise(mode: PolicyMode, refuse_click: bool) {
    let temp = tempfile::tempdir().unwrap();
    let desktop = Arc::new(Desktop {
        refuse_click,
        ..Default::default()
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(DecisionEngine));
    builder.thread_store_factory(Arc::new(
        roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory {
            base_path: temp.path().join("threads"),
        },
    ));
    builder.remote_runner_provider(Arc::new(SharedDesktop(desktop.clone())));
    builder
        .install(CuaExtension::new(CuaConfig {
            enabled: true,
            ..Default::default()
        }))
        .unwrap();
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            roder_core::RuntimeConfig {
                default_provider: "mock".into(),
                policy_mode: mode,
                tool_allowlist: vec![
                    "cua_get_window_state".into(),
                    "cua_click".into(),
                    "cua_press_key".into(),
                ],
                remote_runner_destination: Some(RunnerDestination {
                    id: "desktop".into(),
                    provider_id: "desktop-fixture".into(),
                    config: json!({}),
                    default_manifest: Default::default(),
                }),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let adapter = AcpAdapter::new(LocalAppClient::new(Arc::new(
        AppServer::with_feature_config(
            runtime,
            AppServerFeatureConfig::default()
                .with_workspace_registry_path(temp.path().join("workspaces.json")),
        ),
    )));
    let peer = RecordingPeer::default();
    let rpc = |method: &str, params: Value| JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(method)),
        method: method.into(),
        params: Some(params),
    };
    let session = adapter
        .handle_request(
            rpc("session/new", json!({"cwd":temp.path(),"mcpServers":[]})),
            &peer,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(session.error.is_none(), "{:?}", session.error);
    let session = session.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let response = tokio::time::timeout(
        Duration::from_secs(15),
        adapter.handle_request(
            rpc(
                "session/prompt",
                json!({"sessionId":session,"prompt":[{"type":"text","text":"Use the calculator"}]}),
            ),
            &peer,
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(response.result.unwrap()["stopReason"], "end_turn");
    let notifications = peer.notifications.lock().await;
    let updates: Vec<_> = notifications
        .iter()
        .filter(|n| n.method == "session/update")
        .map(|n| &n.params["update"])
        .collect();
    for index in 0..3 {
        let id = format!("cua-call-{index}");
        assert!(
            updates
                .iter()
                .any(|u| u["sessionUpdate"] == "tool_call" && u["toolCallId"] == id),
            "missing {id}: {updates:?}"
        );
        assert!(
            updates
                .iter()
                .any(|u| u["sessionUpdate"] == "tool_call_update"
                    && u["toolCallId"] == id
                    && u.get("status").is_some())
        );
    }
    let images = updates
        .iter()
        .filter(|u| u["sessionUpdate"] == "tool_call_update")
        .filter(|u| {
            u["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| part["content"]["type"] == "image"))
        })
        .map(|u| u["toolCallId"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>()
        .len();
    if mode == PolicyMode::Plan {
        assert_eq!(*peer.permission_requests.lock().await, 0);
        assert_eq!(desktop.commands.lock().await.len(), 1);
        assert_eq!(images, 1);
        for index in [1, 2] {
            assert!(
                updates
                    .iter()
                    .any(|u| u["toolCallId"] == format!("cua-call-{index}")
                        && u["status"] == "failed")
            );
        }
    } else {
        assert_eq!(*peer.permission_requests.lock().await, 2);
        assert_eq!(desktop.commands.lock().await.len(), 5);
        assert_eq!(images, 3);
        if refuse_click {
            assert!(updates.iter().any(|u| u["toolCallId"] == "cua-call-1"
                && u["status"] == "failed"
                && u["content"].as_array().is_some_and(|parts| {
                    parts.iter().any(|part| part["content"]["type"] == "image")
                })));
            assert_eq!(
                desktop
                    .commands
                    .lock()
                    .await
                    .iter()
                    .filter(|command| command.args[0] == "click")
                    .count(),
                1
            );
        }
        assert!(
            updates
                .iter()
                .any(|u| u["toolCallId"] == "cua-call-1" && u["kind"] == "execute")
        );
    }
}
#[tokio::test]
async fn cua_approval_images_and_runner_binding_through_public_acp() {
    exercise(PolicyMode::Default, false).await;
}
#[tokio::test]
async fn cua_plan_observes_but_refuses_input_through_public_acp() {
    exercise(PolicyMode::Plan, false).await;
}

#[tokio::test]
async fn cua_refusal_keeps_failed_status_and_partial_state_image_through_acp() {
    exercise(PolicyMode::Default, true).await;
}
