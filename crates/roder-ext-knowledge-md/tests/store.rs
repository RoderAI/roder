use std::path::PathBuf;

use roder_api::knowledge::{
    KnowledgeKind, KnowledgeLinkRequest, KnowledgeLinkType, KnowledgeListQuery, KnowledgeQuery,
    KnowledgeSaveRequest, KnowledgeSource, KnowledgeStatus, KnowledgeStore,
    KnowledgeUpdateRequest,
};
use roder_api::memory::MemoryScope;
use roder_ext_knowledge_md::MarkdownKnowledgeStore;

fn temp_store() -> (MarkdownKnowledgeStore, PathBuf) {
    let base = std::env::temp_dir().join(format!("roder-knowledge-{}", uuid::Uuid::new_v4()));
    (MarkdownKnowledgeStore::new(base.clone()), base)
}

fn save_request(title: &str, body: &str, kind: KnowledgeKind) -> KnowledgeSaveRequest {
    KnowledgeSaveRequest {
        scope: MemoryScope::Project("demo".to_string()),
        kind,
        title: title.to_string(),
        tags: vec!["test".to_string()],
        body: body.to_string(),
        source: KnowledgeSource::User,
    }
}

#[tokio::test]
async fn save_writes_markdown_file_and_round_trips() {
    let (store, base) = temp_store();

    let doc = store
        .save(save_request(
            "Use markdown for knowledge",
            "We store knowledge as markdown files.",
            KnowledgeKind::Decision,
        ))
        .await
        .unwrap();

    let path = base
        .join("project-demo")
        .join("docs")
        .join("decision")
        .join("use-markdown-for-knowledge.md");
    assert!(path.exists(), "expected {path:?} to exist");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.starts_with("---\n"));
    assert!(raw.contains("title: Use markdown for knowledge"));

    let loaded = store.get(&doc.id).await.unwrap().unwrap();
    assert_eq!(loaded, doc);
    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.status, KnowledgeStatus::Active);
}

#[tokio::test]
async fn duplicate_titles_get_unique_slugs() {
    let (store, _) = temp_store();

    let first = store
        .save(save_request("Same title", "a", KnowledgeKind::Note))
        .await
        .unwrap();
    let second = store
        .save(save_request("Same title", "b", KnowledgeKind::Note))
        .await
        .unwrap();

    assert_eq!(first.slug, "same-title");
    assert_eq!(second.slug, "same-title-2");
    assert_ne!(first.id, second.id);
}

