use roder_api::events::{RoadmapChanged, RoderEvent};
use roder_roadmap::{
    Document, DocumentSummary, RoadmapEvent, RoadmapEventKind, ThreadAttachment, ValidationResult,
};

use crate::Runtime;

impl Runtime {
    pub async fn list_roadmaps(&self) -> anyhow::Result<Vec<DocumentSummary>> {
        self.roadmaps.lock().await.list_roadmaps()
    }

    pub async fn open_roadmap(&self, path: &str) -> anyhow::Result<Document> {
        let (document, events) = {
            let mut roadmaps = self.roadmaps.lock().await;
            let start = roadmaps.events().len();
            let document = roadmaps.open_roadmap(path)?;
            let events = roadmaps.events()[start..].to_vec();
            (document, events)
        };
        self.publish_roadmap_events(events).await;
        Ok(document)
    }

    pub async fn focus_roadmap_task(&self, path: &str, task_id: &str) -> anyhow::Result<()> {
        let events = {
            let mut roadmaps = self.roadmaps.lock().await;
            let start = roadmaps.events().len();
            roadmaps.focus_roadmap_task(path, task_id)?;
            roadmaps.events()[start..].to_vec()
        };
        self.publish_roadmap_events(events).await;
        Ok(())
    }

    pub async fn set_roadmap_task(
        &self,
        path: &str,
        task_id: &str,
        checked: bool,
        evidence: &str,
    ) -> anyhow::Result<()> {
        let events = {
            let mut roadmaps = self.roadmaps.lock().await;
            let start = roadmaps.events().len();
            roadmaps.set_roadmap_task(path, task_id, checked, evidence)?;
            roadmaps.events()[start..].to_vec()
        };
        self.publish_roadmap_events(events).await;
        Ok(())
    }

    pub async fn validate_roadmap(&self, path: &str) -> anyhow::Result<ValidationResult> {
        let (result, events) = {
            let mut roadmaps = self.roadmaps.lock().await;
            let start = roadmaps.events().len();
            let result = roadmaps.validate_roadmap(path)?;
            let events = roadmaps.events()[start..].to_vec();
            (result, events)
        };
        self.publish_roadmap_events(events).await;
        Ok(result)
    }

    pub async fn list_roadmap_threads(&self, path: &str) -> anyhow::Result<Vec<ThreadAttachment>> {
        self.roadmaps.lock().await.list_roadmap_threads(path)
    }

    pub async fn spawn_roadmap_thread(
        &self,
        path: &str,
        task_id: &str,
    ) -> anyhow::Result<ThreadAttachment> {
        let (attachment, events) = {
            let mut roadmaps = self.roadmaps.lock().await;
            let start = roadmaps.events().len();
            let attachment = roadmaps.spawn_roadmap_thread(path, task_id)?;
            let events = roadmaps.events()[start..].to_vec();
            (attachment, events)
        };
        self.publish_roadmap_events(events).await;
        Ok(attachment)
    }

    pub async fn attach_roadmap_thread(
        &self,
        path: &str,
        task_id: &str,
        thread_id: &str,
        title: Option<String>,
    ) -> anyhow::Result<ThreadAttachment> {
        let (attachment, events) = {
            let mut roadmaps = self.roadmaps.lock().await;
            let start = roadmaps.events().len();
            let attachment = roadmaps.attach_roadmap_thread(path, task_id, thread_id, title)?;
            let events = roadmaps.events()[start..].to_vec();
            (attachment, events)
        };
        self.publish_roadmap_events(events).await;
        Ok(attachment)
    }

    pub async fn enter_roadmap_mode(&self, path: &str) -> anyhow::Result<()> {
        let events = {
            let mut roadmaps = self.roadmaps.lock().await;
            let start = roadmaps.events().len();
            roadmaps.record_mode_changed(path)?;
            roadmaps.events()[start..].to_vec()
        };
        self.publish_roadmap_events(events).await;
        Ok(())
    }

    async fn publish_roadmap_events(&self, events: Vec<RoadmapEvent>) {
        for event in events {
            self.emit(RoderEvent::RoadmapChanged(RoadmapChanged {
                event_kind: roadmap_event_kind(event.kind).to_string(),
                path: event.path.display().to_string(),
                task_id: event.task_id,
                thread_id: event.thread_id,
                timestamp: event.timestamp,
            }))
            .await;
        }
    }
}

fn roadmap_event_kind(kind: RoadmapEventKind) -> &'static str {
    match kind {
        RoadmapEventKind::Opened => "opened",
        RoadmapEventKind::Updated => "updated",
        RoadmapEventKind::TaskFocused => "task_focused",
        RoadmapEventKind::TaskChecked => "task_checked",
        RoadmapEventKind::ThreadAttached => "thread_attached",
        RoadmapEventKind::ThreadSpawned => "thread_spawned",
        RoadmapEventKind::Validated => "validated",
        RoadmapEventKind::ModeChanged => "mode_changed",
    }
}
