use std::path::{Path, PathBuf};
use std::sync::Arc;

mod commands;
mod marketplace;
mod resume_picker;
#[cfg(test)]
mod tui_config;

use marketplace::{run_marketplace_cli, run_plugin_cli, run_setup_cli};
use roder_api::catalog::{DEFAULT_MODEL_ID, PROVIDER_MOCK, normalize_provider_id};
use roder_api::inference::HostedWebSearchConfig;
use roder_api::notifications::NotificationKind;
use roder_api::policy_mode::PolicyMode;
use roder_api::remote_runner::{RunnerDestination, RunnerManifest};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig, validate_edit_tool};
use roder_ext_subagents::{AgentLoadConfig, load_agent_definitions};
use roder_extension_host::{
    CustomInferenceProviderConfig, DefaultNotificationsConfig, DefaultRegistryConfig,
    DefaultSubagentsConfig, DefaultWebSearchConfig, DefaultWebSearchProviderConfig,
    build_default_registry,
};
use roder_protocol::{
    DesktopThread, JsonRpcError, JsonRpcRequest, JsonRpcResponse, MemoryDeleteParams,
    MemoryDeleteResult, MemoryListParams, MemoryListResult, MemoryProviderListResult,
    MemoryProviderSetParams, MemoryQueryParams, MemoryQueryResult, MemoryReadParams,
    MemoryReadResult, MemorySaveParams, MemorySaveResult, MemoryUpdateParams, TasksCancelParams,
    TasksCancelResult, TasksGetParams, TasksGetResult, TasksListResult, TasksSubmitParams,
    TasksSubmitResult, ThreadListParams, ThreadListResult, ThreadReadParams, ThreadReadResult,
    WorkflowEnableParams, WorkflowEnableResult, WorkflowPreviewParams, WorkflowPreviewResult,
    WorkflowScanParams, WorkflowScanResult,
};
use roder_tui::{TuiApp, TuiStartup};
use roder_web_search::WebSearchProviderKind;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.first().map(String::as_str), Some("auth")) {
        return run_auth(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("app-server")) {
        return run_app_server(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("commands")) {
        let cfg = roder_config::load_config()?;
        return commands::run_commands_cli(&args[1..], &cfg);
    }
    if matches!(args.first().map(String::as_str), Some("tasks")) {
        return run_tasks_cli(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("workflow")) {
        return run_workflow_cli(&args[1..]).await;
    }
    if matches!(
        args.first().map(String::as_str),
        Some("marketplace" | "marketplaces")
    ) {
        return run_marketplace_cli(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("plugin" | "plugins")) {
        return run_plugin_cli(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("setup")) {
        return run_setup_cli(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("memory")) {
        return run_memory_cli(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("team")) {
        return run_team_cli(&args[1..]).await;
    }

    let cli_options = parse_cli_options(&args)?;
    let mut startup = cli_options.startup.clone();
    let (runtime, default_model) = build_runtime_from_config(cli_options).await?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    let client = LocalAppClient::new(app_server);

    if matches!(startup, TuiStartup::ResumeMenu) {
        let sessions = list_sessions(&client).await?;
        let Some(thread_id) = resume_picker::pick_session(&sessions)? else {
            return Ok(());
        };
        startup = TuiStartup::ResumeSession(thread_id);
    }

    let mut tui = TuiApp::new_with_startup(client, default_model, startup).await?;
    tui.run().await?;
    print_tui_exit_summary(&tui);
    Ok(())
}

async fn run_memory_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("list") => {
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/list")),
                    method: "memory/list".to_string(),
                    params: Some(serde_json::to_value(MemoryListParams {
                        scope: memory_scope_arg(args),
                        limit: Some(50),
                    })?),
                })
                .await;
            for memory in decode_response::<MemoryListResult>(res)?.memories {
                println!(
                    "{}\t{}\t{}",
                    memory.id.unwrap_or_default(),
                    memory.scope.stable_id(),
                    one_line(&memory.text)
                );
            }
        }
        Some("query") => {
            let Some(text) = args.get(1) else {
                anyhow::bail!(
                    "usage: roder memory query TEXT [--scope project|global] [--include-global]"
                );
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/query")),
                    method: "memory/query".to_string(),
                    params: Some(serde_json::to_value(MemoryQueryParams {
                        scope: memory_scope_arg(args),
                        text: text.clone(),
                        limit: Some(10),
                        include_global: args.iter().any(|arg| arg == "--include-global"),
                    })?),
                })
                .await;
            for result in decode_response::<MemoryQueryResult>(res)?.results {
                println!(
                    "{:.3}\t{}\t{}",
                    result.score,
                    result.record.id.unwrap_or_default(),
                    one_line(&result.record.text)
                );
            }
        }
        Some("save") => {
            let Some(text) = args.get(1) else {
                anyhow::bail!("usage: roder memory save TEXT [--scope project|global]");
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/save")),
                    method: "memory/save".to_string(),
                    params: Some(serde_json::to_value(MemorySaveParams {
                        scope: memory_scope_arg(args).unwrap_or_else(default_project_scope),
                        text: text.clone(),
                        metadata: serde_json::json!({}),
                    })?),
                })
                .await;
            println!("{}", decode_response::<MemorySaveResult>(res)?.memory_id);
        }
        Some("read") => {
            let Some(memory_id) = args.get(1) else {
                anyhow::bail!("usage: roder memory read ID");
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/read")),
                    method: "memory/read".to_string(),
                    params: Some(serde_json::to_value(MemoryReadParams {
                        memory_id: memory_id.clone(),
                    })?),
                })
                .await;
            if let Some(memory) = decode_response::<MemoryReadResult>(res)?.memory {
                println!("{}", memory.text);
            }
        }
        Some("update") => {
            let (Some(memory_id), Some(text)) = (args.get(1), args.get(2)) else {
                anyhow::bail!("usage: roder memory update ID TEXT");
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/update")),
                    method: "memory/update".to_string(),
                    params: Some(serde_json::to_value(MemoryUpdateParams {
                        memory_id: memory_id.clone(),
                        text: text.clone(),
                        metadata: serde_json::json!({}),
                    })?),
                })
                .await;
            println!("{}", decode_response::<MemorySaveResult>(res)?.memory_id);
        }
        Some("delete") => {
            let Some(memory_id) = args.get(1) else {
                anyhow::bail!("usage: roder memory delete ID");
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/delete")),
                    method: "memory/delete".to_string(),
                    params: Some(serde_json::to_value(MemoryDeleteParams {
                        memory_id: memory_id.clone(),
                    })?),
                })
                .await;
            println!(
                "deleted: {}",
                decode_response::<MemoryDeleteResult>(res)?.deleted
            );
        }
        Some("providers") if matches!(args.get(1).map(String::as_str), Some("list")) => {
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/provider/list")),
                    method: "memory/provider/list".to_string(),
                    params: None,
                })
                .await;
            let result = decode_response::<MemoryProviderListResult>(res)?;
            println!(
                "selected\t{}\t{}",
                result.selected.provider_id, result.selected.model
            );
            for provider in result.providers {
                println!("provider\t{}\t{}", provider.id, provider.default_model);
            }
        }
        Some("providers") if matches!(args.get(1).map(String::as_str), Some("set")) => {
            let Some(provider_id) = args.get(2) else {
                anyhow::bail!("usage: roder memory providers set PROVIDER --model MODEL");
            };
            let model = args
                .iter()
                .position(|arg| arg == "--model")
                .and_then(|idx| args.get(idx + 1))
                .cloned()
                .unwrap_or_else(|| "text-embedding-3-large".to_string());
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("memory/provider/set")),
                    method: "memory/provider/set".to_string(),
                    params: Some(serde_json::to_value(MemoryProviderSetParams {
                        provider_id: provider_id.clone(),
                        model,
                    })?),
                })
                .await;
            println!(
                "{}",
                serde_json::to_string_pretty(&decode_response::<serde_json::Value>(res)?)?
            );
        }
        Some("reembed") => {
            let provider = args
                .iter()
                .position(|arg| arg == "--provider")
                .and_then(|idx| args.get(idx + 1))
                .cloned()
                .unwrap_or_else(|| "openai".to_string());
            let model = args
                .iter()
                .position(|arg| arg == "--model")
                .and_then(|idx| args.get(idx + 1))
                .cloned()
                .unwrap_or_else(|| "text-embedding-3-large".to_string());
            println!("queued reembed\t{}\t{}", provider, model);
        }
        _ => anyhow::bail!(
            "usage: roder memory <list|query|save|read|update|delete|providers list|providers set|reembed>"
        ),
    }
    Ok(())
}

