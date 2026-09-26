use super::*;

pub(super) async fn persisted_mailbox_messages(
    runtime: &Arc<Runtime>,
    team_id: &str,
    thread_id: &str,
    turn_id: &str,
    payloads: &[&str],
) -> Vec<String> {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let team = runtime.read_team(team_id).await.unwrap();
            if payloads.iter().all(|payload| {
                team.mailbox
                    .iter()
                    .any(|message| message.text == *payload && message.delivered)
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mailbox input reaches sampling promptly");
    let snapshot = runtime
        .load_thread(&thread_id.to_string())
        .await
        .unwrap()
        .unwrap();
    snapshot
        .turns
        .iter()
        .find(|turn| turn.turn_id == turn_id)
        .unwrap()
        .items
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::UserMessage(message) => Some(message.text.clone()),
            _ => None,
        })
        .collect()
}
