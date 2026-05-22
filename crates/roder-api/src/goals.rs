use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::ThreadId;

pub const MAX_THREAD_GOAL_OBJECTIVE_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadGoalStatus {
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
}

impl ThreadGoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::UsageLimited => "usageLimited",
            Self::BudgetLimited => "budgetLimited",
            Self::Complete => "complete",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadGoal {
    pub thread_id: ThreadId,
    pub objective: String,
    pub status: ThreadGoalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<i64>,
    #[serde(default)]
    pub tokens_used: i64,
    #[serde(default)]
    pub time_used_seconds: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGoalPatch {
    pub objective: Option<String>,
    pub status: Option<ThreadGoalStatus>,
    pub token_budget: Option<Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGoalUpdated {
    pub thread_id: ThreadId,
    pub goal: ThreadGoal,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGoalCleared {
    pub thread_id: ThreadId,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

pub fn validate_thread_goal_objective(objective: &str) -> anyhow::Result<()> {
    let objective = objective.trim();
    if objective.is_empty() {
        anyhow::bail!("goal objective cannot be empty");
    }
    let count = objective.chars().count();
    if count > MAX_THREAD_GOAL_OBJECTIVE_CHARS {
        anyhow::bail!("goal objective cannot exceed {MAX_THREAD_GOAL_OBJECTIVE_CHARS} characters");
    }
    Ok(())
}

pub fn validate_thread_goal_budget(token_budget: Option<i64>) -> anyhow::Result<()> {
    if let Some(token_budget) = token_budget
        && token_budget <= 0
    {
        anyhow::bail!("goal token budget must be positive");
    }
    Ok(())
}

#[async_trait::async_trait]
pub trait ThreadGoalController: Send + Sync + 'static {
    async fn get_thread_goal(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadGoal>>;

    async fn create_thread_goal(
        &self,
        thread_id: &ThreadId,
        objective: String,
        token_budget: Option<i64>,
    ) -> anyhow::Result<ThreadGoal>;

    async fn set_thread_goal(
        &self,
        thread_id: &ThreadId,
        patch: ThreadGoalPatch,
    ) -> anyhow::Result<Option<ThreadGoal>>;

    async fn clear_thread_goal(&self, thread_id: &ThreadId) -> anyhow::Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_thread_goal_objective() {
        assert!(validate_thread_goal_objective("ship it").is_ok());
        assert!(validate_thread_goal_objective("  ").is_err());
        assert!(
            validate_thread_goal_objective(&"a".repeat(MAX_THREAD_GOAL_OBJECTIVE_CHARS + 1))
                .is_err()
        );
    }

    #[test]
    fn validates_thread_goal_budget() {
        assert!(validate_thread_goal_budget(None).is_ok());
        assert!(validate_thread_goal_budget(Some(1)).is_ok());
        assert!(validate_thread_goal_budget(Some(0)).is_err());
    }
}
