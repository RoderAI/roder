use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::Runtime;
use std::sync::Arc;

async fn paused_app() -> (TuiApp, Arc<Runtime>) {
    let runtime = Arc::new(Runtime::fake().unwrap());
    runtime
        .thread_goal_set(
            &"thread-test".into(),
            roder_api::goals::ThreadGoalPatch {
                objective: Some("Original work".into()),
                status: Some(ThreadGoalStatus::Paused),
                token_budget: Some(Some(100)),
            },
        )
        .await
        .unwrap();
    let server = Arc::new(AppServer::new(runtime.clone()));
    let app =
        super::super::tests::test_app_with_client(LocalAppClient::new(server.clone()), server);
    (app, runtime)
}

#[tokio::test]
async fn editing_paused_goal_preserves_status_and_budget() {
    let (mut app, runtime) = paused_app().await;
    app.run_goal_slash_command("edit Revised work").await;
    let goal = runtime
        .thread_goal_get(&app.thread_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(goal.objective, "Revised work");
    assert_eq!(goal.status, ThreadGoalStatus::Paused);
    assert_eq!(goal.token_budget, Some(100));
}

#[tokio::test]
async fn unfinished_replacement_asks_and_cancellation_preserves_goal() {
    let (mut app, runtime) = paused_app().await;
    app.run_goal_slash_command("Replacement work").await;
    assert!(matches!(
        app.confirm_dialog.as_ref().map(|state| &state.dialog),
        Some(ConfirmDialog::ReplaceGoal { .. })
    ));
    app.handle_confirm_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .await;
    assert!(app.confirm_dialog.is_none());
    assert_eq!(
        runtime
            .thread_goal_get(&app.thread_id)
            .await
            .unwrap()
            .unwrap()
            .objective,
        "Original work"
    );
}

#[tokio::test]
async fn invalid_replacement_does_not_clear_existing_goal() {
    let (mut app, runtime) = paused_app().await;
    app.run_goal_slash_command(&"x".repeat(4001)).await;
    app.handle_confirm_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .await;
    assert_eq!(
        runtime
            .thread_goal_get(&app.thread_id)
            .await
            .unwrap()
            .unwrap()
            .objective,
        "Original work"
    );
}

#[test]
fn edits_preserve_stopped_status_except_completed_or_budget_limited() {
    for status in [
        ThreadGoalStatus::Active,
        ThreadGoalStatus::Paused,
        ThreadGoalStatus::Blocked,
        ThreadGoalStatus::UsageLimited,
    ] {
        assert_eq!(edited_goal_status(status), status);
    }
    assert_eq!(
        edited_goal_status(ThreadGoalStatus::Complete),
        ThreadGoalStatus::Active
    );
    assert_eq!(
        edited_goal_status(ThreadGoalStatus::BudgetLimited),
        ThreadGoalStatus::Active
    );
}