async fn list_sessions(client: &LocalAppClient) -> anyhow::Result<Vec<DesktopThread>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/list")),
            method: "thread/list".to_string(),
            params: Some(serde_json::to_value(ThreadListParams { limit: None })?),
        })
        .await;
    let mut threads = Vec::new();
    for thread in decode_response::<ThreadListResult>(res)?.data {
        if let Ok(Some(full_thread)) = read_thread(client, &thread.id).await
            && thread_has_user_message(&full_thread)
        {
            threads.push(full_thread);
        }
    }
    threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
    Ok(threads)
}

async fn read_thread(
    client: &LocalAppClient,
    thread_id: &str,
) -> anyhow::Result<Option<DesktopThread>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/read")),
            method: "thread/read".to_string(),
            params: Some(serde_json::to_value(ThreadReadParams {
                thread_id: thread_id.to_string(),
                include_turns: true,
            })?),
        })
        .await;
    Ok(decode_response::<ThreadReadResult>(res)?.thread)
}

fn thread_has_user_message(thread: &DesktopThread) -> bool {
    thread
        .turns
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|turn| turn.items.iter())
        .any(|item| {
            item.kind == "userMessage"
                && item
                    .text
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
        })
}

async fn run_tasks_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("submit") => {
            let Some(executor_id) = args.get(1) else {
                anyhow::bail!("roder tasks submit requires an executor id");
            };
            let input = match args.get(2) {
                Some(raw) => serde_json::from_str(raw)?,
                None => serde_json::json!({}),
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("tasks/submit")),
                    method: "tasks/submit".to_string(),
                    params: Some(serde_json::to_value(TasksSubmitParams {
                        executor_id: executor_id.clone(),
                        input,
                        thread_id: None,
                        turn_id: None,
                        workspace: None,
                    })?),
                })
                .await;
            let submitted = decode_response::<TasksSubmitResult>(res)?.task;
            println!(
                "{}\t{}\t{:?}\t{}",
                submitted.task_id, submitted.executor_id, submitted.state, submitted.spec.kind
            );
            loop {
                let result = task_get(&client, &submitted.task_id).await?;
                if matches!(
                    result.task.state,
                    roder_api::tasks::TaskState::Completed
                        | roder_api::tasks::TaskState::Failed
                        | roder_api::tasks::TaskState::Cancelled
                ) {
                    for entry in result.logs {
                        print!("{}", entry.chunk);
                    }
                    println!(
                        "\n{}\t{}\t{:?}",
                        result.task.task_id, result.task.executor_id, result.task.state
                    );
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
        Some("list") => {
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("tasks/list")),
                    method: "tasks/list".to_string(),
                    params: None,
                })
                .await;
            for task in decode_response::<TasksListResult>(res)?.tasks {
                println!(
                    "{}\t{}\t{:?}\t{}",
                    task.task_id, task.executor_id, task.state, task.spec.kind
                );
            }
        }
        Some("show") => {
            let Some(task_id) = args.get(1) else {
                anyhow::bail!("roder tasks show requires a task id");
            };
            let result = task_get(&client, task_id).await?;
            println!(
                "{}\t{}\t{:?}",
                result.task.task_id, result.task.executor_id, result.task.state
            );
            for entry in result.logs {
                print!("{}", entry.chunk);
            }
        }
        Some("cancel") => {
            let Some(task_id) = args.get(1) else {
                anyhow::bail!("roder tasks cancel requires a task id");
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("tasks/cancel")),
                    method: "tasks/cancel".to_string(),
                    params: Some(serde_json::to_value(TasksCancelParams {
                        task_id: task_id.clone(),
                        reason: Some("cli cancel".to_string()),
                    })?),
                })
                .await;
            let result = decode_response::<TasksCancelResult>(res)?;
            println!("cancelled: {}", result.cancelled);
        }
        _ => anyhow::bail!("usage: roder tasks <submit EXECUTOR JSON|list|show ID|cancel ID>"),
    }
    Ok(())
}

async fn task_get(client: &LocalAppClient, task_id: &str) -> anyhow::Result<TasksGetResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("tasks/get")),
            method: "tasks/get".to_string(),
            params: Some(serde_json::to_value(TasksGetParams {
                task_id: task_id.to_string(),
            })?),
        })
        .await;
    decode_response::<TasksGetResult>(res)
}

async fn run_workflow_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("scan") => {
            let workspace = workflow_workspace_arg(args, 1);
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("workflow/scan")),
                    method: "workflow/scan".to_string(),
                    params: Some(serde_json::to_value(WorkflowScanParams {
                        workspace,
                        include_user: true,
                    })?),
                })
                .await;
            let result = decode_response::<WorkflowScanResult>(res)?;
            print_workflow_items(&result.scan.items);
            for error in result.scan.errors {
                eprintln!("error\t{}\t{}", error.path, error.message);
            }
        }
        Some("preview") => {
            let item_id = args.get(1).cloned();
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("workflow/preview")),
                    method: "workflow/preview".to_string(),
                    params: Some(serde_json::to_value(WorkflowPreviewParams {
                        workspace: None,
                        item_id,
                    })?),
                })
                .await;
            let result = decode_response::<WorkflowPreviewResult>(res)?;
            println!("{}", serde_json::to_string_pretty(&result.items)?);
        }
        Some("import") | Some("enable") => {
            let Some(item_id) = args.get(1) else {
                anyhow::bail!("usage: roder workflow import ITEM_ID [--approve-side-effects]");
            };
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("workflow/enable")),
                    method: "workflow/enable".to_string(),
                    params: Some(serde_json::to_value(WorkflowEnableParams {
                        workspace: None,
                        item_id: item_id.clone(),
                        approve_side_effects: args
                            .iter()
                            .any(|arg| arg == "--approve-side-effects"),
                    })?),
                })
                .await;
            let result = decode_response::<WorkflowEnableResult>(res)?;
            println!(
                "enabled\t{}\t{}\t{}",
                result.item.id, result.item.title, result.decision.source_hash
            );
        }
        _ => anyhow::bail!("usage: roder workflow <scan|preview [ITEM_ID]|import ITEM_ID>"),
    }
    Ok(())
}

fn workflow_workspace_arg(args: &[String], start: usize) -> Option<String> {
    args.get(start).and_then(|arg| {
        if arg == "--workspace" {
            args.get(start + 1).cloned()
        } else {
            None
        }
    })
}

fn memory_scope_arg(args: &[String]) -> Option<roder_api::memory::MemoryScope> {
    args.iter()
        .position(|arg| arg == "--scope")
        .and_then(|idx| args.get(idx + 1))
        .map(|scope| match scope.as_str() {
            "global" => roder_api::memory::MemoryScope::Global,
            "project" => default_project_scope(),
            value if value.starts_with("project:") => roder_api::memory::MemoryScope::Project(
                value.trim_start_matches("project:").to_string(),
            ),
            value => roder_api::memory::MemoryScope::Project(value.to_string()),
        })
}

fn default_project_scope() -> roder_api::memory::MemoryScope {
    let project = std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "default".to_string());
    roder_api::memory::MemoryScope::Project(project)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn print_workflow_items(items: &[roder_api::workflow::WorkflowImportItem]) {
    for item in items {
        let approval = if item.approval_required {
            "approval"
        } else {
            "passive"
        };
        println!(
            "{}\t{:?}\t{}\t{}\t{}",
            item.id, item.source.source_type, approval, item.title, item.source.path
        );
    }
}

async fn run_team_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("attach") => {
            let mut team_id = None;
            let mut member_id = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--team" => {
                        team_id = args.get(i + 1).cloned();
                        i += 1;
                    }
                    "--member" => {
                        member_id = args.get(i + 1).cloned();
                        i += 1;
                    }
                    _ => {}
                }
                i += 1;
            }
            let Some(team_id) = team_id else {
                anyhow::bail!("roder team attach requires --team <team-id>");
            };
            let Some(member_id) = member_id else {
                anyhow::bail!("roder team attach requires --member <member-id>");
            };
            let (runtime, default_model) = build_runtime_from_config(CliOptions::default()).await?;
            let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
            let client = LocalAppClient::new(app_server);
            let mut tui = TuiApp::new_with_startup(
                client,
                default_model,
                TuiStartup::TeamAttach { team_id, member_id },
            )
            .await?;
            tui.run().await?;
            print_tui_exit_summary(&tui);
            Ok(())
        }
        _ => anyhow::bail!("usage: roder team attach --team <team-id> --member <member-id>"),
    }
}

