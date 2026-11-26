use std::time::Duration;

use roder_ext_mcp::{McpHttpClient, McpServerConfig};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use super::{read_http_request, rpc_result};

async fn expect_request(
    listener: &TcpListener,
    method: &str,
    session: Option<&str>,
) -> (TcpStream, Value) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let (headers, payload) = read_http_request(&mut socket).await.unwrap();
    let actual_session = headers
        .iter()
        .find_map(|header| header.strip_prefix("mcp-session-id: "));
    assert_eq!(actual_session, session);
    assert!(
        headers
            .iter()
            .any(|header| header == "authorization: Bearer session-test")
    );
    let body: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(body["method"], method);
    (socket, body)
}

async fn reply(socket: &mut TcpStream, status: &str, headers: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}

async fn initialize(listener: &TcpListener, session: Option<&str>) {
    let (mut socket, body) = expect_request(listener, "initialize", None).await;
    let headers = session
        .map(|id| format!("Mcp-Session-Id: {id}\r\n"))
        .unwrap_or_default();
    reply(
        &mut socket,
        "200 OK",
        &headers,
        &rpc_result(body["id"].clone(), json!({})),
    )
    .await;
}

async fn next_notification(listener: &TcpListener, session: Option<&str>, status: &str) {
    let (mut socket, _) = expect_request(listener, "notifications/initialized", session).await;
    reply(&mut socket, status, "", "").await;
}

async fn list_tools(listener: &TcpListener, session: Option<&str>) {
    let (mut socket, body) = expect_request(listener, "tools/list", session).await;
    reply(
        &mut socket,
        "200 OK",
        "",
        &rpc_result(body["id"].clone(), json!({"tools": []})),
    )
    .await;
}

async fn client_fixture() -> (McpHttpClient, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let config =
        McpServerConfig::new("local-test", url).with_auth_token("ignored-configured-token");
    let client = McpHttpClient::new(config)
        .unwrap()
        .with_auth_token_override("session-test".to_string());
    (client, listener)
}

#[tokio::test]
async fn expired_call_is_not_replayed_and_late_failure_preserves_replacement_session() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (client, listener) = client_fixture().await;
        let session = Some("reused-id");
        let (slow_started, slow_started_rx) = oneshot::channel();
        let (release_slow, release_slow_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            initialize(&listener, session).await;
            next_notification(&listener, session, "202 Accepted").await;

            let (mut slow, slow_body) =
                expect_request(&listener, "tools/call", Some("reused-id")).await;
            assert_eq!(slow_body["params"]["name"], "slow");
            slow_started.send(()).unwrap();

            let (mut expired, expired_body) =
                expect_request(&listener, "tools/call", Some("reused-id")).await;
            assert_eq!(expired_body["params"]["name"], "mutate");
            reply(&mut expired, "404 Not Found", "", "").await;

            // The replacement deliberately chooses the same ID.
            initialize(&listener, session).await;
            next_notification(&listener, session, "202 Accepted").await;
            list_tools(&listener, session).await;

            release_slow_rx.await.unwrap();
            reply(&mut slow, "404 Not Found", "", "").await;
            list_tools(&listener, session).await;
        });
        let slow_client = client.clone();
        let slow_call = tokio::spawn(async move { slow_client.call_tool("slow", json!({})).await });
        slow_started_rx.await.unwrap();

        let failure = client.call_tool("mutate", json!({})).await.unwrap_err();
        assert!(format!("{failure:#}").contains("HTTP 404: session expired"));
        assert!(client.list_tools().await.unwrap().is_empty());

        release_slow.send(()).unwrap();
        let failure = slow_call.await.unwrap().unwrap_err();
        assert!(format!("{failure:#}").contains("HTTP 404: session expired"));
        assert!(client.list_tools().await.unwrap().is_empty());
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn expiry_during_initialization_discards_the_session_before_sending_tool_calls() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (client, listener) = client_fixture().await;
        let session = Some("reused-id");
        let server = tokio::spawn(async move {
            initialize(&listener, session).await;
            next_notification(&listener, session, "404 Not Found").await;
            initialize(&listener, session).await;
            next_notification(&listener, session, "202 Accepted").await;
            list_tools(&listener, session).await;
        });

        let failure = client.call_tool("mutate", json!({})).await.unwrap_err();
        assert!(format!("{failure:#}").contains("HTTP 404: session expired"));
        assert!(client.list_tools().await.unwrap().is_empty());
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stateless_http_404_leaves_initialization_intact() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (client, listener) = client_fixture().await;
        let server = tokio::spawn(async move {
            initialize(&listener, None).await;
            next_notification(&listener, None, "202 Accepted").await;
            let (mut missing, _) = expect_request(&listener, "tools/list", None).await;
            reply(&mut missing, "404 Not Found", "", "{\"missing\":true}").await;
            list_tools(&listener, None).await;
        });

        let failure = client.list_tools().await.unwrap_err();
        assert!(format!("{failure:#}").contains("HTTP 404 Not Found"));
        assert!(client.list_tools().await.unwrap().is_empty());
        server.await.unwrap();
    })
    .await
    .unwrap();
}
