use roder_api::{extension::ExtensionRegistryBuilder, inference::*};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_ext_openai_responses::provider::OpenAiResponsesEngine;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn exercise(
    engine: OpenAiResponsesEngine,
    fixture: &str,
    prompt: &str,
    initial: &[(&str, &str)],
    expected: &[(&str, Option<&str>)],
) -> anyhow::Result<Value> {
    let dir = tempfile::tempdir()?;
    for (path, text) in initial {
        let path = dir.path().join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
    }
    let mut builder = ExtensionRegistryBuilder::new();
    let provider = engine.id();
    builder.inference_engine(Arc::new(engine));
    builder.tool_contributor(Arc::new(roder_tools::BuiltinCodingToolsContributor::new(
        dir.path(),
    )?));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: dir.path().join(".transcripts"),
    }));
    let mut cfg = RuntimeConfig::default();
    cfg.default_provider = provider.clone();
    cfg.default_model = if provider == "codex" {
        std::env::var("RODER_PARITY_TEST_MODEL").unwrap_or_else(|_| "gpt-6-luna".into())
    } else {
        "mock".into()
    };
    let test_model = cfg.default_model.clone();
    cfg.tool_allowlist = vec![
        "read_file".into(),
        "apply_patch".into(),
        "list_files".into(),
        "grep".into(),
    ];
    cfg.policy_mode = roder_api::policy_mode::PolicyMode::Bypass;
    let runtime = Arc::new(Runtime::new(builder.build()?, cfg)?);
    let thread = runtime.create_thread(Some(fixture.into())).await?.thread_id;
    let mut events = runtime.subscribe_events();
    let started = Instant::now();
    let turn_id = runtime.start_turn(StartTurnRequest{thread_id:thread.clone(),message:prompt.into(),images:vec![],provider_override:None,model_override:None,
        reasoning_override:(provider=="codex").then(||"low".into()),workspace:dir.path().to_string_lossy().into(),instructions:InstructionBundle{
            system:Some("Complete the requested file edits. Use apply_patch for edits and read_file for inspection. Verify by reading the resulting files. Finish with DONE.".into()),
            developer:None,developer_context:None},developer_context:None,task_ledger_required:false,service_tier_override:None}).await?;
    let mut first_delta = None;
    let mut first_tool_start = None;
    let mut tool_count = 0;
    let mut failure = None;
    let mut last_kind = String::new();
    loop {
        let event = match tokio::time::timeout(Duration::from_secs(100), events.recv()).await {
            Ok(event) => event?,
            Err(_) => {
                failure = Some(json!({"timeout":true,"last_event_kind":last_kind}));
                runtime
                    .interrupt_turn(thread.clone(), turn_id.clone())
                    .await?;
                break;
            }
        };
        last_kind = event.kind.clone();
        if std::env::var("RODER_PARITY_TRACE").as_deref() == Ok("1") {
            eprintln!(
                "{fixture} {:.0}ms {}",
                started.elapsed().as_secs_f64() * 1000.0,
                event.kind
            );
        }
        if event.kind == "inference.event_received" && first_delta.is_none() {
            first_delta = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        if event.kind == "tool.call_started" && first_tool_start.is_none() {
            first_tool_start = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        if event.kind == "tool.call_completed" {
            tool_count += 1;
        }
        if event.kind == "turn.failed" {
            failure = Some(serde_json::to_value(&event)?);
            break;
        }
        if event.kind == "turn.completed" {
            break;
        }
    }
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let mut checks = Vec::new();
    for (path, contents) in expected {
        let actual = std::fs::read_to_string(dir.path().join(path)).ok();
        checks.push(json!({"path":path,"passed":actual.as_deref()==*contents}));
    }
    let mut native_compaction = None;
    if fixture == "compaction-smoke" && failure.is_none() {
        while runtime.active_turn_for_thread(&thread).await.is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let compact_turn = "native-compaction-smoke".to_string();
        match runtime
            .force_compact_thread(
                &thread,
                &compact_turn,
                Some("Preserve color BLUE and the verified file state".into()),
            )
            .await
        {
            Ok(outcome) => {
                native_compaction = Some(
                    json!({"compacted":outcome.compacted,"reason":outcome.reason,"estimated_before":outcome.estimated_tokens_before,"estimated_after":outcome.estimated_tokens_after}),
                )
            }
            Err(error) => {
                failure = Some(json!({"compaction_error":error.to_string()}));
            }
        }
        if failure.is_none() {
            runtime
                .start_turn(StartTurnRequest {
                    thread_id: thread.clone(),
                    message: "What color constraint did I ask you to preserve? Answer that color."
                        .into(),
                    images: vec![],
                    provider_override: None,
                    model_override: None,
                    reasoning_override: Some("low".into()),
                    workspace: dir.path().to_string_lossy().into(),
                    instructions: InstructionBundle {
                        system: Some("Answer the user's question from retained context.".into()),
                        developer: None,
                        developer_context: None,
                    },
                    developer_context: None,
                    task_ledger_required: false,
                    service_tier_override: None,
                })
                .await?;
            loop {
                let event = tokio::time::timeout(Duration::from_secs(100), events.recv()).await??;
                if event.kind == "turn.failed" {
                    failure = Some(serde_json::to_value(event)?);
                    break;
                }
                if event.kind == "turn.completed" {
                    break;
                }
            }
        }
    }
    let snapshot = runtime.load_thread(&thread).await?.unwrap();
    let final_text: String = snapshot
        .turns
        .iter()
        .flat_map(|turn| &turn.items)
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::AssistantMessage(message)
                if message.phase.as_deref() != Some("commentary") =>
            {
                Some(message.text.as_str())
            }
            _ => None,
        })
        .collect();
    let follow_up_answer: Option<String> = if fixture == "compaction-smoke" {
        snapshot.turns.last().map(|turn| {
            turn.items
                .iter()
                .filter_map(|item| match item {
                    roder_api::transcript::TranscriptItem::AssistantMessage(message) => {
                        Some(message.text.as_str())
                    }
                    _ => None,
                })
                .collect()
        })
    } else {
        None
    };
    let native_boundary =
        snapshot
            .turns
            .iter()
            .flat_map(|turn| &turn.items)
            .any(|item| match item {
                roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                    metadata.get("compacted_input").is_some()
                }
                _ => false,
            });
    let compaction_passed = fixture != "compaction-smoke"
        || (native_boundary
            && native_compaction
                .as_ref()
                .is_some_and(|outcome| outcome["compacted"] == true)
            && follow_up_answer
                .as_deref()
                .is_some_and(|text| text.to_uppercase().contains("BLUE")));
    let transport_events: Vec<Value> = snapshot
        .turns
        .iter()
        .flat_map(|turn| &turn.items)
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata)
                if metadata["type"] == "responses_transport" =>
            {
                Some(metadata.clone())
            }
            _ => None,
        })
        .collect();
    let patch_results: Vec<_> = snapshot
        .turns
        .iter()
        .flat_map(|turn| &turn.items)
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::ToolResult(result)
                if result.name.as_deref() == Some("apply_patch") =>
            {
                Some(result)
            }
            _ => None,
        })
        .collect();
    let patch_errors: Vec<_> = patch_results
        .iter()
        .filter(|result| result.is_error)
        .map(|result| result.result.clone())
        .collect();
    let budget_events: Vec<_> = snapshot
        .turns
        .iter()
        .flat_map(|turn| &turn.items)
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata)
                if metadata["type"] == "request_budget" =>
            {
                Some(metadata.clone())
            }
            _ => None,
        })
        .collect();
    Ok(
        json!({"fixture":fixture,"model":test_model,"reasoning":if provider=="codex"{"low"}else{"none"},"duration_ms":elapsed,"first_event_ms":first_delta,"first_tool_start_ms":first_tool_start,"patch_calls":patch_results.len(),"patch_errors":patch_errors,"request_budgets":budget_events,
        "native_compaction":native_compaction,"native_boundary":native_boundary,"follow_up_answer":follow_up_answer,"tool_count":tool_count,"transport_events":transport_events,"failure":failure,"checks":checks,"passed":compaction_passed&&failure.is_none()&&checks.iter().all(|check|check["passed"]==true),"final_answer":final_text,"usage":snapshot.metadata.and_then(|metadata|metadata.usage)}),
    )
}