pub(crate) fn decode_response<T: serde::de::DeserializeOwned>(
    res: JsonRpcResponse,
) -> anyhow::Result<T> {
    if let Some(error) = res.error {
        if let Some(data) = error.data {
            anyhow::bail!("{} ({})\n{}", error.message, error.code, data);
        }
        anyhow::bail!("{} ({})", error.message, error.code);
    }
    let Some(result) = res.result else {
        anyhow::bail!("missing result");
    };
    Ok(serde_json::from_value(result)?)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CliOptions {
    policy_mode: Option<PolicyMode>,
    team_display: Option<roder_api::teams::AgentTeamDisplayMode>,
    startup: TuiStartup,
}

#[derive(Debug, Clone)]
struct AppServerOptions {
    listen: String,
    remote: bool,
    auth_token: Option<String>,
    remote_token_ttl: Option<time::Duration>,
    allowed_origins: Vec<String>,
    print_qr: bool,
    cli_options: CliOptions,
}

pub(crate) async fn build_runtime_from_config(
    options: CliOptions,
) -> anyhow::Result<(Arc<Runtime>, String)> {
    let cfg = roder_config::load_config()?;
    let keys = provider_keys(&cfg);
    let web_search = resolve_web_search_config(cfg.web_search.as_ref())?;
    let policy_mode = resolve_policy_mode(&options, &cfg)?;
    let custom_inference_provider_configs = custom_inference_providers(&cfg);
    let (default_provider, configured_model) = resolve_provider_model(cfg.provider, cfg.model);
    let default_model = configured_model.clone().unwrap_or_else(|| {
        if default_provider == PROVIDER_MOCK {
            "mock".to_string()
        } else {
            DEFAULT_MODEL_ID.to_string()
        }
    });
    let subagents = resolve_subagents_config(
        cfg.subagents.as_ref(),
        default_provider.clone(),
        default_model.clone(),
    )
    .await?;
    let model_edit_tools = resolve_model_edit_tools(&cfg.models)?;
    let model_parallel_tool_calls = resolve_model_parallel_tool_calls(&cfg.models);
    let notifications = resolve_notifications_config(cfg.notifications.as_ref())?;
    let remote_runner_destination = resolve_remote_runner_destination(cfg.remote_runners.as_ref())?;
    let tool_path_scope = resolve_tool_path_scope(cfg.tools.as_ref())?;
    if policy_mode == PolicyMode::Bypass
        && cfg
            .policy_modes
            .as_ref()
            .and_then(|policy| policy.warn_on_bypass)
            .unwrap_or(true)
    {
        eprintln!("warning: bypass policy mode is active; tool approvals are auto-approved");
    }

    let workspace = std::env::current_dir().ok();
    let registry = build_default_registry(DefaultRegistryConfig {
        openai_api_key: keys.openai,
        anthropic_api_key: keys.anthropic,
        gemini_api_key: keys.gemini,
        xai_api_key: keys.xai,
        xai_base_url: keys.xai_base_url,
        opencode_api_key: keys.opencode,
        opencode_base_url: keys.opencode_base_url,
        opencode_project_id: keys.opencode_project_id,
        opencode_go_api_key: keys.opencode_go,
        opencode_go_base_url: keys.opencode_go_base_url,
        opencode_go_project_id: keys.opencode_go_project_id,
        poolside_api_key: keys.poolside,
        poolside_base_url: keys.poolside_base_url,
        custom_inference_providers: custom_inference_provider_configs,
        session_dir: None,
        workspace: workspace.clone(),
        tool_path_scope,
        web_search: web_search.external,
        subagents,
        policy_mode,
        notifications,
        remote_runner_destination: remote_runner_destination.clone(),
    })?;

    let runtime = Arc::new(Runtime::new(
        registry,
        RuntimeConfig {
            default_provider,
            default_model: default_model.clone(),
            reasoning: cfg.reasoning,
            auto_compact_token_limit: cfg.auto_compact_token_limit,
            hosted_web_search: web_search.hosted,
            model_edit_tools,
            model_parallel_tool_calls,
            workspace: workspace.map(|p| p.display().to_string()),
            policy_mode,
            remote_runner_destination,
            team_data_dir: None,
        },
    )?);

    Ok((runtime, default_model))
}

fn resolve_tool_path_scope(
    config: Option<&roder_config::ToolsConfig>,
) -> anyhow::Result<roder_tools::ToolPathScope> {
    let Some(config) = config else {
        return Ok(roder_tools::ToolPathScope::default());
    };
    roder_tools::ToolPathScope::parse(&config.path_scope).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid tools.path_scope {:?}; expected global or workspace",
            config.path_scope
        )
    })
}

fn resolve_notifications_config(
    config: Option<&roder_config::NotificationsConfig>,
) -> anyhow::Result<DefaultNotificationsConfig> {
    let Some(config) = config else {
        return Ok(DefaultNotificationsConfig::default());
    };
    let enabled_kinds = if config.kinds.is_empty() {
        DefaultNotificationsConfig::default().enabled_kinds
    } else {
        config
            .kinds
            .iter()
            .map(|kind| parse_notification_kind(kind))
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    Ok(DefaultNotificationsConfig {
        enabled: config.enabled,
        terminal: config.terminal.enabled,
        desktop: config.desktop.enabled,
        enabled_kinds,
    })
}

fn resolve_remote_runner_destination(
    config: Option<&roder_config::RemoteRunnersConfig>,
) -> anyhow::Result<Option<RunnerDestination>> {
    let Some(config) = config else {
        return Ok(None);
    };
    if !config.enabled {
        return Ok(None);
    }
    let destination_id = config
        .default_destination
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("remote_runners.enabled requires default_destination"))?;

    if destination_id == "unix-local" && !config.destinations.contains_key(destination_id) {
        return Ok(Some(RunnerDestination {
            id: "unix-local".to_string(),
            provider_id: "unix-local".to_string(),
            config: serde_json::Value::Null,
            default_manifest: RunnerManifest::default(),
        }));
    }

    let destination = config.destinations.get(destination_id).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown remote runner destination `{destination_id}`; define [remote_runners.destinations.{destination_id}]"
        )
    })?;
    validate_remote_runner_destination(destination_id, destination)?;
    Ok(Some(RunnerDestination {
        id: destination_id.to_string(),
        provider_id: destination.provider.clone(),
        config: destination.config.clone(),
        default_manifest: RunnerManifest::default(),
    }))
}

fn validate_remote_runner_destination(
    destination_id: &str,
    destination: &roder_config::RemoteRunnerDestinationConfig,
) -> anyhow::Result<()> {
    if destination.provider.trim().is_empty() {
        anyhow::bail!("remote runner destination `{destination_id}` requires provider");
    }
    for (name, env) in &destination.secret_env {
        if name.trim().is_empty() || env.trim().is_empty() {
            anyhow::bail!(
                "remote runner destination `{destination_id}` has an empty secret env reference"
            );
        }
    }
    reject_secret_like_runner_config(destination_id, &destination.config)
}

