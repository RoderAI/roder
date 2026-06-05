//! `roder chrome <subcommand>` CLI surface and the `--chrome` startup flag.
//!
//! Each subcommand talks to a local in-process app-server (the same
//! [`LocalAppClient`] mechanism the other CLI commands use) and calls the
//! `chrome/*` JSON-RPC methods. Output is a concise human summary; browser
//! status fields are connection metadata, never untrusted page content.

use std::sync::Arc;

use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{ChromeStatus, JsonRpcRequest};

use crate::{CliOptions, build_runtime_from_config, decode_response};

/// Entry point for `roder chrome ...`.
pub async fn run_chrome_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("status") | None => {
            let client = chrome_client().await?;
            let status = chrome_call::<ChromeStatus>(&client, "chrome/status", None).await?;
            print_status(&status);
        }
        Some("enable") => {
            let mode = flag_value(args, "--mode");
            let params = mode.map(|mode| serde_json::json!({ "mode": mode }));
            let client = chrome_client().await?;
            let status = chrome_call::<ChromeStatus>(&client, "chrome/enable", params).await?;
            println!("chrome tools enabled");
            print_status(&status);
        }
        Some("disable") => {
            let client = chrome_client().await?;
            let status = chrome_call::<ChromeStatus>(&client, "chrome/disable", None).await?;
            println!("chrome tools disabled");
            print_status(&status);
        }
        Some("reconnect") => {
            let client = chrome_client().await?;
            let status = chrome_call::<ChromeStatus>(&client, "chrome/reconnect", None).await?;
            print_status(&status);
        }
        Some("install-host") | Some("uninstall-host") => {
            // Native messaging host install is not yet available. The browser
            // extension currently pairs over the remote WebSocket (`/remote`)
            // using the subprotocol bearer flow, so no host manifest is written.
            println!(
                "native messaging host install is not yet available; pair the browser \
                 extension over the remote WebSocket (`roder remote` / the /remote endpoint) \
                 using the subprotocol bearer flow instead"
            );
        }
        Some(other) => {
            anyhow::bail!(
                "unknown chrome subcommand {other:?}; usage: roder chrome \
                 <status|enable [--mode observe|assist|control]|disable|reconnect|install-host|uninstall-host>"
            );
        }
    }
    Ok(())
}

/// Enable Chrome tools for a session started via the top-level `--chrome` flag.
///
/// Returns the resulting [`ChromeStatus`] so the caller can surface it. Failure
/// to enable is non-fatal to startup and is reported to the caller.
pub async fn enable_chrome_for_session(client: &LocalAppClient) -> anyhow::Result<ChromeStatus> {
    chrome_call::<ChromeStatus>(client, "chrome/enable", None).await
}

async fn chrome_client() -> anyhow::Result<LocalAppClient> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    Ok(LocalAppClient::new(Arc::new(AppServer::new(runtime))))
}

async fn chrome_call<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> anyhow::Result<T> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params,
        })
        .await;
    decode_response::<T>(res)
}

fn print_status(status: &ChromeStatus) {
    println!(
        "connected\t{}\nenabled\t{}\nmode\t{}\nclientCount\t{}\ncapabilities\t{}",
        status.connected,
        status.enabled,
        status.mode.as_str(),
        status.client_count,
        if status.capabilities.is_empty() {
            "-".to_string()
        } else {
            status.capabilities.join(",")
        },
    );
    if let Some(error) = status.last_error.as_deref() {
        println!("lastError\t{error}");
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chrome_status_runs_against_disconnected_bridge() {
        // The global bridge starts disconnected in a fresh process.
        run_chrome_cli(&["status".to_string()]).await.unwrap();
    }

    #[tokio::test]
    async fn chrome_install_host_reports_stub() {
        run_chrome_cli(&["install-host".to_string()])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn chrome_unknown_subcommand_errors() {
        let err = run_chrome_cli(&["bogus".to_string()]).await.unwrap_err();
        assert!(err.to_string().contains("unknown chrome subcommand"));
    }
}
