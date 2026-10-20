use std::{sync::Arc, time::Duration};

use roder_ext_runner_blaxel::{Redacted, forks::BlaxelForkClient};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify},
    time::Instant,
};

fn source() -> Value {
    json!({"metadata":{"name":"source-one","workspace":"snapshot-workspace","externalId":"inherited-parent","url":"https://source-one.bl.run"},
        "spec":{"region":"us-was-1","runtime":{"image":"sandbox/verify:immutable","memory":2048,"ports":[{"target":4330,"protocol":"HTTP"}]}},
        "status":"DEPLOYED","state":"RUNNING","nodeGeneration":"mk3.0"})
}
fn runtime() -> roder_ext_runner_blaxel::forks::ForkRuntime {
    serde_json::from_value(json!({"image":"sandbox/verify:immutable","memory_mb":2048,"region":"us-was-1","node_generation":"mk3.0","ports":[{"target":4330,"protocol":"HTTP"}]})).unwrap()
}
fn response(code: u16, body: Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn request(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 2048];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0 && bytes.len() < 8192);
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(split) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..split]);
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= split + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}

#[tokio::test]
async fn cancelled_create_keeps_settlement_read_only_after_the_provider_received_its_full_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v0", listener.local_addr().unwrap());
    let client = BlaxelForkClient::with_base_url(
        Redacted("fixture-credential".into()),
        "snapshot-workspace",
        &base,
    )
    .unwrap();
    let runtime = runtime();
    let intent = client
        .capture_preview_intent("source-one", "inherited-parent", &runtime, &"a".repeat(64))
        .unwrap();
    let name = intent.preview_name();
    let source = source();
    let preview = json!({"metadata":{"name":name},"spec":{"port":4330,"public":true,"url":"https://owned.us-was-1.preview.bl.run","ttl":"10m"}});
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let entered = entered.clone();
        let release = release.clone();
        let captured = requests.clone();
        async move {
            for index in 0..6 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let received = request(&mut socket).await;
                captured.lock().await.push(received);
                if index == 3 {
                    entered.notify_one();
                    release.notified().await;
                    continue;
                }
                let (code, body) = match index {
                    1 => (404, Value::Null),
                    5 => (200, preview.clone()),
                    _ => (200, source.clone()),
                };
                socket
                    .write_all(response(code, body).as_bytes())
                    .await
                    .unwrap();
            }
        }
    });
    let limit = Instant::now() + Duration::from_secs(3);
    let permit = client
        .prepare_capture_preview(&intent, limit)
        .await
        .unwrap();
    let create = tokio::spawn({
        let client = client.clone();
        async move { client.create_capture_preview(permit).await }
    });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    create.abort();
    assert!(create.await.err().unwrap().is_cancelled());
    release.notify_one();
    let pin = client
        .reconcile_capture_preview(&intent, Instant::now() + Duration::from_secs(3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(pin).unwrap()["preview_name"], name);
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    let requests = requests.lock().await;
    assert_eq!(
        requests.iter().filter(|v| v.starts_with("POST ")).count(),
        1
    );
    assert!(requests[3].starts_with("POST /v0/sandboxes/source-one/previews HTTP/1.1"));
    let sent: Value = serde_json::from_str(requests[3].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(sent["metadata"]["name"], name);
    assert!(requests[5].starts_with(&format!(
        "GET /v0/sandboxes/source-one/previews/{name} HTTP/1.1"
    )));
}
