//! `SubagentDispatcher` adapter backed by a process-hosted child (roadmap
//! phase 93).
//!
//! `subagents/dispatch` is acked immediately by the child; progress then
//! streams back as `subagents/event` notifications until a terminal
//! `completed`/`failed` event. Status events are forwarded into the
//! optional [`SubagentTraceSink`] so process-hosted dispatches surface
//! through the same trace/notification surfaces as in-process subagents.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use roder_api::events::{ThreadId, TurnId};
use roder_api::process_extension::{
    METHOD_SUBAGENTS_CANCEL, METHOD_SUBAGENTS_DEFINITIONS, METHOD_SUBAGENTS_DISPATCH,
    ProcessSubagentCancelParams, ProcessSubagentDefinitionsParams,
    ProcessSubagentDefinitionsResult, ProcessSubagentDispatchAck, ProcessSubagentDispatchParams,
    ProcessSubagentEvent,
};
use roder_api::subagents::{SubagentDefinition, SubagentDispatcher, SubagentRequest, SubagentResult};
use roder_api::trace::{
    ParentTurnRef, SubagentTraceStatus, SubagentTraceSink,
};

use crate::process::ProcessHost;

/// Default ceiling for a dispatch without an explicit request timeout.
/// Remote agents are long-running; callers should set
/// `SubagentRequest.timeout_seconds` for tighter bounds.
const DEFAULT_DISPATCH_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct ProcessSubagentDispatcher {
    host: Arc<ProcessHost>,
    dispatcher_id: String,
    definitions: Arc<RwLock<Option<Vec<SubagentDefinition>>>>,
}

impl ProcessSubagentDispatcher {
    pub fn new(host: Arc<ProcessHost>, dispatcher_id: String) -> Self {
        let dispatcher = Self {
            host,
            dispatcher_id,
            definitions: Arc::new(RwLock::new(None)),
        };
        dispatcher.spawn_definitions_fetch();
        dispatcher
    }

    /// Fetches definitions from the child in the background so the sync
    /// `definitions()` accessor can serve them once cached. The child only
    /// spawns lazily, so this is also the first spawn trigger in practice.
    fn spawn_definitions_fetch(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let host = self.host.clone();
        let dispatcher_id = self.dispatcher_id.clone();
        let cache = self.definitions.clone();
        tokio::spawn(async move {
            if let Ok(result) = fetch_definitions(&host, &dispatcher_id).await
                && let Ok(mut slot) = cache.write()
            {
                *slot = Some(result);
            }
        });
    }

    /// Awaits the child-declared definitions, caching them for the sync
    /// accessor.
    pub async fn fetch_definitions(&self) -> anyhow::Result<Vec<SubagentDefinition>> {
        if let Some(definitions) = self
            .definitions
            .read()
            .ok()
            .and_then(|slot| slot.clone())
        {
            return Ok(definitions);
        }
        let definitions = fetch_definitions(&self.host, &self.dispatcher_id).await?;
        if let Ok(mut slot) = self.definitions.write() {
            *slot = Some(definitions.clone());
        }
        Ok(definitions)
    }
}

async fn fetch_definitions(
    host: &Arc<ProcessHost>,
    dispatcher_id: &str,
) -> anyhow::Result<Vec<SubagentDefinition>> {
    let result: ProcessSubagentDefinitionsResult = host
        .request(
            METHOD_SUBAGENTS_DEFINITIONS,
            serde_json::to_value(ProcessSubagentDefinitionsParams {
                dispatcher_id: dispatcher_id.to_string(),
            })?,
        )
        .await?;
    Ok(result.definitions)
}

#[async_trait::async_trait]
impl SubagentDispatcher for ProcessSubagentDispatcher {
    fn id(&self) -> String {
        self.dispatcher_id.clone()
    }

    fn definitions(&self) -> Vec<SubagentDefinition> {
        // Sync accessor: serve the cache populated by the background fetch
        // (or a prior `fetch_definitions` await). Empty until the child has
        // answered once.
        self.definitions
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_default()
    }

