use std::path::{Path, PathBuf};

use time::OffsetDateTime;

use crate::{
    Document, DocumentSummary, ListOptions, RoadmapState, RoadmapStateStore, ThreadAttachment,
    ValidationResult, list_documents, parse_document, set_task_checked, validate_document,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadmapEventKind {
    Opened,
    Updated,
    TaskFocused,
    TaskChecked,
    ThreadAttached,
    ThreadSpawned,
    Validated,
    ModeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoadmapEvent {
    pub kind: RoadmapEventKind,
    pub path: PathBuf,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct RoadmapRuntime {
    workspace: PathBuf,
    store: RoadmapStateStore,
    events: Vec<RoadmapEvent>,
}

impl RoadmapRuntime {
    pub fn new(workspace: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            store: RoadmapStateStore::new(data_dir),
            events: Vec::new(),
        }
    }

    pub fn events(&self) -> &[RoadmapEvent] {
        &self.events
    }

    pub fn list_roadmaps(&self) -> anyhow::Result<Vec<DocumentSummary>> {
        list_documents(&self.workspace, ListOptions::default())
    }

    pub fn open_roadmap(&mut self, path: impl AsRef<Path>) -> anyhow::Result<Document> {
        let path = self.resolve_roadmap_path(path.as_ref())?;
        let document = self.read_document(&path)?;
        let now = OffsetDateTime::now_utc();
        let mut state = self.state_for(&document)?;
        state.document_id = document.id.clone();
        state.path = document.path.clone();
        if state.focused_task_id.is_none() {
            state.focused_task_id = document.tasks.first().map(|task| task.id.clone());
        }
        state.updated_at = now;
        self.save_state(state)?;
        self.emit(RoadmapEventKind::Opened, &document.path, None, None);
        Ok(document)
    }

    pub fn focus_roadmap_task(
        &mut self,
        path: impl AsRef<Path>,
        task_id: &str,
    ) -> anyhow::Result<()> {
        let path = self.resolve_roadmap_path(path.as_ref())?;
        let document = self.read_document(&path)?;
        ensure_task(&document, task_id)?;
        let mut state = self.state_for(&document)?;
        state.focused_task_id = Some(task_id.to_string());
        state.updated_at = OffsetDateTime::now_utc();
        self.save_state(state)?;
        self.emit(
            RoadmapEventKind::TaskFocused,
            &document.path,
            Some(task_id.to_string()),
            None,
        );
        Ok(())
    }

    pub fn set_roadmap_task(
        &mut self,
        path: impl AsRef<Path>,
        task_id: &str,
        checked: bool,
        evidence: &str,
    ) -> anyhow::Result<()> {
        let path = self.resolve_roadmap_path(path.as_ref())?;
        set_task_checked(&path, task_id, checked, evidence)?;
        let document = self.read_document(&path)?;
        let mut state = self.state_for(&document)?;
        state.focused_task_id = Some(task_id.to_string());
        state.updated_at = OffsetDateTime::now_utc();
        self.save_state(state)?;
        self.emit(
            if checked {
                RoadmapEventKind::TaskChecked
            } else {
                RoadmapEventKind::Updated
            },
            &document.path,
            Some(task_id.to_string()),
            None,
        );
        Ok(())
    }

    pub fn validate_roadmap(&mut self, path: impl AsRef<Path>) -> anyhow::Result<ValidationResult> {
        let path = self.resolve_roadmap_path(path.as_ref())?;
        let document = self.read_document(&path)?;
        let result = validate_document(&document);
        let mut state = self.state_for(&document)?;
        state.last_validation = Some(OffsetDateTime::now_utc());
        state.last_diagnostics = result.diagnostics.clone();
        state.updated_at = OffsetDateTime::now_utc();
        self.save_state(state)?;
        self.emit(RoadmapEventKind::Validated, &document.path, None, None);
        Ok(result)
    }

    pub fn list_roadmap_threads(
        &self,
        path: impl AsRef<Path>,
    ) -> anyhow::Result<Vec<ThreadAttachment>> {
        let path = self.resolve_roadmap_path(path.as_ref())?;
        let state = self.store.load()?.filter(|state| state.path == path);
        Ok(state.map(|state| state.threads).unwrap_or_default())
    }

    pub fn record_mode_changed(&mut self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = self.resolve_roadmap_path(path.as_ref())?;
        let document = self.read_document(&path)?;
        let mut state = self.state_for(&document)?;
        state.updated_at = OffsetDateTime::now_utc();
        self.save_state(state)?;
        self.emit(RoadmapEventKind::ModeChanged, &document.path, None, None);
        Ok(())
    }

    pub fn spawn_roadmap_thread(
        &mut self,
        path: impl AsRef<Path>,
        task_id: &str,
    ) -> anyhow::Result<ThreadAttachment> {
        let thread_id = format!("thread-{}", uuid::Uuid::new_v4());
        self.attach_roadmap_thread(
            path,
            task_id,
            &thread_id,
            Some("Roadmap worker".to_string()),
        )?;
        let attachment = self
            .store
            .load()?
            .and_then(|state| {
                state
                    .threads
                    .into_iter()
                    .find(|thread| thread.thread_id == thread_id)
            })
            .ok_or_else(|| anyhow::anyhow!("spawned thread attachment not found"))?;
        self.emit(
            RoadmapEventKind::ThreadSpawned,
            &attachment_path(&self.store)?,
            Some(task_id.to_string()),
            Some(thread_id),
        );
        Ok(attachment)
    }

    pub fn attach_roadmap_thread(
        &mut self,
        path: impl AsRef<Path>,
        task_id: &str,
        thread_id: &str,
        title: Option<String>,
    ) -> anyhow::Result<ThreadAttachment> {
        let path = self.resolve_roadmap_path(path.as_ref())?;
        let document = self.read_document(&path)?;
        ensure_task(&document, task_id)?;
        let mut state = self.state_for(&document)?;
        let now = OffsetDateTime::now_utc();
        let attachment = ThreadAttachment {
            thread_id: thread_id.to_string(),
            task_id: Some(task_id.to_string()),
            title,
            status: Some("attached".to_string()),
            created_at: now,
            updated_at: now,
        };
        state.attached_thread_id = Some(thread_id.to_string());
        state.threads.push(attachment.clone());
        state.updated_at = now;
        self.save_state(state)?;
        self.emit(
            RoadmapEventKind::ThreadAttached,
            &document.path,
            Some(task_id.to_string()),
            Some(thread_id.to_string()),
        );
        Ok(attachment)
    }

    fn read_document(&self, path: &Path) -> anyhow::Result<Document> {
        let content = std::fs::read_to_string(path)?;
        Ok(parse_document(path, &content))
    }

    fn state_for(&self, document: &Document) -> anyhow::Result<RoadmapState> {
        if let Some(state) = self.store.load()?
            && state.path == document.path
        {
            return Ok(state);
        }
        Ok(RoadmapState {
            document_id: document.id.clone(),
            path: document.path.clone(),
            focused_task_id: None,
            primary_thread_id: None,
            attached_thread_id: None,
            threads: Vec::new(),
            last_validation: None,
            last_diagnostics: Vec::new(),
            updated_at: OffsetDateTime::now_utc(),
        })
    }

    fn save_state(&self, state: RoadmapState) -> anyhow::Result<()> {
        self.store.save(&state)
    }

    fn resolve_roadmap_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace.join(path)
        };
        let roadmap_dir = self.workspace.join("roadmap");
        if candidate
            .parent()
            .map(|parent| parent == roadmap_dir)
            .unwrap_or(false)
            && candidate.extension().and_then(|ext| ext.to_str()) == Some("md")
        {
            Ok(candidate)
        } else {
            anyhow::bail!("roadmap path must be under {}", roadmap_dir.display())
        }
    }

    fn emit(
        &mut self,
        kind: RoadmapEventKind,
        path: &Path,
        task_id: Option<String>,
        thread_id: Option<String>,
    ) {
        self.events.push(RoadmapEvent {
            kind,
            path: path.to_path_buf(),
            task_id,
            thread_id,
            timestamp: OffsetDateTime::now_utc(),
        });
    }
}

fn ensure_task(document: &Document, task_id: &str) -> anyhow::Result<()> {
    if document.tasks.iter().any(|task| task.id == task_id) {
        Ok(())
    } else {
        anyhow::bail!("task not found: {task_id}")
    }
}

fn attachment_path(store: &RoadmapStateStore) -> anyhow::Result<PathBuf> {
    store
        .load()?
        .map(|state| state.path)
        .ok_or_else(|| anyhow::anyhow!("roadmap state missing"))
}
