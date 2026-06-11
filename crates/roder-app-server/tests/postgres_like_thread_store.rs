//! App-server thread semantics against a PostgreSQL-like tenant-scoped
//! session store (roadmap phase 63, Task 4).
//!
//! The fake store mirrors the PostgreSQL store contract without a live
//! database: tenant id baked in at construction, all reads/writes keyed by
//! `(tenant_id, thread_id)`, archive as a state transition, and list results
//! excluding archived rows sorted by `updated_at` descending. Two app-server
//! instances for two tenants share one backing "database" so tenant
//! isolation is proven through public JSON-RPC request paths.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use roder_api::events::{EventEnvelope, ThreadId};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::thread::{ThreadMetadata, ThreadSnapshot, ThreadStore, ThreadStoreFactory};
use roder_app_server::{AppServer, AppServerFeatureConfig, LocalAppClient};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_protocol::{
    JsonRpcRequest, ThreadArchiveParams, ThreadArchiveResult, ThreadListParams, ThreadListResult,
    ThreadReadParams, ThreadReadResult, ThreadStartParams, ThreadStartResult, TurnStartParams,
    TurnStartResult, WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceRootInput,
};

#[derive(Default, Clone)]
struct SharedDatabase {
    rows: Arc<Mutex<BTreeMap<(String, String), DatabaseRow>>>,
}

#[derive(Clone)]
struct DatabaseRow {
    metadata: ThreadMetadata,
    archived: bool,
    events: Vec<EventEnvelope>,
}

struct PostgresLikeThreadStore {
    tenant_id: String,
    database: SharedDatabase,
}

impl PostgresLikeThreadStore {
    fn key(&self, thread_id: &str) -> (String, String) {
        (self.tenant_id.clone(), thread_id.to_string())
    }
}

#[async_trait::async_trait]
impl ThreadStore for PostgresLikeThreadStore {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "postgres-like".to_string()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        self.database.rows.lock().unwrap().insert(
            self.key(&metadata.thread_id),
            DatabaseRow {
                metadata: metadata.clone(),
                archived: false,
                events: Vec::new(),
            },
        );
        Ok(metadata)
    }

    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        let mut rows = self.database.rows.lock().unwrap();
        if let Some(row) = rows.get_mut(&self.key(&metadata.thread_id))
            && !row.archived
        {
            row.metadata = metadata.clone();
        }
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        let rows = self.database.rows.lock().unwrap();
        let mut threads: Vec<ThreadMetadata> = rows
            .iter()
            .filter(|((tenant, _), row)| tenant == &self.tenant_id && !row.archived)
            .map(|(_, row)| row.metadata.clone())
            .collect();
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
        Ok(threads)
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        let rows = self.database.rows.lock().unwrap();
        let Some(row) = rows.get(&self.key(thread_id)).filter(|row| !row.archived) else {
            return Ok(None);
        };
        let turns = roder_api::thread::project_turns_from_events(thread_id, &row.events);
        Ok(Some(ThreadSnapshot {
            metadata: Some(row.metadata.clone()),
            events: row.events.clone(),
            turns,
            item_events: Vec::new(),
            extension_states: Vec::new(),
        }))
    }

    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let mut rows = self.database.rows.lock().unwrap();
        match rows.get_mut(&self.key(thread_id)) {
            Some(row) if !row.archived => {
                row.archived = true;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        if let Some(row) = self
            .database
            .rows
            .lock()
            .unwrap()
            .get_mut(&self.key(thread_id))
        {
            row.events.push(envelope.clone());
        }
        Ok(())
    }
}

struct PostgresLikeThreadStoreFactory {
    tenant_id: String,
    database: SharedDatabase,
}

impl ThreadStoreFactory for PostgresLikeThreadStoreFactory {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "postgres-like".to_string()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        Arc::new(PostgresLikeThreadStore {
            tenant_id: self.tenant_id.clone(),
            database: self.database.clone(),
        })
    }
}

