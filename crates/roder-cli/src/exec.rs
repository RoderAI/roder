use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, ThreadListParams, ThreadListResult, ThreadReadParams, ThreadReadResult,
    ThreadStartParams, ThreadStartResult, TurnInputItem, TurnInterruptParams, TurnStartParams,
    TurnStartResult, WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceRootInput,
};
use tokio::sync::broadcast;

use crate::exec_events::ExecEvent;
use crate::exec_output::{ExecOutput, TurnTerminalState};
use crate::parse_runtime_profile;
use crate::{CliOptions, build_runtime_from_config, decode_response, parse_policy_mode};

#[derive(Debug, Clone)]
pub(crate) struct ExecCli {
    pub prompt: Option<String>,
    pub resume: ExecResume,
    pub json: bool,
    pub output_last_message: Option<PathBuf>,
    pub skip_git_repo_check: bool,
    pub ephemeral: bool,
    pub task_ledger_required: bool,
    pub images: Vec<PathBuf>,
    pub cli_options: CliOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExecResume {
    New,
    Thread(String),
    Last,
}

pub(crate) async fn run_exec_cli(args: &[String]) -> anyhow::Result<()> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_exec_help();
        return Ok(());
    }

    let options = parse_exec_cli(args)?;
    let _skip_git_repo_check = options.skip_git_repo_check;
    let prompt = resolve_prompt(options.prompt.clone())?;
    let (runtime, default_model) = build_runtime_from_config(options.cli_options.clone()).await?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    let client = LocalAppClient::new(app_server.clone());
    let mut notifications = client.subscribe_notifications();
    let mut output = ExecOutput::new(options.json, options.output_last_message.clone());
    let cwd = std::env::current_dir()?.display().to_string();

    let thread_id = match &options.resume {
        ExecResume::New => {
            let workspace = create_single_root_workspace(&client, &cwd).await?;
            let res = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("exec-thread-start")),
                    method: "thread/start".to_string(),
                    params: Some(serde_json::to_value(ThreadStartParams {
                        selection: None,
                        workspace_id: workspace.0,
                        root_id: Some(workspace.1),
                        model: Some(default_model),
                        model_provider: None,
                        reasoning: None,
                        cwd: None,
                        ephemeral: options.ephemeral,
                    })?),
                })
                .await;
            let result = decode_response::<ThreadStartResult>(res)?;
            result.thread.id
        }
        ExecResume::Thread(thread_id) => {
            ensure_thread_exists(&client, thread_id).await?;
            thread_id.clone()
        }
        ExecResume::Last => last_thread_id(&client).await?,
    };

    output.emit_event(ExecEvent::ThreadStarted {
        thread_id: thread_id.clone(),
    })?;

    let turn_result = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("exec-turn-start")),
            method: "turn/start".to_string(),
            params: Some(serde_json::to_value(TurnStartParams {
                thread_id: thread_id.clone(),
                input: turn_input_items(&options.images),
                prompt,
                model_provider: None,
                model: None,
                reasoning: None,
                policy_mode: None,
                task_ledger_required: options.task_ledger_required,
            })?),
        })
        .await;
    let TurnStartResult { turn_id } = decode_response::<TurnStartResult>(turn_result)?;

    let terminal = wait_for_turn(
        &client,
        &mut notifications,
        &mut output,
        &thread_id,
        &turn_id,
    )
    .await?;

    if let Some(items) = read_turn_items(&client, &thread_id, &turn_id).await? {
        output.backfill_final_message(&items);
    }
    output.finish(&terminal)?;

    if let TurnTerminalState::Failed(error) = terminal {
        anyhow::bail!("{error}");
    }
    Ok(())
}

async fn create_single_root_workspace(
    client: &LocalAppClient,
    cwd: &str,
) -> anyhow::Result<(String, String)> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("exec-workspace-create")),
            method: "workspace/create".to_string(),
            params: Some(serde_json::to_value(WorkspaceCreateParams {
                name: None,
                roots: vec![WorkspaceRootInput {
                    path: cwd.to_string(),
                    name: None,
                }],
                default_root_path: Some(cwd.to_string()),
            })?),
        })
        .await;
    let result = decode_response::<WorkspaceCreateResult>(res)?;
    let root_id = result.workspace.default_root_id.clone();
    Ok((result.workspace.id, root_id))
}

