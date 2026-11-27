//! Exercise the real CLI/app-server bridge against offline and opt-in live ACP.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

struct Server {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    root: PathBuf,
    observed: Vec<Value>,
    deadline: Instant,
}

impl Server {
    fn start(fake: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "roder-antigravity-process-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        if fake {
            let agy = root.join("agy");
            std::fs::write(&agy, FAKE_AGY).unwrap();
            std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = std::env::join_paths(std::iter::once(root.clone()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_roder"))
            .args(["app-server", "--backend", "antigravity"])
            .args((!fake).then_some("--yolo"))
            .current_dir(&root)
            .env("RODER_CONFIG_DIR", root.join("config"))
            .env("RODER_DATA_DIR", root.join("config"))
            .env("RODER_PROVIDER", "mock")
            .env("PATH", path)
            .env("GEMINI_API_KEY", "must-not-reach-agy")
            .env("GOOGLE_APPLICATION_CREDENTIALS", "must-not-reach-agy")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(root.join("stderr.log")).unwrap())
            .spawn()
            .expect("spawn real roder app-server");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let message = serde_json::from_str(&line)
                    .unwrap_or_else(|error| panic!("invalid JSON-RPC stdout {line:?}: {error}"));
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            messages,
            root,
            observed: vec![],
            deadline: Instant::now() + Duration::from_secs(if fake { 45 } else { 150 }),
        }
    }

    fn receive(&mut self) -> Value {
        let message = self
            .messages
            .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| {
                panic!(
                    "app-server output failed: {error}; stderr: {}",
                    std::fs::read_to_string(self.root.join("stderr.log")).unwrap_or_default()
                )
            });
        self.observed.push(message.clone());
        message
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.stdin.flush().unwrap();
        for _ in 0..100 {
            let message = self.receive();
            if message["id"] == id {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
        }
        panic!("{method}: too many notifications without a response");
    }

    fn wait_for_turn(&mut self) {
        while !self
            .observed
            .iter()
            .any(|message| message["method"] == "turn/completed")
        {
            let message = self.receive();
            assert!(
                message["method"] != "turn/failed"
                    && message["method"] != "turn/interrupted"
                    && message["method"] != "thread/approvalRequested",
                "turn did not complete autonomously: {message}"
            );
        }
    }

    fn agent_text(&self) -> String {
        self.observed
            .iter()
            .filter(|message| message["method"] == "item/agentMessage/delta")
            .filter_map(|message| message.pointer("/params/event/delta/delta")?.as_str())
            .collect()
    }

    fn observed_tool_activity(&self) -> bool {
        self.observed.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("item/started" | "item/completed")
            ) && message.pointer("/params/event/item/type") == Some(&json!("toolExecution"))
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn antigravity_cli_streams_a_turn_through_official_acp_backend() {
    let mut server = Server::start(true);
    let initialized = server.request(1, "initialize", json!({}));
    assert_eq!(initialized["provider"], "antigravity");

    let providers = server.request(2, "providers/list", json!({}));
    assert_eq!(providers["active_provider"], "antigravity");
    assert_eq!(providers["providers"][0]["name"], "Antigravity");
    assert!(!providers.to_string().contains("Codex"));

    let models = server.request(3, "model/list", json!({}));
    assert_eq!(models["models"][0]["id"], "gemini-test");
    assert_eq!(models["models"][0]["modelProvider"], "antigravity");

    let started = server.request(
        4,
        "thread/start",
        json!({"workspaceId":"test","cwd":server.root,"model":"gemini-test"}),
    );
    let thread_id = started["thread"]["id"].as_str().unwrap().to_string();
    assert_eq!(started["thread"]["modelProvider"], "antigravity");
    let turn = server.request(
        5,
        "turn/start",
        json!({"threadId":thread_id,"prompt":"Reply with exactly: fake ACP response"}),
    );
    assert!(turn["turnId"].as_str().is_some());
    server.wait_for_turn();
    let output = server.agent_text();
    server.request(6, "thread/archive", json!({"threadId":thread_id}));
    assert_eq!(output, "fake ACP response");
}

/// Requires `roder auth login antigravity`; consumes live subscription quota.
#[test]
#[ignore = "requires installed agy, Google subscription login, and RODER_LIVE_ANTIGRAVITY=1"]
fn live_antigravity_cli_reads_a_file_through_subscription_tools() {
    assert_eq!(
        std::env::var("RODER_LIVE_ANTIGRAVITY").as_deref(),
        Ok("1"),
        "set RODER_LIVE_ANTIGRAVITY=1 to permit the live subscription test"
    );
    let mut server = Server::start(false);
    let marker = format!(
        "RODER_SUBSCRIPTION_READ_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::fs::write(server.root.join("subscription-probe.txt"), &marker).unwrap();
    assert_eq!(
        server.request(1, "initialize", json!({}))["provider"],
        "antigravity"
    );
    let providers = server.request(2, "providers/list", json!({}));
    assert_eq!(providers["active_provider"], "antigravity");
    assert_eq!(providers["providers"][0]["name"], "Antigravity");
    let catalog = server.request(3, "model/list", json!({}));
    let models = catalog["models"].as_array().expect("live account models");
    let is_gemini = |model: &&Value| {
        model["id"]
            .as_str()
            .is_some_and(|id| id.to_ascii_lowercase().contains("gemini"))
    };
    let model = models
        .iter()
        .filter(is_gemini)
        .find(|model| model["isDefault"] == true)
        .or_else(|| models.iter().find(is_gemini))
        .expect("Antigravity account must advertise a Gemini model")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let started = server.request(
        4,
        "thread/start",
        json!({"workspaceId":"live-test","cwd":server.root,"model":model}),
    );
    assert_eq!(started["thread"]["modelProvider"], "antigravity");
    let thread_id = started["thread"]["id"].as_str().unwrap().to_string();
    server.request(
        5,
        "turn/start",
        json!({
            "threadId":thread_id,
            "prompt":"Read subscription-probe.txt using a read-only tool and reply only its complete contents. Do not write files or run commands."
        }),
    );
    server.wait_for_turn();
    let output = server.agent_text();
    let tool_activity = server.observed_tool_activity();
    server.request(6, "thread/archive", json!({"threadId":thread_id}));
    assert!(
        output.contains(&marker),
        "agent delta did not include the file-only marker; output={output:?}"
    );
    assert!(
        tool_activity,
        "live turn emitted no tool activity notification"
    );
}

const FAKE_AGY: &str = r#"#!/usr/bin/env python3
import json, os, sys
assert sys.argv[1] == "acp", sys.argv
assert "GEMINI_API_KEY" not in os.environ
assert "GOOGLE_APPLICATION_CREDENTIALS" not in os.environ
def emit(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if "id" not in message:
        continue
    result = {}
    if method == "initialize":
        result = {"protocolVersion":1,"agentCapabilities":{"auth":{"logout":True}},
                  "agentInfo":{"name":"fake-antigravity","version":"1.0"},
                  "authMethods":[{"id":"oauth-personal","name":"Google account"}]}
    elif method == "session/new":
        result = {"sessionId":"fake-agy-session","models":{"currentModelId":"gemini-test",
                  "availableModels":[{"modelId":"gemini-test","name":"Gemini Test"}]}}
    elif method == "session/prompt":
        emit({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"fake-agy-session","update":{"sessionUpdate":"agent_message_chunk",
            "content":{"type":"text","text":"fake ACP response"}}}})
        result = {"stopReason":"end_turn"}
    emit({"jsonrpc":"2.0","id":message["id"],"result":result})
"#;
