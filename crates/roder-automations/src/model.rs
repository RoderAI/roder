use roder_api::automations::{
    AutomationDefinition, AutomationId, AutomationOccurrenceKey, AutomationRunSummary,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledOccurrence {
    pub automation_id: AutomationId,
    pub occurrence_key: AutomationOccurrenceKey,
    #[serde(with = "time::serde::rfc3339")]
    pub scheduled_for: OffsetDateTime,
    pub action: OccurrenceAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OccurrenceAction {
    Run,
    Skip { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredAutomation {
    pub definition: AutomationDefinition,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_checked_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunLogEntry {
    pub run_id: String,
    pub stream: String,
    pub chunk: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

pub type AutomationRunRecord = AutomationRunSummary;

pub fn occurrence_key(automation_id: &str, scheduled_for: OffsetDateTime) -> String {
    format!("{}:{}", automation_id, scheduled_for.unix_timestamp())
}
