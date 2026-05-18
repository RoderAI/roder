use std::time::Instant;

use roder_api::events::{ThreadId, TurnId};
use roder_api::subagents::SubagentRequest;
use roder_api::trace::{
    ParentTurnRef, SubagentDestination, SubagentDestinationKind, SubagentTraceDelta,
    SubagentTraceId, SubagentTraceSink, SubagentTraceStatus, SubagentTraceSummary,
};

pub(crate) const TRACE_TEXT_MAX_CHARS: usize = 4000;

#[derive(Debug, Clone)]
pub(crate) struct TraceIds {
    pub(crate) trace_id: SubagentTraceId,
    pub(crate) child_thread_id: ThreadId,
    pub(crate) child_turn_id: TurnId,
}

pub(crate) struct TraceSummaryArgs<'a> {
    pub(crate) trace_ids: &'a TraceIds,
    pub(crate) parent: &'a ParentTurnRef,
    pub(crate) request: &'a SubagentRequest,
    pub(crate) default_role: &'a str,
    pub(crate) model: Option<String>,
    pub(crate) status: SubagentTraceStatus,
    pub(crate) started_at: Instant,
    pub(crate) usage: Option<roder_api::inference::TokenUsage>,
    pub(crate) latest_activity: Option<String>,
    pub(crate) error_summary: Option<String>,
}

pub(crate) fn trace_summary(args: TraceSummaryArgs<'_>) -> SubagentTraceSummary {
    SubagentTraceSummary {
        trace_id: args.trace_ids.trace_id.clone(),
        parent: args.parent.clone(),
        child_thread_id: args.trace_ids.child_thread_id.clone(),
        child_turn_id: args.trace_ids.child_turn_id.clone(),
        title: args.request.description.clone(),
        role: args
            .request
            .subagent_type
            .clone()
            .unwrap_or_else(|| args.default_role.to_string()),
        model: args.model,
        status: args.status,
        elapsed_ms: args
            .started_at
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        usage: args.usage,
        destination: Some(SubagentDestination {
            kind: SubagentDestinationKind::InProcess,
            label: "in-process".to_string(),
            path: None,
            provider_id: None,
            destination_id: None,
        }),
        latest_activity: args.latest_activity,
        error_summary: args.error_summary,
    }
}

pub(crate) async fn emit_trace_created(
    sink: Option<&dyn SubagentTraceSink>,
    summary: SubagentTraceSummary,
) {
    if let Some(sink) = sink {
        sink.trace_created(summary).await;
    }
}

pub(crate) async fn emit_trace_delta(
    sink: Option<&dyn SubagentTraceSink>,
    delta: SubagentTraceDelta,
) {
    if let Some(sink) = sink {
        sink.trace_delta(delta).await;
    }
}

pub(crate) async fn emit_trace_status_changed(
    sink: Option<&dyn SubagentTraceSink>,
    trace_id: SubagentTraceId,
    parent: ParentTurnRef,
    status: SubagentTraceStatus,
    detail: Option<String>,
) {
    if let Some(sink) = sink {
        sink.trace_status_changed(trace_id, parent, status, detail)
            .await;
    }
}

pub(crate) async fn emit_trace_completed(
    sink: Option<&dyn SubagentTraceSink>,
    summary: SubagentTraceSummary,
) {
    if let Some(sink) = sink {
        sink.trace_completed(summary).await;
    }
}

pub(crate) async fn emit_trace_failed(
    sink: Option<&dyn SubagentTraceSink>,
    summary: SubagentTraceSummary,
    error: String,
) {
    if let Some(sink) = sink {
        sink.trace_failed(summary, error).await;
    }
}
