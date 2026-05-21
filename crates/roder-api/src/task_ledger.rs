use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskLedgerStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

impl TaskLedgerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskLedgerItem {
    pub id: String,
    pub content: String,
    pub status: TaskLedgerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

impl TaskLedgerItem {
    pub fn validate(&self, require_completion_evidence: bool) -> Result<(), TaskLedgerError> {
        validate_nonempty(&self.id, "id")?;
        validate_nonempty(&self.content, "content")?;
        if matches!(self.status, TaskLedgerStatus::Completed)
            && require_completion_evidence
            && self
                .evidence
                .as_deref()
                .is_none_or(|evidence| evidence.trim().is_empty())
        {
            return Err(TaskLedgerError::MissingEvidence {
                id: self.id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskLedgerSnapshot {
    pub tasks: Vec<TaskLedgerItem>,
}

impl TaskLedgerSnapshot {
    pub fn validate(&self, require_completion_evidence: bool) -> Result<(), TaskLedgerError> {
        let mut ids = std::collections::HashSet::new();
        let mut in_progress = 0usize;
        for task in &self.tasks {
            task.validate(require_completion_evidence)?;
            if !ids.insert(task.id.clone()) {
                return Err(TaskLedgerError::DuplicateId {
                    id: task.id.clone(),
                });
            }
            if matches!(task.status, TaskLedgerStatus::InProgress) {
                in_progress += 1;
            }
        }
        if in_progress > 1 {
            return Err(TaskLedgerError::MultipleInProgress);
        }
        Ok(())
    }

    pub fn completed_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| matches!(task.status, TaskLedgerStatus::Completed))
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskLedgerError {
    EmptyField { field: &'static str },
    DuplicateId { id: String },
    MultipleInProgress,
    MissingEvidence { id: String },
}

impl std::fmt::Display for TaskLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField { field } => write!(f, "task ledger {field} must not be empty"),
            Self::DuplicateId { id } => write!(f, "duplicate task ledger id {id:?}"),
            Self::MultipleInProgress => {
                write!(f, "task ledger accepts at most one in_progress task")
            }
            Self::MissingEvidence { id } => {
                write!(f, "completed task ledger item {id:?} requires evidence")
            }
        }
    }
}

impl std::error::Error for TaskLedgerError {}

fn validate_nonempty(value: &str, field: &'static str) -> Result<(), TaskLedgerError> {
    if value.trim().is_empty() {
        Err(TaskLedgerError::EmptyField { field })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_ledger_rejects_duplicate_ids_and_multiple_in_progress() {
        let snapshot = TaskLedgerSnapshot {
            tasks: vec![
                TaskLedgerItem {
                    id: "a".to_string(),
                    content: "First".to_string(),
                    status: TaskLedgerStatus::InProgress,
                    evidence: None,
                },
                TaskLedgerItem {
                    id: "a".to_string(),
                    content: "Second".to_string(),
                    status: TaskLedgerStatus::InProgress,
                    evidence: None,
                },
            ],
        };

        assert_eq!(
            snapshot.validate(false).unwrap_err(),
            TaskLedgerError::DuplicateId {
                id: "a".to_string()
            }
        );

        let snapshot = TaskLedgerSnapshot {
            tasks: vec![
                TaskLedgerItem {
                    id: "a".to_string(),
                    content: "First".to_string(),
                    status: TaskLedgerStatus::InProgress,
                    evidence: None,
                },
                TaskLedgerItem {
                    id: "b".to_string(),
                    content: "Second".to_string(),
                    status: TaskLedgerStatus::InProgress,
                    evidence: None,
                },
            ],
        };

        assert_eq!(
            snapshot.validate(false).unwrap_err(),
            TaskLedgerError::MultipleInProgress
        );
    }

    #[test]
    fn task_ledger_can_require_completion_evidence() {
        let snapshot = TaskLedgerSnapshot {
            tasks: vec![TaskLedgerItem {
                id: "done".to_string(),
                content: "Verify".to_string(),
                status: TaskLedgerStatus::Completed,
                evidence: None,
            }],
        };

        assert_eq!(
            snapshot.validate(true).unwrap_err(),
            TaskLedgerError::MissingEvidence {
                id: "done".to_string()
            }
        );
    }
}
