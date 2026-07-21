use std::time::{Duration, Instant};

use roder_api::remote_runner::{RunnerCommandId, RunnerCommandRequest};
use tokio::sync::watch;

use crate::client::{BlaxelClient, HTTP_REQUEST_TIMEOUT_SECONDS};

mod descendants;
mod reaper;
mod state;

use descendants::cleanup_tagged_descendants;
pub(crate) use descendants::shell_quote;
use reaper::{schedule_confirmed_ack_forget, schedule_process_reap};
use state::{
    AcknowledgementKind, AttemptResolution, CancellationAttempt, complete_process_mapping,
    finish_cancellation_attempt,
};
pub(crate) use state::{RunningProcesses, TrackedProcess};

const CANCEL_CREATION_WINDOW: Duration = Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS + 5);
const CANCEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const CANCEL_RETRY_DELAY: Duration = Duration::from_millis(100);

pub(crate) const CANCELLATION_DIR: &str = "/tmp/roder-cancelled-processes";
pub(crate) const COMMAND_TAG_ENV: &str = "RODER_BLAXEL_COMMAND_TAG";

pub(crate) struct ActiveProcessGuard {
    client: BlaxelClient,
    endpoint: String,
    command_id: RunnerCommandId,
    tracked: TrackedProcess,
    running_processes: RunningProcesses,
    armed: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CancellationOutcome {
    acknowledgement: Option<AcknowledgementKind>,
    reap_fence: ReapFence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReapFence {
    Confirmed,
    PermanentTombstone,
    EstablishPermanentTombstone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupervisorOutcome {
    Confirmed,
    TombstonedAbsent,
    Unsettled { tombstoned: bool },
}

impl ActiveProcessGuard {
    pub(crate) fn new(
        client: BlaxelClient,
        endpoint: String,
        command_id: RunnerCommandId,
        tracked: TrackedProcess,
        running_processes: RunningProcesses,
    ) -> Self {
        Self {
            client,
            endpoint,
            command_id,
            tracked,
            running_processes,
            armed: true,
        }
    }

    pub(crate) fn complete(&mut self) {
        self.armed = false;
        complete_process_mapping(&self.running_processes, &self.command_id, &self.tracked);
    }
}

impl Drop for ActiveProcessGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        let command_id = self.command_id.clone();
        let running_processes = self.running_processes.clone();
        let tracked = self.tracked.clone();
        drop(runtime.spawn(async move {
            cancel_registered_generation(client, endpoint, running_processes, command_id, tracked)
                .await;
        }));
    }
}

pub(crate) fn cancellation_marker(process_name: &str) -> String {
    format!("{CANCELLATION_DIR}/{process_name}")
}

pub(crate) fn tagged_environment(
    request: &RunnerCommandRequest,
    command_tag: &str,
) -> Vec<(String, String)> {
    let mut environment: Vec<_> = request
        .env
        .iter()
        .filter(|(key, _)| key != COMMAND_TAG_ENV)
        .cloned()
        .collect();
    environment.push((COMMAND_TAG_ENV.to_string(), command_tag.to_string()));
    environment
}

async fn cancel_process_with_retry(
    client: &BlaxelClient,
    endpoint: &str,
    tracked: &TrackedProcess,
) -> CancellationOutcome {
    // Sweep immediately so a detached signal-ignoring child cannot mutate the
    // workspace while control-plane registration or DELETE is still racing.
    // Settlement retains the tombstone, and the second sweep below is the
    // authoritative proof after no tagged process can start late.
    let (supervisor, _) = tokio::join!(
        settle_supervisor(client, endpoint, &tracked.name),
        cleanup_tagged_descendants(client, endpoint, &tracked.tag)
    );
    match supervisor {
        SupervisorOutcome::Confirmed | SupervisorOutcome::TombstonedAbsent => {
            let descendants_clean =
                cleanup_tagged_descendants(client, endpoint, &tracked.tag).await;
            cancellation_outcome(supervisor, descendants_clean)
        }
        SupervisorOutcome::Unsettled { tombstoned } => CancellationOutcome {
            acknowledgement: None,
            reap_fence: if tombstoned {
                ReapFence::PermanentTombstone
            } else {
                ReapFence::EstablishPermanentTombstone
            },
        },
    }
}

fn cancellation_outcome(
    supervisor: SupervisorOutcome,
    descendants_clean: bool,
) -> CancellationOutcome {
    match (supervisor, descendants_clean) {
        (SupervisorOutcome::Confirmed, true) => CancellationOutcome {
            acknowledgement: Some(AcknowledgementKind::Confirmed),
            reap_fence: ReapFence::Confirmed,
        },
        (SupervisorOutcome::TombstonedAbsent, true) => CancellationOutcome {
            acknowledgement: Some(AcknowledgementKind::Provisional),
            reap_fence: ReapFence::PermanentTombstone,
        },
        (SupervisorOutcome::Confirmed, false) => CancellationOutcome {
            acknowledgement: None,
            reap_fence: ReapFence::Confirmed,
        },
        (SupervisorOutcome::TombstonedAbsent, false) => CancellationOutcome {
            acknowledgement: None,
            reap_fence: ReapFence::PermanentTombstone,
        },
        (SupervisorOutcome::Unsettled { tombstoned }, _) => CancellationOutcome {
            acknowledgement: None,
            reap_fence: if tombstoned {
                ReapFence::PermanentTombstone
            } else {
                ReapFence::EstablishPermanentTombstone
            },
        },
    }
}

pub(crate) async fn cancel_registered_process(
    client: BlaxelClient,
    endpoint: String,
    running_processes: RunningProcesses,
    command_id: RunnerCommandId,
) -> bool {
    cancel_registered_process_inner(client, endpoint, running_processes, command_id, None).await
}

async fn cancel_registered_generation(
    client: BlaxelClient,
    endpoint: String,
    running_processes: RunningProcesses,
    command_id: RunnerCommandId,
    tracked: TrackedProcess,
) -> bool {
    cancel_registered_process_inner(
        client,
        endpoint,
        running_processes,
        command_id,
        Some(tracked),
    )
    .await
}

async fn cancel_registered_process_inner(
    client: BlaxelClient,
    endpoint: String,
    running_processes: RunningProcesses,
    command_id: RunnerCommandId,
    expected: Option<TrackedProcess>,
) -> bool {
    let (mut result, work) = {
        let mut processes = running_processes.lock().unwrap();
        let Some(process) = processes.get_mut(&command_id) else {
            return false;
        };
        if expected
            .as_ref()
            .is_some_and(|expected| !process.same_generation(expected))
        {
            return false;
        }
        process.cancelled = true;
        if process.acknowledgement.is_some() {
            return true;
        }
        if let Some(attempt) = &process.cancellation {
            (attempt.result.clone(), None)
        } else {
            let attempt_id = uuid::Uuid::new_v4().simple().to_string();
            let (sender, receiver) = watch::channel(None);
            process.cancellation = Some(CancellationAttempt {
                id: attempt_id.clone(),
                result: receiver.clone(),
                sender: sender.clone(),
            });
            (receiver, Some((attempt_id, sender, process.clone())))
        }
    };
    if let Some((attempt_id, sender, tracked)) = work {
        let cleanup_client = client.clone();
        let cleanup_endpoint = endpoint.clone();
        let cleanup_processes = running_processes.clone();
        let cleanup_command_id = command_id.clone();
        tokio::spawn(async move {
            let outcome =
                cancel_process_with_retry(&cleanup_client, &cleanup_endpoint, &tracked).await;
            let resolution = finish_cancellation_attempt(
                &cleanup_processes,
                &cleanup_command_id,
                &tracked,
                &attempt_id,
                outcome.acknowledgement,
                &sender,
            );
            match resolution {
                AttemptResolution::Recorded(AcknowledgementKind::Confirmed) => {
                    schedule_confirmed_ack_forget(
                        cleanup_client,
                        cleanup_endpoint,
                        cleanup_processes.clone(),
                        cleanup_command_id.clone(),
                        tracked.clone(),
                    );
                }
                AttemptResolution::Recorded(AcknowledgementKind::Provisional) => {
                    schedule_process_reap(
                        cleanup_client,
                        cleanup_endpoint,
                        cleanup_processes.clone(),
                        cleanup_command_id.clone(),
                        tracked.clone(),
                        ReapFence::PermanentTombstone,
                        false,
                    );
                }
                AttemptResolution::AlreadyAcknowledged => {}
                AttemptResolution::Failed => {
                    schedule_process_reap(
                        cleanup_client,
                        cleanup_endpoint,
                        cleanup_processes.clone(),
                        cleanup_command_id.clone(),
                        tracked.clone(),
                        outcome.reap_fence,
                        true,
                    );
                }
            }
        });
    }

    loop {
        let observed = *result.borrow();
        if let Some(cancelled) = observed {
            return cancelled;
        }
        if result.changed().await.is_err() {
            return false;
        }
    }
}

async fn write_cancellation_tombstone(
    client: &BlaxelClient,
    endpoint: &str,
    process_name: &str,
) -> bool {
    matches!(
        tokio::time::timeout(
            CANCEL_REQUEST_TIMEOUT,
            client.write_file(endpoint, &cancellation_marker(process_name), b"cancelled"),
        )
        .await,
        Ok(Ok(()))
    )
}

async fn settle_supervisor(
    client: &BlaxelClient,
    endpoint: &str,
    process_name: &str,
) -> SupervisorOutcome {
    // A marker closes the POST-vs-DELETE registration race: if Blaxel accepts
    // the named process after early DELETEs observed 404, the shell guard exits
    // before the tagged user command can execute.
    let mut tombstoned = false;
    let deadline = Instant::now() + CANCEL_CREATION_WINDOW;
    loop {
        if !tombstoned {
            tombstoned = write_cancellation_tombstone(client, endpoint, process_name).await;
        }
        if let Ok(Ok(true)) = tokio::time::timeout(
            CANCEL_REQUEST_TIMEOUT,
            client.kill_process(endpoint, process_name),
        )
        .await
        {
            return SupervisorOutcome::Confirmed;
        }
        if let Ok(Ok(Some(process))) = tokio::time::timeout(
            CANCEL_REQUEST_TIMEOUT,
            client.get_process(endpoint, process_name),
        )
        .await
            && process.is_terminal()
        {
            return SupervisorOutcome::Confirmed;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(CANCEL_RETRY_DELAY).await;
    }

    let observed = tokio::time::timeout(
        CANCEL_REQUEST_TIMEOUT,
        client.get_process(endpoint, process_name),
    )
    .await;
    match observed {
        Ok(Ok(Some(process))) if process.is_terminal() => SupervisorOutcome::Confirmed,
        Ok(Ok(None)) if tombstoned => SupervisorOutcome::TombstonedAbsent,
        _ => SupervisorOutcome::Unsettled { tombstoned },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_window_covers_remote_registration() {
        assert!(CANCEL_CREATION_WINDOW > Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS));
    }

    #[test]
    fn tombstoned_absence_is_only_a_provisional_acknowledgement() {
        let outcome = cancellation_outcome(SupervisorOutcome::TombstonedAbsent, true);

        assert_eq!(
            outcome.acknowledgement,
            Some(AcknowledgementKind::Provisional)
        );
        assert_eq!(outcome.reap_fence, ReapFence::PermanentTombstone);
    }

    #[test]
    fn terminal_supervisor_and_clean_descendants_are_confirmed() {
        let outcome = cancellation_outcome(SupervisorOutcome::Confirmed, true);

        assert_eq!(
            outcome.acknowledgement,
            Some(AcknowledgementKind::Confirmed)
        );
        assert_eq!(outcome.reap_fence, ReapFence::Confirmed);
    }
}
