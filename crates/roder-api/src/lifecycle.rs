use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    events::{ThreadId, TurnId},
    extension_state::{ExtensionStateRecord, ExtensionStoreScope},
};

pub const TURN_LIFECYCLE_EXTENSION_ID: &str = "roder.lifecycle";
pub const TURN_LIFECYCLE_STATE_KEY: &str = "turn_lifecycle";
pub const TURN_LIFECYCLE_CORRUPTION_STATE_KEY: &str = "turn_lifecycle_corruption";
pub const TURN_LIFECYCLE_SCHEMA_VERSION: u32 = 1;

/// Durable state for a turn's runtime lifecycle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnLifecycleState {
    Running,
    InterruptRequested,
    Interrupted,
    Completed,
    Failed,
    RecoveryNeeded,
}

impl TurnLifecycleState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Interrupted | Self::Completed | Self::Failed | Self::RecoveryNeeded
        )
    }

    pub fn requires_recovery(self) -> bool {
        matches!(self, Self::Running | Self::InterruptRequested)
    }
}

/// Whether provider cleanup was requested or observed for a turn transition.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnCleanupState {
    #[default]
    NotRequested,
    Requested,
    Completed,
    TimedOut,
    Unknown,
}

/// What the runtime can prove about work owned by a turn at the time of a
/// lifecycle transition. This deliberately avoids provider names, command
/// lines, PIDs, and other sensitive execution details.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnCleanupOwnership {
    /// The runtime observed only its own async turn task. It has no provider
    /// child-process or remote-job reaping acknowledgement.
    #[default]
    RuntimeTaskOnly,
    /// A provider registered cleanup ownership, but the runtime has not yet
    /// observed its completion acknowledgement.
    ProviderCleanupPending,
    /// A provider-owned child or remote execution reported its cleanup path
    /// complete to the runtime.
    ProviderCleanupConfirmed,
}

/// Why a lifecycle transition occurred, when the runtime can determine one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnLifecycleReason {
    UserInterrupt,
    Shutdown,
    DeadlineExceeded,
    ProviderFailure,
    RuntimeRestart,
    RuntimeFailure,
}

/// A versioned, durable lifecycle transition for an individual turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnLifecycleRecord {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub state: TurnLifecycleState,
    #[serde(default)]
    pub cleanup: TurnCleanupState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<TurnLifecycleReason>,
    #[serde(default)]
    pub ownership: TurnCleanupOwnership,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl TurnLifecycleRecord {
    pub fn new(
        thread_id: ThreadId,
        turn_id: TurnId,
        state: TurnLifecycleState,
        cleanup: TurnCleanupState,
        reason: Option<TurnLifecycleReason>,
        timestamp: OffsetDateTime,
    ) -> Self {
        Self {
            thread_id,
            turn_id,
            state,
            cleanup,
            reason,
            ownership: TurnCleanupOwnership::default(),
            timestamp,
        }
    }

    pub fn with_ownership(mut self, ownership: TurnCleanupOwnership) -> Self {
        self.ownership = ownership;
        self
    }

    pub fn extension_state(&self) -> anyhow::Result<ExtensionStateRecord> {
        Ok(ExtensionStateRecord {
            extension_id: TURN_LIFECYCLE_EXTENSION_ID.to_string(),
            key: TURN_LIFECYCLE_STATE_KEY.to_string(),
            scope: ExtensionStoreScope::Turn {
                thread_id: self.thread_id.clone(),
                turn_id: self.turn_id.clone(),
            },
            schema_version: TURN_LIFECYCLE_SCHEMA_VERSION,
            value: serde_json::to_value(self)?,
        })
    }

    pub fn from_extension_state(record: &ExtensionStateRecord) -> anyhow::Result<Option<Self>> {
        if record.extension_id != TURN_LIFECYCLE_EXTENSION_ID
            || record.key != TURN_LIFECYCLE_STATE_KEY
        {
            return Ok(None);
        }

        anyhow::ensure!(
            record.schema_version == TURN_LIFECYCLE_SCHEMA_VERSION,
            "unsupported turn lifecycle schema version {}",
            record.schema_version
        );

        let decoded: Self = serde_json::from_value(record.value.clone())?;
        match &record.scope {
            ExtensionStoreScope::Turn { thread_id, turn_id }
                if thread_id == &decoded.thread_id && turn_id == &decoded.turn_id => {}
            ExtensionStoreScope::Turn { .. } => anyhow::bail!(
                "turn lifecycle record scope does not match its embedded thread and turn identifiers"
            ),
            _ => anyhow::bail!("turn lifecycle records must use turn scope"),
        }

        Ok(Some(decoded))
    }
}

/// Latest known lifecycle state for each turn plus tolerant-read diagnostics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnLifecycleSnapshot {
    pub records: Vec<TurnLifecycleRecord>,
    pub corrupt_record_count: usize,
}

/// Process-local, redacted lifecycle counters. These deliberately use fixed
/// fields rather than provider, command, process, or thread labels so callers
/// can monitor lifecycle health without exposing execution details.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleMetricsSnapshot {
    pub shutdown_drain_count: u64,
    pub clean_shutdown_count: u64,
    pub deadline_exceeded_count: u64,
    pub persistence_failed_count: u64,
    pub restart_reconciliation_count: u64,
    pub lifecycle_persistence_failure_count: u64,
    pub shutdown_drain_duration_ms_total: u64,
    pub provider_cleanup_confirmed_count: u64,
    pub provider_cleanup_timed_out_count: u64,
    pub provider_cleanup_unknown_count: u64,
}

