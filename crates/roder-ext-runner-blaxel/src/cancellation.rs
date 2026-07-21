use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use roder_api::remote_runner::{RunnerCommandId, RunnerCommandRequest};
use tokio::sync::watch;

use crate::client::{BlaxelClient, HTTP_REQUEST_TIMEOUT_SECONDS};

const CANCEL_CREATION_WINDOW: Duration = Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS + 5);
const CANCEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const CANCEL_RETRY_DELAY: Duration = Duration::from_millis(100);
const DESCENDANT_CLEANUP_LEASE_SECONDS: u64 = 15;
const DESCENDANT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(20);
const PROCESS_REAP_GRACE_SECONDS: u64 = HTTP_REQUEST_TIMEOUT_SECONDS + 30;
const REAP_CLEANUP_ATTEMPTS: usize = 3;
const REAP_CLEANUP_RETRY_DELAY: Duration = Duration::from_secs(30);

pub(crate) const CANCELLATION_DIR: &str = "/tmp/roder-cancelled-processes";
pub(crate) const COMMAND_TAG_ENV: &str = "RODER_BLAXEL_COMMAND_TAG";

pub(crate) type RunningProcesses = Arc<Mutex<HashMap<RunnerCommandId, TrackedProcess>>>;

#[derive(Clone)]
pub(crate) struct TrackedProcess {
    pub(crate) name: String,
    pub(crate) tag: String,
    pub(crate) cancelled: bool,
    pub(crate) reap_after: Instant,
    cancellation: Option<CancellationAttempt>,
}

#[derive(Clone)]
struct CancellationAttempt {
    id: String,
    result: watch::Receiver<Option<bool>>,
}

impl TrackedProcess {
    pub(crate) fn new(timeout_seconds: u64) -> Self {
        let identity = uuid::Uuid::new_v4().simple().to_string();
        Self {
            name: format!("roder-{identity}"),
            tag: identity,
            cancelled: false,
            cancellation: None,
            // The server lease begins only after process registration. Include
            // a complete HTTP registration horizon plus a completion grace so
            // cleanup never races a late process that can still start or run.
            reap_after: Instant::now()
                + Duration::from_secs(timeout_seconds + PROCESS_REAP_GRACE_SECONDS),
        }
    }
}

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
    pub(crate) cancelled: bool,
    pub(crate) safe_to_forget: bool,
}

#[derive(Debug, Clone, Copy)]
struct SupervisorOutcome {
    settled: bool,
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
        drop(runtime.spawn(async move {
            cancel_registered_process(client, endpoint, running_processes, command_id).await;
        }));
    }
}

fn same_process(current: &TrackedProcess, expected: &TrackedProcess) -> bool {
    current.name == expected.name && current.tag == expected.tag
}

fn complete_process_mapping(
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

fn remove_process_mapping(
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

async fn forget_process(
    client: &BlaxelClient,
    endpoint: &str,
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
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

fn schedule_process_reap(
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
        tokio::time::sleep_until(tokio::time::Instant::from_std(tracked.reap_after)).await;
        for attempt in 0..REAP_CLEANUP_ATTEMPTS {
            if !process_is_registered(&running_processes, &command_id, &tracked) {
                return;
            }
            if cleanup_tagged_descendants(&client, &endpoint, &tracked.tag).await {
                forget_process(
                    &client,
                    &endpoint,
                    &running_processes,
                    &command_id,
                    &tracked,
                )
                .await;
                return;
            }
            if attempt + 1 < REAP_CLEANUP_ATTEMPTS {
                tokio::time::sleep(REAP_CLEANUP_RETRY_DELAY).await;
            }
        }
        // Keep both the mapping and tombstone. The named process lease does
        // not bound a detached descendant, so absence cannot be assumed.
    }));
}

fn process_is_registered(
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
    if !supervisor.settled {
        return CancellationOutcome {
            cancelled: false,
            safe_to_forget: false,
        };
    }

    let descendants_clean = cleanup_tagged_descendants(client, endpoint, &tracked.tag).await;
    CancellationOutcome {
        // For an existing mapping, `true` is the provider's proof that the
        // whole tagged command tree is quiescent, even if the named supervisor
        // reached a natural terminal state before DELETE won the race.
        cancelled: descendants_clean,
        safe_to_forget: descendants_clean,
    }
}

