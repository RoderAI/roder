//! `roder agent-node ...` — secure Roder-to-Roder remote node CLI
//! (roadmap phase 67, Stage 2 entrypoints).
//!
//! - `serve`: run this machine as the authoritative agent node over
//!   `wss://` with mTLS-pinned controller trust and a single-use pairing
//!   token printed once at startup.
//! - `connect-check`: connect as a controller (enrolling with a pairing
//!   token on first use), call `initialize` and `thread/list`, and print
//!   the node identity. Proves the controller path without a TUI.

use std::path::PathBuf;
use std::sync::Arc;

use roder_app_server::AppServer;
use roder_app_server_node::agent_node::{
    AgentNodeOptions, DEFAULT_PAIRING_TTL, generate_identity, serve_agent_node,
};
use roder_app_server_node::{RemoteAppClient, RemoteNodeConnection};
use roder_protocol::JsonRpcRequest;

use crate::{CliOptions, build_runtime_from_config};

pub(crate) async fn run_agent_node_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]).await,
        Some("connect-check") => connect_check(&args[1..]).await,
        Some("connect") => connect(&args[1..]).await,
        Some("profiles") => profiles().await,
        Some("trust") => trust(&args[1..]).await,
        _ => {
            println!(
                "Usage:\n  roder agent-node serve [--listen <host:port>] [--name <label>]\n  roder agent-node connect <profile> [--model <id>]\n  roder agent-node connect-check --address <host:port> --fingerprint <sha256> [--token <pairing-token>]\n  roder agent-node profiles\n  roder agent-node trust list\n  roder agent-node trust revoke <controller-fingerprint>\n\nAgent-node control is always TLS; controllers authenticate with mTLS\n(certificates enrolled via a single-use pairing token). `connect` opens the\nfull TUI against a `[[agent_nodes]]` profile from config; pairing tokens\nare read from the profile's `token_env` variable, never from config. See\ndocs/roder-agent-mode-remote-node.md."
            );
            Ok(())
        }
    }
}

fn value_of(args: &[String], name: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == name {
            return iter.next().cloned();
        }
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn state_dir() -> PathBuf {
    roder_config::config_dir().join("agent-node")
}

async fn serve(args: &[String]) -> anyhow::Result<()> {
    let listen = value_of(args, "--listen").unwrap_or_else(|| "127.0.0.1:7878".to_string());
    let node_name = value_of(args, "--name")
        .unwrap_or_else(|| std::env::var("HOSTNAME").unwrap_or_else(|_| "roder-node".to_string()));
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let workspace = std::env::current_dir()
        .ok()
        .map(|dir| dir.display().to_string());
    let app_server = Arc::new(AppServer::new(runtime));
    let controller = serve_agent_node(
        app_server,
        AgentNodeOptions {
            listen,
            node_name: node_name.clone(),
            state_dir: state_dir(),
            workspace,
        },
    )
    .await?;
    let (pairing_token, preview) = controller.handle.tokens.mint(DEFAULT_PAIRING_TTL);

    println!(
        "agent node listening on wss://{}",
        controller.handle.listen_addr
    );
    println!("node id          {}", controller.handle.node_id);
    println!("node name        {node_name}");
    println!("cert fingerprint {}", controller.handle.fingerprint);
    println!(
        "pairing token    {pairing_token}   (single use, valid {} minutes, preview {preview})",
        DEFAULT_PAIRING_TTL.whole_minutes()
    );
    println!();
    println!("enroll a controller from another machine with:");
    println!(
        "  roder agent-node connect-check --address <this-host>:{} --fingerprint {} --token {pairing_token}",
        controller.handle.listen_addr.port(),
        controller.handle.fingerprint
    );
    println!("press Ctrl-C to stop the node");

    tokio::signal::ctrl_c().await?;
    controller.stop().await?;
    Ok(())
}

/// Loads (or creates) the persistent controller identity.
fn controller_identity() -> anyhow::Result<roder_app_server_node::agent_node::TlsIdentity> {
    let dir = state_dir().join("controller");
    std::fs::create_dir_all(&dir)?;
    let cert_path = dir.join("controller-cert.pem");
    let key_path = dir.join("controller-key.pem");
    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        let fingerprint = roder_app_server_node::agent_node::fingerprint_from_pem(&cert_pem)?;
        return Ok(roder_app_server_node::agent_node::TlsIdentity {
            cert_pem,
            key_pem,
            fingerprint,
        });
    }
    let identity = generate_identity("roder-controller")?;
    std::fs::write(&cert_path, &identity.cert_pem)?;
    std::fs::write(&key_path, &identity.key_pem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(identity)
}

async fn connect_check(args: &[String]) -> anyhow::Result<()> {
    let address = value_of(args, "--address")
        .ok_or_else(|| anyhow::anyhow!("--address <host:port> is required"))?;
    let fingerprint = value_of(args, "--fingerprint")
        .ok_or_else(|| anyhow::anyhow!("--fingerprint <node sha256> is required"))?;
    let identity = controller_identity()?;
    println!("controller fingerprint {}", identity.fingerprint);

    let client = RemoteAppClient::connect(RemoteNodeConnection {
        address,
        server_fingerprint: fingerprint,
        controller_identity: identity,
        pairing_token: value_of(args, "--token"),
    })
    .await?;

    let response = client
        .request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({})),
        })
        .await;
    if let Some(error) = response.error {
        anyhow::bail!("initialize failed: {}", error.message);
    }
    let result = response.result.unwrap_or_default();
    let node = &result["node"];
    println!(
        "connected to node {} ({}) auth={} protocol={}",
        node["nodeId"].as_str().unwrap_or("?"),
        node["name"].as_str().unwrap_or("?"),
        node["authMode"].as_str().unwrap_or("?"),
        node["protocolVersion"].as_str().unwrap_or("?"),
    );

    let response = client
        .request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(2)),
            method: "thread/list".to_string(),
            params: Some(serde_json::json!({})),
        })
        .await;
    match response.error {
        None => {
            let count = response.result.as_ref().and_then(|result| {
                result
                    .get("data")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::len)
            });
            println!(
                "thread/list ok ({} thread(s) on the node)",
                count.unwrap_or(0)
            );
        }
        Some(error) => anyhow::bail!("thread/list failed: {}", error.message),
    }
    Ok(())
}

