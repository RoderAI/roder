use super::*;

const IDLE_NOTICE_BODY: &str = "Targeted post queued while the recipient is idle.";
const SILENT_POST_BODY: &str = "A post made after the agent unsubscribed.";
const BODY_BYTES: usize = 1024;

pub(super) struct BurstTimings {
    pub post_round_trips: Vec<Duration>,
    pub burst_delivery: Duration,
    pub idle_recovery: Duration,
    pub post_body_bytes: usize,
    pub notice_bytes: usize,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_message_board_delivers_ordered_burst_and_recovers_idle_mail() {
    exercise_burst(16).await;
}

pub(super) async fn exercise_burst(post_count: usize) -> BurstTimings {
    let dir = tempfile::tempdir().unwrap();
    let mut eval = BoardEvaluation::open(dir.path());
    let mut notifications = eval.notifications();
    let workspace = create_workspace_for_path(&eval.client, dir.path()).await;
    let root = eval.new_thread(&workspace).await;
    eval.turn(&root, "Publish a burst to the build board.")
        .await;
    eval.sample(&root).await.call(
        "spawn",
        "spawn_agent",
        json!({
            "task_name": "subscriber", "message": "Subscribe to the board.", "fork_turns": "none"
        }),
    );
    let mut writer = eval.sample(&root).await;
    assert!(!writer.result("spawn").is_error);
    let teams: TeamListResult = request(&eval.client, "team/list", Some(json!({}))).await;
    let team = &teams.data[0];
    let reader_id = &team.members[1].thread_id;
    let reader_path = team.members[1].agent_path.as_deref().unwrap();

    eval.sample(reader_id).await.board(
        "subscribe",
        json!({
            "action": "subscribe", "channel": "general"
        }),
    );
    let reader = eval.sample(reader_id).await;
    assert_eq!(reader.output("subscribe")["subscribed"], true);
    let burst_started = Instant::now();
    let mut post_round_trips = Vec::with_capacity(post_count);
    let mut post_ids = Vec::with_capacity(post_count);
    let body = "x".repeat(BODY_BYTES);
    for index in 0..post_count {
        let call = format!("burst-{index}");
        let submitted = Instant::now();
        writer.board(
            &call,
            json!({
                "action": "post", "text": body,
                "notify": if index == 0 { vec![reader_path, reader_path, "/root"] } else { vec![] },
            }),
        );
        writer = eval.sample(&root).await;
        post_round_trips.push(writer.observed_at - submitted);
        let post = writer.output(&call);
        post_ids.push(post["id"].as_str().unwrap().to_owned());
    }
    let final_notice = format!("New board post {}", post_ids.last().unwrap());
    let reader = eval.sample_after_mail(reader, &final_notice).await;
    let burst_delivery = reader.observed_at - burst_started;
    let input = reader.mail_text();
    let mut remaining = input.as_str();
    for id in &post_ids {
        let marker = format!("New board post {id}");
        assert_eq!(input.matches(&marker).count(), 1, "notice must be unique");
        (_, remaining) = remaining
            .split_once(&marker)
            .expect("notices retain posting order");
    }
    assert!(!input.contains(&body), "notifications contain pointers");
    assert_eq!(writer.mail_count("New board post"), 0, "no author echo");

    let saved = eval.team(&team.id).await;
    let notices = saved
        .messages
        .iter()
        .filter(|message| {
            message.to_member_id == team.members[1].id && message.text.starts_with("New board post")
        })
        .collect::<Vec<_>>();
    assert_eq!(notices.len(), post_count);
    assert!(notices.iter().all(|notice| notice.delivered));
    let notice_bytes = notices.iter().map(|notice| notice.text.len()).sum();
    reader.board(
        "unsubscribe",
        json!({"action": "unsubscribe", "channel": "general"}),
    );
    let reader = eval.sample(reader_id).await;
    assert_eq!(reader.output("unsubscribe")["subscribed"], false);
    reader.board("channels", json!({"action": "list_channels"}));
    let reader = eval.sample(reader_id).await;
    assert_eq!(
        reader.output("channels")["channels"][0]["subscribed"],
        false
    );
    reader.finish("Subscription closed.");
    wait_for_board_notification(&mut notifications, "team/member/completed", None).await;
    let writer = eval.sample_after_mail(writer, "Subscription closed.").await;

    writer.board(
        "after-unsubscribe",
        json!({"action": "post", "text": SILENT_POST_BODY}),
    );
    let writer = eval.sample(&root).await;
    let silent_post = writer.output("after-unsubscribe");
    writer.board(
        "target-idle",
        json!({
            "action": "post", "text": IDLE_NOTICE_BODY, "notify": [reader_path]
        }),
    );
    let writer = eval.sample(&root).await;
    let idle_post = writer.output("target-idle");
    let idle_notice = format!("New board post {}", idle_post["id"].as_str().unwrap());
    let silent_notice = format!("New board post {}", silent_post["id"].as_str().unwrap());
    let saved = eval.team(&team.id).await;
    let pending = saved
        .messages
        .iter()
        .filter(|message| message.to_member_id == team.members[1].id && !message.delivered)
        .collect::<Vec<_>>();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].text.starts_with(&idle_notice));
    writer.finish("Board burst verified.");
    wait_for_board_notification(&mut notifications, "turn/completed", Some(&root)).await;
    assert!(matches!(
        eval.runtime
            .drain_active_turns(Duration::from_secs(5))
            .await,
        RuntimeDrainOutcome::Clean { .. }
    ));
    drop(eval);

    let reopened_at = Instant::now();
    let mut eval = BoardEvaluation::open(dir.path());
    let mut notifications = eval.notifications();
    eval.turn(reader_id, "Read your queued board notice.").await;
    let reader = eval.sample(reader_id).await;
    let idle_recovery = reader.observed_at - reopened_at;
    assert_eq!(reader.mail_count(&idle_notice), 1);
    assert_eq!(reader.mail_count(&silent_notice), 0);
    assert_eq!(reader.mail_count(IDLE_NOTICE_BODY), 0);
    reader.board(
        "read-idle",
        json!({"action": "read_post", "post_id": idle_post["id"]}),
    );
    let reader = eval.sample(reader_id).await;
    assert_eq!(reader.output("read-idle")["text"], IDLE_NOTICE_BODY);
    reader.finish("Idle board post recovered.");
    wait_for_board_notification(&mut notifications, "team/member/completed", None).await;
    let saved = eval.team(&team.id).await;
    assert_eq!(
        saved
            .messages
            .iter()
            .filter(|message| {
                message.to_member_id == team.members[1].id
                    && message.text.starts_with(&idle_notice)
                    && message.delivered
            })
            .count(),
        1
    );
    BurstTimings {
        post_round_trips,
        burst_delivery,
        idle_recovery,
        post_body_bytes: post_count * BODY_BYTES,
        notice_bytes,
    }
}
