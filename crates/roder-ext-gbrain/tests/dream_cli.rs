use roder_api::memory::MemoryScope;
use roder_ext_gbrain::dream::{DreamMode, DreamPolicy, DreamStatus};
use roder_ext_gbrain::store::{CaptureInput, DreamParams};
use roder_ext_gbrain::tools::{is_read_only_tool, read_only_tool_names};
use roder_ext_gbrain::{Embedder, GbrainStore};

fn store() -> GbrainStore {
    GbrainStore::open_in_memory(Embedder::new(None)).unwrap()
}

#[tokio::test]
async fn dream_run_completes_and_status_is_inspectable() {
    let store = store();
    let scope = MemoryScope::Project("helix".into());
    let mut input = CaptureInput::new(scope.clone(), "Data retention policy is 90 days");
    input.subject = Some("retention-policy".into());
    store.capture(input).await.unwrap();

    let run = store
        .dream(DreamParams {
            mode: DreamMode::Refine,
            scope: scope.clone(),
            since: None,
            run_policy: DreamPolicy::Maintenance,
            workers: 1,
            dry_run: false,
            cancellation_token: None,
            reasoner_model: None,
        })
        .await
        .unwrap();

    assert_eq!(run.status, DreamStatus::Completed);
    assert_eq!(run.mode, DreamMode::Refine);
    assert_eq!(run.input_fact_count, 1);
    assert_eq!(run.workers, 1);

    let status = store.dream_status(&run.id).await.unwrap().unwrap();
    assert_eq!(status.id, run.id);
    assert_eq!(status.status, DreamStatus::Completed);
    assert!(status.finished_at.is_some());
}

#[test]
fn mutating_import_and_dream_tools_are_excluded_from_read_only_set() {
    assert!(!is_read_only_tool("gbrain_import"));
    assert!(!is_read_only_tool("gbrain_dream"));
    assert!(is_read_only_tool("gbrain_dream_status"));
    assert!(read_only_tool_names().contains(&"gbrain_dream_status"));
    assert!(!read_only_tool_names().contains(&"gbrain_import"));
    assert!(!read_only_tool_names().contains(&"gbrain_dream"));
}