async fn profiles() -> anyhow::Result<()> {
    let config = roder_config::load_config()?;
    if config.agent_nodes.is_empty() {
        println!(
            "no [[agent_nodes]] profiles configured; add one to config.toml:\n\n[[agent_nodes]]\nname = \"studio\"\naddress = \"studio.local:7878\"\nfingerprint = \"<node cert sha256>\"\ntoken_env = \"RODER_STUDIO_TOKEN\"   # only needed for first enrollment"
        );
        return Ok(());
    }
    println!("{:<14} {:<24} fingerprint", "name", "address");
    for profile in config.agent_nodes {
        println!(
            "{:<14} {:<24} {}{}",
            profile.name,
            profile.address,
            profile.fingerprint,
            profile
                .token_env
                .as_deref()
                .map(|env| format!("  (token env {env})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

fn profile_named(name: &str) -> anyhow::Result<roder_config::agent_node::AgentNodeProfile> {
    let config = roder_config::load_config()?;
    config
        .agent_nodes
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| {
            let known = config
                .agent_nodes
                .iter()
                .map(|profile| profile.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!(
                "agent-node profile {name:?} is not configured (known: {})",
                if known.is_empty() { "none" } else { &known }
            )
        })
}

/// Connects to a configured node profile and runs the full TUI against it.
async fn connect(args: &[String]) -> anyhow::Result<()> {
    let Some(name) = args.first().filter(|arg| !arg.starts_with('-')) else {
        anyhow::bail!(
            "usage: roder agent-node connect <profile> (see `roder agent-node profiles`)"
        );
    };
    let profile = profile_named(name)?;
    let pairing_token = profile
        .token_env
        .as_deref()
        .and_then(|env| std::env::var(env).ok())
        .filter(|token| !token.trim().is_empty());
    let identity = controller_identity()?;

    let client = RemoteAppClient::connect(RemoteNodeConnection {
        address: profile.address.clone(),
        server_fingerprint: profile.fingerprint.clone(),
        controller_identity: identity,
        pairing_token,
    })
    .await
    .map_err(|error| {
        anyhow::anyhow!(
            "could not connect to agent node {name:?} at {}: {error}. If this controller \
             was never enrolled, mint a pairing token on the node (`roder agent-node serve` \
             prints one) and export it as the profile's token_env.",
            profile.address
        )
    })?;

    // Pick a model the node actually serves.
    let model = match value_of(args, "--model") {
        Some(model) => model,
        None => {
            let response = client
                .request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("model-list")),
                    method: "model/list".to_string(),
                    params: Some(serde_json::json!({})),
                })
                .await;
            let models: Vec<serde_json::Value> = response
                .result
                .as_ref()
                .and_then(|result| result.get("models"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            models
                .iter()
                .find(|model| model["isDefault"].as_bool().unwrap_or(false))
                .or_else(|| models.first())
                .and_then(|model| model["id"].as_str())
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("the node reports no models; pass --model"))?
        }
    };

    // The remote panel feature still needs a local app-server handle; the
    // conversation itself runs entirely on the remote node.
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let panel_server = Arc::new(AppServer::new(runtime));
    let mut tui = roder_tui::TuiApp::new_with_startup_and_remote(
        client,
        model,
        roder_tui::TuiStartup::NewThread,
        crate::remote_panel::remote_panel_for(panel_server),
    )
    .await?;
    tui.announce_remote_node().await;
    tui.run().await?;
    Ok(())
}

async fn trust(args: &[String]) -> anyhow::Result<()> {
    let trust = roder_app_server_node::agent_node::ControllerTrust::open(&state_dir())?;
    match args.first().map(String::as_str) {
        Some("list") => {
            let controllers = trust.controllers();
            if controllers.is_empty() {
                println!("no enrolled controllers");
            }
            for (fingerprint, label) in controllers {
                println!("enrolled\t{fingerprint}\t{label}");
            }
            Ok(())
        }
        Some("revoke") => {
            let Some(fingerprint) = args.get(1) else {
                anyhow::bail!("usage: roder agent-node trust revoke <controller-fingerprint>");
            };
            trust.revoke(fingerprint)?;
            println!(
                "revoked controller {fingerprint}; existing connections close on their next \
                 request and re-enrollment requires a new pairing token"
            );
            Ok(())
        }
        _ => {
            println!("usage: roder agent-node trust list | revoke <controller-fingerprint>");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_node_flag_parsing_supports_separate_and_equals_forms() {
        let args = vec![
            "--address".to_string(),
            "127.0.0.1:7878".to_string(),
            "--fingerprint=abc123".to_string(),
        ];
        assert_eq!(
            value_of(&args, "--address").as_deref(),
            Some("127.0.0.1:7878")
        );
        assert_eq!(value_of(&args, "--fingerprint").as_deref(), Some("abc123"));
        assert_eq!(value_of(&args, "--token"), None);
    }
}