/// Creates an in-memory diagnostic record when a thread store skipped malformed
/// extension-state lines. The marker intentionally carries only a count; raw
/// bytes may contain prompts, command output, or secrets.
pub fn turn_lifecycle_corruption_marker(
    thread_id: ThreadId,
    corrupt_record_count: usize,
) -> ExtensionStateRecord {
    ExtensionStateRecord {
        extension_id: TURN_LIFECYCLE_EXTENSION_ID.to_string(),
        key: TURN_LIFECYCLE_CORRUPTION_STATE_KEY.to_string(),
        scope: ExtensionStoreScope::Thread { thread_id },
        schema_version: TURN_LIFECYCLE_SCHEMA_VERSION,
        value: serde_json::json!({ "count": corrupt_record_count }),
    }
}

/// Selects the latest valid lifecycle record for each turn and counts corrupt
/// lifecycle extension records without making the enclosing thread unreadable.
pub fn latest_turn_lifecycle_records(
    records: &[ExtensionStateRecord],
) -> (BTreeMap<TurnId, TurnLifecycleRecord>, usize) {
    let mut latest = BTreeMap::new();
    let mut corrupt_record_count = 0;

    for record in records {
        match TurnLifecycleRecord::from_extension_state(record) {
            Ok(Some(decoded)) => {
                let should_replace =
                    latest
                        .get(&decoded.turn_id)
                        .is_none_or(|current: &TurnLifecycleRecord| {
                            current.timestamp <= decoded.timestamp
                        });
                if should_replace {
                    latest.insert(decoded.turn_id.clone(), decoded);
                }
            }
            Ok(None) => {}
            Err(_) => corrupt_record_count += 1,
        }
    }

    (latest, corrupt_record_count)
}

pub fn turn_lifecycle_snapshot(records: &[ExtensionStateRecord]) -> TurnLifecycleSnapshot {
    let marker_count: usize = records
        .iter()
        .filter(|record| {
            record.extension_id == TURN_LIFECYCLE_EXTENSION_ID
                && record.key == TURN_LIFECYCLE_CORRUPTION_STATE_KEY
        })
        .map(|record| {
            record
                .value
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(1)
        })
        .sum();
    let (records, corrupt_record_count) = latest_turn_lifecycle_records(records);

    TurnLifecycleSnapshot {
        records: records.into_values().collect(),
        corrupt_record_count: corrupt_record_count + marker_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: TurnLifecycleState, timestamp: OffsetDateTime) -> TurnLifecycleRecord {
        TurnLifecycleRecord {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            state,
            cleanup: TurnCleanupState::NotRequested,
            reason: None,
            ownership: TurnCleanupOwnership::RuntimeTaskOnly,
            timestamp,
        }
    }

    #[test]
    fn lifecycle_record_round_trips_through_extension_state() {
        let original = record(
            TurnLifecycleState::InterruptRequested,
            OffsetDateTime::UNIX_EPOCH,
        );

        let state = original.extension_state().expect("record should encode");
        let decoded = TurnLifecycleRecord::from_extension_state(&state)
            .expect("record should decode")
            .expect("record should be recognized");

        assert_eq!(decoded, original);
    }

    #[test]
    fn ownership_defaults_for_legacy_lifecycle_records() {
        let legacy = serde_json::json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "state": "interrupted",
            "cleanup": "unknown",
            "timestamp": "1970-01-01T00:00:00Z"
        });

        let record: TurnLifecycleRecord = serde_json::from_value(legacy).unwrap();

        assert_eq!(record.ownership, TurnCleanupOwnership::RuntimeTaskOnly);
    }

    #[test]
    fn latest_records_keep_newest_valid_transition_and_count_corruption() {
        let earlier = record(TurnLifecycleState::Running, OffsetDateTime::UNIX_EPOCH);
        let later = record(
            TurnLifecycleState::Interrupted,
            OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60),
        );
        let mut corrupt = later.extension_state().expect("record should encode");
        corrupt.schema_version = 99;

        let (records, corrupt_record_count) = latest_turn_lifecycle_records(&[
            earlier.extension_state().expect("record should encode"),
            corrupt,
            later.extension_state().expect("record should encode"),
        ]);

        assert_eq!(corrupt_record_count, 1);
        assert_eq!(records.get("turn-1"), Some(&later));
    }

    #[test]
    fn only_non_terminal_states_require_recovery() {
        assert!(TurnLifecycleState::Running.requires_recovery());
        assert!(TurnLifecycleState::InterruptRequested.requires_recovery());
        assert!(!TurnLifecycleState::Interrupted.requires_recovery());
        assert!(!TurnLifecycleState::Completed.requires_recovery());
        assert!(!TurnLifecycleState::Failed.requires_recovery());
        assert!(!TurnLifecycleState::RecoveryNeeded.requires_recovery());
    }

    #[test]
    fn corruption_marker_is_reflected_without_exposing_raw_record_data() {
        let snapshot =
            turn_lifecycle_snapshot(&[turn_lifecycle_corruption_marker("thread-1".to_string(), 2)]);

        assert!(snapshot.records.is_empty());
        assert_eq!(snapshot.corrupt_record_count, 2);
    }
}
