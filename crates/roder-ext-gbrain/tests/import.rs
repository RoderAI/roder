use roder_api::memory::MemoryScope;
use roder_ext_gbrain::import::{DedupeMode, ImportBatchInput, ImportBatchParams};
use roder_ext_gbrain::store::RecallParams;
use roder_ext_gbrain::{AsOf, Embedder, GbrainStore};
use rusqlite::Connection;
use std::fs;

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

#[tokio::test]
async fn directory_import_runs_dream_and_records_manifest() {
    let root = temp_path("roder-gbrain-corpus-test");
    let db_path = root.join("gbrain.sqlite3");
    let corpus = root.join("corpus").join("EV-1");
    fs::create_dir_all(&corpus).unwrap();
    fs::write(
        corpus.join("ART-1.md"),
        "<!-- artefact_metadata\nslot_id: ART-1\nevent_id: EV-1\ngenre: meeting_notes\nauthor: Daniel\n-->\n\n2026-06-07\n\nDaniel approved Helix launch readiness.\n",
    )
    .unwrap();

    let store = GbrainStore::open(db_path.clone(), Embedder::new(None)).unwrap();
    let result = store
        .import_batch(ImportBatchParams {
            input: ImportBatchInput::Path(root.join("corpus")),
            format: "directory".to_string(),
            scope: MemoryScope::Project("helix".into()),
            source: Some("helix-corpus".to_string()),
            dedupe: DedupeMode::Both,
            dream_after_import: Some("refine".to_string()),
            metadata: serde_json::json!({"kind": "test"}),
        })
        .await
        .unwrap();

    assert_eq!(result.status, "completed");
    assert_eq!(result.inserted, 1);

    let conn = Connection::open(&db_path).unwrap();
    let dream: (String, String, String, i64) = conn
        .query_row(
            "SELECT mode, run_policy, status, input_fact_count FROM gbrain_dream_runs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        dream,
        ("refine".into(), "import".into(), "completed".into(), 1)
    );

    let graph_counts: (i64, i64, i64) = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM gbrain_nodes WHERE scope_id = 'project:helix'),
                (SELECT COUNT(*) FROM gbrain_edges),
                (SELECT COUNT(*) FROM gbrain_evidence_cards WHERE scope_id = 'project:helix')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(graph_counts.0 >= 4, "expected fact and metadata nodes");
    assert!(graph_counts.1 >= 3, "expected metadata edges");
    assert_eq!(graph_counts.2, 1);

    let manifest: (i64, i64, i64, String, String, String) = conn
        .query_row(
            "SELECT fact_count, node_count, edge_count, source_path, replacement_policy,
                    json_extract(metadata, '$.dream_run_id')
             FROM gbrain_import_manifest
             WHERE import_run_id = ?1",
            [&result.run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(manifest.0, 1);
    assert!(manifest.1 >= 4);
    assert!(manifest.2 >= 3);
    assert!(manifest.3.ends_with("corpus"));
    assert_eq!(manifest.4, "both");
    assert!(!manifest.5.is_empty());

    drop(conn);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

fn temp_path(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()))
}
