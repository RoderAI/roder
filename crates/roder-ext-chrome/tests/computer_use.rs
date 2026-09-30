//! Browser-backed primitive evaluation. No model, external account, or API key.
//! RODER_REQUIRE_CHROME=1 makes a missing browser a hard failure.
use base64::Engine;
use roder_api::{
    chrome::{ChromeCommand, ChromeController, ChromeError, ChromePermissionMode, ChromeStatus},
    policy_mode::PolicyMode,
    tools::{ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry, ToolResult},
};
use roder_ext_chrome::{
    ChromeToolContributor,
    direct::{DirectSession, DirectTab, OpenGuard},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Browser {
    child: Child,
    profile: PathBuf,
    endpoint: String,
}
impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}
impl Browser {
    async fn start() -> Option<Self> {
        let path = std::env::var("RODER_CHROME_BINARY").unwrap_or_else(|_| {
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into()
        });
        if !std::path::Path::new(&path).is_file() {
            assert!(
                std::env::var_os("RODER_REQUIRE_CHROME").is_none(),
                "Chrome required: set RODER_CHROME_BINARY"
            );
            eprintln!(
                "SKIPPED: Chrome unavailable; set RODER_REQUIRE_CHROME=1 to require evaluation"
            );
            return None;
        }
        let profile =
            std::env::temp_dir().join(format!("roder-primitives-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&profile).unwrap();
        let child = Command::new(path)
            .args([
                "--headless=new",
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
            .spawn()
            .unwrap();
        let mut browser = Self {
            child,
            profile,
            endpoint: String::new(),
        };
        for _ in 0..200 {
            if let Ok(port) = std::fs::read_to_string(browser.profile.join("DevToolsActivePort")) {
                browser.endpoint = format!("http://127.0.0.1:{}", port.lines().next().unwrap());
                return Some(browser);
            }
            assert!(
                browser.child.try_wait().unwrap().is_none(),
                "Chrome exited during launch"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("Chrome did not publish its DevTools endpoint");
    }
    async fn tab(&self) -> DirectTab {
        let targets: Value = reqwest::get(format!("{}/json", self.endpoint))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let page = targets
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["type"] == "page")
            .unwrap();
        DirectTab::Page {
            websocket: page["webSocketDebuggerUrl"].as_str().unwrap().into(),
            target_id: page["id"].as_str().unwrap().into(),
        }
    }
}

struct Desktop;
#[async_trait::async_trait]
impl ChromeController for Desktop {
    fn status(&self) -> ChromeStatus {
        ChromeStatus {
            enabled: true,
            mode: ChromePermissionMode::Control,
            ..ChromeStatus::default()
        }
    }
    fn set_enabled(&self, _: bool) {}
    fn set_mode(&self, _: ChromePermissionMode) {}
    async fn dispatch(&self, _: ChromeCommand) -> Result<Value, ChromeError> {
        panic!("extension dispatch must not run for Desktop fallback")
    }
}
async fn call(registry: &ToolRegistry, name: &str, args: Value) -> ToolResult {
    registry
        .get(name)
        .unwrap()
        .execute(
            ToolExecutionContext::new("eval", "turn", PolicyMode::Default),
            ToolCall {
                id: name.into(),
                name: name.into(),
                raw_arguments: args.to_string(),
                arguments: args,
                thread_id: "eval".into(),
                turn_id: "turn".into(),
            },
        )
        .await
        .unwrap()
}
async fn eval(registry: &ToolRegistry, expression: &str) -> Value {
    let result = call(registry, "chrome_eval", json!({"expression":expression})).await;
    assert!(!result.is_error, "{}", result.text);
    result.data["content"]["result"].clone()
}
async fn fixture() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await;
                let body = include_str!("fixtures/primitives.html");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, handle)
}
fn jpeg_size(url: &str) -> (u16, u16) {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(url.split_once(',').unwrap().1)
        .unwrap();
    let mut at = 2;
    while at + 8 < bytes.len() {
        assert_eq!(bytes[at], 0xff);
        let marker = bytes[at + 1];
        at += 2;
        let len = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        if matches!(marker, 0xc0 | 0xc1 | 0xc2) {
            return (
                u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]),
                u16::from_be_bytes([bytes[at + 3], bytes[at + 4]]),
            );
        }
        at += len;
    }
    panic!("JPEG has no image dimensions");
}

#[tokio::test]
async fn evaluates_real_input_observations_and_hidpi_coordinates() {
    let Some(browser) = Browser::start().await else {
        return;
    };
    // This integration binary contains one test; the environment is not shared
    // with another test or any user's browser process.
    unsafe {
        std::env::set_var(
            "RODER_DESKTOP_CDP_PORT",
            browser.endpoint.rsplit(':').next().unwrap(),
        );
    }
    let (url, site) = fixture().await;
    let mut registry = ToolRegistry::default();
    ChromeToolContributor::with_controller(Arc::new(Desktop))
        .contribute(&mut registry)
        .unwrap();
    let loaded = call(&registry, "chrome_tab_open", json!({"url":url})).await;
    assert!(!loaded.is_error, "{}", loaded.text);
    assert!(
        loaded.text.contains("Primitive fixture"),
        "Navigation must return observed page state"
    );
    let looked = call(&registry, "chrome_page_snapshot", json!({})).await;
    assert!(looked.text.contains("Count: 0"));
    assert!(!looked.text.contains("fixture-secret"));
    let elements = looked.data["page"]["elements"].as_array().unwrap();
    let button = elements.iter().find(|e| e["label"] == "Increment").unwrap()["ref"].clone();
    // Insert an earlier control: a snapshot ref must still refer to Increment.
    eval(
        &registry,
        "document.body.insertAdjacentHTML('afterbegin','<button>Distractor</button>'); true",
    )
    .await;
    let clicked = call(&registry, "chrome_click", json!({"ref":button})).await;
    assert!(!clicked.is_error, "{}", clicked.text);
    assert!(
        clicked.text.contains("Count: 1"),
        "Click must return its actual observed outcome"
    );
    assert_eq!(eval(&registry, "window.clickTrusted").await, true);
    let typed = call(
        &registry,
        "chrome_type",
        json!({"selector":"#name", "text":"penguin"}),
    )
    .await;
    assert!(!typed.is_error, "{}", typed.text);
    assert_eq!(
        eval(
            &registry,
            "[document.querySelector('#name').value, window.inputTrusted]"
        )
        .await,
        json!(["penguin", true])
    );
    let key = call(&registry, "chrome_keypress", json!({"key":"Tab"})).await;
    assert!(!key.is_error, "{}", key.text);
    assert_eq!(eval(&registry, "document.activeElement.id").await, "next");
    assert_eq!(eval(&registry, "window.keyTrusted").await, true);
    let missing = call(&registry, "chrome_click", json!({"selector":"#missing"})).await;
    assert!(missing.is_error, "Missing targets must not report success");
    let ambiguous = call(&registry, "chrome_click", json!({"selector":"button"})).await;
    assert!(
        ambiguous.is_error,
        "Ambiguous targets must not silently choose the first"
    );
    let not_field = call(
        &registry,
        "chrome_type",
        json!({"selector":"#inc", "text":"wrong target"}),
    )
    .await;
    assert!(
        not_field.is_error,
        "Typing into a non-editable control must fail"
    );
    assert_eq!(
        eval(&registry, "document.querySelector('#count').textContent").await,
        "Count: 1",
        "Invalid typing must not click the target"
    );
    let tab = browser.tab().await;
    let mut direct = DirectSession::attach(&tab, Arc::new(OpenGuard), false)
        .await
        .unwrap();
    let picture = direct.run("screenshot", &json!({})).await;
    assert!(!picture.is_error, "{}", picture.text);
    assert_eq!(
        picture.data["masked"], 1,
        "Filled secret field must be masked"
    );
    if let Ok(directory) = std::env::var("RODER_PRIMITIVE_REPORT_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(picture.image.as_ref().unwrap().split_once(',').unwrap().1)
            .unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("viewport-dpr2.jpg"),
            bytes,
        )
        .unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("screenshot.json"),
            serde_json::to_vec_pretty(&picture.data).unwrap(),
        )
        .unwrap();
    }
    let size = jpeg_size(picture.image.as_ref().unwrap());
    assert_eq!(
        (size.0 as f64, size.1 as f64),
        (
            picture.data["width"].as_f64().unwrap(),
            picture.data["height"].as_f64().unwrap()
        ),
        "At DPR=2 the image must map 1:1 to input CSS coordinates"
    );
    assert_eq!(eval(&registry, "devicePixelRatio").await, 2);
    let origin = direct.run("click", &json!({"x":0,"y":0})).await;
    assert!(!origin.is_error, "(0,0) is a valid point: {}", origin.text);
    assert_eq!(eval(&registry, "window.originClicked").await, true);
    let outside = direct.run("click", &json!({"x":-1,"y":0})).await;
    assert!(outside.is_error);
    let right = direct
        .run("click", &json!({"x":10,"y":10,"button":"right"}))
        .await;
    assert!(!right.is_error, "{}", right.text);
    assert_eq!(eval(&registry, "window.rightButtons").await, 2);
    let picture = call(&registry, "chrome_screenshot", json!({})).await;
    assert_eq!(picture.data["__view_image"]["detail"], "original");
    let reloaded = direct.run("navigate", &json!({"url":url})).await;
    assert!(!reloaded.is_error, "{}", reloaded.text);
    let stale = direct.run("click", &json!({"ref":button})).await;
    assert!(
        stale.is_error,
        "Refs from the previous document must not resolve in a new document"
    );
    eprintln!(
        "PASS: navigation/state, stable refs, trusted click/type/key, missing/ambiguous targets, secret masking, DPR2 screenshot mapping, origin/out-of-bounds coordinates, right-button mask, screenshot attachment"
    );
    site.abort();
}
