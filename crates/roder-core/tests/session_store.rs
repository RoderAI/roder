//! Runtime session-store contract tests (roadmap phase 63): a non-filesystem
//! session store (like the PostgreSQL store) must provide context-artifact
//! storage through `ThreadStore::context_artifact_store()`, and the runtime
//! must route artifacts through it instead of silently falling back to the
//! legacy local filesystem directory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use roder_api::artifacts::{
    ArtifactGrepPage, ArtifactReadPage, ArtifactTailPage, ContextArtifact, ContextArtifactAccess,
    ContextArtifactKind, ContextArtifactStore, CreateArtifactRequest,
};
use roder_api::events::{EventEnvelope, ThreadId};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::thread::{ThreadMetadata, ThreadSnapshot, ThreadStore, ThreadStoreFactory};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};

#[derive(Default)]
struct InMemoryArtifactState {
    artifacts: HashMap<(String, String), (ContextArtifact, Vec<u8>)>,
    next_id: u64,
}

#[derive(Clone, Default)]
struct InMemoryArtifactAccess {
    state: Arc<Mutex<InMemoryArtifactState>>,
}

impl InMemoryArtifactAccess {
    fn artifact_count(&self) -> usize {
        self.state.lock().unwrap().artifacts.len()
    }
}

impl ContextArtifactAccess for InMemoryArtifactAccess {
    fn create_artifact(
        &self,
        request: CreateArtifactRequest<'_>,
    ) -> anyhow::Result<ContextArtifact> {
        let mut state = self.state.lock().unwrap();
        state.next_id += 1;
        let id = format!("mem-artifact-{}", state.next_id);
        let text = String::from_utf8_lossy(request.bytes);
        let artifact = ContextArtifact {
            id: id.clone(),
            kind: request.kind,
            thread_id: request.thread_id.clone(),
            turn_id: request.turn_id.clone(),
            source_tool_id: request.source_tool_id.map(str::to_string),
            label: request.label.map(str::to_string),
            byte_count: request.bytes.len() as u64,
            line_count: text.lines().count() as u64,
            store_path: format!("memory://{id}"),
            retention_expires_at: None,
            created_at: time::OffsetDateTime::now_utc(),
            roder_owned: true,
        };
        state.artifacts.insert(
            (request.thread_id.clone(), id),
            (artifact.clone(), request.bytes.to_vec()),
        );
        Ok(artifact)
    }

    fn append_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &String,
        bytes: &[u8],
    ) -> anyhow::Result<ContextArtifact> {
        let mut state = self.state.lock().unwrap();
        let entry = state
            .artifacts
            .get_mut(&(thread_id.clone(), artifact_id.to_string()))
            .ok_or_else(|| anyhow::anyhow!("unknown artifact {artifact_id}"))?;
        entry.1.extend_from_slice(bytes);
        entry.0.byte_count = entry.1.len() as u64;
        entry.0.line_count = String::from_utf8_lossy(&entry.1).lines().count() as u64;
        Ok(entry.0.clone())
    }

    fn list_artifacts(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<ContextArtifact>> {
        let state = self.state.lock().unwrap();
        let mut artifacts: Vec<ContextArtifact> = state
            .artifacts
            .iter()
            .filter(|((thread, _), _)| thread == thread_id)
            .map(|(_, (artifact, _))| artifact.clone())
            .collect();
        artifacts.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(artifacts)
    }

    fn read_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &String,
        start_line: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactReadPage> {
        let state = self.state.lock().unwrap();
        let (artifact, bytes) = state
            .artifacts
            .get(&(thread_id.clone(), artifact_id.to_string()))
            .ok_or_else(|| anyhow::anyhow!("unknown artifact {artifact_id}"))?;
        let text = String::from_utf8_lossy(bytes);
        let lines: Vec<&str> = text.lines().collect();
        let start = start_line.saturating_sub(1).min(lines.len());
        let shown: Vec<&str> = lines.iter().skip(start).take(limit).copied().collect();
        Ok(ArtifactReadPage {
            artifact: artifact.descriptor(),
            text: shown.join("\n"),
            start_line,
            limit,
            shown: shown.len(),
            total_lines: lines.len(),
            next_start_line: (start + shown.len() < lines.len())
                .then(|| start + shown.len() + 1),
            truncated: start + shown.len() < lines.len(),
        })
    }

    fn grep_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &String,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactGrepPage> {
        let page = self.read_artifact(thread_id, artifact_id, 1, usize::MAX)?;
        let matches: Vec<&str> = page
            .text
            .lines()
            .filter(|line| line.contains(query))
            .collect();
        let shown: Vec<&str> = matches.iter().skip(offset).take(limit).copied().collect();
        Ok(ArtifactGrepPage {
            artifact: page.artifact,
            query: query.to_string(),
            text: shown.join("\n"),
            offset,
            limit,
            shown: shown.len(),
            total_matches: matches.len(),
            next_offset: (offset + shown.len() < matches.len()).then(|| offset + shown.len()),
            truncated: offset + shown.len() < matches.len(),
        })
    }

    fn tail_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &String,
        lines: usize,
    ) -> anyhow::Result<ArtifactTailPage> {
        let page = self.read_artifact(thread_id, artifact_id, 1, usize::MAX)?;
        let all: Vec<&str> = page.text.lines().collect();
        let start = all.len().saturating_sub(lines);
        let shown: Vec<&str> = all[start..].to_vec();
        Ok(ArtifactTailPage {
            artifact: page.artifact,
            text: shown.join("\n"),
            start_line: start + 1,
            lines,
            shown: shown.len(),
            total_lines: all.len(),
            truncated: start > 0,
        })
    }

    fn delete_artifact(&self, thread_id: &ThreadId, artifact_id: &String) -> anyhow::Result<bool> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .artifacts
            .remove(&(thread_id.clone(), artifact_id.to_string()))
            .is_some())
    }
}

