use std::sync::{Arc, Mutex};
use std::time::Instant;

use roder_api::inference::TokenUsage;
use roder_api::trace::{ParentTurnRef, SubagentTraceSink, SubagentTraceSummary};

use crate::trace::{TraceIds, emit_trace_failed};

pub(super) struct ChildRunContext {
    pub(super) parent: ParentTurnRef,
    pub(super) trace_ids: TraceIds,
    pub(super) started_at: Instant,
    pub(super) usage: DispatchUsage,
}

#[derive(Clone, Default)]
pub(super) struct DispatchUsage(Arc<Mutex<UsageState>>);

#[derive(Default)]
struct UsageState {
    completed: Option<TokenUsage>,
    current: Option<TokenUsage>,
}

fn add_usage(total: &mut Option<TokenUsage>, next: TokenUsage) {
    if let Some(total) = total {
        total.add_assign(&next);
    } else {
        *total = Some(next);
    }
}

impl DispatchUsage {
    pub(super) fn begin_step(&self) {
        let mut state = self.0.lock().unwrap();
        if let Some(current) = state.current.take() {
            add_usage(&mut state.completed, current);
        }
    }

    pub(super) fn observe(&self, usage: TokenUsage) {
        // Providers may publish cumulative snapshots repeatedly in one stream.
        self.0.lock().unwrap().current = Some(usage);
    }

    pub(super) fn snapshot(&self) -> Option<TokenUsage> {
        let state = self.0.lock().unwrap();
        let mut total = state.completed.clone();
        if let Some(current) = state.current.clone() {
            add_usage(&mut total, current);
        }
        total
    }
}

/// Preserve consumed usage when a caller drops an in-flight dispatch future.
pub(super) struct CancellationTraceGuard {
    sink: Option<Arc<dyn SubagentTraceSink>>,
    summary: Option<SubagentTraceSummary>,
    usage: DispatchUsage,
    started_at: Instant,
}

impl CancellationTraceGuard {
    pub(super) fn new(
        sink: Option<Arc<dyn SubagentTraceSink>>,
        summary: SubagentTraceSummary,
        usage: DispatchUsage,
        started_at: Instant,
    ) -> Self {
        Self {
            sink,
            summary: Some(summary),
            usage,
            started_at,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.summary = None;
    }
}

impl Drop for CancellationTraceGuard {
    fn drop(&mut self) {
        let Some(mut summary) = self.summary.take() else {
            return;
        };
        let Some(sink) = self.sink.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        summary.usage = self.usage.snapshot();
        summary.elapsed_ms = self
            .started_at
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        // Drop cannot await. Publish best-effort on the still-running executor.
        runtime.spawn(async move {
            emit_trace_failed(
                Some(sink.as_ref()),
                summary,
                "subagent cancelled".to_string(),
            )
            .await;
        });
    }
}
