use super::*;
use std::os::unix::fs::PermissionsExt;

fn script(directory: &std::path::Path, body: &str) -> CuaConfig {
    let path = directory.join("client");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    CuaConfig {
        backend: crate::CuaBackend::LocalMacos,
        program: Some(path.to_str().unwrap().into()),
        socket_path: Some(directory.join("daemon.sock").to_str().unwrap().into()),
        ..Default::default()
    }
}
async fn wait_file(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_and_timeout_keep_native_dispatch_serialized() {
    let temp = tempfile::tempdir().unwrap();
    let start = temp.path().join("started");
    let done = temp.path().join("done");
    let release = temp.path().join("release");
    let body = format!(
        r#"
if [ "$1" = --version ]; then printf 'cua-driver 0.34.0\n'; exit; fi
if [ "$2" = press_key ]; then
    touch '{start}'
    while [ ! -f '{release}' ]; do /bin/sleep 0.01; done
    touch '{done}'
else
    test -f '{done}' || exit 1
fi
printf '{{"delivered":true}}\n'
"#,
        start = start.display(),
        done = done.display(),
        release = release.display()
    );
    let config = script(temp.path(), &body);
    let _socket =
        std::os::unix::net::UnixListener::bind(config.local_socket_path().unwrap()).unwrap();
    let socket_path = config.local_socket_path().unwrap();
    let transport = Arc::new(LocalMacosTransport::new(config));
    let lease = LocalDesktopLease::acquire().await;
    verify_daemon_generation(&socket_path, false, 0).unwrap();
    let pending_transport = transport.clone();
    let pending_lease = lease.clone();
    let pending = tokio::spawn(async move {
        pending_transport
            .call(
                CuaTarget::LocalMacos(&pending_lease),
                "press_key",
                json(),
                1000,
            )
            .await
    });
    wait_file(&start).await;
    let result = pending.await.unwrap();
    assert!(result.is_err());
    assert!(start.exists() && !done.exists());
    // An after-action call on the same lease waits for the original worker.
    std::fs::write(&release, b"go").unwrap();
    assert!(
        !transport
            .call(
                CuaTarget::LocalMacos(&lease),
                "get_window_state",
                json(),
                1000
            )
            .await
            .unwrap()
            .is_error
    );
    std::fs::remove_file(&done).unwrap();
    std::fs::remove_file(&start).unwrap();
    std::fs::remove_file(&release).unwrap();
    let task = tokio::spawn(async move {
        transport
            .call(CuaTarget::LocalMacos(&lease), "press_key", json(), 1000)
            .await
    });
    wait_file(&start).await;
    task.abort();
    let _ = task.await;
    assert!(
        tokio::time::timeout(Duration::from_millis(40), LocalDesktopLease::acquire())
            .await
            .is_err()
    );
    std::fs::write(&release, b"go").unwrap();
    let next = tokio::time::timeout(Duration::from_secs(2), LocalDesktopLease::acquire())
        .await
        .unwrap();
    assert!(done.exists());
    drop(next);
}

#[tokio::test]
async fn cancellation_during_version_probe_never_dispatches_input() {
    let temp = tempfile::tempdir().unwrap();
    let probed = temp.path().join("probed");
    let dispatched = temp.path().join("dispatched");
    let release = temp.path().join("release");
    let config = script(
        temp.path(),
        &format!(
            r#"
if [ "$1" = --version ]; then
    touch '{}'
    while [ ! -f '{}' ]; do /bin/sleep 0.01; done
    printf 'cua-driver 0.34.0\n'
else
    touch '{}'
    printf '{{}}\n'
fi
"#,
            probed.display(),
            release.display(),
            dispatched.display()
        ),
    );
    let lease = LocalDesktopLease::acquire().await;
    let task = tokio::spawn(async move {
        LocalMacosTransport::new(config)
            .call(CuaTarget::LocalMacos(&lease), "press_key", json(), 1000)
            .await
    });
    wait_file(&probed).await;
    task.abort();
    let _ = task.await;
    std::fs::write(&release, b"go").unwrap();
    let next = tokio::time::timeout(Duration::from_secs(2), LocalDesktopLease::acquire())
        .await
        .unwrap();
    assert!(!dispatched.exists());
    drop(next);
}

#[tokio::test]
async fn command_bounds_output_and_deadline_and_does_not_use_a_shell() {
    let temp = tempfile::tempdir().unwrap();
    let config = script(temp.path(), "printf '%s' \"$1\"");
    let (_, output) = command(
        config.program(),
        &["$(touch nope); $HOME".into()],
        1000,
        100,
    )
    .await
    .unwrap();
    assert_eq!(output, b"$(touch nope); $HOME");
    assert!(
        command(config.program(), &["oversize".into()], 1000, 3)
            .await
            .is_err()
    );
    let slow = script(temp.path(), "/bin/sleep 1");
    assert!(command(slow.program(), &[], 20, 100).await.is_err());
}
fn json() -> Value {
    serde_json::json!({"session":"roder-test"})
}

#[tokio::test]
async fn socket_replacement_invalidates_pre_restart_input() {
    let _lease = LocalDesktopLease::acquire().await;
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("driver.sock");
    let path = socket.to_str().unwrap();
    let first = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    verify_daemon_generation(path, false, 0).unwrap();
    let before = DESKTOP_GENERATION.load(Ordering::SeqCst);
    drop(first);
    std::fs::remove_file(&socket).unwrap();
    let _second = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    assert!(verify_daemon_generation(path, true, before).is_err());
    verify_daemon_generation(path, false, before).unwrap();
}

#[tokio::test]
async fn text_only_refusal_keeps_observation_available_but_lost_reply_blocks_dispatch() {
    let lease = LocalDesktopLease::acquire().await;
    let temp = tempfile::tempdir().unwrap();
    let config = script(
        temp.path(),
        r#"
if [ "$1" = --version ]; then printf 'cua-driver 0.34.0\n'; exit; fi
case "$2" in
    click) printf 'AXPress is unavailable for this text field\n'; exit 1 ;;
    press_key) exit 1 ;;
    *) printf '{"capture_id":"read"}\n' ;;
esac
"#,
    );
    let _socket =
        std::os::unix::net::UnixListener::bind(config.local_socket_path().unwrap()).unwrap();
    let transport = LocalMacosTransport::new(config);
    let read = transport
        .call(
            CuaTarget::LocalMacos(&lease),
            "get_window_state",
            json(),
            1000,
        )
        .await
        .unwrap();
    assert!(!read.is_error);
    let refusal = transport
        .call(CuaTarget::LocalMacos(&lease), "click", json(), 1000)
        .await
        .unwrap();
    assert!(refusal.is_error);
    assert!(
        refusal.observation["summary"]
            .as_str()
            .unwrap()
            .contains("AXPress")
    );
    assert!(
        transport
            .call(
                CuaTarget::LocalMacos(&lease),
                "get_window_state",
                json(),
                1000
            )
            .await
            .is_ok()
    );
    assert!(
        transport
            .call(CuaTarget::LocalMacos(&lease), "press_key", json(), 1000)
            .await
            .is_err()
    );
    let blocked = transport
        .call(
            CuaTarget::LocalMacos(&lease),
            "get_window_state",
            json(),
            1000,
        )
        .await
        .err()
        .unwrap();
    assert!(
        blocked
            .to_string()
            .contains("restart CuaDriver.app and Roder")
    );
    DESKTOP_UNCERTAIN.store(false, Ordering::SeqCst);
}