fn print_exec_help() {
    println!(
        "Usage: roder exec [OPTIONS] [PROMPT]\n       roder exec resume [THREAD_ID|--last] [PROMPT]\n\nOptions:\n  --json                         emit JSONL events to stdout\n  --output-last-message <FILE>   write final assistant text to FILE\n  --skip-git-repo-check          allow benchmark sandboxes without git metadata\n  --ephemeral                    request an ephemeral thread where supported\n  --task-ledger-required         require eval task ledger updates before work\n  --profile <PROFILE>            select runtime profile, for example eval\n  --mode <MODE>                  select policy mode, for example bypass\n  --image <FILE>                 attach local image input\n  -                              read prompt from stdin\n  -h, --help                     show this help\n\nDefault stdout is the final assistant message only; diagnostics are written to stderr."
    );
}

pub(crate) fn parse_exec_cli(args: &[String]) -> anyhow::Result<ExecCli> {
    let mut cli_options = CliOptions::default();
    let mut prompt_parts = Vec::new();
    let mut resume = ExecResume::New;
    let mut json = false;
    let mut output_last_message = None;
    let mut skip_git_repo_check = false;
    let mut ephemeral = false;
    let mut task_ledger_required = false;
    let mut images = Vec::new();
    let mut i = 0;

    if args.first().map(String::as_str) == Some("resume") {
        resume = ExecResume::Last;
        i = 1;
        if let Some(arg) = args.get(i)
            && !arg.starts_with('-')
        {
            resume = ExecResume::Thread(arg.clone());
            i += 1;
        }
    }

    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--output-last-message" => {
                let Some(path) = args.get(i + 1) else {
                    anyhow::bail!("--output-last-message requires a file");
                };
                output_last_message = Some(PathBuf::from(path));
                i += 1;
            }
            arg if arg.starts_with("--output-last-message=") => {
                output_last_message = Some(PathBuf::from(&arg["--output-last-message=".len()..]));
            }
            "--skip-git-repo-check" => skip_git_repo_check = true,
            "--ephemeral" => ephemeral = true,
            "--task-ledger-required" => task_ledger_required = true,
            "--image" => {
                let Some(path) = args.get(i + 1) else {
                    anyhow::bail!("--image requires a file");
                };
                images.push(PathBuf::from(path));
                i += 1;
            }
            arg if arg.starts_with("--image=") => {
                images.push(PathBuf::from(&arg["--image=".len()..]));
            }
            "--last" if matches!(resume, ExecResume::Last) => {
                resume = ExecResume::Last;
            }
            "--mode" => {
                let Some(mode) = args.get(i + 1) else {
                    anyhow::bail!("--mode requires a value");
                };
                cli_options.policy_mode = Some(parse_policy_mode(mode)?);
                i += 1;
            }
            arg if arg.starts_with("--mode=") => {
                cli_options.policy_mode = Some(parse_policy_mode(&arg["--mode=".len()..])?);
            }
            "--profile" => {
                let Some(profile) = args.get(i + 1) else {
                    anyhow::bail!("--profile requires a value");
                };
                cli_options.runtime_profile = Some(parse_runtime_profile(profile)?);
                i += 1;
            }
            arg if arg.starts_with("--profile=") => {
                cli_options.runtime_profile =
                    Some(parse_runtime_profile(&arg["--profile=".len()..])?);
            }
            "--record-api-transcript" => {
                let Some(path) = args.get(i + 1) else {
                    anyhow::bail!("--record-api-transcript requires a path");
                };
                cli_options.record_api_transcript = Some(PathBuf::from(path));
                i += 1;
            }
            arg if arg.starts_with("--record-api-transcript=") => {
                cli_options.record_api_transcript =
                    Some(PathBuf::from(&arg["--record-api-transcript=".len()..]));
            }
            "-" => prompt_parts.push("-".to_string()),
            arg => prompt_parts.push(arg.to_string()),
        }
        i += 1;
    }

    Ok(ExecCli {
        prompt: if prompt_parts.is_empty() {
            None
        } else {
            Some(prompt_parts.join(" "))
        },
        resume,
        json,
        output_last_message,
        skip_git_repo_check,
        ephemeral,
        task_ledger_required,
        images,
        cli_options,
    })
}

fn resolve_prompt(prompt: Option<String>) -> anyhow::Result<Option<String>> {
    match prompt.as_deref() {
        Some("-") => read_stdin_prompt().map(Some),
        Some(_) => Ok(prompt),
        None if !std::io::stdin().is_terminal() => read_stdin_prompt().map(Some),
        None => Ok(None),
    }
}

fn read_stdin_prompt() -> anyhow::Result<String> {
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt)?;
    Ok(prompt)
}

fn turn_input_items(images: &[PathBuf]) -> Vec<TurnInputItem> {
    images
        .iter()
        .map(|path| TurnInputItem {
            kind: "image".to_string(),
            text: None,
            path: Some(path.display().to_string()),
            image_url: None,
        })
        .collect()
}

