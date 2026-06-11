//! `InferenceEngine` adapter backed by a process-hosted child.

use std::sync::Arc;

use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceTurnContext, ModelDescriptor,
};
use roder_api::process_extension::{
    METHOD_LIST_MODELS, METHOD_STREAM_TURN, ProcessListModelsParams, ProcessListModelsResult,
    ProcessStreamTurnAck, ProcessStreamTurnParams,
};

use crate::process::ProcessHost;

pub struct ProcessInferenceEngine {
    host: Arc<ProcessHost>,
    engine_id: String,
}

impl ProcessInferenceEngine {
    pub fn new(host: Arc<ProcessHost>, engine_id: String) -> Self {
        Self { host, engine_id }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ProcessInferenceEngine {
    fn id(&self) -> String {
        self.engine_id.clone()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: false,
            image_input: false,
            prompt_cache: false,
            provider_metadata: true,
            tool_search: false,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        let result: ProcessListModelsResult = self
            .host
            .request(
                METHOD_LIST_MODELS,
                serde_json::to_value(ProcessListModelsParams {
                    engine_id: self.engine_id.clone(),
                })?,
            )
            .await?;
        Ok(result.models)
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let stream_id = uuid::Uuid::new_v4().to_string();
        let receiver = self.host.register_stream(stream_id.clone()).await?;
        let ack: ProcessStreamTurnAck = self
            .host
            .request(
                METHOD_STREAM_TURN,
                serde_json::to_value(ProcessStreamTurnParams {
                    engine_id: self.engine_id.clone(),
                    stream_id: stream_id.clone(),
                    thread_id: ctx.thread_id.to_string(),
                    turn_id: ctx.turn_id.to_string(),
                    request,
                })?,
            )
            .await?;
        anyhow::ensure!(
            ack.stream_id == stream_id,
            "process extension acknowledged stream {:?} but {:?} was requested",
            ack.stream_id,
            stream_id
        );
        Ok(Box::pin(tokio_stream_from(receiver)))
    }
}

fn tokio_stream_from(
    mut receiver: tokio::sync::mpsc::Receiver<anyhow::Result<roder_api::inference::InferenceEvent>>,
) -> impl futures::Stream<Item = anyhow::Result<roder_api::inference::InferenceEvent>> + Send + 'static
{
    async_stream_poll(move |cx| receiver.poll_recv(cx))
}

fn async_stream_poll<T, F>(poll: F) -> PollStream<F>
where
    F: FnMut(&mut std::task::Context<'_>) -> std::task::Poll<Option<T>>,
{
    PollStream(poll)
}

struct PollStream<F>(F);

impl<T, F> futures::Stream for PollStream<F>
where
    F: FnMut(&mut std::task::Context<'_>) -> std::task::Poll<Option<T>> + Unpin,
{
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<T>> {
        (self.get_mut().0)(cx)
    }
}
