//! Runtime conversation-fork tests (roadmap phase 90, Task 3). All tests use
//! temporary local Git repositories and the offline fake provider; no
//! network access.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use roder_api::events::{
    EventEnvelope, EventSource, RoderEvent, TranscriptItemAppended, TurnCompleted, TurnStarted,
};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::thread::{ThreadStore, ThreadStoreFactory, WorktreeForkStatus};
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_core::conversation_forks::ForkThreadRequest;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{CreateThreadRequest, Runtime, RuntimeConfig, StartTurnRequest};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use time::OffsetDateTime;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roder-conversation-fork-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_repo(label: &str) -> PathBuf {
    let root = temp_dir(label);
    git(&root, &["init", "--initial-branch", "main"]);
    git(&root, &["config", "user.email", "test@roder.dev"]);
    git(&root, &["config", "user.name", "Roder Test"]);
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    root.canonicalize().unwrap()
}

struct Fixture {
    runtime: Arc<Runtime>,
    store: Arc<dyn ThreadStore>,
    repo: PathBuf,
}

fn fixture(label: &str) -> Fixture {
    let repo = temp_repo(label);
    let thread_dir = temp_dir(&format!("{label}-threads"));
    let factory = JsonlThreadStoreFactory {
        base_path: thread_dir,
    };
    let store = factory.create();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(factory));
    builder.tool_contributor(
        roder_tools::builtin_coding_tools_contributor(&repo).expect("coding tools"),
    );
    // Bypass tool approvals so the offline fake-provider write turn can
    // complete without an interactive approval client.
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    Fixture {
        runtime,
        store,
        repo,
    }
}

async fn create_parent(fixture: &Fixture) -> String {
    fixture
        .runtime
        .create_thread_with(CreateThreadRequest {
            title: Some("Parent".to_string()),
            workspace: fixture.repo.display().to_string(),
            workspace_id: None,
            root_id: None,
            provider: None,
            model: None,
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: None,
        })
        .await
        .unwrap()
        .thread_id
}

fn envelope(thread_id: &str, seq: u64, turn_id: &str, event: RoderEvent) -> EventEnvelope {
    EventEnvelope {
        event_id: format!("seed-{seq}"),
        seq,
        timestamp: OffsetDateTime::UNIX_EPOCH,
        source: EventSource::Core,
        kind: event.kind().to_string(),
        thread_id: Some(thread_id.to_string()),
        turn_id: Some(turn_id.to_string()),
        event,
    }
}