async fn ensure_thread_exists(client: &LocalAppClient, thread_id: &str) -> anyhow::Result<()> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("exec-thread-read")),
            method: "thread/read".to_string(),
            params: Some(serde_json::to_value(ThreadReadParams {
                thread_id: thread_id.to_string(),
                include_turns: false,
            })?),
        })
        .await;
    let result = decode_response::<ThreadReadResult>(res)?;
    if result.thread.is_none() {
        anyhow::bail!("thread {thread_id:?} was not found");
    }
    Ok(())
}

async fn last_thread_id(client: &LocalAppClient) -> anyhow::Result<String> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("exec-thread-list")),
            method: "thread/list".to_string(),
            params: Some(serde_json::to_value(ThreadListParams {
                limit: Some(1),
                cursor: None,
            })?),
        })
        .await;
    let result = decode_response::<ThreadListResult>(res)?;
    result
        .data
        .into_iter()
        .next()
        .map(|thread| thread.id)
        .ok_or_else(|| anyhow::anyhow!("no previous thread found"))
}

async fn wait_for_turn(
    client: &LocalAppClient,
    notifications: &mut broadcast::Receiver<roder_protocol::JsonRpcNotification>,
    output: &mut ExecOutput,
    thread_id: &str,
    turn_id: &str,
) -> anyhow::Result<TurnTerminalState> {
    loop {
        tokio::select! {
            recv = notifications.recv() => {
                match recv {
                    Ok(notification) => {
                        if let Some(state) = output.process_notification(&notification, thread_id, turn_id)? {
                            return Ok(state);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        output.emit_error(format!("warning: skipped {count} app-server notifications"))?;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Ok(TurnTerminalState::Failed("app-server notification stream closed".to_string()));
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                let _ = client
                    .send_request(JsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        id: Some(serde_json::json!("exec-turn-interrupt")),
                        method: "turn/interrupt".to_string(),
                        params: Some(serde_json::to_value(TurnInterruptParams {
                            thread_id: thread_id.to_string(),
                            turn_id: Some(turn_id.to_string()),
                        })?),
                    })
                    .await;
                return Ok(TurnTerminalState::Failed("interrupted".to_string()));
            }
        }
    }
}

async fn read_turn_items(
    client: &LocalAppClient,
    thread_id: &str,
    turn_id: &str,
) -> anyhow::Result<Option<Vec<roder_protocol::Item>>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("exec-thread-read-final")),
            method: "thread/read".to_string(),
            params: Some(serde_json::to_value(ThreadReadParams {
                thread_id: thread_id.to_string(),
                include_turns: true,
            })?),
        })
        .await;
    let result = decode_response::<ThreadReadResult>(res)?;
    Ok(result.thread.and_then(|thread| {
        thread
            .turns
            .unwrap_or_default()
            .into_iter()
            .find(|turn| turn.id == turn_id)
            .map(|turn| turn.items)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::RuntimeProfile;
    use roder_api::policy_mode::PolicyMode;

    #[test]
    fn exec_cli_parses_prompt_and_harness_flags() {
        let args = vec![
            "--json".to_string(),
            "--profile".to_string(),
            "eval".to_string(),
            "--mode=bypass".to_string(),
            "--skip-git-repo-check".to_string(),
            "--task-ledger-required".to_string(),
            "--output-last-message".to_string(),
            "/tmp/last.txt".to_string(),
            "reply".to_string(),
            "ok".to_string(),
        ];

        let parsed = parse_exec_cli(&args).unwrap();
        assert_eq!(parsed.prompt.as_deref(), Some("reply ok"));
        assert!(parsed.json);
        assert!(parsed.skip_git_repo_check);
        assert!(parsed.task_ledger_required);
        assert_eq!(parsed.cli_options.policy_mode, Some(PolicyMode::Bypass));
        assert_eq!(
            parsed.cli_options.runtime_profile,
            Some(RuntimeProfile::Eval)
        );
        assert_eq!(
            parsed.output_last_message.as_deref(),
            Some(std::path::Path::new("/tmp/last.txt"))
        );
    }

    #[test]
    fn exec_cli_parses_resume_last() {
        let args = vec!["resume".to_string(), "--last".to_string(), "-".to_string()];
        let parsed = parse_exec_cli(&args).unwrap();
        assert_eq!(parsed.resume, ExecResume::Last);
        assert_eq!(parsed.prompt.as_deref(), Some("-"));
    }

    #[test]
    fn exec_cli_parses_resume_thread() {
        let args = vec![
            "resume".to_string(),
            "thread_1".to_string(),
            "continue".to_string(),
        ];
        let parsed = parse_exec_cli(&args).unwrap();
        assert_eq!(parsed.resume, ExecResume::Thread("thread_1".to_string()));
        assert_eq!(parsed.prompt.as_deref(), Some("continue"));
    }
}
