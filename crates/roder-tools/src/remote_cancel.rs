use std::sync::Arc;
use std::time::Duration;

use roder_api::remote_runner::{RemoteRunnerSession, RunnerCommandId};

const REMOTE_CANCEL_GRACE_MS: u64 = 1_000;

/// Requests cooperative provider cancellation whenever a remote command
/// future is dropped before it reports a terminal result. This covers tool
/// deadlines and explicit turn interruption without extending the caller's
/// own deadline. Once the provider future returns, the provider owns any
/// cleanup implied by its success or error result.
pub(crate) struct RemoteCancelOnDrop {
    session: Option<Arc<dyn RemoteRunnerSession>>,
    command_id: Option<RunnerCommandId>,
}

impl RemoteCancelOnDrop {
    pub(crate) fn new(session: Arc<dyn RemoteRunnerSession>, command_id: RunnerCommandId) -> Self {
        Self {
            session: Some(session),
            command_id: Some(command_id),
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.session = None;
        self.command_id = None;
    }
}

impl Drop for RemoteCancelOnDrop {
    fn drop(&mut self) {
        if let (Some(session), Some(command_id)) = (self.session.take(), self.command_id.take()) {
            request_remote_cancel(session, command_id);
        }
    }
}

fn request_remote_cancel(session: Arc<dyn RemoteRunnerSession>, command_id: RunnerCommandId) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    drop(runtime.spawn(async move {
        let _ = tokio::time::timeout(
            Duration::from_millis(REMOTE_CANCEL_GRACE_MS),
            session.cancel_command(&command_id),
        )
        .await;
    }));
}
