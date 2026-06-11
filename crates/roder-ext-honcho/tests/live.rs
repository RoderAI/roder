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
    let client = reqwest::Client::new();
    let _ = client
        .delete(format!(
            "{}/v3/workspaces/{workspace_id}",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(&api_key)
        .send()
        .await;
}
