use roder_api::memory::MemoryScope;
use roder_ext_gbrain::import::{DedupeMode, ImportBatchInput, ImportBatchParams};
use roder_ext_gbrain::store::RecallParams;
use roder_ext_gbrain::{AsOf, Embedder, GbrainStore};

fn store() -> GbrainStore {
    GbrainStore::open_in_memory(Embedder::new(None)).unwrap()
}

#[tokio::test]
async fn jsonl_import_preserves_metadata_and_is_idempotent() {
    let store = store();
    let payload = r#"
{"slug":"artifact-a","source_id":"src-1","text":"Acme approved 90-day retention on 2022-05-01.","timestamp":"2022-05-01","thread_id":"thread-1","provenance":["meeting-notes"],"metadata":{"author":"Maya"}}
{"slug":"artifact-b","source_id":"src-2","text":"Acme retention owner is Daniel.","timestamp":"2022-05-02","thread_id":"thread-1","provenance":["ticket-42"],"metadata":{"author":"Iris"}}
"#;

    let params = ImportBatchParams {
        input: ImportBatchInput::JsonlString(payload.to_string()),
        format: "jsonl".to_string(),
        scope: MemoryScope::Project("helix".into()),
        source: Some("fixture".to_string()),
        dedupe: DedupeMode::Both,
        dream_after_import: None,
        metadata: serde_json::json!({"kind": "test"}),
    };

    let first = store.import_batch(params.clone()).await.unwrap();
    assert_eq!(first.inserted, 2);
    assert_eq!(first.skipped_duplicates, 0);
    assert_eq!(first.total, 2);
    assert_eq!(first.status, "completed");

    let second = store.import_batch(params).await.unwrap();
    assert_eq!(second.inserted, 0);
    assert_eq!(second.skipped_duplicates, 2);
    assert_eq!(second.total, 2);

    let recall = store
        .recall(RecallParams {
            query: "retention".into(),
            as_of: AsOf::now(),
            scope: Some(MemoryScope::Project("helix".into())),
            include_global: false,
            limit: 10,
            expand: true,
        })
        .await
        .unwrap();
    assert_eq!(recall.hits.len(), 2);
    let fact = recall
        .hits
        .iter()
        .find(|hit| {
            hit.fact
                .provenance
                .first()
                .is_some_and(|slug| slug == "artifact-a")
        })
        .unwrap();
    assert_eq!(
        fact.fact.valid_at,
        roder_ext_gbrain::model::parse_flexible("2022-05-01").unwrap()
    );
    assert_eq!(fact.fact.metadata["source_id"], "src-1");
    assert_eq!(fact.fact.metadata["source"], "fixture");
    assert_eq!(fact.fact.metadata["thread_id"], "thread-1");
    assert_eq!(fact.fact.metadata["author"], "Maya");
    assert!(
        fact.fact
            .provenance
            .iter()
            .any(|item| item == "meeting-notes")
    );
}
