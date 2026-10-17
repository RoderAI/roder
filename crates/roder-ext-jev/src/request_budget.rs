//! Reserve actual provider requests inside one bounded browser decision.
use std::cell::Cell;
use std::future::Future;

use crate::{JevDecisionTransport, JevStatus, JevStop};
use async_trait::async_trait;
use serde_json::Value;

tokio::task_local! { static REMAINING: Cell<usize>; }

pub(crate) async fn scope<T>(limit: usize, future: impl Future<Output = T>) -> (T, usize) {
    REMAINING
        .scope(Cell::new(limit), async {
            let result = future.await;
            (result, limit.saturating_sub(REMAINING.with(Cell::get)))
        })
        .await
}

/// A leaf provider boundary. Composed adapters must count their native calls,
/// rather than also counting the enclosing abstract decision request.
pub(crate) struct Transport<'a>(pub &'a dyn JevDecisionTransport);

#[async_trait]
impl JevDecisionTransport for Transport<'_> {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        self.0.decide_budgeted(request).await
    }
}

/// Shared by built-in transports and the public custom-client boundary.
pub(crate) fn reserve() -> anyhow::Result<()> {
    let allowed = REMAINING
        .try_with(|remaining| {
            let n = remaining.get();
            if n == 0 {
                false
            } else {
                remaining.set(n - 1);
                true
            }
        })
        .unwrap_or(true);
    if !allowed {
        return Err(JevStop::new(
            JevStatus::BudgetExceeded,
            "Reached the demo's model-call budget",
        )
        .into());
    }
    Ok(())
}
