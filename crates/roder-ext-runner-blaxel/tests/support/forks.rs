use roder_ext_runner_blaxel::{
    Redacted,
    forks::{BlaxelForkClient, ForkRuntime},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};

pub struct Server {
    pub url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start(responses: Vec<String>) -> Self {
        Self::start_with(|_| responses).await
    }

    pub async fn start_with(responses: impl FnOnce(&str) -> Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v0", listener.local_addr().unwrap());
        let responses = responses(&url);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..split]);
                        let length: usize = header
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                captured
                    .lock()
                    .await
                    .push(String::from_utf8(bytes).unwrap());
                if !response.is_empty() {
                    socket.write_all(response.as_bytes()).await.unwrap();
                }
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    pub fn client(&self) -> BlaxelForkClient {
        BlaxelForkClient::with_base_url(
            Redacted("fixture-credential".into()),
            "snapshot-workspace",
            &self.url,
        )
        .unwrap()
    }

    pub async fn requests(&self) -> Vec<String> {
        self.requests.lock().await.clone()
    }
}

pub fn response(status: u16, body: Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 {status} test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub fn source() -> Value {
    json!({"metadata":{"name":"source-one","workspace":"snapshot-workspace","externalId":"inherited-parent","url":"https://source-one.bl.run"},
        "spec":{"region":"us-was-1","runtime":{"image":"sandbox/verify:immutable","memory":2048,"ports":[{"target":4319,"protocol":"HTTP"}]}},
        "status":"DEPLOYED","state":"RUNNING","nodeGeneration":"mk3.0"})
}

pub fn runtime() -> ForkRuntime {
    serde_json::from_value(json!({"image":"sandbox/verify:immutable","memory_mb":2048,
        "region":"us-was-1","node_generation":"mk3.0","ports":[{"target":4319,"protocol":"HTTP"}]}))
    .unwrap()
}
