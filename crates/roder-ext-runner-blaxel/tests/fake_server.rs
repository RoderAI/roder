use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// In-memory fake of the Blaxel control plane + per-sandbox API. Routes by
/// method/path, stores written files, and tracks external-id -> name mappings
/// so resume/rejoin flows can be exercised offline.
#[derive(Clone)]
pub struct FakeBlaxelServer {
    base_url: String,
    state: Arc<Mutex<ServerState>>,
}

#[derive(Default)]
struct ServerState {
    requests: Vec<String>,
    sandboxes: HashMap<String, Option<String>>,
    by_external: HashMap<String, String>,
    files: HashMap<String, String>,
}

impl FakeBlaxelServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(ServerState::default()));
        let server = Self {
            base_url: base_url.clone(),
            state: state.clone(),
        };
        let handler_base = base_url.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                let state = state.clone();
                let base = handler_base.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 65536];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let (status, body) = route(&state, &base, &request);
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        server
    }

    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }
}

fn route(state: &Arc<Mutex<ServerState>>, base: &str, request: &str) -> (&'static str, String) {
    let first = request.lines().next().unwrap_or_default().to_string();
    state.lock().unwrap().requests.push(request.to_string());

    if !request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-token")
    {
        return ("401 Unauthorized", json_error("missing or invalid token"));
    }

    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let full_path = parts.next().unwrap_or_default();
    let path = full_path.split('?').next().unwrap_or_default();
    let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();

    match (method, path) {
        ("POST", "/sandboxes") => {
            let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let name = parsed["metadata"]["name"].as_str().unwrap_or("sandbox").to_string();
            let external = parsed["metadata"]["externalId"].as_str().map(str::to_string);
            {
                let mut guard = state.lock().unwrap();
                guard.sandboxes.insert(name.clone(), external.clone());
                if let Some(ext) = &external {
                    guard.by_external.insert(ext.clone(), name.clone());
                }
            }
            ("200 OK", sandbox_json(base, &name, external.as_deref()))
        }
        ("GET", _) if path == "/health" => ("200 OK", "{}".to_string()),
        ("GET", _) if path.starts_with("/sandboxes/by-external-id/") => {
            let ext = path.trim_start_matches("/sandboxes/by-external-id/");
            let guard = state.lock().unwrap();
            match guard.by_external.get(ext) {
                Some(name) => ("200 OK", sandbox_json(base, name, Some(ext))),
                None => ("404 Not Found", json_error("no sandbox for external id")),
            }
        }
        ("POST", _) if path.starts_with("/sandboxes/") && path.ends_with("/previews") => {
            let port_label = body
                .parse_json()
                .and_then(|v| v["spec"]["port"].as_u64())
                .unwrap_or(0);
            (
                "200 OK",
                serde_json::json!({
                    "metadata": { "name": format!("port-{port_label}") },
                    "spec": { "url": format!("https://preview.example/port-{port_label}") }
                })
                .to_string(),
            )
        }
        ("GET", _) if path.starts_with("/sandboxes/") => {
            let name = path.trim_start_matches("/sandboxes/");
            let guard = state.lock().unwrap();
            match guard.sandboxes.get(name) {
                Some(external) => ("200 OK", sandbox_json(base, name, external.as_deref())),
                None => ("404 Not Found", json_error("sandbox not found")),
            }
        }
        ("DELETE", _) if path.starts_with("/sandboxes/") => {
            let name = path.trim_start_matches("/sandboxes/").to_string();
            state.lock().unwrap().sandboxes.remove(&name);
            ("200 OK", sandbox_json(base, &name, None))
        }
        ("POST", "/process") => {
            let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let command = parsed["command"].as_str().unwrap_or_default();
            let stdout = command
                .strip_prefix("echo ")
                .map(|rest| format!("{rest}\n"))
                .unwrap_or_default();
            (
                "200 OK",
                serde_json::json!({
                    "exitCode": 0,
                    "stdout": stdout,
                    "stderr": "",
                    "status": "completed"
                })
                .to_string(),
            )
        }
        ("PUT", _) if path.starts_with("/filesystem/") => {
            let key = path.trim_start_matches("/filesystem/").to_string();
            let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            if let Some(content) = parsed["content"].as_str() {
                state.lock().unwrap().files.insert(key, content.to_string());
            }
            ("200 OK", serde_json::json!({ "message": "ok" }).to_string())
        }
        ("GET", _) if path.starts_with("/filesystem/") => {
            let key = path.trim_start_matches("/filesystem/");
            let guard = state.lock().unwrap();
            match guard.files.get(key) {
                Some(content) => (
                    "200 OK",
                    serde_json::json!({ "path": key, "content": content }).to_string(),
                ),
                None => ("404 Not Found", json_error("file not found")),
            }
        }
        ("DELETE", _) if path.starts_with("/filesystem/") => {
            let key = path.trim_start_matches("/filesystem/").to_string();
            state.lock().unwrap().files.remove(&key);
            ("200 OK", serde_json::json!({ "message": "ok" }).to_string())
        }
        _ => ("404 Not Found", json_error("unrouted")),
    }
}

trait ParseJson {
    fn parse_json(&self) -> Option<serde_json::Value>;
}

impl ParseJson for &str {
    fn parse_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(self).ok()
    }
}

fn sandbox_json(base: &str, name: &str, external: Option<&str>) -> String {
    let mut metadata = serde_json::json!({ "name": name, "url": base });
    if let Some(ext) = external {
        metadata["externalId"] = serde_json::json!(ext);
    }
    serde_json::json!({
        "metadata": metadata,
        "state": "RUNNING",
        "status": "DEPLOYED"
    })
    .to_string()
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}