async fn fixture_server(samples: usize) -> anyhow::Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move {
        for _ in 0..samples {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buf = [0; 4096];
            let header_end = loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    return;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(end) = data.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let header = String::from_utf8_lossy(&data[..header_end]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while data.len() < header_end + length {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    return;
                }
                data.extend_from_slice(&buf[..n]);
            }
            let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"DONE\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture-response\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"id\":\"fixture-message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"DONE\"}]}]}}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });
    Ok(base)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let live = args.iter().any(|arg| arg == "--live");
    let output = args.last().expect("output path");
    let mut results = Vec::new();
    if live {
        anyhow::ensure!(
            std::env::var("RODER_RESPONSES_PARITY_LIVE").as_deref() == Ok("1"),
            "live guard missing"
        );
        let (token, account) =
            if std::env::var("RODER_PARITY_CODEX_AUTH_SOURCE").as_deref() == Ok("codex-cli") {
                // Evaluation-only reuse of an existing login. Never refresh, copy,
                // or overwrite either product's saved credentials.
                let home =
                    std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("home unavailable"))?;
                let auth: Value = serde_json::from_slice(&std::fs::read(
                    std::path::PathBuf::from(home).join(".codex/auth.json"),
                )?)?;
                (
                    auth.pointer("/tokens/access_token")
                        .and_then(Value::as_str)
                        .filter(|token| !token.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("Codex CLI access token unavailable"))?
                        .to_string(),
                    auth.pointer("/tokens/account_id")
                        .and_then(Value::as_str)
                        .map(String::from),
                )
            } else {
                roder_codex_auth::access_token()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Codex authentication unavailable"))?
            };
        let mut headers = vec![
            ("originator".into(), "roder".into()),
            ("User-Agent".into(), "roder/0.1.0".into()),
        ];
        if let Some(account) = account {
            headers.push(("ChatGPT-Account-Id".into(), account));
        }
        let engine = || {
            OpenAiResponsesEngine::new_with_config(
                Some(token.clone()),
                "codex",
                "https://chatgpt.com/backend-api/codex",
                headers.clone(),
            )
        };
        if args.iter().any(|arg| arg == "--compact-smoke") {
            results.push(exercise(engine(),"compaction-smoke","Read src/settings.txt. Change enabled=false to enabled=true with apply_patch, verify the result, and remember that the durable color constraint is BLUE.",
                &[("src/settings.txt","enabled=false\n")],&[("src/settings.txt",Some("enabled=true\n"))]).await?);
        } else {
            results.push(exercise(engine(),"update-unicode","Read src/settings.txt. Change enabled=false to enabled=true and retries=1 to retries=3. Preserve the comment exactly. Use apply_patch and verify the file.",
            &[("src/settings.txt","# café — settings\nenabled=false\nretries=1\n")],&[("src/settings.txt",Some("# café — settings\nenabled=true\nretries=3\n"))]).await?);
            results.push(exercise(engine(),"move-add-delete","Read source.txt and destination.txt. With apply_patch move source.txt to destination.txt, replacing its content with moved=yes followed by a newline. Delete obsolete.txt. Create nested/new.txt containing created=yes followed by a newline. Verify the results.",
            &[("source.txt","moved=no\n"),("destination.txt","old destination\n"),("obsolete.txt","obsolete\n")],&[("source.txt",None),("destination.txt",Some("moved=yes\n")),("obsolete.txt",None),("nested/new.txt",Some("created=yes\n"))]).await?);
        }
    } else {
        // Held-open terminal responses quantify the same deterministic defect
        // for both builds; these numbers do not measure model quality.
        for sample in 0..20 {
            let base = fixture_server(1).await?;
            let engine = OpenAiResponsesEngine::new_with_config(
                Some("fixture-key".into()),
                "mock",
                base,
                vec![],
            );
            results.push(
                exercise(
                    engine,
                    &format!("held-open-{sample}"),
                    "Finish with DONE.",
                    &[],
                    &[],
                )
                .await?,
            );
        }
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&json!({"live":live,"results":results}))?,
    )?;
    println!("wrote {} fixture results", results.len());
    anyhow::ensure!(
        results.iter().all(|result| result["passed"] == true),
        "fixture failed; inspect the saved result artifact"
    );
    Ok(())
}
