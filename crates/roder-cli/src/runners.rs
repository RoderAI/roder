//! `roder runners ...` CLI: inspect remote-runner providers and drive the
//! thread-scoped lifecycle (pause, resume, detach, rejoin) for runner-bound
//! threads (e.g. Blaxel sandboxes). Talks to an in-process app-server.

use std::sync::Arc;

use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, RunnersLifecycleParams, RunnersLifecycleResult, RunnersListResult,
};

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub(crate) async fn run_runners_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("list") => runners_list().await,
        Some(action @ ("pause" | "resume" | "detach" | "rejoin")) => {
            lifecycle(action, &args[1..]).await
        }
        _ => {
            println!(
                "Usage:\n  \
                 roder runners list\n  \
                 roder runners pause <thread-id>\n  \
                 roder runners resume <thread-id>\n  \
                 roder runners detach <thread-id>\n  \
                 roder runners rejoin <thread-id> [--sandbox <name>]\n\n\
                 Pause lets a sandbox scale to standby; resume wakes it; detach \
                 keeps the sandbox alive and releases the local session; rejoin \
                 reattaches from persisted thread state without creating a new \
                 sandbox."
            );
            Ok(())
        }
    }
}

async fn client() -> anyhow::Result<LocalAppClient> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    Ok(LocalAppClient::new(Arc::new(AppServer::new(runtime))))
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned();
        }
    }
    None
}

async fn call<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<T> {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    decode_response(response)
}

async fn runners_list() -> anyhow::Result<()> {
    let client = client().await?;
    let result: RunnersListResult = call(&client, "runners/list", serde_json::json!({})).await?;
    if let Some(active) = &result.active {
        println!(
            "active\t{}\t{}\t{}",
            active.destination_id, active.provider_id, active.state
        );
    }
    println!("{:<14} capabilities", "provider");
    for provider in result.providers {
        let caps = &provider.capabilities;
        let mut labels = Vec::new();
        if caps.command_exec {
            labels.push("exec");
        }
        if caps.file_read || caps.file_write {
            labels.push("files");
        }
        if caps.port_preview {
            labels.push("ports");
        }
        if caps.pausable {
            labels.push("pause/resume");
        }
        if caps.detachable {
            labels.push("detach/rejoin");
        }
        println!("{:<14} {}", provider.provider_id, labels.join(", "));
        if let Some(hint) = provider.setup_hint {
            println!("               setup: {hint}");
        }
    }
    Ok(())
}

async fn lifecycle(action: &str, args: &[String]) -> anyhow::Result<()> {
    let Some(thread_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!("usage: roder runners {action} <thread-id>");
    };
    let params = RunnersLifecycleParams {
        thread_id: thread_id.clone(),
        sandbox: flag_value(args, "--sandbox"),
    };
    let client = client().await?;
    let result: RunnersLifecycleResult = call(
        &client,
        &format!("runners/{action}"),
        serde_json::to_value(params)?,
    )
    .await?;
    println!("action\t{}", result.action);
    println!("provider\t{}", result.provider_id);
    if let Some(session_id) = result.session_id {
        println!("session\t{session_id}");
    }
    println!("paused\t{}", result.paused);
    println!("detached\t{}", result.detached);
    Ok(())
}
