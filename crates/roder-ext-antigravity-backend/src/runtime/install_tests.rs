use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use crate::runtime::tests::{Temp, digest};

async fn response(body: &[u8], status: &str, extra_headers: &str) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let bytes = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body.iter().copied())
    .collect::<Vec<_>>();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        let _ = socket.read(&mut request).await;
        socket.write_all(&bytes).await.unwrap();
    });
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn streaming_download_verifies_digest_and_rejects_redirects() {
    let temp = Temp::new();
    let path = temp.0.join("archive");
    download(
        response(b"payload", "200 OK", "").await,
        &path,
        &digest(b"payload"),
        &|_| {},
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"payload");
    std::fs::remove_file(&path).unwrap();
    let error = download(
        response(b"payload", "200 OK", "").await,
        &path,
        &"0".repeat(128),
        &|_| {},
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("SHA-512"));
    assert!(
        read_bounded(
            response(b"", "302 Found", "Location: https://evil.example/\r\n").await,
            100
        )
        .await
        .is_err()
    );
    assert!(
        read_bounded(response(b"oversized", "200 OK", "").await, 3)
            .await
            .is_err()
    );
}

#[test]
fn publication_is_atomic_and_preserves_previous_release_on_failure() {
    let root = Temp::new();
    let stage = Stage::new(&root.0).unwrap();
    let executable = stage.path.join(release::executable_name());
    std::fs::write(executable, b"first").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            stage.path.join(release::executable_name()),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    let first = publish(&root.0, stage, "1.5.18").unwrap();
    assert_eq!(
        crate::runtime::managed_runtime(&root.0).unwrap().unwrap(),
        first
    );
    let old_record = std::fs::read(root.0.join("active.json")).unwrap();
    let stage = Stage::new(&root.0).unwrap();
    assert!(publish(&root.0, stage, "1.5.19").is_err());
    assert_eq!(
        std::fs::read(root.0.join("active.json")).unwrap(),
        old_record
    );
    assert_eq!(std::fs::read(first).unwrap(), b"first");
}

#[test]
fn owned_staging_and_lock_cleanup() {
    let root = Temp::new();
    let stage = Stage::new(&root.0).unwrap();
    let path = stage.path.clone();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let sentinel = root.0.join("user-work");
    std::fs::write(&sentinel, "keep").unwrap();
    drop(stage);
    assert!(!path.exists());
    assert!(sentinel.exists());
    let lock = installation_lock(&root.0).unwrap();
    assert!(installation_lock(&root.0).is_err());
    drop(lock);
    // A process another test forks in this instant briefly shares the lock's
    // open file until its exec closes it, so allow a short retry.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while installation_lock(&root.0).is_err() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(installation_lock(&root.0).is_ok());
}

#[tokio::test]
async fn cancellation_removes_partial_download_and_cannot_publish() {
    let root = Temp::new();
    let stage = Stage::new(&root.0).unwrap();
    let path = stage.path.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        let _ = socket.read(&mut request).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\npartial")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
        let _ = ready_tx.send(());
        let result = download(
            response,
            &stage.path.join("download"),
            &"0".repeat(128),
            &|_| {},
        )
        .await;
        drop(stage);
        result
    });
    ready_rx.await.unwrap();
    worker.abort();
    let _ = worker.await;
    server.abort();
    assert!(!path.exists());
    assert!(!root.0.join("active.json").exists());
}

#[tokio::test]
async fn cancellation_during_blocking_extraction_cleans_after_worker_exits() {
    let root = Temp::new();
    let stage = Stage::new(&root.0).unwrap();
    let path = stage.path.clone();
    let root_path = root.0.clone();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let stage = tokio::task::spawn_blocking(move || {
            let _ = ready_tx.send(());
            release_rx.recv().unwrap();
            stage
        })
        .await
        .unwrap();
        publish(&root_path, stage, "1.5.18")
    });
    ready_rx.await.unwrap();
    task.abort();
    let _ = task.await;
    assert!(
        path.exists(),
        "blocking worker still owns its staging directory"
    );
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!root.0.join("active.json").exists());
}
