//! Offline fake-WSS coverage for the Sprites exec and proxy WebSocket
//! channels (roadmap phase 88, Task 4/6). A local tokio-tungstenite server
//! stands in for `wss://api.sprites.dev`; no network access.

use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use roder_api::remote_runner::{RunnerDestination, RunnerManifest};
use roder_ext_runner_sprites::{PROVIDER_ID, SpritesClient, SpritesConfig, WsExecRequest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::Message;

#[derive(Clone, Default)]
struct CapturedHandshake {
    path: Arc<Mutex<String>>,
    authorization: Arc<Mutex<String>>,
}

/// Ambient Sprites env vars (from live smokes) take precedence over config;
/// clear them so the fake-server token/base-url are used.
fn clear_sprites_env() {
    use roder_ext_runner_sprites::config::{
        BASE_URL_ENV, RODER_BASE_URL_ENV, RODER_TOKEN_ENV, TOKEN_ENV,
    };
    unsafe {
        std::env::remove_var(RODER_TOKEN_ENV);
        std::env::remove_var(TOKEN_ENV);
        std::env::remove_var(RODER_BASE_URL_ENV);
        std::env::remove_var(BASE_URL_ENV);
    }
}

fn config_for(base_url: &str) -> SpritesConfig {
    SpritesConfig::from_destination(&RunnerDestination {
        id: "sprites-dev".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "token": "test-token",
            "base_url": base_url,
            "sprite_name": "roder-test",
            "cleanup": "keep"
        }),
        default_manifest: RunnerManifest::default(),
    })
    .unwrap()
}

// The tungstenite handshake callback must return its large Response type.
#[allow(clippy::result_large_err)]
async fn accept_ws(
    listener: &TcpListener,
    captured: &CapturedHandshake,
) -> tokio_tungstenite::WebSocketStream<tokio::net::TcpStream> {
    let (stream, _) = listener.accept().await.unwrap();
    let path = captured.path.clone();
    let authorization = captured.authorization.clone();
    tokio_tungstenite::accept_hdr_async(stream, move |request: &Request, response: Response| {
        *path.lock().unwrap() = request.uri().to_string();
        *authorization.lock().unwrap() = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        Ok(response)
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_exec_streams_frames_and_json_info_with_bearer_auth() {
    clear_sprites_env();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let captured = CapturedHandshake::default();
    let server_capture = captured.clone();

    let server = tokio::spawn(async move {
        let mut ws = accept_ws(&listener, &server_capture).await;
        ws.send(Message::Text(
            serde_json::json!({"type": "session", "id": "sess-1"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Binary(vec![1, b'o', b'k', b'\n'].into()))
            .await
            .unwrap();
        ws.send(Message::Binary(vec![2, b'w', b'a', b'r', b'n'].into()))
            .await
            .unwrap();
        ws.send(Message::Binary(vec![3, 0].into())).await.unwrap();
        let _ = ws.close(None).await;
    });

    let client = SpritesClient::new(reqwest::Client::new(), config_for(&base_url));
    let outcome = client
        .exec_ws(
            "roder-test",
            &WsExecRequest {
                cmd: vec!["echo".to_string(), "ok".to_string()],
                cwd: Some("/home/sprite/roder".to_string()),
                env: vec![("RUST_LOG".to_string(), "info".to_string())],
            },
        )
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(outcome.stdout, "ok\n");
    assert_eq!(outcome.stderr, "warn");
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.info.len(), 1);
    assert_eq!(outcome.info[0]["type"], "session");

    let path = captured.path.lock().unwrap().clone();
    assert!(
        path.starts_with("/v1/sprites/roder-test/exec?cmd=echo&cmd=ok"),
        "{path}"
    );
    assert!(path.contains("cwd=%2Fhome%2Fsprite%2Froder"), "{path}");
    assert!(path.contains("env=RUST_LOG%3Dinfo"), "{path}");
    assert_eq!(
        captured.authorization.lock().unwrap().as_str(),
        "Bearer test-token"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_proxy_relays_tcp_bytes_after_init_message() {
    clear_sprites_env();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let captured = CapturedHandshake::default();
    let server_capture = captured.clone();
    let init_seen = Arc::new(Mutex::new(serde_json::Value::Null));
    let init_store = init_seen.clone();

    let server = tokio::spawn(async move {
        let mut ws = accept_ws(&listener, &server_capture).await;
        // First message must be the init JSON naming the target host/port.
        let Some(Ok(Message::Text(init))) = ws.next().await else {
            panic!("expected init text frame first");
        };
        *init_store.lock().unwrap() = serde_json::from_str(&init).unwrap();
        // Echo every binary frame back, simulating the sprite-side service.
        while let Some(Ok(message)) = ws.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
            let Message::Binary(bytes) = message else {
                continue;
            };
            if ws.send(Message::Binary(bytes)).await.is_err() {
                break;
            }
        }
    });

    let client = SpritesClient::new(reqwest::Client::new(), config_for(&base_url));
    let (local_addr, relay) = client
        .serve_port_proxy("roder-test", "127.0.0.1", 8080)
        .await
        .unwrap();

    let mut tcp = tokio::net::TcpStream::connect(local_addr).await.unwrap();
    tcp.write_all(b"ping through the sprite proxy").await.unwrap();
    let mut echoed = vec![0_u8; 29];
    tcp.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"ping through the sprite proxy");

    drop(tcp);
    relay.abort();
    let _ = server.await;

    assert_eq!(
        captured.path.lock().unwrap().as_str(),
        "/v1/sprites/roder-test/proxy"
    );
    assert_eq!(
        captured.authorization.lock().unwrap().as_str(),
        "Bearer test-token"
    );
    let init = init_seen.lock().unwrap().clone();
    assert_eq!(init["host"], "127.0.0.1");
    assert_eq!(init["port"], 8080);
}