fn tenant_client(tenant_id: &str, database: &SharedDatabase) -> LocalAppClient {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(PostgresLikeThreadStoreFactory {
        tenant_id: tenant_id.to_string(),
        database: database.clone(),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let feature_config =
        AppServerFeatureConfig::default().with_workspace_registry_path(std::env::temp_dir().join(
            format!("roder-pg-like-workspaces-{tenant_id}-{}.json", uuid::Uuid::new_v4()),
        ));
    LocalAppClient::new(Arc::new(AppServer::with_feature_config(
        runtime,
        feature_config,
    )))
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> T {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params,
        })
        .await;
    assert!(
        response.error.is_none(),
        "RPC error for {method}: {:?}",
        response.error
    );
    serde_json::from_value(response.result.unwrap()).unwrap()
}

async fn start_thread(client: &LocalAppClient, workspace_dir: &std::path::Path) -> String {
    let workspace: WorkspaceCreateResult = request(
        client,
        "workspace/create",
        Some(
            serde_json::to_value(WorkspaceCreateParams {
                name: None,
                roots: vec![WorkspaceRootInput {
                    path: workspace_dir.display().to_string(),
                    name: None,
                }],
                default_root_path: Some(workspace_dir.display().to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    let started: ThreadStartResult = request(
        client,
        "thread/start",
        Some(
            serde_json::to_value(ThreadStartParams {
                selection: None,
                workspace_id: workspace.workspace.id.clone(),
                root_id: Some(workspace.workspace.default_root_id.clone()),
                model: Some("mock".to_string()),
                model_provider: None,
                reasoning: None,
                cwd: None,
                tool_allowlist: None,
                developer_instructions: None,
                external_tools: None,
                runner: None,
                ephemeral: false,
            })
            .unwrap(),
        ),
    )
    .await;
    started.thread.id
}

async fn run_turn(client: &LocalAppClient, thread_id: &str) {
    let started: TurnStartResult = request(
        client,
        "turn/start",
        Some(
            serde_json::to_value(TurnStartParams {
                thread_id: thread_id.to_string(),
                input: Vec::new(),
                prompt: Some("Reply with exactly: ok".to_string()),
                model_provider: None,
                model: None,
                reasoning: None,
                policy_mode: None,
                task_ledger_required: false,
            })
            .unwrap(),
        ),
    )
    .await;
    // Poll thread/read until the turn completes through the store-backed path.
    for _ in 0..200 {
        let read: ThreadReadResult = request(
            client,
            "thread/read",
            Some(
                serde_json::to_value(ThreadReadParams {
                    thread_id: thread_id.to_string(),
                    include_turns: true,
                })
                .unwrap(),
            ),
        )
        .await;
        let completed = read
            .thread
            .as_ref()
            .and_then(|thread| thread.turns.as_ref())
            .is_some_and(|turns| {
                turns
                    .iter()
                    .any(|turn| turn.id == started.turn_id && !turn.items.is_empty())
            });
        if completed {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("turn {} did not complete in time", started.turn_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_like_store_keeps_thread_lifecycle_and_tenant_isolation() {
    let database = SharedDatabase::default();
    let tenant_a = tenant_client("tenant-a", &database);
    let tenant_b = tenant_client("tenant-b", &database);
    let workspace_dir = std::env::temp_dir().join(format!(
        "roder-pg-like-workspace-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace_dir).unwrap();

    // Tenant A: full lifecycle through public JSON-RPC paths.
    let thread_a = start_thread(&tenant_a, &workspace_dir).await;
    run_turn(&tenant_a, &thread_a).await;

    let read_a: ThreadReadResult = request(
        &tenant_a,
        "thread/read",
        Some(
            serde_json::to_value(ThreadReadParams {
                thread_id: thread_a.clone(),
                include_turns: true,
            })
            .unwrap(),
        ),
    )
    .await;
    let thread = read_a.thread.expect("tenant A reads its own thread");
    assert_eq!(thread.id, thread_a);
    assert!(
        thread.turns.as_ref().is_some_and(|turns| !turns.is_empty()),
        "store-backed read must include the completed turn"
    );

    let list_a: ThreadListResult = request(
        &tenant_a,
        "thread/list",
        Some(serde_json::to_value(ThreadListParams { limit: Some(10), cursor: None }).unwrap()),
    )
    .await;
    assert!(
        list_a.data.iter().any(|thread| thread.id == thread_a),
        "tenant A lists its own thread"
    );

    // Tenant B shares the database but must not see tenant A's session.
    let list_b: ThreadListResult = request(
        &tenant_b,
        "thread/list",
        Some(serde_json::to_value(ThreadListParams { limit: Some(10), cursor: None }).unwrap()),
    )
    .await;
    assert!(
        list_b.data.iter().all(|thread| thread.id != thread_a),
        "tenant B must not list tenant A threads"
    );

    let read_b: ThreadReadResult = request(
        &tenant_b,
        "thread/read",
        Some(
            serde_json::to_value(ThreadReadParams {
                thread_id: thread_a.clone(),
                include_turns: true,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(
        read_b.thread.is_none(),
        "tenant B must not read tenant A threads"
    );

    let archive_b: ThreadArchiveResult = request(
        &tenant_b,
        "thread/archive",
        Some(serde_json::to_value(ThreadArchiveParams { thread_id: thread_a.clone() }).unwrap()),
    )
    .await;
    assert!(
        !archive_b.archived,
        "tenant B must not archive tenant A threads"
    );

    // Archive is a state transition for the owning tenant and excluded from
    // subsequent lists, matching the PostgreSQL store contract.
    let archive_a: ThreadArchiveResult = request(
        &tenant_a,
        "thread/archive",
        Some(serde_json::to_value(ThreadArchiveParams { thread_id: thread_a.clone() }).unwrap()),
    )
    .await;
    assert!(archive_a.archived, "owning tenant archives its thread");

    let list_after: ThreadListResult = request(
        &tenant_a,
        "thread/list",
        Some(serde_json::to_value(ThreadListParams { limit: Some(10), cursor: None }).unwrap()),
    )
    .await;
    assert!(
        list_after.data.iter().all(|thread| thread.id != thread_a),
        "archived threads are excluded from thread/list"
    );

    // The shared database still holds the archived row for tenant A only.
    let rows = database.rows.lock().unwrap();
    let row = rows
        .get(&("tenant-a".to_string(), thread_a.clone()))
        .expect("archived row remains durable");
    assert!(row.archived);
    assert!(!rows.contains_key(&("tenant-b".to_string(), thread_a)));
}
