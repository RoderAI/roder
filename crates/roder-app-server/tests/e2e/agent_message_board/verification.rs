use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_message_board_evidence_flows_through_ledger_and_verification_gate() {
    const TASK: &str = "Write verified.txt and have a reviewer check it.";
    const FILE: &str = "verified.txt";
    const CONTENT: &str = "board scenario fixture";
    const EVIDENCE: &str = "Reviewer read verified.txt and matched the expected content.";
    let dir = tempfile::tempdir().unwrap();
    let mut eval = BoardEvaluation::open_with_profile(dir.path(), RuntimeProfile::Eval);
    let mut notifications = eval.notifications();
    let workspace = create_workspace_for_path(&eval.client, dir.path()).await;
    let root = eval.new_thread(&workspace).await;
    eval.turn_with_ledger(&root, TASK, true).await;

    eval.sample(&root).await.call(
        "start-ledger",
        "task_ledger.update",
        json!({
            "tasks": [{"id": "review", "content": TASK, "status": "in_progress"}],
            "requireCompletionEvidence": true
        }),
    );
    let sample = eval.sample(&root).await;
    assert!(!sample.result("start-ledger").is_error);
    sample.call(
        "write",
        "write_file",
        json!({"path": FILE, "content": CONTENT}),
    );
    let sample = eval.sample(&root).await;
    assert!(!sample.result("write").is_error);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(FILE)).unwrap(),
        CONTENT
    );
    sample.board(
        "offer",
        json!({
            "action": "post", "claimable": true, "text": "Read verified.txt and check its content."
        }),
    );
    let sample = eval.sample(&root).await;
    let offer = sample.output("offer");
    let post_id = &offer["id"];
    sample.call(
        "spawn",
        "spawn_agent",
        json!({
            "task_name": "reviewer", "message": "Claim the board post and verify the fixture.",
            "fork_turns": "none"
        }),
    );
    let root_sample = eval.sample(&root).await;
    assert!(!root_sample.result("spawn").is_error);
    let teams: TeamListResult = request(&eval.client, "team/list", Some(json!({}))).await;
    let child = &teams.data[0].members[1].thread_id;

    eval.sample(child)
        .await
        .board("claim", json!({"action": "claim", "post_id": post_id}));
    let sample = eval.sample(child).await;
    assert_eq!(sample.output("claim")["work"]["status"], "claimed");
    sample.call("read", "read_file", json!({"path": FILE}));
    let sample = eval.sample(child).await;
    let read = sample.result("read");
    assert!(!read.is_error);
    assert!(read.result.contains(CONTENT));
    sample.board(
        "complete",
        json!({
            "action": "complete", "post_id": post_id, "evidence": EVIDENCE
        }),
    );
    let sample = eval.sample(child).await;
    assert_eq!(sample.output("complete")["work"]["evidence"], EVIDENCE);
    sample.finish("Fixture review finished.");

    let sample = eval
        .sample_after_mail(root_sample, "Fixture review finished.")
        .await;
    sample.board(
        "read-evidence",
        json!({"action": "read_post", "post_id": post_id}),
    );
    let sample = eval.sample(&root).await;
    let result = sample.output("read-evidence");
    let evidence = result["work"]["evidence"].as_str().unwrap();
    assert_eq!(evidence, EVIDENCE);
    sample.call("complete-ledger", "task_ledger.update", json!({
        "tasks": [{"id": "review", "content": TASK, "status": "completed", "evidence": evidence}],
        "requireCompletionEvidence": true
    }));
    let sample = eval.sample(&root).await;
    assert!(!sample.result("complete-ledger").is_error);
    sample.finish("Review done.");

    let sample = eval.sample(&root).await;
    assert_eq!(
        sample.mail_count("Verification gate blocked final completion"),
        1
    );
    sample.call(
        "review",
        "verification_review",
        json!({
            "originalTask": TASK,
            "status": "completed",
            "changedFiles": [FILE],
            "toolEvidence": [evidence],
            "testsRun": ["Reviewed the fixture with read_file."]
        }),
    );
    let sample = eval.sample(&root).await;
    let review = sample.result("review");
    assert!(!review.is_error);
    assert!(review.result.starts_with("Verification completed"));
    sample.finish("Verified and complete.");
    let terminal =
        wait_for_board_notification(&mut notifications, "turn/completed", Some(&root)).await;
    assert_eq!(terminal.params["turn"]["status"], "completed");
}