async fn seed_parent_turn(
    store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    seq: u64,
    turn_id: &str,
    text: &str,
) {
    let events = vec![
        envelope(
            thread_id,
            seq,
            turn_id,
            RoderEvent::TurnStarted(TurnStarted {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.to_string(),
                runtime_profile: Default::default(),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ),
        envelope(
            thread_id,
            seq + 1,
            turn_id,
            RoderEvent::TranscriptItemAppended(TranscriptItemAppended {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.to_string(),
                item_type: "user_message".to_string(),
                item_index: None,
                item: Some(TranscriptItem::UserMessage(UserMessage::text(text))),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ),
        envelope(
            thread_id,
            seq + 2,
            turn_id,
            RoderEvent::TurnCompleted(TurnCompleted {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.to_string(),
                usage: None,
                finish_reason: Some("stop".to_string()),
                timestamp: OffsetDateTime::UNIX_EPOCH,
            }),
        ),
    ];
    for event in events {
        store
            .append_event(&thread_id.to_string(), &event)
            .await
            .unwrap();
    }
}

async fn wait_for_turn_completion(runtime: &Arc<Runtime>, thread_id: &str, turn_id: &str) {
    for _ in 0..400 {
        let snapshot = runtime
            .load_thread(&thread_id.to_string())
            .await
            .unwrap()
            .expect("thread snapshot");
        if snapshot
            .turns
            .iter()
            .any(|turn| turn.turn_id == turn_id && turn.completed_at.is_some())
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("turn {turn_id} did not complete in time");
}

#[tokio::test(flavor = "multi_thread")]
async fn conversation_fork_creates_isolated_child_with_seeded_transcript() {
    let fixture = fixture("seeded");
    let parent_id = create_parent(&fixture).await;
    seed_parent_turn(&fixture.store, &parent_id, 1, "turn-1", "hello from parent").await;

    let outcome = fixture
        .runtime
        .fork_thread_worktree(ForkThreadRequest {
            parent_thread_id: parent_id.clone(),
            name: "experiment".to_string(),
            from_turn_id: None,
        })
        .await
        .unwrap();

    let child = &outcome.child;
    assert_eq!(child.parent_thread_id.as_deref(), Some(parent_id.as_str()));
    let fork = child.worktree_fork.as_ref().expect("fork provenance");
    assert_eq!(fork.status, WorktreeForkStatus::Active);
    assert_eq!(fork.source_workspace, fixture.repo.display().to_string());
    assert_eq!(child.workspace, fork.worktree_path);
    assert_ne!(child.workspace, fixture.repo.display().to_string());
    assert!(
        Path::new(&child.workspace).join("README.md").exists(),
        "worktree carries committed source files"
    );

    // The child transcript is seeded from the parent conversation.
    let snapshot = fixture
        .runtime
        .load_thread(&child.thread_id)
        .await
        .unwrap()
        .expect("child snapshot");
    let seeded_texts: Vec<String> = snapshot
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter_map(|item| match item {
            TranscriptItem::UserMessage(message) => Some(message.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(seeded_texts, vec!["hello from parent".to_string()]);

    // A tool write in the child stays inside the child worktree.
    let turn_id = fixture
        .runtime
        .start_turn(StartTurnRequest {
            thread_id: child.thread_id.clone(),
            message: "FAKE_WRITE_FILE please".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: fixture
                .runtime
                .workspace_for_thread(&child.thread_id)
                .await
                .unwrap(),
            instructions: roder_core::default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();
    wait_for_turn_completion(&fixture.runtime, &child.thread_id, &turn_id).await;

    assert!(
        Path::new(&child.workspace).join("src/lib.rs").exists(),
        "tool write lands in the child worktree"
    );
    assert!(
        !fixture.repo.join("src/lib.rs").exists(),
        "parent workspace must remain untouched"
    );

    // Parent metadata still points at the original workspace.
    let parent_metadata = fixture
        .runtime
        .load_thread_metadata(&parent_id)
        .await
        .unwrap()
        .expect("parent metadata");
    assert_eq!(
        parent_metadata.workspace,
        fixture.repo.display().to_string()
    );
    assert!(parent_metadata.worktree_fork.is_none());

    let _ = std::fs::remove_dir_all(&fixture.repo);
}

#[tokio::test(flavor = "multi_thread")]
async fn conversation_fork_truncates_at_requested_turn() {
    let fixture = fixture("truncate");
    let parent_id = create_parent(&fixture).await;
    seed_parent_turn(&fixture.store, &parent_id, 1, "turn-1", "first message").await;
    seed_parent_turn(&fixture.store, &parent_id, 10, "turn-2", "second message").await;

    let outcome = fixture
        .runtime
        .fork_thread_worktree(ForkThreadRequest {
            parent_thread_id: parent_id.clone(),
            name: "from-turn-1".to_string(),
            from_turn_id: Some("turn-1".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(
        outcome.child.forked_from_turn_id.as_deref(),
        Some("turn-1")
    );
    let snapshot = fixture
        .runtime
        .load_thread(&outcome.child.thread_id)
        .await
        .unwrap()
        .unwrap();
    let texts: Vec<String> = snapshot
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter_map(|item| match item {
            TranscriptItem::UserMessage(message) => Some(message.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["first message".to_string()]);

    // Forking at an unknown turn fails closed without creating a thread.
    let error = fixture
        .runtime
        .fork_thread_worktree(ForkThreadRequest {
            parent_thread_id: parent_id.clone(),
            name: "bad-turn".to_string(),
            from_turn_id: Some("missing".to_string()),
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing"), "{error}");

    let _ = std::fs::remove_dir_all(&fixture.repo);
}

#[tokio::test(flavor = "multi_thread")]
async fn conversation_fork_removal_is_path_confirmed_and_missing_worktrees_fail_closed() {
    let fixture = fixture("removal");
    let parent_id = create_parent(&fixture).await;

    let outcome = fixture
        .runtime
        .fork_thread_worktree(ForkThreadRequest {
            parent_thread_id: parent_id.clone(),
            name: "short-lived".to_string(),
            from_turn_id: None,
        })
        .await
        .unwrap();
    let child_id = outcome.child.thread_id.clone();
    let worktree_path = outcome.child.workspace.clone();

    // Removal requires the exact worktree path as confirmation.
    let error = fixture
        .runtime
        .remove_thread_worktree_fork(&child_id, "/wrong/path")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("path-confirmed"), "{error}");
    assert!(Path::new(&worktree_path).exists());

    let removed = fixture
        .runtime
        .remove_thread_worktree_fork(&child_id, &worktree_path)
        .await
        .unwrap();
    assert_eq!(removed.status, WorktreeForkStatus::Removed);
    assert!(!Path::new(&worktree_path).exists());

    // The conversation is preserved; metadata records the removed status.
    let metadata = fixture
        .runtime
        .load_thread_metadata(&child_id)
        .await
        .unwrap()
        .expect("child metadata after removal");
    assert_eq!(
        metadata.worktree_fork.as_ref().map(|fork| fork.status),
        Some(WorktreeForkStatus::Removed)
    );

    // Double removal fails closed.
    let error = fixture
        .runtime
        .remove_thread_worktree_fork(&child_id, &worktree_path)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("already removed"), "{error}");

    // A fork whose worktree disappeared out-of-band fails closed before any
    // write: workspace resolution names the fork and the missing path.
    let second = fixture
        .runtime
        .fork_thread_worktree(ForkThreadRequest {
            parent_thread_id: parent_id.clone(),
            name: "orphaned".to_string(),
            from_turn_id: None,
        })
        .await
        .unwrap();
    std::fs::remove_dir_all(&second.child.workspace).unwrap();
    let error = fixture
        .runtime
        .workspace_for_thread(&second.child.thread_id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing its worktree"), "{error}");
    assert!(
        error
            .to_string()
            .contains(&second.child.worktree_fork.as_ref().unwrap().fork_id),
        "{error}"
    );

    let _ = std::fs::remove_dir_all(&fixture.repo);
}

#[tokio::test(flavor = "multi_thread")]
async fn conversation_fork_refuses_dirty_parent_workspace() {
    let fixture = fixture("dirty");
    let parent_id = create_parent(&fixture).await;
    std::fs::write(fixture.repo.join("wip.txt"), "uncommitted").unwrap();

    let error = fixture
        .runtime
        .fork_thread_worktree(ForkThreadRequest {
            parent_thread_id: parent_id,
            name: "blocked".to_string(),
            from_turn_id: None,
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("uncommitted changes"), "{error}");
    let _ = std::fs::remove_dir_all(&fixture.repo);
}
