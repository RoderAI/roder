mod fake_server;

use std::sync::Arc;
use std::time::Duration;

use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerSession, RunnerCommandRequest, RunnerCommandResult,
    RunnerDestination, RunnerManifest,
};
use roder_ext_runner_blaxel::config::TOKEN_ENV;
use roder_ext_runner_blaxel::{BlaxelRunnerProvider, PROVIDER_ID};
use tokio::sync::Mutex;

use fake_server::FakeBlaxelServer;

const COMMAND_TAG_ENV: &str = "RODER_BLAXEL_COMMAND_TAG";
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

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
    let _guard = ENV_LOCK.lock().await;
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
        2
    );
    assert!(cleanup.get("env").is_none());
    assert!(
        cleanup["command"]
            .as_str()
            .is_some_and(|command| command.contains(&format!("{COMMAND_TAG_ENV}={tag}")))
    );

    assert!(
        session.cancel_command(&command_id).await.unwrap(),
        "a confirmed cancellation must remain idempotently acknowledged"
    );
    assert!(
        !tombstone_was_deleted(&server),
        "the acknowledgement retention window must keep its unique tombstone"
    );

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn background_success_racing_a_retry_returns_true_and_stays_idempotent() {
    let _guard = ENV_LOCK.lock().await;
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    server.fail_next_descendant_cleanups(2);
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
    assert!(!tombstone_was_deleted(&server));

    wait_for(
        || !server.has_tagged_descendant(),
        "the prompt background sweep did not start",
    )
    .await;
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            session.cancel_command(&"retry-cleanup".to_string())
        )
        .await
        .expect("the retry should join or observe the background acknowledgement")
        .unwrap(),
        "a background acknowledgement must win over an in-flight retry's result"
    );
    command.await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("retry-cleanup-leak.txt"));
    assert!(
        session
            .cancel_command(&"retry-cleanup".to_string())
            .await
            .unwrap(),
        "a successful retry must leave an idempotent acknowledgement"
    );
    assert!(
        !tombstone_was_deleted(&server),
        "successful cancellation retains its tombstone during acknowledgement retention"
    );

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn timed_out_public_cancel_keeps_provider_owned_cleanup_running() {
    let _guard = ENV_LOCK.lock().await;
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
    wait_for(
        || !server.has_tagged_descendant(),
        "provider-owned cleanup did not reap the descendant",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("cancel-timeout-leak.txt"));
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            session.cancel_command(&"cancel-timeout".to_string())
        )
        .await
        .expect("the provider-owned cancellation should settle")
        .unwrap(),
        "a later retry must observe the retained acknowledgement"
    );
    assert!(!tombstone_was_deleted(&server));

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn dropped_run_command_uses_the_same_descendant_cleanup_path() {
    let _guard = ENV_LOCK.lock().await;
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
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            session.cancel_command(&"drop-run".to_string())
        )
        .await
        .expect("drop cleanup did not finish")
        .unwrap()
    );
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("drop-run-leak.txt"));
    assert!(!server.has_tagged_descendant());
    assert!(!tombstone_was_deleted(&server));

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn terminal_result_without_exit_code_still_cleans_descendants() {
    let _guard = ENV_LOCK.lock().await;
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
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            session.cancel_command(&"server-killed".to_string())
        )
        .await
        .expect("terminal result did not trigger cleanup")
        .unwrap()
    );
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("server-killed-leak.txt"));
    assert!(!server.has_tagged_descendant());
    assert!(!tombstone_was_deleted(&server));

    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn an_old_guard_cannot_cancel_a_reused_command_id_generation() {
    let _guard = ENV_LOCK.lock().await;
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    server.delay_killed_user_process_for_polls(20);
    let session = create_session(&server).await;
    let first = start_detached_command(
        &server,
        &session,
        "reused-id",
        "first-generation-leak.txt",
        Vec::new(),
    )
    .await;

    assert!(
        session
            .cancel_command(&"reused-id".to_string())
            .await
            .unwrap()
    );
    assert!(
        !first.is_finished(),
        "the fake must keep generation A's run future alive for the guard race"
    );

    let second = start_detached_command(
        &server,
        &session,
        "reused-id",
        "second-generation-leak.txt",
        Vec::new(),
    )
    .await;
    let second_name = process_bodies(&server)
        .into_iter()
        .rfind(|body| {
            body["command"]
                .as_str()
                .is_some_and(|command| command.contains("detached-signal-ignoring"))
        })
        .and_then(|body| body["name"].as_str().map(str::to_string))
        .unwrap();

    first.abort();
    let _ = first.await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !server.requests().iter().any(|request| {
            request.starts_with(&format!("DELETE /process/{second_name}/kill "))
        }),
        "generation A's dropped guard must not cancel generation B"
    );
    assert!(!second.is_finished());

    assert!(
        session
            .cancel_command(&"reused-id".to_string())
            .await
            .unwrap()
    );
    second.abort();
    let _ = second.await;
    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}

#[tokio::test]
async fn cleanup_starts_before_named_process_settlement_finishes() {
    let _guard = ENV_LOCK.lock().await;
    unsafe { std::env::set_var(TOKEN_ENV, "test-token") };
    let server = FakeBlaxelServer::start().await;
    server.fail_named_process_kill(true);
    let session = create_session(&server).await;
    let command = start_detached_command(
        &server,
        &session,
        "slow-settlement",
        "slow-settlement-leak.txt",
        Vec::new(),
    )
    .await;

    let timed_out = tokio::time::timeout(
        Duration::from_millis(25),
        session.cancel_command(&"slow-settlement".to_string()),
    )
    .await;
    assert!(timed_out.is_err());
    wait_for(
        || {
            process_bodies(&server).iter().any(|body| {
                body["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("roder-cleanup-"))
            })
        },
        "early descendant cleanup did not start",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(!server.has_file("slow-settlement-leak.txt"));
    assert!(!server.has_tagged_descendant());
    assert!(!tombstone_was_deleted(&server));

    command.abort();
    let _ = command.await;
    session.close().await.unwrap();
    unsafe { std::env::remove_var(TOKEN_ENV) };
}
