use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use roder_api::{
    ExtensionManifest, ExtensionRegistryBuilder, Notification, NotificationKind, NotificationSink,
    ProvidedService, RoderExtension,
};
use tokio::sync::Mutex;

pub const TERMINAL_BELL_SINK_ID: &str = "terminal-bell";

pub trait TerminalBellWriter: Send + Sync + 'static {
    fn write_bell(&self) -> anyhow::Result<()>;
}

pub struct StderrBellWriter;

impl TerminalBellWriter for StderrBellWriter {
    fn write_bell(&self) -> anyhow::Result<()> {
        std::io::stderr().write_all(b"\x07")?;
        std::io::stderr().flush()?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct TerminalBellSink {
    writer: Arc<dyn TerminalBellWriter>,
    enabled_kinds: Vec<NotificationKind>,
    throttle: Duration,
    last_delivered: Arc<Mutex<Option<Instant>>>,
}

impl TerminalBellSink {
    pub fn new(writer: Arc<dyn TerminalBellWriter>, enabled_kinds: Vec<NotificationKind>) -> Self {
        Self {
            writer,
            enabled_kinds,
            throttle: Duration::from_secs(2),
            last_delivered: Arc::new(Mutex::new(None)),
        }
    }

    pub fn default_kinds() -> Vec<NotificationKind> {
        vec![
            NotificationKind::NeedsInput,
            NotificationKind::TaskCompleted,
            NotificationKind::TaskFailed,
        ]
    }

    pub fn with_throttle(mut self, throttle: Duration) -> Self {
        self.throttle = throttle;
        self
    }

    fn kind_enabled(&self, kind: &NotificationKind) -> bool {
        self.enabled_kinds.iter().any(|enabled| enabled == kind)
    }
}

#[async_trait::async_trait]
impl NotificationSink for TerminalBellSink {
    fn id(&self) -> String {
        TERMINAL_BELL_SINK_ID.to_string()
    }

    async fn deliver(&self, notification: Notification) -> anyhow::Result<()> {
        if !self.kind_enabled(&notification.kind) {
            return Ok(());
        }
        let mut last_delivered = self.last_delivered.lock().await;
        if last_delivered.is_some_and(|last| last.elapsed() < self.throttle) {
            return Ok(());
        }
        self.writer.write_bell()?;
        *last_delivered = Some(Instant::now());
        Ok(())
    }
}

#[derive(Default, Clone)]
pub struct CapturedNotificationSink {
    notifications: Arc<Mutex<Vec<Notification>>>,
}

impl CapturedNotificationSink {
    pub async fn notifications(&self) -> Vec<Notification> {
        self.notifications.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl NotificationSink for CapturedNotificationSink {
    fn id(&self) -> String {
        "captured".to_string()
    }

    async fn deliver(&self, notification: Notification) -> anyhow::Result<()> {
        self.notifications.lock().await.push(notification);
        Ok(())
    }
}

pub struct TerminalNotifyExtension {
    enabled_kinds: Vec<NotificationKind>,
}

impl TerminalNotifyExtension {
    pub fn new(enabled_kinds: Vec<NotificationKind>) -> Self {
        Self { enabled_kinds }
    }
}

impl RoderExtension for TerminalNotifyExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-notify-terminal".to_string(),
            name: "Terminal Notification Sink".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Terminal bell notification sink".to_string()),
            provides: vec![ProvidedService::NotificationSink(
                TERMINAL_BELL_SINK_ID.to_string(),
            )],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.notification_sink(Arc::new(TerminalBellSink::new(
            Arc::new(StderrBellWriter),
            self.enabled_kinds.clone(),
        )));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use time::OffsetDateTime;

    use super::*;

    #[derive(Default)]
    struct CountingWriter {
        count: AtomicUsize,
    }

    impl TerminalBellWriter for CountingWriter {
        fn write_bell(&self) -> anyhow::Result<()> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn notification(kind: NotificationKind) -> Notification {
        Notification {
            id: "notice-1".to_string(),
            kind,
            title: "notice".to_string(),
            body: None,
            task_id: None,
            thread_id: None,
            turn_id: None,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn terminal_bell_filters_kinds() {
        let writer = Arc::new(CountingWriter::default());
        let sink = TerminalBellSink::new(writer.clone(), vec![NotificationKind::NeedsInput])
            .with_throttle(Duration::ZERO);

        sink.deliver(notification(NotificationKind::TurnIdle))
            .await
            .unwrap();
        sink.deliver(notification(NotificationKind::NeedsInput))
            .await
            .unwrap();

        assert_eq!(writer.count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn terminal_bell_throttles_delivery() {
        let writer = Arc::new(CountingWriter::default());
        let sink = TerminalBellSink::new(writer.clone(), vec![NotificationKind::TaskCompleted]);

        sink.deliver(notification(NotificationKind::TaskCompleted))
            .await
            .unwrap();
        sink.deliver(notification(NotificationKind::TaskCompleted))
            .await
            .unwrap();

        assert_eq!(writer.count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn captured_sink_records_notifications() {
        let sink = CapturedNotificationSink::default();
        sink.deliver(notification(NotificationKind::TaskFailed))
            .await
            .unwrap();

        let notifications = sink.notifications().await;
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].kind, NotificationKind::TaskFailed);
    }
}
