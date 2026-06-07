//! End-to-end proofs of the bi-temporal contract: as-of date-travel,
//! supersession, invalidate-never-delete, and contradiction detection.

use roder_api::memory::{MemoryQuery, MemoryRecord, MemoryScope, MemoryStore};
use roder_ext_gbrain::model::parse_flexible;
use roder_ext_gbrain::store::RecallParams;
use roder_ext_gbrain::store::{CaptureInput, GbrainStore};
use roder_ext_gbrain::{AsOf, Embedder};
use time::OffsetDateTime;

fn store() -> GbrainStore {
    GbrainStore::open_in_memory(Embedder::new(None)).unwrap()
}

fn at(date: &str) -> time::OffsetDateTime {
    parse_flexible(date).unwrap()
}

fn captured(scope: MemoryScope, text: &str, subject: &str, valid: &str) -> CaptureInput {
    let mut input = CaptureInput::new(scope, text);
    input.subject = Some(subject.to_string());
    input.valid_at = Some(at(valid));
    input.ingested_at = Some(at(valid));
    input
}

#[tokio::test]
async fn as_of_supersession_and_history() {
    let store = store();
    let scope = MemoryScope::Project("helix".into());

    let v1 = store
        .capture(captured(
            scope.clone(),
            "Acme account owner is Maya Patel",
            "acme-owner",
            "2022-01-01",
        ))
        .await
        .unwrap();

    let v2 = store
        .supersede(
            &v1.id,
            "Acme account owner is Daniel Kim",
            "Maya transferred the account when she moved to product",
            Some(at("2024-01-01")),
        )
        .await
        .unwrap();

    // Predecessor is invalidated + linked, never deleted.
    let old = store.get_fact(&v1.id).await.unwrap().unwrap();
    assert_eq!(old.superseded_by.as_deref(), Some(v2.id.as_str()));
    assert_eq!(old.invalid_at, Some(at("2024-01-01")));
    assert!(
        old.expired_at.is_none(),
        "supersession must not retract the record"
    );

    // Current belief -> Daniel.
    let now = store
        .recall(RecallParams {
            query: "who owns the acme account".into(),
            as_of: AsOf::now(),
            scope: Some(scope.clone()),
            include_global: false,
            limit: 5,
            expand: false,
        })
        .await
        .unwrap();
    assert_eq!(now.hits.len(), 1, "only the current fact is believed now");
    assert!(now.hits[0].fact.text.contains("Daniel"));

    // As of 2023 -> Maya (Daniel's fact wasn't valid yet).
    let past = store
        .as_of(
            at("2023-06-01"),
            "who owns the acme account",
            Some(scope.clone()),
            5,
        )
        .await
        .unwrap();
    assert!(
        past.hits.iter().any(|h| h.fact.text.contains("Maya")),
        "as-of 2023 should recover Maya: {:?}",
        past.hits.iter().map(|h| &h.fact.text).collect::<Vec<_>>()
    );
    assert!(
        !past.hits.iter().any(|h| h.fact.text.contains("Daniel")),
        "Daniel's fact was not valid as of 2023"
    );

    // History returns both versions, oldest first.
    let history = store
        .history(None, Some("acme-owner"), Some(scope.clone()))
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(history[0].text.contains("Maya"));
    assert!(history[1].text.contains("Daniel"));
}

#[tokio::test]
async fn invalidate_never_delete_preserves_audit_trail() {
    let store = store();
    let scope = MemoryScope::Project("helix".into());

    let fact = store
        .capture(captured(
            scope.clone(),
            "Primary datacenter is us-east-1",
            "primary-dc",
            "2020-01-01",
        ))
        .await
        .unwrap();

    // "delete" is a retraction: the row stays, expired_at is set.
    store.delete(&fact.id).await.unwrap();
    let retracted = store.get_fact(&fact.id).await.unwrap().unwrap();
    assert!(retracted.expired_at.is_some(), "row must survive a delete");

    // Current search no longer surfaces it.
    let current = store
        .search(MemoryQuery {
            scope: Some(scope.clone()),
            text: "datacenter".into(),
            limit: 5,
            include_global: false,
            provider_id: None,
            model: None,
        })
        .await
        .unwrap();
    assert!(current.is_empty(), "retracted fact must not be current");

    // But a transaction-time snapshot from before the retraction still sees it.
    let snapshot = store
        .as_of(at("2021-01-01"), "datacenter", Some(scope.clone()), 5)
        .await
        .unwrap();
    assert!(
        snapshot.hits.iter().any(|h| h.fact.id == fact.id),
        "pre-retraction as-of must recover the fact"
    );
}

