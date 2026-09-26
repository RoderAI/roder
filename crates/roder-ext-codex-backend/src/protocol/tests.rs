use super::*;

#[test]
fn maps_codex_tool_and_text_events() {
    let command =
        json!({"type":"commandExecution","id":"i1","command":"cargo test","status":"inProgress"});
    assert_eq!(
        tool_event(&command, false),
        Some(BackendEvent::ToolStarted {
            id: "i1".into(),
            label: "cargo test".into(),
        })
    );
    assert_eq!(
        tool_event(&command, true),
        Some(BackendEvent::ToolFinished {
            id: "i1".into(),
            label: "cargo test".into(),
            summary: "cargo test: inProgress".into(),
        })
    );
    assert_eq!(
        tool_event(
            &json!({"type":"agentMessage","id":"i2","text":"hello"}),
            true
        ),
        None
    );
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_handshake_and_turn_stream() {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!(
        "roder-codex-fake-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let script = r##"#!/bin/sh
read line
printf '%s\n' '{"id":1,"result":{"userAgent":"fake"}}'
read line
read line
printf '%s\n' '{"id":2,"result":{"thread":{"id":"thread-test"}}}'
read line
printf '%s\n' '{"id":3,"result":{"turn":{"id":"turn-test"}}}'
printf '%s\n' '{"method":"turn/started","params":{"turn":{"id":"turn-test"}}}'
printf '%s\n' '{"method":"item/agentMessage/delta","params":{"itemId":"item-test","delta":"hello"}}'
printf '%s\n' '{"method":"turn/completed","params":{"turn":{"id":"turn-test","status":"completed"}}}'
read line
"##;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let backend = CodexBackend::with_program(path.clone());
    let mut session = backend
        .connect(BackendStart {
            cwd: std::env::current_dir().unwrap(),
            model: None,
            resume_thread: None,
            policy_mode: PolicyMode::Default,
        })
        .await
        .unwrap();
    assert_eq!(session.thread_id(), "thread-test");
    assert_eq!(
        session.send("hi".into(), None, None, None).await.unwrap(),
        "turn-test"
    );
    assert_eq!(
        session.next_event().await.unwrap(),
        BackendEvent::TurnStarted {
            turn_id: "turn-test".into()
        }
    );
    assert_eq!(
        session.next_event().await.unwrap(),
        BackendEvent::Text("hello".into())
    );
    assert_eq!(
        session.next_event().await.unwrap(),
        BackendEvent::TurnFinished {
            turn_id: "turn-test".into(),
            status: "completed".into()
        }
    );
    drop(session);
    std::fs::remove_file(path).unwrap();
}
