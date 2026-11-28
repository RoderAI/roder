use std::time::Instant;

use roder_api::teams::TeamMailboxMessageKind;
use roder_core::runtime::RuntimeDrainOutcome;
use serde_json::json;

use super::*;

#[path = "agent_message_board/harness.rs"]
mod harness;
use harness::BoardEvaluation;

#[path = "agent_message_board/burst.rs"]
mod burst;
#[path = "agent_message_board/measurements.rs"]
mod measurements;
#[path = "agent_message_board/verification.rs"]
mod verification;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_message_board_evaluates_peer_handoff_and_interrupt_recovery() {
    exercise_handoff(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_message_board_recovers_child_replies_after_restart_before_resuming() {
    exercise_handoff(true).await;
}

pub(super) struct HandoffTimings {
    pub claim_race: Duration,
    pub direct_delivery: Duration,
    pub handoff_delivery: Duration,
    pub interrupt_recovery: Duration,
}

pub(super) async fn exercise_handoff(restart_before_resume: bool) -> HandoffTimings {
    let dir = tempfile::tempdir().unwrap();
    let mut eval = BoardEvaluation::open(dir.path());
    let mut notifications = eval.notifications();
    let workspace = create_workspace_for_path(&eval.client, dir.path()).await;
    let root = eval.new_thread(&workspace).await;
    let root_turn = eval
        .turn(&root, "Set up a board and delegate the synthetic build.")
        .await;

    eval.sample(&root).await.board(
        "create",
        json!({
            "action": "create_channel", "channel": "build", "description": "Build verification"
        }),
    );
    let sample = eval.sample(&root).await;
    assert_eq!(sample.output("create")["channel"], "build");
    sample.board(
        "offer",
        json!({
            "action": "post", "channel": "build", "text": "Verify the synthetic build.",
            "claimable": true
        }),
    );
    let sample = eval.sample(&root).await;
    let offer = sample.output("offer");
    let post_id = &offer["id"];
    assert_eq!(offer["work"]["status"], "open");
    sample.call(
        "spawn-alpha",
        "spawn_agent",
        json!({
            "task_name": "alpha", "message": "Find and claim the build work.",
            "fork_turns": "none"
        }),
    );
    let sample = eval.sample(&root).await;
    let spawn = &sample.result("spawn-alpha").result;
    let teams: TeamListResult = request(&eval.client, "team/list", Some(json!({}))).await;
    let team_id = teams.data[0].id.clone();
    let alpha = eval.team(&team_id).await.team.unwrap().members[1].clone();
    assert!(spawn.contains(&alpha.thread_id));
    sample.call(
        "spawn-beta",
        "spawn_agent",
        json!({
            "task_name": "beta", "message": "Find and claim the build work.",
            "fork_turns": "none"
        }),
    );
    let root_pending = eval.sample(&root).await;
    let beta = eval.team(&team_id).await.team.unwrap().members[2].clone();
    assert!(!root_pending.result("spawn-beta").is_error);
    root_pending.call("pause", "board_eval_pause", json!({}));
    tokio::time::timeout(Duration::from_secs(10), eval.root_paused.notified())
        .await
        .expect("parent reaches the pause tool");

    for thread in [&alpha.thread_id, &beta.thread_id] {
        eval.sample(thread).await.board(
            "subscribe",
            json!({"action": "subscribe", "channel": "build"}),
        );
        let sample = eval.sample(thread).await;
        assert_eq!(sample.output("subscribe")["subscribed"], true);
        sample.board(
            "discover",
            json!({"action": "list_posts", "channel": "build", "query": "SYNTHETIC"}),
        );
    }
    let alpha_sample = eval.sample(&alpha.thread_id).await;
    let beta_sample = eval.sample(&beta.thread_id).await;
    let claim_started = Instant::now();
    for sample in [alpha_sample, beta_sample] {
        assert_eq!(sample.output("discover")["posts"][0]["id"], *post_id);
        sample.board("claim", json!({"action": "claim", "post_id": post_id}));
    }
    let alpha_sample = eval.sample(&alpha.thread_id).await;
    let beta_sample = eval.sample(&beta.thread_id).await;
    let claim_race = alpha_sample.observed_at.max(beta_sample.observed_at) - claim_started;
    let alpha_won = !alpha_sample.result("claim").is_error;
    assert_eq!(
        alpha_won,
        beta_sample.result("claim").is_error,
        "exactly one claim succeeds"
    );
    let (owner, claimant, peer, receiver) = if alpha_won {
        (&alpha, alpha_sample, &beta, beta_sample)
    } else {
        (&beta, beta_sample, &alpha, alpha_sample)
    };
    assert_eq!(claimant.output("claim")["work"]["owner"], owner.id);
    receiver.board(
        "unauthorized-release",
        json!({"action": "release", "post_id": post_id}),
    );
    let receiver = eval.sample(&peer.thread_id).await;
    assert!(receiver.result("unauthorized-release").is_error);
    let peer_path = peer.agent_path.as_deref().unwrap();
    claimant.board(
        "reply",
        json!({
            "action": "post", "channel": "build", "reply_to": post_id,
            "text": "Synthetic build passed on my machine.", "notify": [peer_path]
        }),
    );
    let sample = eval.sample(&owner.thread_id).await;
    let reply = sample.output("reply");
    let reply_id = reply["id"].as_str().unwrap();
    assert_eq!(reply["author"], owner.id);
    assert_eq!(reply["reply_to"], *post_id);
    let direct_started = Instant::now();
    sample.call(
        "direct",
        "send_message",
        json!({
            "target": peer_path, "message": "Synthetic peer-only message."
        }),
    );
    let sample = eval.sample(&owner.thread_id).await;
    assert!(!sample.result("direct").is_error);
    let receiver = eval
        .sample_after_mail(receiver, "Synthetic peer-only message.")
        .await;
    let direct_delivery = receiver.observed_at - direct_started;
    let handoff_started = Instant::now();
    sample.board(
        "handoff",
        json!({"action": "handoff", "post_id": post_id, "target": peer_path}),
    );
    let sample = eval.sample(&owner.thread_id).await;
    assert_eq!(sample.output("handoff")["work"]["owner"], peer.id);
    sample.finish("Synthetic claimant finished.");
    let receiver = eval
        .sample_after_mail(receiver, "was handed off to you in #build.")
        .await;
    let handoff_delivery = receiver.observed_at - handoff_started;
    receiver.board("read", json!({"action": "read_post", "post_id": post_id}));
    let sample = eval.sample(&peer.thread_id).await;
    assert_eq!(sample.output("read")["work"]["owner"], peer.id);
    assert_eq!(
        sample.mail_count(&format!("New board post {reply_id} in #build.")),
        1
    );
    assert_eq!(sample.mail_count("Synthetic peer-only message."), 1);
    assert_eq!(sample.mail_count("was handed off to you in #build."), 1);
    assert!(sample.mail_count(&format!("Sender: {}", owner.agent_path.as_deref().unwrap())) >= 1);
    sample.board(
        "complete",
        json!({
            "action": "complete", "post_id": post_id, "evidence": "Synthetic verifier exited 0"
        }),
    );
    let sample = eval.sample(&peer.thread_id).await;
    assert_eq!(sample.output("complete")["work"]["status"], "completed");
    sample.finish("Synthetic verifying child finished.");

    // Child terminal replies are queued while the root is still sampling.
    for _ in 0..2 {
        wait_for_notification(&mut notifications, "team/member/completed", None).await;
    }
    let team = eval.team(&team_id).await;
    assert_eq!(
        team.messages
            .iter()
            .filter(|message| {
                message.kind == TeamMailboxMessageKind::FinalAnswer
                    && message.to_member_id == "lead"
                    && !message.delivered
            })
            .count(),
        2
    );
    let interrupt_started = Instant::now();
    let _: TurnInterruptResult = request(
        &eval.client,
        "turn/interrupt",
        Some(json!({
            "threadId": root, "turnId": root_turn
        })),
    )
    .await;
    if !restart_before_resume {
        eval.turn(&root, "Summarize the verified work.").await;
    }
    let interrupted =
        wait_for_notification(&mut notifications, "turn/completed", Some(&root)).await;
    assert_eq!(interrupted.params["turn"]["status"], "interrupted");
    if restart_before_resume {
        assert!(matches!(
            eval.runtime
                .drain_active_turns(Duration::from_secs(5))
                .await,
            RuntimeDrainOutcome::Clean { .. }
        ));
        drop(eval);
        eval = BoardEvaluation::open(dir.path());
        notifications = eval.notifications();
        eval.turn(&root, "Summarize the verified work.").await;
    }
    let sample = eval.sample(&root).await;
    let interrupt_recovery = sample.observed_at - interrupt_started;
    assert_eq!(sample.mail_count("Synthetic claimant finished."), 1);
    assert_eq!(sample.mail_count("Synthetic verifying child finished."), 1);
    assert_eq!(sample.mail_count("Synthetic peer-only message."), 0);
    sample.finish("Verification complete.");
    wait_for_notification(&mut notifications, "turn/completed", Some(&root)).await;
    let drained = eval
        .runtime
        .drain_active_turns(Duration::from_secs(5))
        .await;
    assert!(
        matches!(drained, RuntimeDrainOutcome::Clean { .. }),
        "{drained:?}"
    );
    drop(eval);

    // Reopen the same on-disk team and thread through a fresh app server.
    let mut eval = BoardEvaluation::open(dir.path());
    let mut notifications = eval.notifications();
    eval.turn(&root, "Read the stored board.").await;
    let sample = eval.sample(&root).await;
    assert_eq!(sample.mail_count("Synthetic verifying child finished."), 1);
    sample.board(
        "page",
        json!({"action": "list_posts", "channel": "build", "limit": 1}),
    );
    let sample = eval.sample(&root).await;
    let page = sample.output("page");
    assert_eq!(page["posts"][0]["id"], reply_id);
    sample.board(
        "next-page",
        json!({
            "action": "list_posts", "channel": "build", "limit": 1, "before": page["next_before"]
        }),
    );
    let sample = eval.sample(&root).await;
    let page = sample.output("next-page");
    assert_eq!(page["posts"][0]["id"], *post_id);
    assert!(page["next_before"].is_null());
    sample.board(
        "read-restarted",
        json!({"action": "read_post", "post_id": post_id}),
    );
    let sample = eval.sample(&root).await;
    let completed = sample.output("read-restarted");
    assert_eq!(completed["work"]["status"], "completed");
    assert_eq!(completed["work"]["evidence"], "Synthetic verifier exited 0");
    sample.call(
        "idle-peer",
        "send_message",
        json!({"target": peer_path, "message": "Synthetic idle peer mail."}),
    );
    let sample = eval.sample(&root).await;
    assert!(!sample.result("idle-peer").is_error);
    sample.finish("Stored board verified.");
    wait_for_notification(&mut notifications, "turn/completed", Some(&root)).await;
    let team = eval.team(&team_id).await;
    assert!(team.messages.iter().any(|message| {
        message.to_member_id == peer.id
            && message.text == "Synthetic idle peer mail."
            && !message.delivered
    }));
    eval.turn(&peer.thread_id, "Read your queued messages.")
        .await;
    let sample = eval.sample(&peer.thread_id).await;
    assert_eq!(sample.mail_count("Synthetic idle peer mail."), 1);
    sample.finish("Idle mail received.");
    wait_for_notification(&mut notifications, "team/member/completed", None).await;

    let outsider = eval.new_thread(&workspace).await;
    eval.turn(&outsider, "Inspect my own board.").await;
    eval.sample(&outsider).await.board(
        "other-team",
        json!({"action": "read_post", "post_id": post_id}),
    );
    let sample = eval.sample(&outsider).await;
    assert!(sample.result("other-team").is_error);
    sample.board(
        "outside-notify",
        json!({"action": "post", "text": "private", "notify": [peer_path]}),
    );
    let sample = eval.sample(&outsider).await;
    assert!(sample.result("outside-notify").is_error);
    sample.finish("No access to the other team.");
    wait_for_notification(&mut notifications, "turn/completed", Some(&outsider)).await;
    HandoffTimings {
        claim_race,
        direct_delivery,
        handoff_delivery,
        interrupt_recovery,
    }
}
