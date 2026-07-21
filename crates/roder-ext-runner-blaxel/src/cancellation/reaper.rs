use std::time::Duration;

use roder_api::remote_runner::RunnerCommandId;

use crate::client::BlaxelClient;

use super::descendants::cleanup_tagged_descendants;
use super::state::{
    AcknowledgementKind, RunningProcesses, TrackedProcess, acknowledgement_kind, claim_reaper,
    finish_failed_reaper, process_is_registered, record_background_acknowledgement, release_reaper,
    remove_process_mapping,
};
use super::{CANCEL_REQUEST_TIMEOUT, ReapFence, cancellation_marker, write_cancellation_tombstone};

const CONFIRMED_ACK_RETENTION: Duration = Duration::from_secs(120);
const REAP_CLEANUP_ATTEMPTS: usize = 3;
const PROMPT_CLEANUP_RETRY_DELAY: Duration = Duration::from_secs(1);
const REAP_CLEANUP_RETRY_DELAY: Duration = Duration::from_secs(30);

async fn forget_process(
    client: &BlaxelClient,
    endpoint: &str,
    running_processes: &RunningProcesses,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
) {
    remove_process_mapping(running_processes, command_id, tracked);
    let _ = tokio::time::timeout(
        CANCEL_REQUEST_TIMEOUT,
        client.delete_file(endpoint, &cancellation_marker(&tracked.name)),
    )
    .await;
}

pub(super) fn schedule_confirmed_ack_forget(
    client: BlaxelClient,
    endpoint: String,
    running_processes: RunningProcesses,
    command_id: RunnerCommandId,
    tracked: TrackedProcess,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    drop(runtime.spawn(async move {
        tokio::time::sleep(CONFIRMED_ACK_RETENTION).await;
        // Removal is generation-safe; deletion targets the old process's
        // unique marker even if the command id has since been reused.
        forget_process(
            &client,
            &endpoint,
            &running_processes,
            &command_id,
            &tracked,
        )
        .await;
    }));
}

fn schedule_permanent_ack_forget(
    running_processes: RunningProcesses,
    command_id: RunnerCommandId,
    tracked: TrackedProcess,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    drop(runtime.spawn(async move {
        tokio::time::sleep(CONFIRMED_ACK_RETENTION).await;
        // The unique tombstone is permanent because process registration was
        // never positively closed. Only the generation-safe local map expires.
        remove_process_mapping(&running_processes, &command_id, &tracked);
    }));
}

pub(super) fn schedule_process_reap(
    client: BlaxelClient,
    endpoint: String,
    running_processes: RunningProcesses,
    command_id: RunnerCommandId,
    tracked: TrackedProcess,
    fence: ReapFence,
    retry_promptly: bool,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let Some(reaper_owner) = claim_reaper(
        &running_processes,
        &command_id,
        &tracked,
        !retry_promptly && fence == ReapFence::PermanentTombstone,
    ) else {
        return;
    };
    drop(runtime.spawn(async move {
        if retry_promptly {
            for attempt in 0..REAP_CLEANUP_ATTEMPTS {
                if !process_is_registered(&running_processes, &command_id, &tracked) {
                    return;
                }
                let fenced = fence == ReapFence::Confirmed
                    || write_cancellation_tombstone(&client, &endpoint, &tracked.name).await;
                let descendants_clean =
                    cleanup_tagged_descendants(&client, &endpoint, &tracked.tag).await;
                if descendants_clean && fence == ReapFence::Confirmed {
                    let transitioned = record_background_acknowledgement(
                        &running_processes,
                        &command_id,
                        &tracked,
                        AcknowledgementKind::Confirmed,
                    );
                    release_reaper(&running_processes, &command_id, &tracked, &reaper_owner);
                    if transitioned {
                        schedule_confirmed_ack_forget(
                            client,
                            endpoint,
                            running_processes,
                            command_id,
                            tracked,
                        );
                    }
                    return;
                }
                if descendants_clean && fenced {
                    let transitioned = record_background_acknowledgement(
                        &running_processes,
                        &command_id,
                        &tracked,
                        AcknowledgementKind::Provisional,
                    );
                    if !transitioned
                        && acknowledgement_kind(&running_processes, &command_id, &tracked)
                            != Some(AcknowledgementKind::Provisional)
                    {
                        release_reaper(&running_processes, &command_id, &tracked, &reaper_owner);
                        return;
                    }
                    break;
                }
                if attempt + 1 < REAP_CLEANUP_ATTEMPTS {
                    tokio::time::sleep(PROMPT_CLEANUP_RETRY_DELAY).await;
                }
            }
        }

        tokio::time::sleep_until(tokio::time::Instant::from_std(tracked.reap_after)).await;
        for attempt in 0..REAP_CLEANUP_ATTEMPTS {
            if !process_is_registered(&running_processes, &command_id, &tracked) {
                return;
            }
            if fence != ReapFence::Confirmed
                && !write_cancellation_tombstone(&client, &endpoint, &tracked.name).await
            {
                if attempt + 1 < REAP_CLEANUP_ATTEMPTS {
                    tokio::time::sleep(REAP_CLEANUP_RETRY_DELAY).await;
                }
                continue;
            }
            if cleanup_tagged_descendants(&client, &endpoint, &tracked.tag).await {
                let transitioned = record_background_acknowledgement(
                    &running_processes,
                    &command_id,
                    &tracked,
                    AcknowledgementKind::Confirmed,
                );
                release_reaper(&running_processes, &command_id, &tracked, &reaper_owner);
                if transitioned {
                    if fence == ReapFence::Confirmed {
                        schedule_confirmed_ack_forget(
                            client,
                            endpoint,
                            running_processes,
                            command_id,
                            tracked,
                        );
                    } else {
                        // A timed-out POST can commit arbitrarily late. Its
                        // unique tombstone is the permanent fence; only the
                        // local mapping is reaped after the full lease horizon.
                        schedule_permanent_ack_forget(running_processes, command_id, tracked);
                    }
                }
                return;
            }
            if attempt + 1 < REAP_CLEANUP_ATTEMPTS {
                tokio::time::sleep(REAP_CLEANUP_RETRY_DELAY).await;
            }
        }
        finish_failed_reaper(&running_processes, &command_id, &tracked, &reaper_owner);
        // Keep both mapping and tombstone. The named process lease does not
        // bound a detached descendant, so absence cannot be assumed.
    }));
}
