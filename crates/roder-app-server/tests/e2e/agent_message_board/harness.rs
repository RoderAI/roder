use std::collections::HashMap;
use std::path::Path;

use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutionMode, ToolExecutor, ToolRegistry,
    ToolResult, ToolSpec,
};
use roder_api::transcript::TranscriptItem;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, oneshot};

use super::super::*;

type InferenceBatch = Vec<InferenceEvent>;

struct ScriptedBoardEngine(mpsc::UnboundedSender<Sample>);

pub(super) struct Sample {
    thread: String,
    request: AgentInferenceRequest,
    respond: oneshot::Sender<InferenceBatch>,
}

impl Sample {
    pub(super) fn call(self, id: &str, name: &str, arguments: Value) {
        self.respond
            .send(vec![
                InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments: arguments.to_string(),
                }),
                InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: None,
                    provider_response_id: None,
                }),
            ])
            .expect("model request still live");
    }

    pub(super) fn board(self, id: &str, arguments: Value) {
        assert!(
            self.request
                .tools
                .iter()
                .any(|tool| tool.name == "agent_message_board")
        );
        self.call(id, "agent_message_board", arguments);
    }

    pub(super) fn finish(self, text: &str) {
        self.respond
            .send(vec![
                InferenceEvent::MessageDelta(MessageDelta {
                    text: text.to_owned(),
                    phase: None,
                }),
                InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".into()),
                    provider_response_id: None,
                }),
            ])
            .expect("model request still live");
    }

    pub(super) fn result(&self, id: &str) -> &roder_api::transcript::ToolResultRecord {
        self.request
            .transcript
            .iter()
            .find_map(|item| match item {
                TranscriptItem::ToolResult(result) if result.id == id => Some(result),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {id} in {} transcript", self.thread))
    }

    pub(super) fn output(&self, id: &str) -> Value {
        let result = self.result(id);
        assert!(!result.is_error, "{id}: {}", result.result);
        serde_json::from_str(&result.result).expect("board result is JSON")
    }

    pub(super) fn mail_count(&self, text: &str) -> usize {
        self.request
            .transcript
            .iter()
            .map(|item| match item {
                TranscriptItem::UserMessage(user) => user.text.matches(text).count(),
                _ => 0,
            })
            .sum()
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ScriptedBoardEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_owned()
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
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let (respond, response) = oneshot::channel();
        self.0
            .send(Sample {
                thread: ctx.thread_id.to_owned(),
                request,
                respond,
            })
            .map_err(|_| anyhow::anyhow!("evaluation session closed"))?;
        let items = response.await?;
        Ok(Box::pin(stream::iter(items.into_iter().map(Ok))))
    }
}

struct PauseRoot(Arc<Notify>);

impl ToolContributor for PauseRoot {
    fn id(&self) -> String {
        "board-eval".into()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(Self(self.0.clone())))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for PauseRoot {
    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::SharedRead
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "board_eval_pause".into(),
            description: "Hold the parent between inference rounds.".into(),
            parameters: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        _call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        self.0.notify_one();
        std::future::pending().await
    }
}

pub(super) struct BoardEvaluation {
    pub(super) client: LocalAppClient,
    pub(super) runtime: Arc<Runtime>,
    samples: mpsc::UnboundedReceiver<Sample>,
    buffered: HashMap<String, Sample>,
    pub(super) root_paused: Arc<Notify>,
}

impl BoardEvaluation {
    pub(super) fn open(dir: &Path) -> Self {
        let (send, samples) = mpsc::unbounded_channel();
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(ScriptedBoardEngine(send)));
        let root_paused = Arc::new(Notify::new());
        builder.tool_contributor(Arc::new(PauseRoot(root_paused.clone())));
        builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: dir.join("threads"),
        }));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    team_data_dir: Some(dir.join("teams")),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let server = app_server_with_feature_config(
            runtime.clone(),
            AppServerFeatureConfig::default()
                .with_workspace_registry_path(dir.join("workspaces.json")),
        );
        Self {
            client: LocalAppClient::new(Arc::new(server)),
            runtime,
            samples,
            buffered: HashMap::new(),
            root_paused,
        }
    }

    pub(super) async fn new_thread(&self, workspace: &TestWorkspaceRef) -> String {
        let started: Value = request(
            &self.client,
            "thread/start",
            Some(json!({
                "model": "mock", "modelProvider": PROVIDER_MOCK,
                "workspaceId": workspace.workspace_id, "rootId": workspace.root_id,
                "ephemeral": false
            })),
        )
        .await;
        started["thread"]["id"].as_str().unwrap().to_owned()
    }

    pub(super) async fn turn(&self, thread: &str, text: &str) -> String {
        let started: Value = request(
            &self.client,
            "turn/start",
            Some(json!({
                "threadId": thread, "input": [{ "type": "text", "text": text }]
            })),
        )
        .await;
        started["turnId"].as_str().unwrap().to_owned()
    }

    pub(super) async fn sample(&mut self, thread: &str) -> Sample {
        if let Some(sample) = self.buffered.remove(thread)
            && !sample.respond.is_closed()
        {
            return sample;
        }
        loop {
            let sample = tokio::time::timeout(Duration::from_secs(10), self.samples.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for inference on {thread}"))
                .expect("scripted engine supplies a model request");
            if sample.respond.is_closed() {
                continue;
            }
            if sample.thread == thread {
                return sample;
            }
            let previous = self.buffered.insert(sample.thread.clone(), sample);
            assert!(previous.is_none_or(|sample| sample.respond.is_closed()));
        }
    }

    pub(super) async fn sample_after_mail(&mut self, mut sample: Sample, text: &str) -> Sample {
        while sample.mail_count(text) == 0 {
            tokio::time::timeout(Duration::from_secs(10), sample.respond.closed())
                .await
                .expect("queued mail preempts sampling and reaches the next request");
            sample = self.sample(&sample.thread).await;
        }
        sample
    }

    pub(super) async fn team(&self, team_id: &str) -> TeamReadResult {
        request(
            &self.client,
            "team/read",
            Some(json!({ "teamId": team_id })),
        )
        .await
    }

    pub(super) fn notifications(
        &self,
    ) -> tokio::sync::broadcast::Receiver<roder_protocol::JsonRpcNotification> {
        self.client.subscribe_notifications()
    }
}