fn reject_secret_like_runner_config(
    destination_id: &str,
    value: &serde_json::Value,
) -> anyhow::Result<()> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if secret_like_runner_key(key) {
                    anyhow::bail!(
                        "remote runner destination `{destination_id}` config key `{key}` looks secret-like; use secret_env instead"
                    );
                }
                reject_secret_like_runner_config(destination_id, value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_secret_like_runner_config(destination_id, value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn secret_like_runner_key(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("secret") || lower.contains("token") || lower.contains("api_key")
}

fn parse_notification_kind(kind: &str) -> anyhow::Result<NotificationKind> {
    match kind.trim().replace('-', "_").to_ascii_lowercase().as_str() {
        "needs_input" => Ok(NotificationKind::NeedsInput),
        "turn_idle" => Ok(NotificationKind::TurnIdle),
        "task_completed" => Ok(NotificationKind::TaskCompleted),
        "task_failed" => Ok(NotificationKind::TaskFailed),
        other if !other.is_empty() => Ok(NotificationKind::Custom(other.to_string())),
        _ => anyhow::bail!("notification kind cannot be empty"),
    }
}

fn resolve_model_edit_tools(
    models: &std::collections::HashMap<String, roder_config::ModelConfig>,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut edit_tools = std::collections::HashMap::new();
    for (model, cfg) in models {
        let Some(edit_tool) = cfg.edit_tool.as_deref().map(str::trim) else {
            continue;
        };
        if edit_tool.is_empty() {
            continue;
        }
        validate_edit_tool(edit_tool)?;
        edit_tools.insert(model.clone(), edit_tool.to_string());
    }
    Ok(edit_tools)
}

fn resolve_model_parallel_tool_calls(
    models: &std::collections::HashMap<String, roder_config::ModelConfig>,
) -> std::collections::HashMap<String, bool> {
    models
        .iter()
        .filter_map(|(model, cfg)| {
            cfg.parallel_tool_calls
                .map(|parallel| (model.clone(), parallel))
        })
        .collect()
}

fn parse_cli_options(args: &[String]) -> anyhow::Result<CliOptions> {
    let mut options = CliOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "resume" => {
                let thread_id = args.get(i + 1).filter(|value| !value.starts_with("--"));
                options.startup = match thread_id {
                    Some(thread_id) => {
                        i += 1;
                        TuiStartup::ResumeSession(thread_id.clone())
                    }
                    None => TuiStartup::ResumeMenu,
                };
            }
            "--yolo" => options.policy_mode = Some(PolicyMode::Bypass),
            "--mode" => {
                let Some(mode) = args.get(i + 1) else {
                    anyhow::bail!("--mode requires a value");
                };
                options.policy_mode = Some(parse_policy_mode(mode)?);
                i += 1;
            }
            arg if arg.starts_with("--mode=") => {
                options.policy_mode = Some(parse_policy_mode(&arg["--mode=".len()..])?);
            }
            "--team-display" => {
                let Some(mode) = args.get(i + 1) else {
                    anyhow::bail!("--team-display requires a value");
                };
                options.team_display = Some(parse_team_display_mode(mode)?);
                i += 1;
            }
            arg if arg.starts_with("--team-display=") => {
                options.team_display =
                    Some(parse_team_display_mode(&arg["--team-display=".len()..])?);
            }
            _ => {}
        }
        i += 1;
    }
    Ok(options)
}

fn print_tui_exit_summary(tui: &TuiApp) {
    let summary = tui.exit_summary();
    println!("Session: {}", summary.title);
    println!(
        "Saved as {} ({}, {} message{})",
        summary.thread_id,
        summary.model,
        summary.message_count,
        if summary.message_count == 1 { "" } else { "s" }
    );
    println!("Resume: {}", summary.resume_command);
}

fn parse_app_server_options(args: &[String]) -> anyhow::Result<AppServerOptions> {
    let mut listen = "stdio://".to_string();
    let mut remote = false;
    let mut auth_token = None;
    let mut remote_token_ttl = None;
    let mut allowed_origins = Vec::new();
    let mut print_qr = true;
    let mut passthrough = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--remote" => {
                remote = true;
                if listen == "stdio://" {
                    listen = "ws://0.0.0.0:0".to_string();
                }
            }
            "--listen" => {
                let Some(value) = args.get(i + 1) else {
                    anyhow::bail!("--listen requires a value");
                };
                listen = value.clone();
                i += 1;
            }
            arg if arg.starts_with("--listen=") => {
                listen = arg["--listen=".len()..].to_string();
            }
            "--auth-token" => {
                let Some(value) = args.get(i + 1) else {
                    anyhow::bail!("--auth-token requires a value");
                };
                auth_token = Some(resolve_auth_token_arg(value)?);
                i += 1;
            }
            arg if arg.starts_with("--auth-token=") => {
                auth_token = Some(resolve_auth_token_arg(&arg["--auth-token=".len()..])?);
            }
            "--remote-token-ttl" => {
                let Some(value) = args.get(i + 1) else {
                    anyhow::bail!("--remote-token-ttl requires seconds");
                };
                remote_token_ttl = Some(parse_remote_token_ttl(value)?);
                i += 1;
            }
            arg if arg.starts_with("--remote-token-ttl=") => {
                remote_token_ttl =
                    Some(parse_remote_token_ttl(&arg["--remote-token-ttl=".len()..])?);
            }
            "--allowed-origin" => {
                let Some(value) = args.get(i + 1) else {
                    anyhow::bail!("--allowed-origin requires a value");
                };
                allowed_origins.push(value.clone());
                i += 1;
            }
            arg if arg.starts_with("--allowed-origin=") => {
                allowed_origins.push(arg["--allowed-origin=".len()..].to_string());
            }
            "--print-qr=false" => {
                print_qr = false;
            }
            "--print-qr=true" => {
                print_qr = true;
            }
            other => passthrough.push(other.to_string()),
        }
        i += 1;
    }

    Ok(AppServerOptions {
        listen,
        remote,
        auth_token,
        remote_token_ttl,
        allowed_origins,
        print_qr,
        cli_options: parse_cli_options(&passthrough)?,
    })
}

async fn run_app_server(args: &[String]) -> anyhow::Result<()> {
    let options = parse_app_server_options(args)?;
    if options.listen != "stdio://" && !options.remote {
        anyhow::bail!(
            "unsupported app-server listen address {:?}; only stdio:// is currently supported",
            options.listen
        );
    }

    let (runtime, _) = build_runtime_from_config(options.cli_options).await?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    if options.remote {
        let token = match options.auth_token {
            Some(token) => roder_app_server::remote::RemoteToken::new(token)?,
            None => roder_app_server::remote::generate_remote_token_from_os()?,
        };
        let handle = roder_app_server::remote::listen_remote_websocket(
            app_server,
            roder_app_server::remote::RemoteServerOptions {
                listen: options.listen,
                token,
                token_ttl: options.remote_token_ttl,
                allowed_origins: options.allowed_origins,
                print_qr: options.print_qr,
                workspace: std::env::current_dir()
                    .ok()
                    .map(|path| path.display().to_string()),
            },
        )
        .await?;
        if options.print_qr {
            eprintln!(
                "{}",
                roder_app_server::remote::render_terminal_pairing(&handle)
            );
        } else {
            eprintln!(
                "Remote app-server listening at {}; token {}",
                handle.listen_addr, handle.token_preview
            );
        }
        std::future::pending::<()>().await;
        return Ok(());
    }
    run_stdio_app_server(app_server).await
}

fn resolve_auth_token_arg(value: &str) -> anyhow::Result<String> {
    if let Some(env_name) = value.strip_prefix("env:") {
        let token = std::env::var(env_name)
            .map_err(|_| anyhow::anyhow!("--auth-token env:{env_name} is not set"))?;
        if token.trim().is_empty() {
            anyhow::bail!("--auth-token env:{env_name} is empty");
        }
        Ok(token)
    } else {
        if value.trim().is_empty() {
            anyhow::bail!("--auth-token cannot be empty");
        }
        Ok(value.to_string())
    }
}

fn parse_remote_token_ttl(value: &str) -> anyhow::Result<time::Duration> {
    let seconds = value
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("--remote-token-ttl requires a positive second count"))?;
    if seconds <= 0 {
        anyhow::bail!("--remote-token-ttl requires a positive second count");
    }
    Ok(time::Duration::seconds(seconds))
}

async fn run_stdio_app_server(app_server: Arc<AppServer>) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = rx.recv().await {
            stdout
                .write_all(serde_json::to_string(&message)?.as_bytes())
                .await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        anyhow::Ok(())
    });

    let mut notifications = app_server.subscribe_notifications();
    let notification_tx = tx.clone();
    let notification_writer = tokio::spawn(async move {
        while let Ok(notification) = notifications.recv().await {
            if notification_tx
                .send(serde_json::to_value(notification)?)
                .is_err()
            {
                break;
            }
        }
        anyhow::Ok(())
    });

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(request) => app_server.handle_request(request).await,
            Err(err) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: None,
                result: None,
                error: Some(JsonRpcError {
                    code: -32700,
                    message: format!("Parse error: {err}"),
                    data: None,
                }),
            },
        };
        if tx.send(serde_json::to_value(response)?).is_err() {
            break;
        }
    }
    drop(tx);
    notification_writer.abort();
    writer.await??;
    Ok(())
}

fn parse_policy_mode(mode: &str) -> anyhow::Result<PolicyMode> {
    match mode.trim() {
        "default" => Ok(PolicyMode::Default),
        "accept_all" | "accept-all" | "accept_edits" | "accept-edits" => Ok(PolicyMode::AcceptAll),
        "plan" => Ok(PolicyMode::Plan),
        "bypass" | "yolo" => Ok(PolicyMode::Bypass),
        other => anyhow::bail!(
            "unsupported policy mode {other:?}; expected default, accept_all, plan, or bypass"
        ),
    }
}