pub(crate) async fn cancel_registered_process(
    client: BlaxelClient,
    endpoint: String,
    running_processes: RunningProcesses,
    command_id: RunnerCommandId,
) -> bool {
    let (mut result, work) = {
        let mut processes = running_processes.lock().unwrap();
        let Some(process) = processes.get_mut(&command_id) else {
            return false;
        };
        process.cancelled = true;
        if let Some(attempt) = &process.cancellation {
            (attempt.result.clone(), None)
        } else {
            let attempt_id = uuid::Uuid::new_v4().simple().to_string();
            let (sender, receiver) = watch::channel(None);
            process.cancellation = Some(CancellationAttempt {
                id: attempt_id.clone(),
                result: receiver.clone(),
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
            if outcome.safe_to_forget {
                forget_process(
                    &cleanup_client,
                    &cleanup_endpoint,
                    &cleanup_processes,
                    &cleanup_command_id,
                    &tracked,
                )
                .await;
            } else {
                schedule_process_reap(
                    cleanup_client,
                    cleanup_endpoint,
                    cleanup_processes.clone(),
                    cleanup_command_id.clone(),
                    tracked.clone(),
                );
            }
            if !outcome.safe_to_forget {
                clear_cancellation_attempt(
                    &cleanup_processes,
                    &cleanup_command_id,
                    &tracked,
                    &attempt_id,
                );
            }
            let _ = sender.send(Some(outcome.cancelled));
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

fn clear_cancellation_attempt(
    running_processes: &Mutex<HashMap<RunnerCommandId, TrackedProcess>>,
    command_id: &RunnerCommandId,
    tracked: &TrackedProcess,
    attempt_id: &str,
) {
    let mut processes = running_processes.lock().unwrap();
    let Some(current) = processes.get_mut(command_id) else {
        return;
    };
    if same_process(current, tracked)
        && current
            .cancellation
            .as_ref()
            .is_some_and(|attempt| attempt.id == attempt_id)
    {
        current.cancellation = None;
    }
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
            tombstoned = matches!(
                tokio::time::timeout(
                    CANCEL_REQUEST_TIMEOUT,
                    client.write_file(endpoint, &cancellation_marker(process_name), b"cancelled",),
                )
                .await,
                Ok(Ok(()))
            );
        }
        if let Ok(Ok(true)) = tokio::time::timeout(
            CANCEL_REQUEST_TIMEOUT,
            client.kill_process(endpoint, process_name),
        )
        .await
        {
            return SupervisorOutcome { settled: true };
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
        Ok(Ok(Some(process))) if process.is_terminal() => SupervisorOutcome { settled: true },
        Ok(Ok(None)) if tombstoned => SupervisorOutcome { settled: true },
        _ => SupervisorOutcome { settled: false },
    }
}

async fn cleanup_tagged_descendants(
    client: &BlaxelClient,
    endpoint: &str,
    command_tag: &str,
) -> bool {
    let cleanup_name = format!("roder-cleanup-{}", uuid::Uuid::new_v4().simple());
    let command = descendant_cleanup_command(command_tag);
    matches!(
        tokio::time::timeout(
            DESCENDANT_CLEANUP_TIMEOUT,
            client.exec(
                endpoint,
                &cleanup_name,
                &command,
                None,
                &[],
                DESCENDANT_CLEANUP_LEASE_SECONDS,
            ),
        )
        .await,
        Ok(Ok(process)) if process.exit_code == Some(0)
    )
}

fn descendant_cleanup_command(command_tag: &str) -> String {
    let exact_tag = shell_quote(&format!("{COMMAND_TAG_ENV}={command_tag}"));
    let script = DESCENDANT_CLEANUP_SCRIPT.replacen("__RODER_EXACT_TAG__", &exact_tag, 1);
    format!("/bin/sh -c {}", shell_quote(&script))
}

const DESCENDANT_CLEANUP_SCRIPT: &str = r#"tag=__RODER_EXACT_TAG__
command -v tr >/dev/null 2>&1 || exit 125
command -v grep >/dev/null 2>&1 || exit 125
command -v sleep >/dev/null 2>&1 || exit 125
[ -r /proc/self/environ ] || exit 125
tagged() {
  environment=$1
  [ -r "$environment" ] || return 1
  tr '\000' '\n' 2>/dev/null < "$environment" | grep -Fqx "$tag"
}
has_tagged() {
  for environment in /proc/[0-9]*/environ; do
    tagged "$environment" && return 0
  done
  return 1
}
signal_tagged() {
  signal=$1
  for environment in /proc/[0-9]*/environ; do
    tagged "$environment" || continue
    pid=${environment#/proc/}
    pid=${pid%/environ}
    tagged "$environment" || continue
    kill "-$signal" "$pid" 2>/dev/null || :
  done
}
reap_tagged() {
  signal=$1
  remaining=$2
  quiet=0
  while [ "$remaining" -gt 0 ]; do
    if has_tagged; then
      quiet=0
      signal_tagged "$signal"
    else
      quiet=$((quiet + 1))
      [ "$quiet" -ge 2 ] && return 0
    fi
    remaining=$((remaining - 1))
    sleep 1
  done
  return 1
}
reap_tagged TERM 2 && exit 0
reap_tagged KILL 5 && exit 0
exit 1
"#;

pub(crate) fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'=')
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_windows_cover_registration_and_descendant_cleanup() {
        assert!(CANCEL_CREATION_WINDOW > Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECONDS));
        assert!(DESCENDANT_CLEANUP_TIMEOUT > Duration::from_secs(DESCENDANT_CLEANUP_LEASE_SECONDS));
    }

    #[test]
    fn descendant_cleanup_matches_only_the_exact_environment_entry() {
        let command = descendant_cleanup_command("tag-with-'quote");

        assert!(command.contains("RODER_BLAXEL_COMMAND_TAG=tag-with-"));
        assert!(DESCENDANT_CLEANUP_SCRIPT.contains("tr '\\000' '\\n'"));
        assert!(DESCENDANT_CLEANUP_SCRIPT.contains("grep -Fqx \"$tag\""));
        assert!(DESCENDANT_CLEANUP_SCRIPT.contains("reap_tagged TERM 2"));
        assert!(DESCENDANT_CLEANUP_SCRIPT.contains("reap_tagged KILL 5"));
    }
}
