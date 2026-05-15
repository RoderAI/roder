use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};
use crate::extension::NotificationSinkId;
use crate::tasks::TaskId;

pub type NotificationId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    NeedsInput,
    TurnIdle,
    TaskCompleted,
    TaskFailed,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Notification {
    pub id: NotificationId,
    pub kind: NotificationKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[async_trait::async_trait]
pub trait NotificationSink: Send + Sync + 'static {
    fn id(&self) -> NotificationSinkId;

    async fn deliver(&self, notification: Notification) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct CapturingSink {
        delivered: Arc<Mutex<Vec<Notification>>>,
    }

    #[async_trait::async_trait]
    impl NotificationSink for CapturingSink {
        fn id(&self) -> NotificationSinkId {
            "capture".to_string()
        }

        async fn deliver(&self, notification: Notification) -> anyhow::Result<()> {
            self.delivered.lock().unwrap().push(notification);
            Ok(())
        }
    }

    #[test]
    fn notification_round_trips_json() {
        let notification = Notification {
            id: "notice-1".to_string(),
            kind: NotificationKind::TaskCompleted,
            title: "Task completed".to_string(),
            body: Some("process finished".to_string()),
            task_id: Some("task-1".to_string()),
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
            metadata: serde_json::json!({ "sink": "test" }),
        };

        let encoded = serde_json::to_value(&notification).expect("serialize notification");
        assert_eq!(encoded["kind"], "task_completed");

        let decoded: Notification =
            serde_json::from_value(encoded).expect("deserialize notification");
        assert_eq!(decoded, notification);
    }

    #[tokio::test]
    async fn notification_sink_trait_is_object_safe() {
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn NotificationSink> = Arc::new(CapturingSink {
            delivered: Arc::clone(&delivered),
        });

        sink.deliver(Notification {
            id: "notice-1".to_string(),
            kind: NotificationKind::NeedsInput,
            title: "Approval needed".to_string(),
            body: None,
            task_id: None,
            thread_id: Some("thread-a".to_string()),
            turn_id: Some("turn-a".to_string()),
            timestamp: OffsetDateTime::UNIX_EPOCH,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

        assert_eq!(sink.id(), "capture");
        assert_eq!(delivered.lock().unwrap().len(), 1);
    }
}
