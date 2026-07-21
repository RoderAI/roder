mod fake_server;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerSession, RunnerCommandRequest, RunnerCommandResult,
    RunnerDestination, RunnerManifest,
};
use roder_ext_runner_blaxel::config::TOKEN_ENV;
use roder_ext_runner_blaxel::{BlaxelRunnerProvider, PROVIDER_ID};

use fake_server::FakeBlaxelServer;

const COMMAND_TAG_ENV: &str = "RODER_BLAXEL_COMMAND_TAG";
static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn create_session(server: &FakeBlaxelServer) -> Arc<dyn RemoteRunnerSession> {
    BlaxelRunnerProvider::default()
        .create_session(RunnerDestination {
            id: "blaxel-cancellation".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            config: serde_json::json!({
                "base_url": server.base_url(),
                "working_dir": "/home/user/roder",
                "cleanup": "delete-on-close"
            }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap()
}

async fn start_detached_command(
    server: &FakeBlaxelServer,
    session: &Arc<dyn RemoteRunnerSession>,
    command_id: &str,
    marker: &str,
    env: Vec<(String, String)>,
) -> tokio::task::JoinHandle<anyhow::Result<RunnerCommandResult>> {
    let command_session = session.clone();
    let command_id = command_id.to_string();
    let marker = marker.to_string();
    let command = tokio::spawn(async move {
        command_session
            .run_command(RunnerCommandRequest {
                command_id,
                program: "detached-signal-ignoring".to_string(),
                args: vec![marker],
                cwd: None,
                env,
                timeout_ms: Some(10_000),
            })
            .await
    });
    wait_for(
        || server.has_tagged_descendant(),
        "tagged detached descendant was not registered",
    )
    .await;
    command
}

async fn wait_for(mut condition: impl FnMut() -> bool, message: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect(message);
}

fn process_bodies(server: &FakeBlaxelServer) -> Vec<serde_json::Value> {
    server
        .requests()
        .into_iter()
        .filter(|request| request.starts_with("POST /process "))
        .filter_map(|request| {
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap_or_default()).ok()
        })
        .collect()
}

fn tombstone_was_deleted(server: &FakeBlaxelServer) -> bool {
    server.requests().iter().any(|request| {
        request.starts_with("DELETE /filesystem/") && request.contains("roder-cancelled-processes")
    })
}

#[tokio::test]
async fn cancellation_overrides_the_user_tag_and_waits_for_untagged_cleanup() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    let session = create_session(&server).await;
    let command = start_detached_command(
        &server,
        &session,
        "cancel-tagged",
        "cancel-tagged-leak.txt",
        vec![(COMMAND_TAG_ENV.to_string(), "user-controlled".to_string())],
    )
    .await;

    let command_id = "cancel-tagged".to_string();
    let (first_cancel, second_cancel) = tokio::join!(
        session.cancel_command(&command_id),
        session.cancel_command(&command_id)
    );
    assert!(first_cancel.unwrap());
    assert!(second_cancel.unwrap());
    let output = command.await.unwrap().unwrap();
    assert_eq!(output.exit_code, None);
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("cancel-tagged-leak.txt"));
    assert!(!server.has_tagged_descendant());

    let bodies = process_bodies(&server);
    let user = bodies
        .iter()
        .find(|body| {
            body["command"]
                .as_str()
                .is_some_and(|command| command.contains("detached-signal-ignoring"))
        })
        .unwrap();
    let tag = user["env"][COMMAND_TAG_ENV].as_str().unwrap();
    assert_ne!(tag, "user-controlled");
    let cleanup = bodies
        .iter()
        .find(|body| {
            body["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("roder-cleanup-"))
        })
        .unwrap();
    assert_eq!(
        bodies
            .iter()
            .filter(|body| {
                body["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("roder-cleanup-"))
            })
            .count(),
        1
    );
    assert!(cleanup.get("env").is_none());
    assert!(
        cleanup["command"]
            .as_str()
            .is_some_and(|command| command.contains(&format!("{COMMAND_TAG_ENV}={tag}")))
    );

    let requests = server.requests();
    let cleanup_started = requests
        .iter()
        .position(|request| request.contains("roder-cleanup-"))
        .unwrap();
    let tombstone_deleted = requests
        .iter()
        .position(|request| {
            request.starts_with("DELETE /filesystem/")
                && request.contains("roder-cancelled-processes")
        })
        .unwrap();
    assert!(cleanup_started < tombstone_deleted);

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn failed_cleanup_returns_false_and_keeps_cancellation_retryable() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    server.fail_descendant_cleanup(true);
    let session = create_session(&server).await;
    let command = start_detached_command(
        &server,
        &session,
        "retry-cleanup",
        "retry-cleanup-leak.txt",
        Vec::new(),
    )
    .await;

    assert!(
        !session
            .cancel_command(&"retry-cleanup".to_string())
            .await
            .unwrap()
    );
    command.await.unwrap().unwrap();
    assert!(server.has_tagged_descendant());
    assert!(!tombstone_was_deleted(&server));

    server.fail_descendant_cleanup(false);
    assert!(
        session
            .cancel_command(&"retry-cleanup".to_string())
            .await
            .unwrap()
    );
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("retry-cleanup-leak.txt"));
    assert!(tombstone_was_deleted(&server));
    assert_eq!(
        process_bodies(&server)
            .iter()
            .filter(|body| {
                body["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("roder-cleanup-"))
            })
            .count(),
        2
    );

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn timed_out_public_cancel_keeps_provider_owned_cleanup_running() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    server.set_cleanup_polls_before_terminal(8);
    let session = create_session(&server).await;
    let command = start_detached_command(
        &server,
        &session,
        "cancel-timeout",
        "cancel-timeout-leak.txt",
        Vec::new(),
    )
    .await;

    let timed_out = tokio::time::timeout(
        Duration::from_millis(25),
        session.cancel_command(&"cancel-timeout".to_string()),
    )
    .await;
    assert!(timed_out.is_err());
    command.await.unwrap().unwrap();
    wait_for(|| tombstone_was_deleted(&server), "cleanup did not finish").await;
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("cancel-timeout-leak.txt"));
    assert!(
        !session
            .cancel_command(&"cancel-timeout".to_string())
            .await
            .unwrap()
    );

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn dropped_run_command_uses_the_same_descendant_cleanup_path() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    let session = create_session(&server).await;
    let command = start_detached_command(
        &server,
        &session,
        "drop-run",
        "drop-run-leak.txt",
        Vec::new(),
    )
    .await;

    command.abort();
    let _ = command.await;
    wait_for(
        || tombstone_was_deleted(&server),
        "drop cleanup did not finish",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("drop-run-leak.txt"));
    assert!(!server.has_tagged_descendant());

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn terminal_result_without_exit_code_still_cleans_descendants() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    let session = create_session(&server).await;
    let command = start_detached_command(
        &server,
        &session,
        "server-killed",
        "server-killed-leak.txt",
        Vec::new(),
    )
    .await;

    server.terminate_user_processes_without_exit();
    let output = command.await.unwrap().unwrap();
    assert_eq!(output.exit_code, None);
    wait_for(
        || tombstone_was_deleted(&server),
        "terminal result did not trigger cleanup",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("server-killed-leak.txt"));
    assert!(!server.has_tagged_descendant());

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}
