use std::future::Future;
use std::sync::Arc;

/// Dedicated single-worker Tokio runtime that owns all MySQL I/O.
///
/// The app-server runs on a current-thread runtime where
/// `tokio::task::block_in_place` panics, and sqlx connections must not
/// outlive the runtime that created them. Pinning the pool and every query
/// to this executor makes the store usable from any caller runtime (and
/// from the sync `ContextArtifactAccess` trait via a plain mpsc bridge).
pub(crate) struct DbExecutor {
    runtime: Option<tokio::runtime::Runtime>,
}

impl DbExecutor {
    pub(crate) fn new() -> anyhow::Result<Arc<Self>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("roder-mysql-session")
            .enable_all()
            .build()?;
        Ok(Arc::new(Self {
            runtime: Some(runtime),
        }))
    }

    fn handle(&self) -> &tokio::runtime::Handle {
        self.runtime
            .as_ref()
            .expect("runtime present until drop")
            .handle()
    }

    /// Runs a future on the DB runtime from an async context.
    pub(crate) async fn run<T, Fut>(&self, future: Fut) -> anyhow::Result<T>
    where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        self.handle()
            .spawn(future)
            .await
            .map_err(|err| anyhow::anyhow!("MySQL session task failed: {err}"))?
    }

    /// Runs a future on the DB runtime from a sync context, blocking the
    /// calling thread with a std channel (safe on any runtime flavor since
    /// the work happens on this executor's own threads).
    pub(crate) fn run_blocking<T, Fut>(&self, future: Fut) -> anyhow::Result<T>
    where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.handle().spawn(async move {
            let _ = tx.send(future.await);
        });
        rx.recv()
            .map_err(|err| anyhow::anyhow!("MySQL session task dropped: {err}"))?
    }
}

impl Drop for DbExecutor {
    fn drop(&mut self) {
        // Dropping a runtime inside another runtime's context panics;
        // shutdown_background is safe everywhere.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