fn parse_team_display_mode(mode: &str) -> anyhow::Result<roder_api::teams::AgentTeamDisplayMode> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(roder_api::teams::AgentTeamDisplayMode::Auto),
        "in-process" | "in_process" | "inprocess" => {
            Ok(roder_api::teams::AgentTeamDisplayMode::InProcess)
        }
        "tmux" => Ok(roder_api::teams::AgentTeamDisplayMode::Tmux),
        "iterm2" | "iterm" => Ok(roder_api::teams::AgentTeamDisplayMode::Iterm2),
        other => anyhow::bail!(
            "unsupported team display mode {other:?}; expected auto, in-process, tmux, or iterm2"
        ),
    }
}

fn resolve_policy_mode(
    options: &CliOptions,
    cfg: &roder_config::Config,
) -> anyhow::Result<PolicyMode> {
    if let Some(mode) = options.policy_mode {
        return Ok(mode);
    }
    cfg.policy_modes
        .as_ref()
        .and_then(|policy| policy.default.as_deref())
        .map(parse_policy_mode)
        .transpose()
        .map(|mode| mode.unwrap_or_default())
}

async fn resolve_subagents_config(
    cfg: Option<&roder_config::SubagentsConfig>,
    default_provider: String,
    default_model: String,
) -> anyhow::Result<Option<DefaultSubagentsConfig>> {
    let Some(cfg) = cfg else {
        return Ok(None);
    };
    if !cfg.enabled {
        return Ok(Some(DefaultSubagentsConfig {
            enabled: false,
            ..DefaultSubagentsConfig::default()
        }));
    }

    let load_config = AgentLoadConfig {
        user_dir: resolve_user_agent_dir(cfg),
        workspace_dir: resolve_workspace_agent_dir(cfg)?,
    };
    let definitions = load_agent_definitions(&load_config).await?;
    Ok(Some(DefaultSubagentsConfig {
        enabled: true,
        definitions,
        default_agent: trim_nonempty(cfg.default_agent.clone())
            .unwrap_or_else(|| DefaultSubagentsConfig::default().default_agent),
        default_provider: Some(default_provider),
        default_model,
        max_concurrent: cfg
            .max_concurrent
            .unwrap_or_else(|| DefaultSubagentsConfig::default().max_concurrent),
        max_depth: cfg
            .max_depth
            .unwrap_or_else(|| DefaultSubagentsConfig::default().max_depth),
        default_timeout_seconds: cfg
            .default_timeout_seconds
            .unwrap_or_else(|| DefaultSubagentsConfig::default().default_timeout_seconds),
        include_child_transcript: cfg.include_child_transcript,
        expose_per_type: cfg.expose_per_type,
    }))
}

fn resolve_user_agent_dir(cfg: &roder_config::SubagentsConfig) -> Option<PathBuf> {
    cfg.disk
        .user_dir
        .as_deref()
        .map(expand_tilde)
        .or_else(roder_ext_subagents::default_user_agent_dir)
}

