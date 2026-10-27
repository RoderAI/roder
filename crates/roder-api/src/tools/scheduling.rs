use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::ToolExecutionMode;

/// A fair shared-read/exclusive-write resource barrier. Share the same instance
/// with nested dispatchers through `ToolExecutionHandles::execution_barrier`.
#[derive(Default)]
pub struct ToolExecutionBarrier {
    barrier: RwLock<()>,
}

/// Releasing or cancelling an execution drops its scheduling guard.
pub struct ToolExecutionGuard<'a> {
    _guard: BarrierGuard<'a>,
}

enum BarrierGuard<'a> {
    Read { _guard: RwLockReadGuard<'a, ()> },
    Write { _guard: RwLockWriteGuard<'a, ()> },
}

impl ToolExecutionBarrier {
    /// Coordinators hold no resource guard while awaiting nested work.
    pub async fn acquire(&self, mode: ToolExecutionMode) -> Option<ToolExecutionGuard<'_>> {
        let guard = match mode {
            ToolExecutionMode::SharedRead => BarrierGuard::Read {
                _guard: self.barrier.read().await,
            },
            ToolExecutionMode::Exclusive => BarrierGuard::Write {
                _guard: self.barrier.write().await,
            },
            ToolExecutionMode::Coordinator => return None,
        };
        Some(ToolExecutionGuard { _guard: guard })
    }
}
