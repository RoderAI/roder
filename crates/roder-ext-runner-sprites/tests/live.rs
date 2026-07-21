use std::path::PathBuf;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RunnerCommandRequest, RunnerDestination, RunnerManifest,
    RunnerManifestEntry,
};
use roder_ext_runner_sprites::{
    DEFAULT_APP_SERVER_TOKEN_ENV, LIVE_ENV, PROVIDER_ID, RODER_TOKEN_ENV, SpritesRunnerProvider,
    TOKEN_ENV,
};
use roder_protocol::{
    InitializeResult, JsonRpcRequest, JsonRpcResponse, ToolCallResult, ToolsListResult,
};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;

const LIVE_APP_SERVER_ENV: &str = "RODER_LIVE_SPRITES_APP_SERVER";
const LIVE_REMOTE_RODER_BIN_ENV: &str = "RODER_LIVE_SPRITES_REMOTE_RODER_BIN";
const LIVE_REPO_SOURCE_ENV: &str = "RODER_LIVE_SPRITES_REPO_SOURCE";

#[tokio::test]
#[ignore]
async fn live_sprites_smoke() {
    roder_ext_runner_sprites::run_live_smoke_if_enabled().await;
}

#[tokio::test]
#[ignore]
async fn live_sprites_repo_app_server_accepts_remote_control() -> anyhow::Result<()> {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1")
        || std::env::var(LIVE_APP_SERVER_ENV).ok().as_deref() != Some("1")
    {
        eprintln!(
            "set {LIVE_ENV}=1 and {LIVE_APP_SERVER_ENV}=1 to run the live Sprites app-server smoke"
        );
        return Ok(());
    }
    if std::env::var(RODER_TOKEN_ENV)
        .or_else(|_| std::env::var(TOKEN_ENV))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_none()
    {
        eprintln!("set {RODER_TOKEN_ENV} or {TOKEN_ENV} to run the live Sprites app-server smoke");
        return Ok(());
    }
    if std::env::var(DEFAULT_APP_SERVER_TOKEN_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_none()
    {
        eprintln!("set {DEFAULT_APP_SERVER_TOKEN_ENV} to run the live Sprites app-server smoke");
        return Ok(());
    }

    let repo_source = std::env::var(LIVE_REPO_SOURCE_ENV)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let cleanup = if std::env::var("RODER_SPRITES_LIVE_KEEP").ok().as_deref() == Some("1") {
        "keep"
    } else {
        "delete-on-close"
    };

    let mut app_server = serde_json::json!({
        "enabled": true,
        "workspace_path": "repo",
        "auth_token_env": DEFAULT_APP_SERVER_TOKEN_ENV,
        "env_passthrough": ["OPENAI_API_KEY", "RODER_SPRITES_TOKEN", "SPRITES_TOKEN"],
    });
    if let Ok(binary_path) = std::env::var(LIVE_REMOTE_RODER_BIN_ENV)
        && !binary_path.trim().is_empty()
    {
        app_server["local_binary_path"] = serde_json::json!(binary_path);
    }

    let provider = SpritesRunnerProvider::default();
    let destination = RunnerDestination {
        id: "sprites-live-app-server".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "sprite_name_prefix": "roder-live-app",
            "cleanup": cleanup,
            "url_auth": "public",
            "working_dir": "/home/sprite/roder-live-app",
            "app_server": app_server,
        }),
        default_manifest: RunnerManifest {
            entries: vec![RunnerManifestEntry {
                source: repo_source.clone(),
                target: PathBuf::from("repo"),
                writable: true,
            }],
            mounts: Vec::new(),
        },
    };

    let session = provider.create_session(destination).await?;
    let state = session.state();
    let app_server = state
        .metadata
        .get("remote_app_server")
        .ok_or_else(|| anyhow::anyhow!("missing remote_app_server metadata"))?;
    assert_eq!(app_server["workspace_path"], "repo");
    assert!(
        !state
            .metadata
            .to_string()
            .contains(&std::env::var(DEFAULT_APP_SERVER_TOKEN_ENV)?)
    );

    let command = session
        .run_command(RunnerCommandRequest {
            command_id: "live-app-server-pwd".to_string(),
            program: "pwd".to_string(),
            args: Vec::new(),
            cwd: Some(PathBuf::from("repo")),
            env: Vec::new(),
            timeout_ms: None,
        })
        .await?;
    assert_eq!(command.exit_code, Some(0));
    assert_eq!(command.stdout.trim(), "/home/sprite/roder-live-app/repo");

    let health_url = metadata_string(app_server, "health_url")?;
    wait_for_health(&health_url).await?;
    let connect_url = metadata_string(app_server, "connect_url")?;
    let token = std::env::var(DEFAULT_APP_SERVER_TOKEN_ENV)?;
    let mut request = connect_url.into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse()?);
    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await?;

    let initialize: InitializeResult =
        remote_request(&mut websocket, "init", "initialize", None).await?;
    assert!(
        initialize
            .cwd
            .as_deref()
            .is_some_and(|cwd| cwd.ends_with("/repo")),
        "remote app-server cwd should be the forked repo, got {:?}",
        initialize.cwd
    );

    let tools: ToolsListResult =
        remote_request(&mut websocket, "tools", "tools/list", None).await?;
    assert_tool_names(&tools);

    let runners: serde_json::Value =
        remote_request(&mut websocket, "runners", "runners/list", None).await?;
    let has_sprites_provider =
        runners["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|provider| {
                provider["providerId"].as_str() == Some("sprites")
                    || provider["provider_id"].as_str() == Some("sprites")
            });
    assert!(
        has_sprites_provider,
        "remote app-server should expose Sprites runner provider: {runners}"
    );

    let workspace: serde_json::Value = remote_request(
        &mut websocket,
        "workspace",
        "workspace/create",
        Some(serde_json::json!({
            "roots": [{ "path": initialize.cwd.as_deref().unwrap_or("/home/sprite/roder-live-app/repo") }]
        })),
    )
    .await?;
    let workspace = workspace
        .get("workspace")
        .ok_or_else(|| anyhow::anyhow!("workspace/create returned no workspace: {workspace}"))?;
    let workspace_id = metadata_string(workspace, "id")?;
    let root_id = metadata_string(workspace, "defaultRootId")?;

    let thread: serde_json::Value = remote_request(
        &mut websocket,
        "thread",
        "thread/start",
        Some(serde_json::json!({
            "model": null,
            "modelProvider": null,
            "reasoning": null,
            "workspaceId": workspace_id,
            "rootId": root_id,
            "cwd": initialize.cwd,
            "ephemeral": true,
        })),
    )
    .await?;
    let thread_id = metadata_string(
        thread
            .get("thread")
            .ok_or_else(|| anyhow::anyhow!("thread/start returned no thread: {thread}"))?,
        "id",
    )?;

    let created_goal: ToolCallResult = remote_request(
        &mut websocket,
        "create-goal",
        "tools/call",
        Some(serde_json::json!({
            "thread_id": thread_id,
            "tool_name": "create_goal",
            "arguments": { "objective": "prove remote sprite tool execution" },
        })),
    )
    .await?;
    assert!(
        !created_goal.is_error,
        "remote create_goal failed: {created_goal:?}"
    );
    assert!(created_goal.text.contains("remote sprite tool execution"));

    let current_goal: ToolCallResult = remote_request(
        &mut websocket,
        "get-goal",
        "tools/call",
        Some(serde_json::json!({
            "thread_id": thread_id,
            "tool_name": "get_goal",
            "arguments": {},
        })),
    )
    .await?;
    assert!(
        !current_goal.is_error,
        "remote get_goal failed: {current_goal:?}"
    );
    assert!(current_goal.text.contains("remote sprite tool execution"));

    session.close().await?;
    let _ = repo_source;
    Ok(())
}

