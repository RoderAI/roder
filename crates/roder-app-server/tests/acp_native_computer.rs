// ACP -> Roder runtime -> actual Responses HTTP/SSE -> real Chrome -> native
// screenshot continuation. Only the decision-model endpoint is scripted.
use super::*;
use roder_api::inference::HostedWebSearchConfig;
use roder_api::policy_mode::PolicyMode;
use roder_core::RuntimeConfig;
use roder_ext_chrome::{ComputerCdpBinding, ComputerToolContributor};
use roder_ext_openai_responses::OpenAiResponsesEngine;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

#[path = "../../roder-ext-chrome/examples/native_computer/support.rs"]
mod support;

struct Endpoint {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Endpoint {
    async fn start(batches: Vec<Value>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let recorded = recorded.clone();
                let batches = batches.clone();
                tokio::spawn(async move {
                    let (path, body) = support::read_request(&mut socket).await.unwrap();
                    assert_eq!(path, "/responses");
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let index = {
                        let mut all = recorded.lock().await;
                        let index = all.len();
                        all.push(request);
                        index
                    };
                    let output = if let Some(actions) = batches.get(index) {
                        vec![
                            json!({"type":"computer_call","id":format!("cu_{index}"),"call_id":format!("native_{index}"),"actions":actions,"status":"completed"}),
                        ]
                    } else {
                        vec![
                            json!({"type":"message","id":"msg_final","role":"assistant","phase":"final_answer","status":"completed","content":[{"type":"output_text","text":"Browser actions finished; check the independent fixture grader."}]}),
                        ]
                    };
                    let mut sse = String::new();
                    for item in &output {
                        let mut started = item.clone();
                        started["status"] = json!("in_progress");
                        if started["type"] == "computer_call" {
                            started["actions"] = json!([]);
                        }
                        for (kind, item) in [
                            ("response.output_item.added", started),
                            ("response.output_item.done", item.clone()),
                        ] {
                            sse.push_str(&format!(
                                "event: {kind}\ndata: {}\n\n",
                                json!({"type":kind,"item":item})
                            ));
                        }
                    }
                    sse.push_str(&format!("event: response.completed\ndata: {}\n\n",json!({"type":"response.completed","response":{"id":format!("resp_{index}"),"output":output,"usage":{"input_tokens":50,"output_tokens":50}}})));
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        sse.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    // Split in the middle of JSON to exercise incremental SSE decoding.
                    for part in sse.as_bytes().chunks(37) {
                        socket.write_all(part).await.unwrap();
                    }
                });
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}

async fn exercise(batches: Vec<Value>, expected_errors: usize) -> Option<(Value, Vec<Value>)> {
    let browser = support::Browser::start().await.unwrap()?;
    let fixture = support::Fixture::start().await.unwrap();
    let endpoint = Endpoint::start(batches.clone()).await;
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(OpenAiResponsesEngine::new_with_config(
        Some("fixture-key".into()),
        "openai",
        &endpoint.url,
        Vec::new(),
    )));
    builder.tool_contributor(Arc::new(ComputerToolContributor::new(Arc::new(
        ComputerCdpBinding::new(&browser.endpoint, &fixture.url),
    ))));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: "openai".into(),
                default_model: "gpt-6-sol".into(),
                policy_mode: PolicyMode::Bypass,
                hosted_web_search: HostedWebSearchConfig::disabled(),
                tool_allowlist: vec!["computer".into()],
                auto_compact_token_limit: None,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let temp = tempfile::tempdir().unwrap();
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig::default()
            .with_workspace_registry_path(temp.path().join("workspaces.json")),
    ));
    let adapter = AcpAdapter::new(LocalAppClient::new(server));
    let peer = RecordingPeer::default();
    let request = |method: &str, params: Value| JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(method)),
        method: method.into(),
        params: Some(params),
    };
    let created = adapter
        .handle_request(
            request(
                "session/new",
                serde_json::to_value(acp::NewSessionRequest::new(
                    std::env::current_dir().unwrap(),
                ))
                .unwrap(),
            ),
            &peer,
        )
        .await
        .unwrap()
        .unwrap();
    let session = created.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let response = tokio::time::timeout(
        Duration::from_secs(100),
        adapter.handle_request(
            request(
                "session/prompt",
                serde_json::to_value(acp::PromptRequest::new(
                    session.clone(),
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "Use native computer actions on this fixture",
                    ))],
                ))
                .unwrap(),
            ),
            &peer,
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(response.result.unwrap()["stopReason"], "end_turn");
    let requests = endpoint.requests.lock().await.clone();
    assert_eq!(requests.len(), batches.len() + 1);
    for (round, body) in requests.iter().enumerate() {
        assert_eq!(body["tools"], json!([{"type":"computer"}]));
        assert_eq!(body["parallel_tool_calls"], false);
        let items = body["input"].as_array().unwrap();
        assert!(
            !items.iter().any(|item| matches!(
                item["type"].as_str(),
                Some("function_call" | "function_call_output")
            )),
            "{items:?}"
        );
        for prior in 0..round {
            let id = format!("native_{prior}");
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item["type"] == "computer_call" && item["call_id"] == id)
                    .count(),
                1
            );
            let output = items
                .iter()
                .find(|item| item["type"] == "computer_call_output" && item["call_id"] == id)
                .unwrap();
            assert_eq!(output["output"]["type"], "computer_screenshot");
            assert_eq!(output["output"]["detail"], "original");
            assert!(
                output["output"]["image_url"]
                    .as_str()
                    .unwrap()
                    .starts_with("data:image/jpeg;base64,")
            );
        }
    }
    let notifications = peer.notifications.lock().await;
    let updates: Vec<_> = notifications
        .iter()
        .filter(|n| n.method == "session/update")
        .map(|n| &n.params["update"])
        .collect();
    for (index, actions) in batches.iter().enumerate() {
        let id = format!("native_{index}");
        assert!(
            updates.iter().any(|u| u["sessionUpdate"] == "tool_call"
                && u["toolCallId"] == id
                && u["rawInput"] == json!({"actions":actions})),
            "Missing raw native call {id}"
        );
        assert!(
            updates
                .iter()
                .any(|u| u["sessionUpdate"] == "tool_call_update"
                    && u["toolCallId"] == id
                    && matches!(u["status"].as_str(), Some("completed" | "failed")))
        );
    }
    assert_eq!(
        updates
            .iter()
            .filter(|u| u["sessionUpdate"] == "tool_call_update" && u["status"] == "failed")
            .filter_map(|u| u["toolCallId"].as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        expected_errors
    );
    let grade = fixture.grade(expected_errors == 0).await;
    if let Ok(path) = std::env::var("RODER_NATIVE_EVAL_REPORT") {
        let screenshot_path = std::path::PathBuf::from(&path).with_extension("jpg");
        if let Some(image) = requests
            .last()
            .and_then(|body| body["input"].as_array())
            .and_then(|items| {
                items
                    .iter()
                    .rev()
                    .find(|item| item["type"] == "computer_call_output")
            })
            .and_then(|item| item["output"]["image_url"].as_str())
        {
            use base64::Engine;
            std::fs::write(
                &screenshot_path,
                base64::engine::general_purpose::STANDARD
                    .decode(image.split_once(',').unwrap().1)
                    .unwrap(),
            )
            .unwrap();
        }
        let report = json!({"mode":"scripted_native_responses_real_browser_acp","openai_api_validated":false,"requests":requests.len(),"computer_calls":batches.len(),
            "action_count":batches.iter().map(|actions|actions.as_array().unwrap().len()).sum::<usize>(),
            "action_types":batches.iter().flat_map(|actions|actions.as_array().unwrap().iter().map(|action|action["type"].as_str().unwrap())).collect::<std::collections::BTreeSet<_>>(),
            "errors":expected_errors,"grade":grade,"screenshot_path":screenshot_path});
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    Some((grade, requests))
}

#[tokio::test]
async fn native_computer_protocol_all_primitives_through_acp_and_real_browser() {
    let Some((grade, _)) = exercise(support::scripted_batches(), 0).await else {
        return;
    };
    assert_eq!(grade["passed"], true, "{grade}");
}

#[tokio::test]
async fn native_computer_partial_failure_returns_screen_and_stops_remaining_actions() {
    let batches = vec![json!([{"type":"click","button":"left","x":60,"y":40},
        {"type":"click","button":"invalid","x":60,"y":100},
        {"type":"type","text":"must not be typed"}])];
    let Some((grade, requests)) = exercise(batches, 1).await else {
        return;
    };
    let events = grade["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e["type"] == "filters"));
    assert!(!events.iter().any(|e| e["type"] == "input"));
    assert!(requests[1]["input"].as_array().unwrap().iter().any(
        |item| item["role"] == "user" && item.to_string().contains("Computer execution failed")
    ));
}

#[tokio::test]
async fn malformed_computer_actions_emit_failed_acp_update_with_observation_and_no_input() {
    let Some((grade, _)) = exercise(vec![json!([{"type":"bogus"}])], 1).await else {
        return;
    };
    assert!(grade["events"].as_array().unwrap().is_empty());
}
