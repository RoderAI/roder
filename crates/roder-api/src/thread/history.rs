use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::{ThreadItem, ThreadMetadata};
use crate::events::TurnId;
use crate::inference::TokenUsage;

/// Exclusive older-page cursor and hard selection budgets.
#[derive(Debug, Clone)]
pub struct ThreadHistoryReadOptions {
    pub before: Option<String>,
    pub limit: usize,
    pub max_bytes: usize,
}

impl Default for ThreadHistoryReadOptions {
    fn default() -> Self {
        Self {
            before: None,
            limit: 160,
            max_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadHistoryPage {
    pub metadata: ThreadMetadata,
    pub turns: Vec<ThreadHistoryTurn>,
    pub before: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadHistoryTurnStatus {
    InProgress,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadHistoryTurn {
    pub turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
    pub status: ThreadHistoryTurnStatus,
    pub error: Option<String>,
    pub usage: Option<TokenUsage>,
    pub finish_reason: Option<String>,
    pub items: Vec<ThreadItem>,
}