fn resolve_workspace_agent_dir(
    cfg: &roder_config::SubagentsConfig,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = cfg.disk.workspace_dir.as_deref() {
        return Ok(Some(expand_tilde(path)));
    }
    Ok(None)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        home_dir().unwrap_or_else(|| path.to_path_buf())
    } else if let Some(rest) = text.strip_prefix("~/") {
        home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

struct ProviderKeys {
    openai: Option<String>,
    anthropic: Option<String>,
    gemini: Option<String>,
    xai: Option<String>,
    xai_base_url: Option<String>,
    opencode: Option<String>,
    opencode_base_url: Option<String>,
    opencode_project_id: Option<String>,
    opencode_go: Option<String>,
    opencode_go_base_url: Option<String>,
    opencode_go_project_id: Option<String>,
    poolside: Option<String>,
    poolside_base_url: Option<String>,
}

fn provider_keys(cfg: &roder_config::Config) -> ProviderKeys {
    ProviderKeys {
        openai: std::env::var("OPENAI_API_KEY")
            .ok()
            .or_else(|| cfg.providers.get("openai").and_then(|p| p.api_key.clone()))
            .or_else(|| {
                cfg.providers
                    .get("openai_responses")
                    .and_then(|p| p.api_key.clone())
            }),
        anthropic: std::env::var("ANTHROPIC_API_KEY").ok().or_else(|| {
            cfg.providers
                .get("anthropic")
                .and_then(|p| p.api_key.clone())
        }),
        gemini: std::env::var("GEMINI_API_TOKEN")
            .ok()
            .or_else(|| std::env::var("GEMINI_API_KEY").ok())
            .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
            .or_else(|| std::env::var("GOOGLE_GENAI_API_KEY").ok())
            .or_else(|| std::env::var("GOOGLE_AI_API_KEY").ok())
            .or_else(|| cfg.providers.get("gemini").and_then(|p| p.api_key.clone())),
        xai: std::env::var("XAI_API_KEY")
            .ok()
            .or_else(|| std::env::var("RODER_XAI_API_KEY").ok())
            .or_else(|| cfg.providers.get("xai").and_then(|p| p.api_key.clone())),
        xai_base_url: std::env::var("RODER_XAI_BASE_URL")
            .ok()
            .or_else(|| std::env::var("XAI_BASE_URL").ok()),
        opencode: std::env::var("OPENCODE_API_KEY")
            .ok()
            .or_else(|| std::env::var("OPENCODE_ZEN_API_KEY").ok())
            .or_else(|| std::env::var("RODER_OPENCODE_API_KEY").ok())
            .or_else(|| {
                cfg.providers
                    .get("opencode")
                    .and_then(|p| p.api_key.clone())
            })
            .or_else(|| {
                cfg.providers
                    .get("opencode")
                    .and_then(|p| p.api_key_env.as_deref())
                    .and_then(env_nonempty)
            }),
        opencode_base_url: cfg
            .providers
            .get("opencode")
            .and_then(|p| p.base_url.clone())
            .or_else(|| std::env::var("RODER_OPENCODE_BASE_URL").ok())
            .or_else(|| std::env::var("OPENCODE_BASE_URL").ok())
            .or_else(|| std::env::var("OPENCODE_ZEN_BASE_URL").ok()),
        opencode_project_id: cfg
            .providers
            .get("opencode")
            .and_then(|p| p.project_id.clone())
            .or_else(|| {
                cfg.providers
                    .get("opencode")
                    .and_then(|p| p.project_id_env.as_deref())
                    .and_then(env_nonempty)
            }),
        opencode_go: std::env::var("OPENCODE_GO_API_KEY")
            .ok()
            .or_else(|| std::env::var("RODER_OPENCODE_GO_API_KEY").ok())
            .or_else(|| std::env::var("OPENCODE_API_KEY").ok())
            .or_else(|| {
                cfg.providers
                    .get("opencode-go")
                    .and_then(|p| p.api_key.clone())
            })
            .or_else(|| {
                cfg.providers
                    .get("opencode-go")
                    .and_then(|p| p.api_key_env.as_deref())
                    .and_then(env_nonempty)
            }),
        opencode_go_base_url: cfg
            .providers
            .get("opencode-go")
            .and_then(|p| p.base_url.clone())
            .or_else(|| std::env::var("RODER_OPENCODE_GO_BASE_URL").ok())
            .or_else(|| std::env::var("OPENCODE_GO_BASE_URL").ok()),
        opencode_go_project_id: cfg
            .providers
            .get("opencode-go")
            .and_then(|p| p.project_id.clone())
            .or_else(|| {
                cfg.providers
                    .get("opencode-go")
                    .and_then(|p| p.project_id_env.as_deref())
                    .and_then(env_nonempty)
            }),
        poolside: std::env::var("POOLSIDE_API_KEY")
            .ok()
            .or_else(|| std::env::var("RODER_POOLSIDE_API_KEY").ok())
            .or_else(|| {
                cfg.providers
                    .get("poolside")
                    .and_then(|p| p.api_key.clone())
            })
            .or_else(|| {
                cfg.providers
                    .get("poolside")
                    .and_then(|p| p.api_key_env.as_deref())
                    .and_then(env_nonempty)
            }),
        poolside_base_url: cfg
            .providers
            .get("poolside")
            .and_then(|p| p.base_url.clone())
            .or_else(|| std::env::var("RODER_POOLSIDE_BASE_URL").ok())
            .or_else(|| std::env::var("POOLSIDE_BASE_URL").ok()),
    }
}

fn custom_inference_providers(cfg: &roder_config::Config) -> Vec<CustomInferenceProviderConfig> {
    cfg.providers
        .iter()
        .filter_map(|(id, provider)| {
            let id = normalize_provider_id(id);
            if is_builtin_provider_id(&id) {
                return None;
            }
            let base_url = trim_nonempty(provider.base_url.clone())?;
            let api_key = trim_nonempty(provider.api_key.clone())
                .or_else(|| provider.api_key_env.as_deref().and_then(env_nonempty));
            Some(CustomInferenceProviderConfig {
                id: id.clone(),
                name: Some(id),
                api_key,
                base_url,
            })
        })
        .collect()
}

fn is_builtin_provider_id(id: &str) -> bool {
    matches!(
        id,
        "mock"
            | "openai"
            | "codex"
            | "anthropic"
            | "gemini"
            | "xai"
            | "supergrok"
            | "opencode"
            | "opencode-go"
            | "poolside"
    )
}

#[derive(Debug, Clone)]
struct ResolvedWebSearchConfig {
    external: Option<DefaultWebSearchConfig>,
    hosted: HostedWebSearchConfig,
}

fn resolve_web_search_config(
    cfg: Option<&roder_config::WebSearchConfig>,
) -> anyhow::Result<ResolvedWebSearchConfig> {
    let Some(cfg) = cfg else {
        return Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::cached(),
        });
    };

    match web_search_mode(cfg)? {
        ResolvedWebSearchMode::HostedCached => Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::cached(),
        }),
        ResolvedWebSearchMode::HostedLive => Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::live(),
        }),
        ResolvedWebSearchMode::External => Ok(ResolvedWebSearchConfig {
            external: Some(resolve_external_web_search_config(cfg, true)?),
            hosted: HostedWebSearchConfig::disabled(),
        }),
        ResolvedWebSearchMode::Disabled => Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::disabled(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedWebSearchMode {
    HostedCached,
    HostedLive,
    External,
    Disabled,
}

fn web_search_mode(cfg: &roder_config::WebSearchConfig) -> anyhow::Result<ResolvedWebSearchMode> {
    match cfg.mode.as_deref().map(normalize_mode) {
        Some(mode) if matches!(mode.as_str(), "codex" | "hosted" | "native" | "cached") => {
            Ok(ResolvedWebSearchMode::HostedCached)
        }
        Some(mode) if mode == "live" => Ok(ResolvedWebSearchMode::HostedLive),
        Some(mode) if matches!(mode.as_str(), "external" | "router" | "local") => {
            Ok(ResolvedWebSearchMode::External)
        }
        Some(mode) if matches!(mode.as_str(), "disabled" | "off" | "none" | "false") => {
            Ok(ResolvedWebSearchMode::Disabled)
        }
        Some(mode) => anyhow::bail!(
            "unsupported web_search.mode {mode:?}; expected codex, hosted, cached, live, external, or disabled"
        ),
        None if cfg
            .provider
            .as_deref()
            .is_some_and(is_hosted_web_search_provider) =>
        {
            Ok(ResolvedWebSearchMode::HostedCached)
        }
        None if cfg.enabled || cfg.provider.is_some() => Ok(ResolvedWebSearchMode::External),
        None => Ok(ResolvedWebSearchMode::Disabled),
    }
}

fn is_hosted_web_search_provider(provider: &str) -> bool {
    matches!(
        normalize_mode(provider).as_str(),
        "codex" | "openai" | "hosted" | "native"
    )
}

fn normalize_mode(mode: &str) -> String {
    mode.trim().to_ascii_lowercase().replace(['-', '_'], "")
}

fn resolve_external_web_search_config(
    cfg: &roder_config::WebSearchConfig,
    force_enabled: bool,
) -> anyhow::Result<DefaultWebSearchConfig> {
    let provider = match cfg.provider.as_deref() {
        Some(provider) => Some(parse_web_search_provider(provider)?),
        None => None,
    };
    Ok(DefaultWebSearchConfig {
        enabled: force_enabled || cfg.enabled,
        provider,
        firecrawl: resolve_web_search_provider_config(
            &cfg.firecrawl,
            "FIRECRAWL_API_KEY",
            "FIRECRAWL_BASE_URL",
            None,
        ),
        perplexity: resolve_web_search_provider_config(
            &cfg.perplexity,
            "PERPLEXITY_API_KEY",
            "PERPLEXITY_BASE_URL",
            None,
        ),
        tavily: resolve_web_search_provider_config(
            &cfg.tavily,
            "TAVILY_API_KEY",
            "TAVILY_BASE_URL",
            Some("TAVILY_PROJECT"),
        ),
        parallel: resolve_web_search_provider_config(
            &cfg.parallel,
            "PARALLEL_API_KEY",
            "PARALLEL_BASE_URL",
            None,
        ),
        timeout_seconds: cfg.timeout_seconds,
        max_results: cfg.max_results,
        namespaced_tools: cfg.namespaced_tools,
    })
}

fn parse_web_search_provider(provider: &str) -> anyhow::Result<WebSearchProviderKind> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "firecrawl" => Ok(WebSearchProviderKind::Firecrawl),
        "perplexity" => Ok(WebSearchProviderKind::Perplexity),
        "tavily" => Ok(WebSearchProviderKind::Tavily),
        "parallel" | "parallel.ai" | "parallel_ai" => Ok(WebSearchProviderKind::Parallel),
        _ => anyhow::bail!(
            "unsupported web_search provider {provider:?}; expected firecrawl, perplexity, tavily, or parallel"
        ),
    }
}

fn resolve_web_search_provider_config(
    cfg: &roder_config::WebSearchProviderConfig,
    default_api_key_env: &str,
    default_base_url_env: &str,
    default_project_env: Option<&str>,
) -> DefaultWebSearchProviderConfig {
    let api_key_env = cfg.api_key_env.as_deref().unwrap_or(default_api_key_env);
    let base_url_env = default_base_url_env;
    let project_env = cfg.project_env.as_deref().or(default_project_env);
    DefaultWebSearchProviderConfig {
        enabled: cfg.enabled,
        api_key: trim_nonempty(cfg.api_key.clone()).or_else(|| env_nonempty(api_key_env)),
        base_url: trim_nonempty(cfg.base_url.clone()).or_else(|| env_nonempty(base_url_env)),
        project_id: trim_nonempty(cfg.project.clone())
            .or_else(|| project_env.and_then(env_nonempty)),
        search_depth: trim_nonempty(cfg.search_depth.clone()),
        mode: trim_nonempty(cfg.mode.clone()),
        debug_raw_response: cfg.debug_raw_response,
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| trim_nonempty(Some(value)))
}

fn trim_nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn run_auth(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("login") => {
            let provider = args.get(1).map(String::as_str).unwrap_or("codex");
            match auth_provider_kind(provider)? {
                AuthProviderKind::Codex => {
                    eprintln!("Opening browser for Codex sign-in...");
                    let tokens = roder_codex_auth::login().await?;
                    if tokens.account_id.is_empty() {
                        eprintln!("Signed in with Codex");
                    } else {
                        eprintln!("Signed in with Codex account {}", tokens.account_id);
                    }
                }
                AuthProviderKind::SuperGrok => {
                    eprintln!("Opening browser for SuperGrok sign-in...");
                    let tokens = roder_supergrok_auth::login().await?;
                    if tokens.email.is_empty() {
                        eprintln!("Signed in with SuperGrok");
                    } else {
                        eprintln!("Signed in with SuperGrok account {}", tokens.email);
                    }
                }
            }
            Ok(())
        }
        Some("status") => {
            let provider = args.get(1).map(String::as_str).unwrap_or("codex");
            match auth_provider_kind(provider)? {
                AuthProviderKind::Codex => match roder_codex_auth::status().await? {
                    Some(tokens) if !tokens.account_id.is_empty() => {
                        println!("codex: signed in ({})", tokens.account_id);
                    }
                    Some(_) => println!("codex: signed in"),
                    None => println!("codex: signed out"),
                },
                AuthProviderKind::SuperGrok => match roder_supergrok_auth::status().await? {
                    Some(tokens) if !tokens.email.is_empty() => {
                        println!("supergrok: signed in ({})", tokens.email);
                    }
                    Some(_) => println!("supergrok: signed in"),
                    None => println!("supergrok: signed out"),
                },
            }
            Ok(())
        }
        Some("logout") => {
            let provider = args.get(1).map(String::as_str).unwrap_or("codex");
            match auth_provider_kind(provider)? {
                AuthProviderKind::Codex => {
                    roder_codex_auth::logout()?;
                    println!("codex: signed out");
                }
                AuthProviderKind::SuperGrok => {
                    roder_supergrok_auth::logout()?;
                    println!("supergrok: signed out");
                }
            }
            Ok(())
        }
        _ => anyhow::bail!("usage: roder auth login|status|logout [codex|supergrok]"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthProviderKind {
    Codex,
    SuperGrok,
}

fn auth_provider_kind(provider: &str) -> anyhow::Result<AuthProviderKind> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "codex" => Ok(AuthProviderKind::Codex),
        "supergrok" | "grok-oauth" | "xai-oauth" | "x-ai-oauth" | "xai-grok-oauth" => {
            Ok(AuthProviderKind::SuperGrok)
        }
        provider => anyhow::bail!("unsupported auth provider {provider:?}"),
    }
}