/// A non-filesystem session store shaped like the PostgreSQL store: no local
/// thread root, with database-style context-artifact storage.
struct NonFilesystemThreadStore {
    artifacts: InMemoryArtifactAccess,
    threads: Arc<Mutex<HashMap<String, ThreadSnapshot>>>,
}

#[async_trait::async_trait]
impl ThreadStore for NonFilesystemThreadStore {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "postgres-like".to_string()
    }

    fn context_artifact_store(&self) -> Option<ContextArtifactStore> {
        Some(ContextArtifactStore::new(Arc::new(self.artifacts.clone())))
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        self.threads.lock().unwrap().insert(
            metadata.thread_id.clone(),
            ThreadSnapshot {
                metadata: Some(metadata.clone()),
                events: Vec::new(),
                turns: Vec::new(),
                item_events: Vec::new(),
                extension_states: Vec::new(),
            },
        );
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        Ok(self
            .threads
            .lock()
            .unwrap()
            .values()
            .filter_map(|snapshot| snapshot.metadata.clone())
            .collect())
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        Ok(self.threads.lock().unwrap().get(thread_id).cloned())
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        if let Some(snapshot) = self.threads.lock().unwrap().get_mut(thread_id) {
            snapshot.events.push(envelope.clone());
        }
        Ok(())
    }
}

struct NonFilesystemThreadStoreFactory {
    artifacts: InMemoryArtifactAccess,
}

impl ThreadStoreFactory for NonFilesystemThreadStoreFactory {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "postgres-like".to_string()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        Arc::new(NonFilesystemThreadStore {
            artifacts: self.artifacts.clone(),
            threads: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_routes_context_artifacts_through_non_filesystem_session_store() {
    let artifacts = InMemoryArtifactAccess::default();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(NonFilesystemThreadStoreFactory {
        artifacts: artifacts.clone(),
    }));
    let runtime =
        Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();

    let store = runtime.context_artifacts();
    let thread_id = "thread-pg-like".to_string();
    let turn_id = "turn-1".to_string();
    let created = store
        .create(roder_api::artifacts::CreateArtifactRequest {
            kind: ContextArtifactKind::CommandStdout,
            thread_id: &thread_id,
            turn_id: &turn_id,
            source_tool_id: Some("shell"),
            label: Some("command output"),
            bytes: b"line one\nline two with TOKEN\nline three",
        })
        .expect("create artifact through runtime store");

    // The artifact landed in the store-provided backend, proving the runtime
    // did not fall back to the legacy local filesystem directory.
    assert_eq!(artifacts.artifact_count(), 1);

    let page = store
        .read_artifact(&thread_id, &created.id, 1, 10)
        .expect("read artifact back");
    assert_eq!(page.total_lines, 3);
    assert!(page.text.contains("line two with TOKEN"));

    let grep = store
        .grep_artifact(&thread_id, &created.id, "TOKEN", 0, 10)
        .expect("grep artifact");
    assert_eq!(grep.total_matches, 1);

    // Artifact access stays scoped by thread id, mirroring the tenant/session
    // scoping rule of the PostgreSQL store.
    let other_thread = "thread-other".to_string();
    assert!(store.list_artifacts(&other_thread).unwrap().is_empty());
    assert!(store.read_artifact(&other_thread, &created.id, 1, 10).is_err());
}
