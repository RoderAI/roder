use std::sync::Arc;

use roder_api::{
    CapabilityRequest, ExtensionManifest, ExtensionRegistryBuilder, Notification, NotificationKind,
    NotificationSink, ProvidedService, RoderExtension,
};

pub const DESKTOP_NOTIFICATION_SINK_ID: &str = "desktop";

#[derive(Clone)]
pub struct DesktopNotificationSink {
    enabled_kinds: Vec<NotificationKind>,
}

impl DesktopNotificationSink {
    pub fn new(enabled_kinds: Vec<NotificationKind>) -> Self {
        Self { enabled_kinds }
    }

    pub fn default_kinds() -> Vec<NotificationKind> {
        vec![
            NotificationKind::NeedsInput,
            NotificationKind::TurnIdle,
            NotificationKind::TaskCompleted,
            NotificationKind::TaskFailed,
        ]
    }

    fn kind_enabled(&self, kind: &NotificationKind) -> bool {
        self.enabled_kinds.iter().any(|enabled| enabled == kind)
    }
}

#[async_trait::async_trait]
impl NotificationSink for DesktopNotificationSink {
    fn id(&self) -> String {
        DESKTOP_NOTIFICATION_SINK_ID.to_string()
    }

    async fn deliver(&self, notification: Notification) -> anyhow::Result<()> {
        if !self.kind_enabled(&notification.kind) {
            return Ok(());
        }
        deliver_desktop(notification);
        Ok(())
    }
}

#[cfg(feature = "desktop")]
fn deliver_desktop(notification: Notification) {
    let mut desktop = notify_rust::Notification::new();
    desktop.summary(&notification.title);
    if let Some(body) = notification.body {
        desktop.body(&body);
    }
    let _ = desktop.show();
}

#[cfg(not(feature = "desktop"))]
fn deliver_desktop(_notification: Notification) {}

pub struct DesktopNotifyExtension {
    enabled_kinds: Vec<NotificationKind>,
}

impl DesktopNotifyExtension {
    pub fn new(enabled_kinds: Vec<NotificationKind>) -> Self {
        Self { enabled_kinds }
    }
}

impl Default for DesktopNotifyExtension {
    fn default() -> Self {
        Self::new(DesktopNotificationSink::default_kinds())
    }
}

impl RoderExtension for DesktopNotifyExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-notify-desktop".to_string(),
            name: "Desktop Notification Sink".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Best-effort desktop notification sink".to_string()),
            provides: vec![ProvidedService::NotificationSink(
                DESKTOP_NOTIFICATION_SINK_ID.to_string(),
            )],
            required_capabilities: vec![CapabilityRequest::new("desktop.notification")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.notification_sink(Arc::new(DesktopNotificationSink::new(
            self.enabled_kinds.clone(),
        )));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;

    fn notification(kind: NotificationKind) -> Notification {
        Notification {
            id: "notice-1".to_string(),
            kind,
            title: "notice".to_string(),
            body: Some("body".to_string()),
            task_id: None,
            thread_id: None,
            turn_id: None,
            timestamp: OffsetDateTime::UNIX_EPOCH,
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn desktop_sink_noops_without_platform_feature() {
        let sink = DesktopNotificationSink::new(vec![NotificationKind::NeedsInput]);

        sink.deliver(notification(NotificationKind::NeedsInput))
            .await
            .unwrap();
        sink.deliver(notification(NotificationKind::TurnIdle))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn live_desktop_notification_is_explicitly_opt_in() {
        if std::env::var("RODER_LIVE_NOTIFICATIONS").ok().as_deref() != Some("1") {
            return;
        }

        let sink = DesktopNotificationSink::new(vec![NotificationKind::NeedsInput]);
        sink.deliver(notification(NotificationKind::NeedsInput))
            .await
            .unwrap();
    }
}