fn resolve_provider_model(
    provider: Option<String>,
    model: Option<String>,
) -> (String, Option<String>) {
    let Some(provider) = provider else {
        return (PROVIDER_MOCK.to_string(), model);
    };
    if let Some((provider_id, model_id)) = provider.split_once('/') {
        let provider_id = provider_id.trim();
        let model_id = model_id.trim();
        if !provider_id.is_empty() && !model_id.is_empty() {
            return (
                normalize_provider_id(provider_id),
                model.or_else(|| Some(model_id.to_string())),
            );
        }
    }
    (normalize_provider_id(&provider), model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_slash_model_sets_default_model() {
        let (provider, model) = resolve_provider_model(Some("codex/gpt-5.5".to_string()), None);
        assert_eq!(provider, "codex");
        assert_eq!(model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn xai_provider_aliases_normalize_with_provider_slash_model() {
        let (provider, model) = resolve_provider_model(Some("x.ai/grok-4.3".to_string()), None);
        assert_eq!(provider, "xai");
        assert_eq!(model.as_deref(), Some("grok-4.3"));

        let (provider, model) =
            resolve_provider_model(Some("xai-oauth/grok-4.20-0309-reasoning".to_string()), None);
        assert_eq!(provider, "supergrok");
        assert_eq!(model.as_deref(), Some("grok-4.20-0309-reasoning"));
    }

    #[test]
    fn auth_provider_aliases_route_to_distinct_backends() {
        assert_eq!(
            auth_provider_kind("codex").unwrap(),
            AuthProviderKind::Codex
        );
        assert_eq!(
            auth_provider_kind("supergrok").unwrap(),
            AuthProviderKind::SuperGrok
        );
        assert_eq!(
            auth_provider_kind("xai-oauth").unwrap(),
            AuthProviderKind::SuperGrok
        );
        assert!(auth_provider_kind("xai").is_err());
    }

    #[test]
    fn explicit_model_wins_over_provider_slash_model() {
        let (provider, model) = resolve_provider_model(
            Some("codex/gpt-5.4-mini".to_string()),
            Some("gpt-5.5".to_string()),
        );
        assert_eq!(provider, "codex");
        assert_eq!(model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn model_parallel_tool_call_config_keeps_only_explicit_overrides() {
        let models = std::collections::HashMap::from([
            (
                "custom-serial".to_string(),
                roder_config::ModelConfig {
                    edit_tool: None,
                    parallel_tool_calls: Some(false),
                },
            ),
            (
                "custom-default".to_string(),
                roder_config::ModelConfig {
                    edit_tool: Some("patch".to_string()),
                    parallel_tool_calls: None,
                },
            ),
        ]);

        let resolved = resolve_model_parallel_tool_calls(&models);

        assert_eq!(resolved.get("custom-serial"), Some(&false));
        assert!(!resolved.contains_key("custom-default"));
    }

    #[test]
    fn custom_inference_providers_use_user_provider_base_urls() {
        let mut cfg = roder_config::Config::default();
        cfg.providers.insert(
            "local-openai".to_string(),
            roder_config::ProviderConfig {
                api_key: Some("secret".to_string()),
                base_url: Some("http://127.0.0.1:11434/v1".to_string()),
                ..roder_config::ProviderConfig::default()
            },
        );
        cfg.providers.insert(
            "opencode".to_string(),
            roder_config::ProviderConfig {
                api_key: Some("builtin".to_string()),
                base_url: Some("http://ignored.example/v1".to_string()),
                ..roder_config::ProviderConfig::default()
            },
        );

        let providers = custom_inference_providers(&cfg);

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "local-openai");
        assert_eq!(providers[0].api_key.as_deref(), Some("secret"));
        assert_eq!(providers[0].base_url, "http://127.0.0.1:11434/v1");
    }

    #[test]
    fn parses_policy_mode_cli_flags() {
        let options = parse_cli_options(&["--mode".to_string(), "plan".to_string()]).unwrap();
        assert_eq!(options.policy_mode, Some(PolicyMode::Plan));

        let options = parse_cli_options(&["--mode=accept-all".to_string()]).unwrap();
        assert_eq!(options.policy_mode, Some(PolicyMode::AcceptAll));

        let options = parse_cli_options(&["--mode=accept-edits".to_string()]).unwrap();
        assert_eq!(options.policy_mode, Some(PolicyMode::AcceptAll));

        let options = parse_cli_options(&["--yolo".to_string()]).unwrap();
        assert_eq!(options.policy_mode, Some(PolicyMode::Bypass));
    }

    #[test]
    fn parses_team_display_cli_flags() {
        let options =
            parse_cli_options(&["--team-display".to_string(), "in-process".to_string()]).unwrap();
        assert_eq!(
            options.team_display,
            Some(roder_api::teams::AgentTeamDisplayMode::InProcess)
        );

        let options = parse_cli_options(&["--team-display=tmux".to_string()]).unwrap();
        assert_eq!(
            options.team_display,
            Some(roder_api::teams::AgentTeamDisplayMode::Tmux)
        );
    }

    #[test]
    fn parses_resume_menu_cli_command() {
        let options = parse_cli_options(&["resume".to_string()]).unwrap();

        assert_eq!(options.startup, TuiStartup::ResumeMenu);
    }

    #[test]
    fn parses_resume_session_cli_command() {
        let options = parse_cli_options(&[
            "--mode=plan".to_string(),
            "resume".to_string(),
            "abc".to_string(),
        ])
        .unwrap();

        assert_eq!(options.policy_mode, Some(PolicyMode::Plan));
        assert_eq!(
            options.startup,
            TuiStartup::ResumeSession("abc".to_string())
        );
    }

    #[test]
    fn decode_response_includes_error_data() {
        let err = decode_response::<serde_json::Value>(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            result: None,
            error: Some(JsonRpcError {
                code: -32000,
                message: "parse failed".to_string(),
                data: Some(serde_json::json!({
                    "details": "parse session metadata /tmp/metadata.json"
                })),
            }),
        })
        .unwrap_err();

        let rendered = err.to_string();
        assert!(rendered.contains("parse failed (-32000)"));
        assert!(rendered.contains("parse session metadata /tmp/metadata.json"));
    }

    #[test]
    fn config_policy_mode_is_validated() {
        let cfg = roder_config::Config {
            policy_modes: Some(roder_config::PolicyModesConfig {
                default: Some("plna".to_string()),
                ..roder_config::PolicyModesConfig::default()
            }),
            ..roder_config::Config::default()
        };

        let err = resolve_policy_mode(&CliOptions::default(), &cfg).unwrap_err();
        assert!(err.to_string().contains("unsupported policy mode"));
    }

    #[test]
    fn web_search_defaults_to_codex_hosted_cached() {
        let resolved = resolve_web_search_config(None).unwrap();

        assert!(resolved.external.is_none());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Cached
        );
    }

    #[test]
    fn web_search_live_mode_uses_codex_hosted_live() {
        let cfg = roder_config::WebSearchConfig {
            mode: Some("live".to_string()),
            ..roder_config::WebSearchConfig::default()
        };

        let resolved = resolve_web_search_config(Some(&cfg)).unwrap();

        assert!(resolved.external.is_none());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Live
        );
    }

    #[test]
    fn web_search_external_mode_uses_local_router() {
        let cfg = roder_config::WebSearchConfig {
            mode: Some("external".to_string()),
            provider: Some("tavily".to_string()),
            tavily: roder_config::WebSearchProviderConfig {
                api_key: Some("secret".to_string()),
                ..roder_config::WebSearchProviderConfig::default()
            },
            ..roder_config::WebSearchConfig::default()
        };

        let resolved = resolve_web_search_config(Some(&cfg)).unwrap();

        assert!(resolved.external.is_some());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Disabled
        );
    }

    #[test]
    fn web_search_disabled_mode_disables_hosted_and_external() {
        let cfg = roder_config::WebSearchConfig {
            mode: Some("disabled".to_string()),
            ..roder_config::WebSearchConfig::default()
        };

        let resolved = resolve_web_search_config(Some(&cfg)).unwrap();

        assert!(resolved.external.is_none());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Disabled
        );
    }

    #[tokio::test]
    async fn subagents_config_loads_agent_definitions_from_disk() {
        let root = std::env::temp_dir()
            .join(format!("roder-cli-subagents-{}", std::process::id()))
            .join("loads");
        let user_dir = root.join("user");
        let workspace_dir = root.join("workspace");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::write(
            user_dir.join("explore.md"),
            r#"---
name: explore
description: Explore the workspace
tools: [echo]
---

Report findings.
"#,
        )
        .unwrap();

        let cfg = roder_config::SubagentsConfig {
            enabled: true,
            default_agent: Some("explore".to_string()),
            disk: roder_config::SubagentsDiskConfig {
                user_dir: Some(user_dir),
                workspace_dir: Some(workspace_dir),
            },
            ..roder_config::SubagentsConfig::default()
        };

        let resolved =
            resolve_subagents_config(Some(&cfg), PROVIDER_MOCK.to_string(), "mock".to_string())
                .await
                .unwrap()
                .unwrap();

        assert!(resolved.enabled);
        assert_eq!(resolved.default_agent, "explore");
        assert_eq!(resolved.definitions.len(), 1);
        assert_eq!(resolved.definitions[0].agent_type, "explore");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn subagents_do_not_default_to_workspace_roder_dir() {
        let cfg = roder_config::SubagentsConfig {
            enabled: true,
            disk: roder_config::SubagentsDiskConfig {
                user_dir: None,
                workspace_dir: None,
            },
            ..roder_config::SubagentsConfig::default()
        };

        assert!(resolve_workspace_agent_dir(&cfg).unwrap().is_none());
    }

    #[tokio::test]
    async fn subagents_disabled_config_skips_loading() {
        let cfg = roder_config::SubagentsConfig {
            enabled: false,
            disk: roder_config::SubagentsDiskConfig {
                user_dir: Some(PathBuf::from("/definitely/not/a/real/agent/dir")),
                workspace_dir: Some(PathBuf::from("/definitely/not/a/real/workspace/dir")),
            },
            ..roder_config::SubagentsConfig::default()
        };

        let resolved =
            resolve_subagents_config(Some(&cfg), PROVIDER_MOCK.to_string(), "mock".to_string())
                .await
                .unwrap()
                .unwrap();

        assert!(!resolved.enabled);
        assert!(resolved.definitions.is_empty());
    }

    #[test]
    fn remote_runner_disabled_defaults_to_local_filesystem() {
        let cfg = roder_config::RemoteRunnersConfig {
            enabled: false,
            default_destination: Some("docker".to_string()),
            destinations: std::collections::HashMap::new(),
        };

        let resolved = resolve_remote_runner_destination(Some(&cfg)).unwrap();

        assert!(resolved.is_none());
    }

    #[test]
    fn app_server_remote_defaults_to_websocket_listen() {
        let options = parse_app_server_options(&["--remote".to_string()]).unwrap();
        assert!(options.remote);
        assert_eq!(options.listen, "ws://0.0.0.0:0");
        assert!(options.print_qr);
    }

    #[test]
    fn app_server_remote_options_do_not_change_stdio_default() {
        let options = parse_app_server_options(&[
            "--auth-token".to_string(),
            "remote-secret".to_string(),
            "--remote-token-ttl".to_string(),
            "60".to_string(),
            "--allowed-origin=https://client.example".to_string(),
            "--print-qr=false".to_string(),
        ])
        .unwrap();

        assert!(!options.remote);
        assert_eq!(options.listen, "stdio://");
        assert_eq!(options.auth_token.as_deref(), Some("remote-secret"));
        assert_eq!(options.remote_token_ttl, Some(time::Duration::seconds(60)));
        assert_eq!(
            options.allowed_origins,
            vec!["https://client.example".to_string()]
        );
        assert!(!options.print_qr);
    }

    #[test]
    fn app_server_remote_accepts_auth_token_env() {
        unsafe {
            std::env::set_var("RODER_TEST_REMOTE_TOKEN", "remote-secret");
        }
        let options = parse_app_server_options(&[
            "--remote".to_string(),
            "--auth-token".to_string(),
            "env:RODER_TEST_REMOTE_TOKEN".to_string(),
            "--print-qr=false".to_string(),
        ])
        .unwrap();
        assert_eq!(options.auth_token.as_deref(), Some("remote-secret"));
        assert!(!options.print_qr);
    }

    #[test]
    fn app_server_remote_accepts_token_ttl_seconds() {
        let options = parse_app_server_options(&[
            "--remote".to_string(),
            "--remote-token-ttl".to_string(),
            "60".to_string(),
        ])
        .unwrap();
        assert_eq!(options.remote_token_ttl, Some(time::Duration::seconds(60)));

        let err =
            parse_app_server_options(&["--remote".to_string(), "--remote-token-ttl=0".to_string()])
                .unwrap_err()
                .to_string();
        assert!(err.contains("positive second count"));
    }

    #[test]
    fn app_server_remote_accepts_allowed_origins() {
        let options = parse_app_server_options(&[
            "--remote".to_string(),
            "--allowed-origin".to_string(),
            "https://client.example".to_string(),
            "--allowed-origin=https://second.example".to_string(),
        ])
        .unwrap();
        assert_eq!(
            options.allowed_origins,
            vec![
                "https://client.example".to_string(),
                "https://second.example".to_string(),
            ]
        );
    }

    #[test]
    fn remote_runner_resolves_unix_local_env_style_destination() {
        let cfg = roder_config::RemoteRunnersConfig {
            enabled: true,
            default_destination: Some("unix-local".to_string()),
            destinations: std::collections::HashMap::new(),
        };

        let resolved = resolve_remote_runner_destination(Some(&cfg))
            .unwrap()
            .unwrap();

        assert_eq!(resolved.id, "unix-local");
        assert_eq!(resolved.provider_id, "unix-local");
    }

    #[test]
    fn remote_runner_resolves_configured_destination_without_secret_values() {
        let mut destinations = std::collections::HashMap::new();
        destinations.insert(
            "docker-dev".to_string(),
            roder_config::RemoteRunnerDestinationConfig {
                provider: "docker".to_string(),
                config: serde_json::json!({ "image": "rust:latest" }),
                secret_env: std::collections::HashMap::from([(
                    "DOCKER_TOKEN".to_string(),
                    "RODER_DOCKER_TOKEN".to_string(),
                )]),
            },
        );
        let cfg = roder_config::RemoteRunnersConfig {
            enabled: true,
            default_destination: Some("docker-dev".to_string()),
            destinations,
        };

        let resolved = resolve_remote_runner_destination(Some(&cfg))
            .unwrap()
            .unwrap();

        assert_eq!(resolved.id, "docker-dev");
        assert_eq!(resolved.provider_id, "docker");
        assert_eq!(resolved.config["image"], "rust:latest");
    }

    #[test]
    fn remote_runner_rejects_unknown_destination_and_raw_secret_keys() {
        let cfg = roder_config::RemoteRunnersConfig {
            enabled: true,
            default_destination: Some("missing".to_string()),
            destinations: std::collections::HashMap::new(),
        };
        let err = resolve_remote_runner_destination(Some(&cfg)).unwrap_err();
        assert!(
            err.to_string()
                .contains("unknown remote runner destination")
        );

        let mut destinations = std::collections::HashMap::new();
        destinations.insert(
            "docker-dev".to_string(),
            roder_config::RemoteRunnerDestinationConfig {
                provider: "docker".to_string(),
                config: serde_json::json!({ "api_key": "secret" }),
                secret_env: std::collections::HashMap::new(),
            },
        );
        let cfg = roder_config::RemoteRunnersConfig {
            enabled: true,
            default_destination: Some("docker-dev".to_string()),
            destinations,
        };
        let err = resolve_remote_runner_destination(Some(&cfg)).unwrap_err();
        assert!(err.to_string().contains("secret_env"));
    }
}