#[tokio::test]
async fn contradiction_detection_and_consolidate_idempotent() {
    let store = store();
    let scope = MemoryScope::Project("helix".into());

    store
        .capture(captured(
            scope.clone(),
            "Data retention policy is 30 days",
            "retention-policy",
            "2022-01-01",
        ))
        .await
        .unwrap();
    store
        .capture(captured(
            scope.clone(),
            "Data retention policy is 90 days",
            "retention-policy",
            "2023-01-01",
        ))
        .await
        .unwrap();

    // Two coexisting, unlinked facts about the same subject => contradiction.
    let pairs = store
        .contradictions(Some(scope.clone()), Some("retention-policy"), 10)
        .await
        .unwrap();
    assert_eq!(pairs.len(), 1, "expected one contradiction");

    // Consolidate persists the link; a second pass is a no-op.
    let first = store.consolidate(Some(scope.clone())).await.unwrap();
    assert_eq!(first.contradiction_links, 1);
    let second = store.consolidate(Some(scope.clone())).await.unwrap();
    assert_eq!(
        second.contradiction_links, 0,
        "consolidate must be idempotent"
    );
}

#[tokio::test]
async fn store_id_is_gbrain_bitemporal() {
    assert_eq!(MemoryStore::id(&store()), "gbrain-bitemporal");
}

// --- regression tests for review findings ---

#[tokio::test]
async fn supersede_records_transaction_time_as_now_not_backdated_valid_at() {
    let store = store();
    let scope = MemoryScope::Project("helix".into());
    let v1 = store
        .capture(captured(
            scope.clone(),
            "owner is Maya",
            "owner",
            "2022-01-01",
        ))
        .await
        .unwrap();
    // Backdated valid_at (2024) — but the correction is RECORDED now.
    let v2 = store
        .supersede(&v1.id, "owner is Daniel", "moved", Some(at("2024-01-01")))
        .await
        .unwrap();
    assert_eq!(v2.valid_at, at("2024-01-01"));
    assert!(
        v2.ingested_at > at("2025-01-01"),
        "transaction time must be recorded-now, not the backdated valid_at: {}",
        v2.ingested_at
    );
}

#[tokio::test]
async fn backdated_supersede_does_not_invert_predecessor_interval() {
    let store = store();
    let scope = MemoryScope::Project("helix".into());
    let v1 = store
        .capture(captured(scope.clone(), "v1", "s", "2022-01-01"))
        .await
        .unwrap();
    // Supersede with a valid_at (2020) EARLIER than the predecessor's (2022).
    store
        .supersede(&v1.id, "v2", "backdated", Some(at("2020-01-01")))
        .await
        .unwrap();
    let old = store.get_fact(&v1.id).await.unwrap().unwrap();
    let invalid = old.invalid_at.expect("predecessor invalidated");
    assert!(
        invalid >= old.valid_at,
        "interval must not invert: valid={} invalid={invalid}",
        old.valid_at
    );
}

#[tokio::test]
async fn supersede_unknown_target_errors_without_orphan() {
    let store = store();
    let scope = MemoryScope::Project("helix".into());
    let now = OffsetDateTime::now_utc();
    let record = MemoryRecord {
        id: None,
        scope: scope.clone(),
        text: "orphan replacement".into(),
        content_hash: None,
        metadata: serde_json::json!({ "supersedes": "does-not-exist" }),
        usage: None,
        deleted: false,
        created_at: now,
        updated_at: now,
    };
    assert!(
        store.put(record).await.is_err(),
        "supersede to an unknown target must error"
    );
    // The transaction rolled back — no orphan fact was committed.
    let all = store.list(Some(scope), 50).await.unwrap();
    assert!(
        all.iter().all(|r| r.text != "orphan replacement"),
        "no orphan replacement fact should persist"
    );
}
