//! End-to-end proof for roadmap phase 64: the Python chat-completions POC
//! registers through the process-extension host and serves a full turn over
//! the public app-server JSON-RPC surface. Offline — the OpenAI-compatible
//! endpoint is a local fake SSE server and the child is the real
//! `roder_python_chat_provider` package run with `python3`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::process_extension::{ProcessEventFilter, ProcessExtensionConfig};
use roder_api::thread::ThreadStoreFactory;
use roder_app_server::{AppServer, AppServerFeatureConfig, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_ext_process_host::{ProcessHostExtension, load_process_extension};
use roder_protocol::{
    ExtensionsListResult, JsonRpcRequest, ProvidersListResult, ThreadReadParams, ThreadReadResult,
    ThreadStartParams, ThreadStartResult, TurnStartParams, TurnStartResult, WorkspaceCreateParams,
    WorkspaceCreateResult, WorkspaceRootInput,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/non-rust-extensions/python-chat-completions")
        .canonicalize()
        .unwrap()
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roder-process-ext-e2e-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Minimal OpenAI-compatible fake: answers one `/chat/completions` POST
/// with a streamed SSE body.
async fn start_fake_openai() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = vec![0_u8; 65536];
                let mut read_total = 0;
                loop {
                    let Ok(read) = socket.read(&mut buffer[read_total..]).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    read_total += read;
                    let text = String::from_utf8_lossy(&buffer[..read_total]);
                    if let Some(header_end) = text.find("\r\n\r\n") {
                        let content_length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if read_total >= header_end + 4 + content_length {
                            break;
                        }
                    }
                }
                let chunks = [
                    serde_json::json!({"id": "chatcmpl-e2e", "choices": [{"delta": {"content": "Hello from "}}]}),
                    serde_json::json!({"id": "chatcmpl-e2e", "choices": [{"delta": {"content": "python"}}]}),
                    serde_json::json!({"id": "chatcmpl-e2e", "choices": [{"delta": {}, "finish_reason": "stop"}]}),
                    serde_json::json!({"id": "chatcmpl-e2e", "choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 3, "total_tokens": 12}}),
                ];
                let body = chunks
                    .iter()
                    .map(|chunk| format!("data: {chunk}\n\n"))
                    .collect::<String>()
                    + "data: [DONE]\n\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    base_url
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: serde_json::Value,
) -> T {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    assert!(
        response.error.is_none(),
        "RPC error for {method}: {:?}",
        response.error
    );
    serde_json::from_value(response.result.unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn process_extension_python_provider_serves_a_full_turn() {
    let base_url = start_fake_openai().await;
    let example = example_dir();

    let config = ProcessExtensionConfig {
        id: "python-chat-completions".to_string(),
        enabled: true,
        manifest: example.join("roder-extension.toml").display().to_string(),
        command: "python3".to_string(),
        args: vec!["-m".to_string(), "roder_python_chat_provider".to_string()],
        cwd: Some(example.display().to_string()),
        env: BTreeMap::from([
            ("PYTHONPATH".to_string(), "src".to_string()),
            ("PYTHONUNBUFFERED".to_string(), "1".to_string()),
            (
                "PY_CHAT_COMPLETIONS_API_KEY".to_string(),
                "test-key".to_string(),
            ),
            ("PY_CHAT_COMPLETIONS_BASE_URL".to_string(), base_url),
        ]),
        startup_timeout_ms: 20_000,
        event_filter: ProcessEventFilter {
            kinds: vec!["turn.".to_string()],
        },
    };
    let loaded = load_process_extension(config, &example).unwrap();

    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(ProcessHostExtension::new(loaded)).unwrap();
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: temp_dir("threads"),
    }));
    // The Python provider is the only inference engine: the runtime treats
    // it exactly like a native provider.
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: "python-chat-completions".to_string(),
                default_model: "gpt-5.5".to_string(),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let feature_config = AppServerFeatureConfig::default()
        .with_workspace_registry_path(temp_dir("registry").join("workspaces.json"));
    let client = LocalAppClient::new(Arc::new(AppServer::with_feature_config(
        runtime,
        feature_config,
    )));

    // The extension and provider are visible through the public surfaces,
    // indistinguishable from native extensions except by metadata.
    let extensions: ExtensionsListResult =
        request(&client, "extensions/list", serde_json::json!({})).await;
    assert!(
        extensions
            .extensions
            .iter()
            .any(|extension| extension.id == "roder-ext-python-chat-completions"),
        "{extensions:?}"
    );
    let providers: ProvidersListResult =
        request(&client, "providers/list", serde_json::json!({})).await;
    assert!(
        providers
            .providers
            .iter()
            .any(|provider| provider.id == "python-chat-completions"),
        "{providers:?}"
    );

    // Full turn through the standard thread/turn surfaces.
    let workspace_dir = temp_dir("workspace");
    let workspace: WorkspaceCreateResult = request(
        &client,
        "workspace/create",
        serde_json::to_value(WorkspaceCreateParams {
            name: None,
            roots: vec![WorkspaceRootInput {
                path: workspace_dir.display().to_string(),
                name: None,
            }],
            default_root_path: Some(workspace_dir.display().to_string()),
        })
        .unwrap(),
    )
    .await;
    let started: ThreadStartResult = request(
        &client,
        "thread/start",
        serde_json::to_value(ThreadStartParams {
            selection: None,
            workspace_id: workspace.workspace.id.clone(),
            root_id: Some(workspace.workspace.default_root_id.clone()),
            model: Some("gpt-5.5".to_string()),
            model_provider: Some("python-chat-completions".to_string()),
            reasoning: None,
            cwd: None,
            tool_allowlist: None,
            developer_instructions: None,
            external_tools: None,
            runner: None,
            ephemeral: false,
        })
        .unwrap(),
    )
    .await;

    let turn: TurnStartResult = request(
        &client,
        "turn/start",
        serde_json::to_value(TurnStartParams {
            thread_id: started.thread.id.clone(),
            input: Vec::new(),
            prompt: Some("say hello".to_string()),
            developer_context: None,
            model_provider: None,
            model: None,
            reasoning: None,
            policy_mode: None,
            task_ledger_required: false,
        })
        .unwrap(),
    )
    .await;

    // Poll thread/read until the streamed Python answer lands.
    let mut answer = String::new();
    for _ in 0..400 {
        let read: ThreadReadResult = request(
            &client,
            "thread/read",
            serde_json::to_value(ThreadReadParams {
                thread_id: started.thread.id.clone(),
                include_turns: true,
            })
            .unwrap(),
        )
        .await;
        let turns = read
            .thread
            .and_then(|thread| thread.turns)
            .unwrap_or_default();
        if let Some(record) = turns.iter().find(|record| record.id == turn.turn_id)
            && record.status == "completed"
        {
            for item in &record.items {
                if let roder_protocol::Item::AgentMessage { text, .. } = item {
                    answer.push_str(text);
                }
            }
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(
        answer, "Hello from python",
        "the Python provider's streamed answer must drive the normal turn surfaces"
    );
}

/// Opt-in live check against a real OpenAI-compatible endpoint:
///
/// ```sh
/// RODER_PROCESS_EXT_LIVE=1 \
/// PY_CHAT_COMPLETIONS_API_KEY=... \
/// PY_CHAT_COMPLETIONS_BASE_URL="https://api.openai.com/v1" \
/// PY_CHAT_COMPLETIONS_MODEL="gpt-5.5" \
/// cargo test -p roder-app-server --features e2e-tests \
///   --test process_extension_python_provider -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live provider check; set RODER_PROCESS_EXT_LIVE=1 and provider env vars"]
async fn process_extension_python_provider_live() {
    if std::env::var("RODER_PROCESS_EXT_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_PROCESS_EXT_LIVE=1 to run the live python provider check");
        return;
    }
    let api_key = std::env::var("PY_CHAT_COMPLETIONS_API_KEY").expect("api key env");
    let base_url = std::env::var("PY_CHAT_COMPLETIONS_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model = std::env::var("PY_CHAT_COMPLETIONS_MODEL").expect("model env");
    let example = example_dir();

    let config = ProcessExtensionConfig {
        id: "python-chat-completions".to_string(),
        enabled: true,
        manifest: example.join("roder-extension.toml").display().to_string(),
        command: "python3".to_string(),
        args: vec!["-m".to_string(), "roder_python_chat_provider".to_string()],
        cwd: Some(example.display().to_string()),
        env: BTreeMap::from([
            ("PYTHONPATH".to_string(), "src".to_string()),
            ("PYTHONUNBUFFERED".to_string(), "1".to_string()),
            ("PY_CHAT_COMPLETIONS_API_KEY".to_string(), api_key),
            ("PY_CHAT_COMPLETIONS_BASE_URL".to_string(), base_url),
            ("PY_CHAT_COMPLETIONS_MODEL".to_string(), model.clone()),
        ]),
        startup_timeout_ms: 20_000,
        event_filter: ProcessEventFilter::default(),
    };
    let loaded = load_process_extension(config, &example).unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.install(ProcessHostExtension::new(loaded)).unwrap();
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: "python-chat-completions".to_string(),
                default_model: model.clone(),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );

    // A single direct engine round-trip proves auth and request mapping.
    let engine = runtime
        .registry
        .inference_engine("python-chat-completions")
        .expect("python engine registered");
    let models = engine
        .list_models(roder_api::inference::InferenceProviderContext {
            provider_id: "python-chat-completions",
        })
        .await
        .unwrap();
    assert!(models.iter().any(|descriptor| descriptor.id == model));
    eprintln!("live python provider lists model {model}");
}