#[tokio::test]
async fn update_writes_revision_and_preserves_history() {
    let (store, _) = temp_store();
    let doc = store
        .save(save_request(
            "Auth requirements",
            "v1 body with the launch codeword heron",
            KnowledgeKind::Requirement,
        ))
        .await
        .unwrap();

    let updated = store
        .update(KnowledgeUpdateRequest {
            id: doc.id.clone(),
            title: None,
            body: Some("v2 body".to_string()),
            status: None,
            tags: None,
            source: KnowledgeSource::Agent,
        })
        .await
        .unwrap();

    assert_eq!(updated.revision, 2);
    assert_eq!(updated.body, "v2 body");

    let original = store.get_revision(&doc.id, 1).await.unwrap().unwrap();
    assert!(original.body.contains("heron"));
    assert_eq!(original.revision, 1);

    let revisions = store.revisions(&doc.id).await.unwrap();
    assert_eq!(
        revisions.iter().map(|info| info.revision).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[tokio::test]
async fn archive_hides_from_list_but_keeps_document_readable() {
    let (store, _) = temp_store();
    let doc = store
        .save(save_request("Old runbook", "steps", KnowledgeKind::Runbook))
        .await
        .unwrap();

    assert!(store.archive(&doc.id).await.unwrap());
    // Second archive is a no-op.
    assert!(!store.archive(&doc.id).await.unwrap());

    let listed = store
        .list(KnowledgeListQuery {
            scope: Some(MemoryScope::Project("demo".to_string())),
            kind: None,
            tag: None,
            status: None,
            include_archived: false,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(listed.is_empty());

    let read = store.get(&doc.id).await.unwrap().unwrap();
    assert_eq!(read.status, KnowledgeStatus::Archived);
}

#[tokio::test]
async fn list_filters_by_kind_tag_and_status() {
    let (store, _) = temp_store();
    store
        .save(save_request("Decision A", "x", KnowledgeKind::Decision))
        .await
        .unwrap();
    let mut research = save_request("Research B", "y", KnowledgeKind::Research);
    research.tags = vec!["api".to_string()];
    store.save(research).await.unwrap();

    let decisions = store
        .list(KnowledgeListQuery {
            scope: Some(MemoryScope::Project("demo".to_string())),
            kind: Some(KnowledgeKind::Decision),
            tag: None,
            status: None,
            include_archived: false,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].title, "Decision A");

    let tagged = store
        .list(KnowledgeListQuery {
            scope: Some(MemoryScope::Project("demo".to_string())),
            kind: None,
            tag: Some("API".to_string()),
            status: None,
            include_archived: false,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0].title, "Research B");
}

#[tokio::test]
async fn search_scores_lexically_and_includes_global_when_asked() {
    let (store, _) = temp_store();
    store
        .save(save_request(
            "Database choice",
            "We use postgres for tenant session storage.",
            KnowledgeKind::Decision,
        ))
        .await
        .unwrap();
    store
        .save(KnowledgeSaveRequest {
            scope: MemoryScope::Global,
            kind: KnowledgeKind::Note,
            title: "Global convention".to_string(),
            tags: Vec::new(),
            body: "All projects use postgres in production.".to_string(),
            source: KnowledgeSource::User,
        })
        .await
        .unwrap();

    let scoped = store
        .search(KnowledgeQuery {
            scope: Some(MemoryScope::Project("demo".to_string())),
            text: "postgres".to_string(),
            kind: None,
            limit: 10,
            include_global: false,
        })
        .await
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert!(scoped[0].snippet.contains("postgres"));
    assert_eq!(scoped[0].citation.scope_id, "project:demo");

    let with_global = store
        .search(KnowledgeQuery {
            scope: Some(MemoryScope::Project("demo".to_string())),
            text: "postgres".to_string(),
            kind: None,
            limit: 10,
            include_global: true,
        })
        .await
        .unwrap();
    assert_eq!(with_global.len(), 2);
}

#[tokio::test]
async fn links_can_be_set_and_removed() {
    let (store, _) = temp_store();
    let a = store
        .save(save_request("Doc A", "a", KnowledgeKind::Note))
        .await
        .unwrap();
    let b = store
        .save(save_request("Doc B", "b", KnowledgeKind::Note))
        .await
        .unwrap();

    let linked = store
        .set_link(KnowledgeLinkRequest {
            from: a.id.clone(),
            to: b.id.clone(),
            link_type: KnowledgeLinkType::Supersedes,
            remove: false,
        })
        .await
        .unwrap();
    assert_eq!(linked.links.len(), 1);
    assert_eq!(linked.links[0].to, b.id);

    // Linking to a missing target fails.
    let error = store
        .set_link(KnowledgeLinkRequest {
            from: a.id.clone(),
            to: "kn-missing".to_string(),
            link_type: KnowledgeLinkType::RelatesTo,
            remove: false,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("target not found"));

    let removed = store
        .set_link(KnowledgeLinkRequest {
            from: a.id.clone(),
            to: b.id.clone(),
            link_type: KnowledgeLinkType::Supersedes,
            remove: true,
        })
        .await
        .unwrap();
    assert!(removed.links.is_empty());
}

#[tokio::test]
async fn out_of_band_markdown_edits_are_picked_up() {
    let (store, base) = temp_store();
    let doc = store
        .save(save_request(
            "Editable",
            "original body",
            KnowledgeKind::Note,
        ))
        .await
        .unwrap();

    let path = base
        .join("project-demo")
        .join("docs")
        .join("note")
        .join("editable.md");
    let raw = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, raw.replace("original body", "edited outside roder")).unwrap();

    let loaded = store.get(&doc.id).await.unwrap().unwrap();
    assert_eq!(loaded.body, "edited outside roder");
    assert_ne!(loaded.content_hash, doc.content_hash);
}

#[tokio::test]
async fn oversized_bodies_are_rejected() {
    let (store, _) = temp_store();
    let error = store
        .save(save_request(
            "Too big",
            &"x".repeat(roder_ext_knowledge_md::store::MAX_BODY_BYTES + 1),
            KnowledgeKind::Note,
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("split the document"));
}
