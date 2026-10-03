use super::Runtime;
use roder_api::thread::{ThreadItem, ThreadItemEventKind};
use time::OffsetDateTime;

#[tokio::test]
async fn independent_runtimes_do_not_reuse_projected_event_identity() {
    let left = Runtime::fake().expect("runtime");
    let right = Runtime::fake().expect("runtime");
    let kind = ThreadItemEventKind::ItemStarted {
        item: ThreadItem::UserMessage {
            id: "item".into(),
            text: "hello".into(),
            images: Vec::new(),
            status: None,
        },
    };
    let thread = "shared-thread".to_string();
    let turn = "shared-turn".to_string();
    let a = left
        .record_thread_item_event_kind(&thread, &turn, OffsetDateTime::UNIX_EPOCH, kind.clone())
        .await
        .expect("left event");
    let b = right
        .record_thread_item_event_kind(&thread, &turn, OffsetDateTime::UNIX_EPOCH, kind)
        .await
        .expect("right event");
    assert_eq!(
        a.seq, b.seq,
        "independent caches start at the same sequence"
    );
    assert_ne!(
        a.event_id, b.event_id,
        "different events must not deduplicate at persistence"
    );
}
