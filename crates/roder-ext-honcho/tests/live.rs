use roder_api::memory::{MemoryQuery, MemoryRecord, MemoryScope, MemoryStore};
use roder_ext_honcho::{
    API_KEY_ENV, BASE_URL_ENV, DEFAULT_BASE_URL, HonchoMemoryConfig, HonchoMemoryStore, LIVE_ENV,
};
use serde_json::json;
use time::OffsetDateTime;

/// Searches go through Honcho's ingestion pipeline, so freshly written
/// memories may take a while to become searchable.
const SEARCH_ATTEMPTS: usize = 30;
const SEARCH_RETRY_DELAY_SECS: u64 = 3;

fn record(text: &str, scope: MemoryScope) -> MemoryRecord {
    MemoryRecord {
        id: None,
        scope,
        text: text.to_string(),
        content_hash: None,
        metadata: json!({ "origin": "roder-ext-honcho-live-test" }),
        usage: None,
        deleted: false,
        created_at: OffsetDateTime::now_utc(),
        updated_at: OffsetDateTime::now_utc(),
    }
}

async fn search_with_retry(
    store: &HonchoMemoryStore,
    text: &str,
    scope: MemoryScope,
) -> Vec<roder_api::memory::MemorySearchResult> {
    for _ in 0..SEARCH_ATTEMPTS {
        let results = store
            .search(MemoryQuery {
                scope: Some(scope.clone()),
                text: text.to_string(),
                limit: 5,
                include_global: false,
                provider_id: None,
                model: None,
            })
            .await
            .unwrap();
        if !results.is_empty() {
            return results;
        }
        tokio::time::sleep(std::time::Duration::from_secs(SEARCH_RETRY_DELAY_SECS)).await;
    }
    Vec::new()
}

#[tokio::test]
async fn live_lifecycle_against_real_honcho() {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("set {LIVE_ENV}=1 (plus {API_KEY_ENV}) to run the live honcho check");
        return;
    }
    let api_key = std::env::var(API_KEY_ENV).expect("live check requires HONCHO_API_KEY");
    let base_url = std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let workspace_id = format!("roder-ext-honcho-live-{run_id}");
    let store = HonchoMemoryStore::new(HonchoMemoryConfig {
        api_key: api_key.clone(),
        base_url: base_url.clone(),
        workspace_id: workspace_id.clone(),
        peer_id: "roder-memory".to_string(),
        session_id: None,
    });
    let scope = MemoryScope::Workspace(format!("live-{run_id}"));

    let needle_id = store
        .put(record(
            "the deploy pipeline rotates credentials every tuesday",
            scope.clone(),
        ))
        .await
        .unwrap();
    store
        .put(record("the team mascot is a heron", scope.clone()))
        .await
        .unwrap();

    let fetched = store.get(&needle_id).await.unwrap().unwrap();
    assert_eq!(
        fetched.text,
        "the deploy pipeline rotates credentials every tuesday"
    );
    assert_eq!(fetched.scope, scope);

    let results = search_with_retry(&store, "credential rotation deploys", scope.clone()).await;
    assert!(
        !results.is_empty(),
        "live search returned no results after retries"
    );
    assert_eq!(results[0].record.id.as_deref(), Some(needle_id.as_str()));

    let mut updated = record("credentials now rotate every wednesday", scope.clone());
    updated.id = Some(needle_id.clone());
    let new_id = store.put(updated).await.unwrap();
    assert_ne!(new_id, needle_id);
    let via_old_id = store.get(&needle_id).await.unwrap().unwrap();
    assert_eq!(via_old_id.text, "credentials now rotate every wednesday");

    store.delete(&needle_id).await.unwrap();
    let tombstone = store.get(&new_id).await.unwrap().unwrap();
    assert!(tombstone.deleted);

    let listed = store.list(Some(scope.clone()), 10).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].text, "the team mascot is a heron");

    // Scratch workspace cleanup; best effort.
    cleanup_workspace(&base_url, &api_key, &workspace_id).await;
}

/// Two peers sharing one workspace and one scope map to the SAME derived
/// session (`roder-memory-project-project`). This proves against the real
/// API that (a) the second peer's get-or-create + writes into the existing
/// session succeed (membership is auto-admitted), and (b) no read path —
/// search, list, get — crosses the peer boundary.
#[tokio::test]
async fn live_cross_peer_isolation_against_real_honcho() {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("set {LIVE_ENV}=1 (plus {API_KEY_ENV}) to run the live honcho check");
        return;
    }
    let api_key = std::env::var(API_KEY_ENV).expect("live check requires HONCHO_API_KEY");
    let base_url = std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let workspace_id = format!("roder-ext-honcho-live-xpeer-{run_id}");
    let store = |peer: &str| {
        HonchoMemoryStore::new(HonchoMemoryConfig {
            api_key: api_key.clone(),
            base_url: base_url.clone(),
            workspace_id: workspace_id.clone(),
            peer_id: peer.to_string(),
            session_id: None,
        })
    };
    let store_a = store("peer-a");
    let store_b = store("peer-b");
    // The default tool scope: every peer derives the same session id from it.
    let scope = MemoryScope::Project("project".to_string());

    let a_id = store_a
        .put(record(
            "the alpha launch checklist lives in the wiki",
            scope.clone(),
        ))
        .await
        .unwrap();
    // Second peer writes into the already-existing derived session.
    let b_id = store_b
        .put(record("peer b retro notes from thursday", scope.clone()))
        .await
        .unwrap();

    // Wait until peer-a's record is searchable from peer-a's own view, so
    // the cross-peer emptiness below is meaningful rather than just index lag.
    let results = search_with_retry(&store_a, "alpha launch checklist wiki", scope.clone()).await;
    assert!(
        !results.is_empty(),
        "live search returned no results for the authoring peer after retries"
    );
    assert!(
        results
            .iter()
            .all(|result| result.record.id.as_deref() == Some(a_id.as_str()))
    );

    // Cross-peer reads stay empty on every path.
    let cross_search = store_b
        .search(MemoryQuery {
            scope: Some(scope.clone()),
            text: "alpha launch checklist wiki".to_string(),
            limit: 5,
            include_global: false,
            provider_id: None,
            model: None,
        })
        .await
        .unwrap();
    assert!(
        cross_search
            .iter()
            .all(|result| result.record.id.as_deref() != Some(a_id.as_str())),
        "peer-b search surfaced peer-a's record"
    );
    let listed_b = store_b.list(Some(scope.clone()), 10).await.unwrap();
    assert_eq!(listed_b.len(), 1, "peer-b must list exactly its own record");
    assert_eq!(listed_b[0].id.as_deref(), Some(b_id.as_str()));
    assert!(store_b.get(&a_id).await.unwrap().is_none());

    let listed_a = store_a.list(Some(scope.clone()), 10).await.unwrap();
    assert_eq!(listed_a.len(), 1, "peer-a must list exactly its own record");
    assert_eq!(listed_a[0].id.as_deref(), Some(a_id.as_str()));
    assert!(store_a.get(&b_id).await.unwrap().is_none());

    cleanup_workspace(&base_url, &api_key, &workspace_id).await;
}

async fn cleanup_workspace(base_url: &str, api_key: &str, workspace_id: &str) {
    let client = reqwest::Client::new();
    let _ = client
        .delete(format!(
            "{}/v3/workspaces/{workspace_id}",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .send()
        .await;
}
