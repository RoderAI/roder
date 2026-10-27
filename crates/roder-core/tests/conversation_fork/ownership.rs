use super::*;

#[tokio::test]
async fn conversation_fork_records_actual_creator_and_child_before_publication() {
    let fixture = fixture("owner");
    let parent = create_parent(&fixture).await;
    let outcome = fixture
        .runtime
        .fork_thread(ForkThreadRequest::new(parent.clone(), "owned"))
        .await
        .unwrap();
    let fork = outcome.child.workspace_fork.as_ref().unwrap();
    let owner = fork.ownership.as_ref().unwrap();
    assert_eq!(owner.creator_thread_id.as_ref(), Some(&parent));
    assert_eq!(
        owner.owner_thread_id.as_ref(),
        Some(&outcome.child.thread_id)
    );
    let resumed = fixture
        .runtime
        .resume_workspace_fork(&fork.provider_id, &fork.id)
        .await
        .unwrap();
    assert_eq!(resumed.ownership, fork.ownership);
    fixture
        .runtime
        .remove_thread_workspace_fork(&outcome.child.thread_id, &outcome.child.workspace)
        .await
        .unwrap();
    let _ = std::fs::remove_dir_all(fixture.repo);
}

#[tokio::test]
async fn invalid_conversation_boundary_allocates_no_worktree_or_branch() {
    let fixture = fixture("invalid-boundary");
    let parent = create_parent(&fixture).await;
    let mut request = ForkThreadRequest::new(parent, "must-not-exist");
    request.from_turn_id = Some("unknown-turn".into());
    let error = fixture.runtime.fork_thread(request).await.unwrap_err();
    assert!(error.to_string().contains("was not found"), "{error}");
    assert!(
        fixture
            .runtime
            .list_workspace_forks("git-worktree", &fixture.repo)
            .await
            .unwrap()
            .is_empty()
    );
    let branches = Command::new("git")
        .arg("-C")
        .arg(&fixture.repo)
        .args(["branch", "--list", "roder/fork/*"])
        .output()
        .unwrap();
    assert!(branches.status.success());
    assert!(branches.stdout.is_empty());
    let _ = std::fs::remove_dir_all(fixture.repo);
}