    async fn dispatch(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
    ) -> anyhow::Result<SubagentResult> {
        self.dispatch_traced(parent_thread_id, parent_turn_id, request, None)
            .await
    }

    async fn dispatch_traced(
        &self,
        parent_thread_id: ThreadId,
        parent_turn_id: TurnId,
        request: SubagentRequest,
        trace_sink: Option<Arc<dyn SubagentTraceSink>>,
    ) -> anyhow::Result<SubagentResult> {
        let dispatch_id = uuid::Uuid::new_v4().to_string();
        let timeout = request
            .timeout_seconds
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_DISPATCH_TIMEOUT);
        let mut receiver = self.host.register_subagent_stream(dispatch_id.clone()).await?;

        let ack: ProcessSubagentDispatchAck = self
            .host
            .request(
                METHOD_SUBAGENTS_DISPATCH,
                serde_json::to_value(ProcessSubagentDispatchParams {
                    dispatcher_id: self.dispatcher_id.clone(),
                    dispatch_id: dispatch_id.clone(),
                    parent_thread_id: parent_thread_id.clone(),
                    parent_turn_id: parent_turn_id.clone(),
                    request,
                })?,
            )
            .await
            .inspect_err(|_| {
                let host = self.host.clone();
                let dispatch_id = dispatch_id.clone();
                if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::spawn(async move {
                        host.unregister_subagent_stream(&dispatch_id).await;
                    });
                }
            })?;
        anyhow::ensure!(
            ack.dispatch_id == dispatch_id,
            "process extension acknowledged dispatch {:?} but {:?} was requested",
            ack.dispatch_id,
            dispatch_id
        );

        let parent = ParentTurnRef {
            thread_id: parent_thread_id,
            turn_id: parent_turn_id,
        };
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let event = tokio::time::timeout_at(deadline, receiver.recv()).await;
            match event {
                Err(_) => {
                    // Host-side timeout: tell the child to cancel, then fail.
                    self.cancel_dispatch(&dispatch_id, Some("dispatch timed out".to_string()))
                        .await;
                    anyhow::bail!(
                        "subagent dispatcher {} timed out after {}s",
                        self.dispatcher_id,
                        timeout.as_secs()
                    );
                }
                Ok(None) => {
                    anyhow::bail!(
                        "subagent dispatcher {} closed the dispatch stream without a terminal \
                         event",
                        self.dispatcher_id
                    );
                }
                Ok(Some(Err(error))) => {
                    // Child crashed mid-dispatch; the stream error is
                    // already redacted by the host.
                    return Err(error.context(format!(
                        "subagent dispatcher {} failed mid-dispatch",
                        self.dispatcher_id
                    )));
                }
                Ok(Some(Ok(ProcessSubagentEvent::Status { status, detail }))) => {
                    if let Some(sink) = &trace_sink {
                        sink.trace_status_changed(
                            dispatch_id.clone(),
                            parent.clone(),
                            SubagentTraceStatus::Running,
                            Some(match &detail {
                                Some(detail) => format!("{status}: {detail}"),
                                None => status.clone(),
                            }),
                        )
                        .await;
                    }
                }
                Ok(Some(Ok(ProcessSubagentEvent::Completed { result }))) => {
                    return Ok(result);
                }
                Ok(Some(Ok(ProcessSubagentEvent::Failed { error }))) => {
                    anyhow::bail!("subagent dispatcher {} failed: {error}", self.dispatcher_id);
                }
            }
        }
    }
}

impl ProcessSubagentDispatcher {
    async fn cancel_dispatch(&self, dispatch_id: &str, reason: Option<String>) {
        self.host.unregister_subagent_stream(dispatch_id).await;
        let params = ProcessSubagentCancelParams {
            dispatcher_id: self.dispatcher_id.clone(),
            dispatch_id: dispatch_id.to_string(),
            reason,
        };
        if let Ok(params) = serde_json::to_value(params) {
            let _: Result<serde_json::Value, _> =
                self.host.request(METHOD_SUBAGENTS_CANCEL, params).await;
        }
    }
}
