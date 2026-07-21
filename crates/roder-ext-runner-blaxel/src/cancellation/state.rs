use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use roder_api::remote_runner::RunnerCommandId;
use tokio::sync::watch;

use crate::client::HTTP_REQUEST_TIMEOUT_SECONDS;

const PROCESS_REAP_GRACE_SECONDS: u64 = HTTP_REQUEST_TIMEOUT_SECONDS + 30;

pub(crate) type RunningProcesses = Arc<Mutex<HashMap<RunnerCommandId, TrackedProcess>>>;

#[derive(Clone)]
pub(crate) struct TrackedProcess {
    pub(crate) name: String,
    pub(crate) tag: String,
    pub(crate) cancelled: bool,
    pub(super) reap_after: Instant,
    pub(super) cancellation: Option<CancellationAttempt>,
    pub(super) acknowledgement: Option<AcknowledgementKind>,
    reaper_owner: Option<String>,
}

#[derive(Clone)]
pub(super) struct CancellationAttempt {
    pub(super) id: String,
    pub(super) result: watch::Receiver<Option<bool>>,
    pub(super) sender: watch::Sender<Option<bool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AcknowledgementKind {
    Confirmed,
    Provisional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptResolution {
    Recorded(AcknowledgementKind),
    AlreadyAcknowledged,
    Failed,
}

impl TrackedProcess {
    pub(crate) fn new(timeout_seconds: u64) -> Self {
        let identity = uuid::Uuid::new_v4().simple().to_string();
        Self {
            name: format!("roder-{identity}"),
            tag: identity,
            cancelled: false,
            cancellation: None,
            acknowledgement: None,
            reaper_owner: None,
            // The server lease begins only after process registration. Include
            // a complete HTTP registration horizon plus a completion grace so
            // cleanup never races a late process that can still start or run.
            reap_after: Instant::now()
                + Duration::from_secs(timeout_seconds + PROCESS_REAP_GRACE_SECONDS),
        }
    }

    pub(crate) fn can_be_replaced(&self) -> bool {
        self.acknowledgement == Some(AcknowledgementKind::Confirmed)
    }

    pub(super) fn same_generation(&self, other: &Self) -> bool {
        self.name == other.name && self.tag == other.tag
    }
}

fn same_process(current: &TrackedProcess, expected: &TrackedProcess) -> bool {
    current.same_generation(expected)
}

pub(super) fn complete_process_mapping(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
) {
    let mut processes = running_processes.lock().unwrap();
    if processes
        .get(command_id)
        .is_some_and(|current| same_process(current, tracked) && !current.cancelled)
    {
        processes.remove(command_id);
    }
}

pub(super) fn remove_process_mapping(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
) {
    let mut processes = running_processes.lock().unwrap();
    if processes
        .get(command_id)
        .is_some_and(|current| same_process(current, tracked))
    {
        processes.remove(command_id);
    }
}

pub(super) fn process_is_registered(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
) -> bool {
    running_processes
        .lock()
        .unwrap()
        .get(command_id)
        .is_some_and(|current| same_process(current, tracked))
}

pub(super) fn claim_reaper(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
    allow_provisional: bool,
) -> Option<String> {
    let mut processes = running_processes.lock().unwrap();
    let current = processes.get_mut(command_id)?;
    let acknowledgement_allows_claim = current.acknowledgement.is_none()
        || (allow_provisional && current.acknowledgement == Some(AcknowledgementKind::Provisional));
    if !same_process(current, tracked)
        || current.reaper_owner.is_some()
        || !acknowledgement_allows_claim
    {
        return None;
    }
    let owner = uuid::Uuid::new_v4().simple().to_string();
    current.reaper_owner = Some(owner.clone());
    Some(owner)
}

pub(super) fn release_reaper(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
    owner: &str,
) {
    let mut processes = running_processes.lock().unwrap();
    let Some(current) = processes.get_mut(command_id) else {
        return;
    };
    if same_process(current, tracked) && current.reaper_owner.as_deref() == Some(owner) {
        current.reaper_owner = None;
    }
}

pub(super) fn acknowledgement_kind(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
) -> Option<AcknowledgementKind> {
    running_processes
        .lock()
        .unwrap()
        .get(command_id)
        .filter(|current| same_process(current, tracked))
        .and_then(|current| current.acknowledgement)
}

pub(super) fn finish_cancellation_attempt(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
    attempt_id: &str,
    outcome: Option<AcknowledgementKind>,
    sender: &watch::Sender<Option<bool>>,
) -> AttemptResolution {
    let mut processes = running_processes.lock().unwrap();
    let Some(current) = processes.get_mut(command_id) else {
        publish_attempt_result(sender, false);
        return AttemptResolution::Failed;
    };
    if !same_process(current, tracked) {
        publish_attempt_result(sender, false);
        return AttemptResolution::Failed;
    }
    if current.acknowledgement.is_some() {
        if current
            .cancellation
            .as_ref()
            .is_some_and(|attempt| attempt.id == attempt_id)
        {
            current.cancellation = None;
        }
        publish_attempt_result(sender, true);
        return AttemptResolution::AlreadyAcknowledged;
    }
    if current
        .cancellation
        .as_ref()
        .is_none_or(|attempt| attempt.id != attempt_id)
    {
        publish_attempt_result(sender, false);
        return AttemptResolution::Failed;
    }
    current.cancellation = None;
    let Some(kind) = outcome else {
        publish_attempt_result(sender, false);
        return AttemptResolution::Failed;
    };
    current.acknowledgement = Some(kind);
    publish_attempt_result(sender, true);
    AttemptResolution::Recorded(kind)
}

fn publish_attempt_result(sender: &watch::Sender<Option<bool>>, result: bool) {
    // A positive cancellation proof is monotonic. Never let a stale worker
    // overwrite a true result already published by a background reaper.
    if *sender.borrow() != Some(true) {
        let _ = sender.send(Some(result));
    }
}

pub(super) fn record_background_acknowledgement(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
    kind: AcknowledgementKind,
) -> bool {
    let mut processes = running_processes.lock().unwrap();
    let Some(current) = processes.get_mut(command_id) else {
        return false;
    };
    if !same_process(current, tracked) {
        return false;
    }
    let upgraded = match (current.acknowledgement, kind) {
        (None, kind) => Some(kind),
        (Some(AcknowledgementKind::Provisional), AcknowledgementKind::Confirmed) => {
            Some(AcknowledgementKind::Confirmed)
        }
        _ => None,
    };
    let Some(upgraded) = upgraded else {
        return false;
    };
    current.acknowledgement = Some(upgraded);
    if let Some(attempt) = &current.cancellation {
        publish_attempt_result(&attempt.sender, true);
    }
    true
}

pub(super) fn finish_failed_reaper(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
    owner: &str,
) {
    let mut processes = running_processes.lock().unwrap();
    let Some(current) = processes.get_mut(command_id) else {
        return;
    };
    if !same_process(current, tracked) || current.reaper_owner.as_deref() != Some(owner) {
        return;
    }
    if current.acknowledgement == Some(AcknowledgementKind::Provisional) {
        current.acknowledgement = None;
    }
    current.reaper_owner = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_generation_has_only_one_reaper_owner() {
        let command_id = "same-command".to_string();
        let tracked = TrackedProcess::new(1);
        let processes = Mutex::new(HashMap::from([(command_id.clone(), tracked.clone())]));

        let owner = claim_reaper(&processes, &command_id, &tracked, false).unwrap();
        for _ in 0..10 {
            assert!(claim_reaper(&processes, &command_id, &tracked, false).is_none());
        }

        release_reaper(&processes, &command_id, &tracked, "not-the-owner");
        assert!(claim_reaper(&processes, &command_id, &tracked, false).is_none());
        release_reaper(&processes, &command_id, &tracked, &owner);
        assert!(claim_reaper(&processes, &command_id, &tracked, false).is_some());
    }

    #[test]
    fn failed_reaper_atomically_clears_provisional_ack_and_ownership() {
        let command_id = "retry-after-horizon".to_string();
        let tracked = TrackedProcess::new(1);
        let processes = Mutex::new(HashMap::from([(command_id.clone(), tracked.clone())]));
        let owner = claim_reaper(&processes, &command_id, &tracked, false).unwrap();
        assert!(record_background_acknowledgement(
            &processes,
            &command_id,
            &tracked,
            AcknowledgementKind::Provisional,
        ));

        finish_failed_reaper(&processes, &command_id, &tracked, "not-the-owner");
        assert_eq!(
            acknowledgement_kind(&processes, &command_id, &tracked),
            Some(AcknowledgementKind::Provisional)
        );
        assert!(claim_reaper(&processes, &command_id, &tracked, false).is_none());

        finish_failed_reaper(&processes, &command_id, &tracked, &owner);
        assert_eq!(
            acknowledgement_kind(&processes, &command_id, &tracked),
            None
        );
        assert!(claim_reaper(&processes, &command_id, &tracked, false).is_some());
    }

    #[test]
    fn background_ack_publishes_true_before_a_failed_worker_can_finish() {
        let command_id = "ack-linearization".to_string();
        let mut tracked = TrackedProcess::new(1);
        let (sender, receiver) = watch::channel(None);
        tracked.cancellation = Some(CancellationAttempt {
            id: "attempt".to_string(),
            result: receiver.clone(),
            sender: sender.clone(),
        });
        let processes = Mutex::new(HashMap::from([(command_id.clone(), tracked.clone())]));

        assert!(record_background_acknowledgement(
            &processes,
            &command_id,
            &tracked,
            AcknowledgementKind::Provisional,
        ));
        assert_eq!(*receiver.borrow(), Some(true));
        assert_eq!(
            finish_cancellation_attempt(
                &processes,
                &command_id,
                &tracked,
                "attempt",
                None,
                &sender,
            ),
            AttemptResolution::AlreadyAcknowledged
        );
        assert_eq!(*receiver.borrow(), Some(true));
    }
}
