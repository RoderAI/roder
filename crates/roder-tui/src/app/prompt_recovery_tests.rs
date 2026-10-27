use super::*;

#[tokio::test]
async fn discarding_a_permanent_rejection_unblocks_only_its_thread_queue() {
    let (client, mut replies) = ControlledClient::new(2);
    let mut app = app(client);
    app.start_prepared_prompt(prompt("permanently invalid"))
        .await;
    app.queue_prepared_prompt(prompt("later valid input"));
    replies
        .pop_front()
        .unwrap()
        .send(rejected("invalid input"))
        .unwrap();
    settle(&mut app).await;
    assert!(!app.can_dispatch_prompt());
    app.run_retry_slash_command("discard").await;
    assert!(app.can_dispatch_prompt());
    assert_eq!(app.queued_prompts.len(), 1);
    assert!(app.submit_next_queued_prompt().await);
    assert_eq!(
        app.pending_turn_requests[0].prompt.message,
        "later valid input"
    );
    replies
        .pop_front()
        .unwrap()
        .send(accepted("later"))
        .unwrap();
    settle(&mut app).await;
}

#[tokio::test]
async fn discard_does_not_remove_another_threads_recovery() {
    let (client, mut replies) = ControlledClient::new(1);
    let mut app = app(client);
    app.start_prepared_prompt(prompt("original")).await;
    replies
        .pop_front()
        .unwrap()
        .send(rejected("invalid input"))
        .unwrap();
    settle(&mut app).await;
    app.thread_id = "other-thread".into();
    app.run_retry_slash_command("discard").await;
    assert!(app.prompt_admission.has_recovery("thread-test"));
}
