use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn resumed_active_turn_defers_context_updates_until_it_finishes() {
    let path = std::env::temp_dir().join(format!(
        "roder-codex-active-context-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let script = r##"#!/bin/sh
set -eu
expect() {
    read -r line
    case "$line" in *"$1"*) ;; *) exit 1 ;; esac
}
expect '"method":"initialize"'
printf '%s\n' '{"id":1,"result":{}}'
expect '"method":"initialized"'
expect '"method":"thread/resume"'
printf '%s\n' '{"id":2,"result":{"thread":{"id":"thread-test","turns":[{"id":"turn-active","status":"inProgress","items":[]}]},"model":"gpt-6.2-sol","reasoningEffort":"low"}}'
expect '"method":"turn/steer"'
case "$line" in *'"expectedTurnId":"turn-active"'*) ;; *) exit 1 ;; esac
printf '%s\n' '{"id":3,"result":{"turnId":"turn-active"}}'
printf '%s\n' '{"method":"turn/completed","params":{"turn":{"id":"turn-active","status":"completed"}}}'
expect '"method":"thread/unsubscribe"'
printf '%s\n' '{"id":4,"result":{"status":"unsubscribed"}}'
expect '"method":"thread/resume"'
case "$line" in *'"model_context_window":200000'*) ;; *) exit 1 ;; esac
printf '%s\n' '{"id":5,"result":{"thread":{"id":"thread-test"}}}'
read -r line
"##;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut session = CodexBackend::with_program(path.clone())
        .connect(BackendStart {
            cwd: std::env::current_dir().unwrap(),
            model: None,
            resume_thread: Some("thread-test".into()),
            policy_mode: PolicyMode::Default,
        })
        .await
        .unwrap();
    session.set_context_budget(200_000, 180_000).await.unwrap();
    assert_eq!(
        session
            .send("steer".into(), None, None, None)
            .await
            .unwrap(),
        "turn-active"
    );
    assert!(matches!(
        session.next_event().await.unwrap(),
        BackendEvent::TurnFinished { .. }
    ));
    session.set_context_budget(200_000, 180_000).await.unwrap();
    session.set_context_budget(200_000, 180_000).await.unwrap();
    drop(session);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn context_updates_are_cached_deferred_during_turns_and_retried_on_failure() {
    let path = std::env::temp_dir().join(format!(
        "roder-codex-context-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let script = r##"#!/bin/sh
set -eu
expect() {
    read -r line
    case "$line" in *"$1"*) ;; *) exit 1 ;; esac
}
expect '"method":"initialize"'
printf '%s\n' '{"id":1,"result":{}}'
expect '"method":"initialized"'
expect '"method":"thread/start"'
printf '%s\n' '{"id":2,"result":{"thread":{"id":"thread-test"},"model":"gpt-6.2-sol","reasoningEffort":"low"}}'
expect '"method":"thread/unsubscribe"'
printf '%s\n' '{"id":3,"result":{"status":"unsubscribed"}}'
expect '"method":"thread/resume"'
case "$line" in *'"model_context_window":200000'*) ;; *) exit 1 ;; esac
case "$line" in *'"model_auto_compact_token_limit":180000'*) ;; *) exit 1 ;; esac
case "$line" in *'"threadId":"thread-test"'*) ;; *) exit 1 ;; esac
printf '%s\n' '{"id":4,"result":{"thread":{"id":"thread-test"}}}'
expect '"method":"turn/start"'
printf '%s\n' '{"id":5,"result":{"turn":{"id":"turn-test"}}}'
printf '%s\n' '{"method":"turn/started","params":{"turn":{"id":"turn-test"}}}'
expect '"method":"turn/steer"'
printf '%s\n' '{"id":6,"result":{"turnId":"turn-test"}}'
printf '%s\n' '{"method":"turn/completed","params":{"turn":{"id":"turn-test","status":"completed"}}}'
expect '"method":"thread/unsubscribe"'
printf '%s\n' '{"id":7,"result":{"status":"unsubscribed"}}'
expect '"method":"thread/resume"'
printf '%s\n' '{"id":8,"error":{"code":-32000,"message":"config rejected"}}'
expect '"method":"thread/unsubscribe"'
printf '%s\n' '{"id":9,"result":{"status":"notSubscribed"}}'
expect '"method":"thread/resume"'
case "$line" in *'"model_context_window":100000'*) ;; *) exit 1 ;; esac
case "$line" in *'"model_auto_compact_token_limit":90000'*) ;; *) exit 1 ;; esac
printf '%s\n' '{"id":10,"result":{"thread":{"id":"thread-test"}}}'
read -r line
"##;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut session = CodexBackend::with_program(path.clone())
        .connect(BackendStart {
            cwd: std::env::current_dir().unwrap(),
            model: Some("gpt-6.2-sol".into()),
            resume_thread: None,
            policy_mode: PolicyMode::Default,
        })
        .await
        .unwrap();
    session.set_context_budget(200_000, 180_000).await.unwrap();
    session.set_context_budget(200_000, 180_000).await.unwrap();
    session
        .send("start".into(), None, None, None)
        .await
        .unwrap();
    session.set_context_budget(100_000, 90_000).await.unwrap();
    session
        .send("steer".into(), None, None, None)
        .await
        .unwrap();
    assert!(matches!(
        session.next_event().await.unwrap(),
        BackendEvent::TurnStarted { .. }
    ));
    assert!(matches!(
        session.next_event().await.unwrap(),
        BackendEvent::TurnFinished { .. }
    ));
    assert!(
        session
            .set_context_budget(100_000, 90_000)
            .await
            .unwrap_err()
            .to_string()
            .contains("config rejected")
    );
    session.set_context_budget(100_000, 90_000).await.unwrap();
    session.set_context_budget(100_000, 90_000).await.unwrap();
    assert_eq!(session.thread_id(), "thread-test");
    drop(session);
    std::fs::remove_file(path).unwrap();
}
