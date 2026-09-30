//! Real Chrome and an independent HTTP event grader, shared by native evals.
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

pub struct Browser {
    child: Child,
    profile: PathBuf,
    pub endpoint: String,
}
impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}
impl Browser {
    pub async fn focus_tab(&self, target_id: &str) -> anyhow::Result<()> {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let targets: Vec<Value> = reqwest::get(format!("{}/json/list", self.endpoint))
            .await?
            .error_for_status()?
            .json()
            .await?;
        let websocket = targets
            .iter()
            .find(|target| target["id"] == target_id)
            .and_then(|target| target["webSocketDebuggerUrl"].as_str())
            .ok_or_else(|| anyhow::anyhow!("Demo tab has no page websocket"))?;
        let (mut socket, _) = tokio_tungstenite::connect_async(websocket).await?;
        socket
            .send(Message::Text(
                json!({"id":1,"method":"Page.bringToFront"})
                    .to_string()
                    .into(),
            ))
            .await?;
        while let Some(message) = socket.next().await {
            if let Message::Text(text) = message? {
                let result: Value = serde_json::from_str(&text)?;
                if result["id"] == 1 {
                    anyhow::ensure!(
                        result.get("error").is_none(),
                        "Bring demo to front failed: {result}"
                    );
                    return Ok(());
                }
            }
        }
        anyhow::bail!("Demo page websocket closed before focusing")
    }

    pub async fn start() -> anyhow::Result<Option<Self>> {
        let path = std::env::var("RODER_CHROME_BINARY").unwrap_or_else(|_| {
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into()
        });
        if !std::path::Path::new(&path).is_file() {
            anyhow::ensure!(
                std::env::var_os("RODER_REQUIRE_CHROME").is_none(),
                "Chrome required; set RODER_CHROME_BINARY"
            );
            eprintln!("SKIPPED: native browser eval needs RODER_CHROME_BINARY");
            return Ok(None);
        }
        let profile =
            std::env::temp_dir().join(format!("roder-native-computer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&profile)?;
        let mut command = Command::new(path);
        if std::env::var("RODER_NATIVE_EVAL_VISIBLE").as_deref() != Ok("1") {
            command.arg("--headless=new");
        }
        let child = command
            .args([
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--force-device-scale-factor=2",
                "--window-size=800,600",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut browser = Self {
            child,
            profile,
            endpoint: String::new(),
        };
        for _ in 0..200 {
            if let Ok(port) = std::fs::read_to_string(browser.profile.join("DevToolsActivePort")) {
                browser.endpoint = format!("http://127.0.0.1:{}", port.lines().next().unwrap());
                return Ok(Some(browser));
            }
            anyhow::ensure!(
                browser.child.try_wait()?.is_none(),
                "Chrome exited during launch"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        anyhow::bail!("Chrome did not publish a DevTools endpoint")
    }
}

pub struct Fixture {
    pub url: String,
    pub events: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    pub async fn start() -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let Ok((path, body)) = read_request(&mut socket).await else {
                        return;
                    };
                    let content = if path == "/event" {
                        if let Ok(event) = serde_json::from_slice(&body) {
                            recorded.lock().await.push(event);
                        }
                        "ok".to_string()
                    } else if path == "/start" {
                        include_str!("start.html").to_string()
                    } else {
                        include_str!("fixture.html").to_string()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{content}",
                        content.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        Ok(Self { url, events, task })
    }
    /// Grade application events, independently of the model's final answer.
    pub async fn grade(&self, primitives: bool) -> Value {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let events = self.events.lock().await;
        let seen = |kind: &str| {
            events
                .iter()
                .any(|e| e["type"] == kind && e["trusted"] == true)
        };
        let submitted = events
            .iter()
            .any(|e| e["type"] == "submit" && e["value"] == "orcaA" && e["trusted"] == true);
        let scroll = events
            .iter()
            .any(|e| e["type"] == "scroll" && e["top"].as_i64().unwrap_or(0) > 0);
        let drag = events.iter().any(|e| {
            e["type"] == "drag"
                && e["path"].as_array().is_some_and(|path| {
                    path.contains(&json!([80, 320])) && path.contains(&json!([120, 285]))
                })
        });
        let checks = if primitives {
            json!({"filters":seen("filters"),"submitted_value":submitted,
            "double_click":seen("double"),"right_click":seen("right"),"wheel_click":seen("wheel"),"move":seen("move"),
            "nested_scroll":scroll,"drag_path":drag,
            "shift_click":events.iter().any(|e|e["type"]=="filters" && e["shift"]==true)})
        } else {
            json!({"filters":seen("filters"),"submitted_value":submitted})
        };
        let passed = checks.as_object().unwrap().values().all(|v| v == true);
        json!({"passed":passed,"checks":checks,"events":*events})
    }
}

pub async fn read_request(socket: &mut tokio::net::TcpStream) -> anyhow::Result<(String, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    let end = loop {
        let n = socket.read(&mut chunk).await?;
        anyhow::ensure!(n > 0, "HTTP request closed");
        bytes.extend_from_slice(&chunk[..n]);
        anyhow::ensure!(bytes.len() < 20 * 1024 * 1024, "fixture request too large");
        if let Some(index) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = String::from_utf8_lossy(&bytes[..end]);
    let path = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    let len = header
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    anyhow::ensure!(len < 20 * 1024 * 1024, "fixture body too large");
    while bytes.len() < end + len {
        let n = socket.read(&mut chunk).await?;
        anyhow::ensure!(n > 0, "HTTP body closed");
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok((path, bytes[end..end + len].to_vec()))
}

pub fn scripted_batches() -> Vec<Value> {
    let select = if cfg!(target_os = "macos") {
        "CMD"
    } else {
        "CTRL"
    };
    vec![
        json!([{"type":"screenshot"}]),
        json!([{"type":"click","button":"left","keys":null,"x":60,"y":40},
            {"type":"click","button":"left","x":60,"y":100},{"type":"type","text":"penguin 🐧"},
            {"type":"keypress","keys":[select,"a"]},{"type":"type","text":"orca"},
            {"type":"keypress","keys":["SHIFT","a"]},{"type":"keypress","keys":["ENTER"]}]),
        json!([{"type":"double_click","x":60,"y":170},
            {"type":"click","button":"right","x":230,"y":170},{"type":"move","x":60,"y":230},
            {"type":"click","button":"left","x":60,"y":40,"keys":["SHIFT"]},
            {"type":"click","button":"wheel","x":230,"y":170},
            {"type":"click","button":"left","x":0,"y":0}]),
        json!([{"type":"scroll","x":550,"y":100,"scroll_x":0,"scroll_y":200}]),
        json!([{"type":"drag","path":[{"x":40,"y":300},{"x":80,"y":320},{"x":120,"y":285},{"x":180,"y":300}]}]),
        json!([{"type":"wait"},{"type":"screenshot"}]),
    ]
}