async fn wait_for_health(url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let mut last_error = None;
    for _ in 0..60 {
        match client.get(url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                last_error = Some(anyhow::anyhow!("health returned {}", response.status()))
            }
            Err(error) => last_error = Some(error.into()),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("remote app-server health did not become ready")))
}

async fn remote_request<T: serde::de::DeserializeOwned>(
    websocket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: &str,
    method: &str,
    params: Option<serde_json::Value>,
) -> anyhow::Result<T> {
    websocket
        .send(Message::Text(
            serde_json::to_string(&JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(id)),
                method: method.to_string(),
                params,
            })?
            .into(),
        ))
        .await?;
    while let Some(message) = websocket.next().await {
        let Message::Text(text) = message? else {
            continue;
        };
        let response: JsonRpcResponse = serde_json::from_str(&text)?;
        if response.id.as_ref() != Some(&serde_json::json!(id)) {
            continue;
        }
        if let Some(error) = response.error {
            anyhow::bail!("{method} failed: {} ({})", error.message, error.code);
        }
        return Ok(serde_json::from_value(
            response
                .result
                .ok_or_else(|| anyhow::anyhow!("{method} returned no result"))?,
        )?);
    }
    anyhow::bail!("{method} connection closed before response")
}

fn metadata_string<'a>(metadata: &'a serde_json::Value, field: &str) -> anyhow::Result<&'a str> {
    metadata
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("remote_app_server metadata is missing {field}"))
}

fn assert_tool_names(tools: &ToolsListResult) {
    let names = tools
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "read_file",
        "list_files",
        "grep",
        "glob",
        "shell",
        "exec_command",
        "write_stdin",
        "update_plan",
        "get_goal",
        "create_goal",
        "update_goal",
        "request_user_input",
        "apply_patch",
        "spawn_agent",
        "send_message",
        "followup_task",
        "wait_agent",
        "list_agents",
        "close_agent",
        "memory_save",
        "memory_read",
        "media_generate_image",
        "design_read",
        "webwright.run_script",
        "zerolang_edit",
    ] {
        assert!(
            names.contains(&expected),
            "tools/list should expose {expected}: {names:?}"
        );
    }
}
