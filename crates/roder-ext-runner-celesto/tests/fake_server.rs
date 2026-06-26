use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Minimal in-process HTTP server that mimics the Celesto Computers API so the
/// contract tests never contact `api.celesto.ai`.
#[derive(Clone, Debug)]
pub struct FakeCelestoServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeCelestoServer {
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
                let (status, body) = response_for(&first, &request, auth_ok);
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n",
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

fn response_for(first: &str, request: &str, auth_ok: bool) -> (&'static str, String) {
    if !auth_ok {
        return (
            "401 Unauthorized",
            serde_json::json!({ "error": "authorization token rejected" }).to_string(),
        );
    }

    let computer = serde_json::json!({
        "id": "comp-123",
        "name": "roder-comp",
        "status": "running",
        "vcpus": 2
    })
    .to_string();

    if first.starts_with("POST /v1/computers/comp-123/exec ") {
        // Differentiate exec calls by inspecting the embedded command.
        if request.contains("base64 -d") {
            // write_file
            return (
                "200 OK",
                serde_json::json!({ "exit_code": 0, "stdout": "", "stderr": "" }).to_string(),
            );
        }
        if request.contains("base64 -- ") {
            // read_file: base64 of "hello"
            return (
                "200 OK",
                serde_json::json!({ "exit_code": 0, "stdout": "aGVsbG8=\n", "stderr": "" })
                    .to_string(),
            );
        }
        // mkdir working dir or run_command
        return (
            "200 OK",
            serde_json::json!({ "exit_code": 0, "stdout": "4\n", "stderr": "" }).to_string(),
        );
    }
    if first.starts_with("POST /v1/computers/comp-123/ports ") {
        return (
            "200 OK",
            serde_json::json!({
                "port": 8000,
                "url": "https://comp-123-8000.celesto.app",
                "status": "published"
            })
            .to_string(),
        );
    }
    if first.starts_with("DELETE /v1/computers/comp-123/ports/") {
        return ("200 OK", "{}".to_string());
    }
    if first.starts_with("POST /v1/computers/comp-123/start ")
        || first.starts_with("POST /v1/computers/comp-123/stop ")
    {
        return ("200 OK", "{}".to_string());
    }
    if first.starts_with("DELETE /v1/computers/comp-123 ") {
        return ("200 OK", "{}".to_string());
    }
    if first.starts_with("GET /v1/computers/comp-123 ") {
        return ("200 OK", computer);
    }
    if first.starts_with("POST /v1/computers ") {
        return ("201 Created", computer);
    }

    (
        "404 Not Found",
        serde_json::json!({ "error": "unhandled" }).to_string(),
    )
}
