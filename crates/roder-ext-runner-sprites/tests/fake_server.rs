use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct FakeSpritesServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeSpritesServer {
    pub async fn start(expected_requests: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        tokio::spawn(async move {
            for _ in 0..expected_requests {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0_u8; 65536];
                let read = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let first = request.lines().next().unwrap_or_default().to_string();
                captured.lock().unwrap().push(request.clone());
                let auth_ok = request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token");
                let (status, content_type, body) = response_for(&first, auth_ok);
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\ncontent-type: {content_type}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.write_all(body.as_bytes()).await.unwrap();
            }
        });
        Self { base_url, requests }
    }

    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn response_for(first: &str, auth_ok: bool) -> (&'static str, &'static str, String) {
    if !auth_ok {
        return (
            "401 Unauthorized",
            "application/json",
            serde_json::json!({"error":"authorization token rejected"}).to_string(),
        );
    }
    if first.starts_with("GET /v1/sprites/existing-sprite ") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({
                "id": "sprite-existing",
                "name": "existing-sprite",
                "status": "running",
                "url": "https://existing.sprites.app"
            })
            .to_string(),
        );
    }
    if first.starts_with("POST /v1/sprites ") {
        return (
            "201 Created",
            "application/json",
            serde_json::json!({
                "id": "sprite-created",
                "name": "roder-test",
                "status": "cold",
                "url": "https://roder-test.sprites.app",
                "url_settings": {"auth": "sprite"}
            })
            .to_string(),
        );
    }
    if first.starts_with("PUT /v1/sprites/roder-test/policy/") {
        return ("200 OK", "application/json", "{}".to_string());
    }
    if first.starts_with("POST /v1/sprites/roder-test/exec?") {
        return (
            "200 OK",
            "application/octet-stream",
            String::from_utf8(vec![1, b'4', b'\n', 3, 0]).unwrap(),
        );
    }
    if first.starts_with("PUT /v1/sprites/roder-test/services/roder-app-server ") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({
                "name": "roder-app-server",
                "cmd": ".roder/bin/roder",
                "args": ["app-server"],
                "needs": [],
                "http_port": 17373,
                "state": {"status": "stopped"}
            })
            .to_string(),
        );
    }
    if first.starts_with("POST /v1/sprites/roder-test/services/roder-app-server/restart?") {
        return (
            "200 OK",
            "application/x-ndjson",
            "{\"type\":\"started\",\"timestamp\":1}\n{\"type\":\"complete\",\"timestamp\":2}\n"
                .to_string(),
        );
    }
    if first.starts_with("GET /v1/sprites/roder-test/services/roder-app-server ") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({
                "name": "roder-app-server",
                "cmd": ".roder/bin/roder",
                "args": ["app-server"],
                "needs": [],
                "http_port": 17373,
                "state": {"status": "running", "pid": 42}
            })
            .to_string(),
        );
    }
    if first.starts_with("PUT /v1/sprites/roder-test/fs/write?") {
        return (
            "200 OK",
            "application/json",
            serde_json::json!({"path":"hello.txt","size":5,"mode":"0644"}).to_string(),
        );
    }
    if first.starts_with("GET /v1/sprites/roder-test/fs/read?") {
        return ("200 OK", "application/octet-stream", "hello".to_string());
    }
    if first.starts_with("GET /v1/sprites/") {
        return (
            "404 Not Found",
            "application/json",
            serde_json::json!({"error":"missing"}).to_string(),
        );
    }
    if first.starts_with("POST /v1/sprites/roder-test/checkpoint ") {
        return (
            "200 OK",
            "application/x-ndjson",
            "{\"type\":\"info\",\"data\":\"starting\"}\n{\"type\":\"complete\",\"id\":\"v1\",\"data\":\"checkpoint v1 complete\"}\n".to_string(),
        );
    }
    if first.starts_with("POST /v1/sprites/roder-test/exec/cmd-1/kill ") {
        return (
            "200 OK",
            "application/x-ndjson",
            "{\"type\":\"complete\",\"exit_code\":0}\n".to_string(),
        );
    }
    if first.starts_with("DELETE /v1/sprites/roder-test ") {
        return ("200 OK", "application/json", "{}".to_string());
    }
    (
        "404 Not Found",
        "application/json",
        serde_json::json!({"error": format!("missing route {first}")}).to_string(),
    )
}
