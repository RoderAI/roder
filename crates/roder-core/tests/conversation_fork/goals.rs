use super::*;
use roder_api::goals::{ThreadGoalPatch, ThreadGoalStatus};

#[tokio::test]
async fn conversation_fork_inherits_goal_snapshot_independently() {
    let fixture = fixture("goal-snapshot");
    let parent_id = create_parent(&fixture).await;
    let mut expected = fixture
        .runtime
        .thread_goal_set(
            &parent_id,
            ThreadGoalPatch {
                objective: Some("Finish this task after resuming".into()),
                status: Some(ThreadGoalStatus::Paused),
                token_budget: Some(Some(100)),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let child = fixture
        .runtime
        .fork_thread(ForkThreadRequest::new(parent_id.clone(), "goal-fork"))
        .await
        .unwrap()
        .child;
    expected.thread_id = child.thread_id.clone();
    assert_eq!(
        fixture
            .runtime
            .thread_goal_get(&child.thread_id)
            .await
            .unwrap(),
        Some(expected.clone())
    );
    fixture.runtime.thread_goal_clear(&parent_id).await.unwrap();
    assert_eq!(
        fixture
            .runtime
            .thread_goal_get(&child.thread_id)
            .await
            .unwrap(),
        Some(expected)
    );
    fixture
        .runtime
        .remove_thread_workspace_fork(&child.thread_id, &child.workspace)
        .await
        .unwrap();
}
