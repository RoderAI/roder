use std::sync::Arc;

use anyhow::Context;
use roder_api::tasks::TaskState;
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, WebwrightArtifactsResult, WebwrightExportParams, WebwrightExportResult,
    WebwrightPrepareParams, WebwrightRerunParams, WebwrightRerunResult, WebwrightSetupParams,
    WebwrightSetupResult, WebwrightSubmitParams, WebwrightSubmitResult, WebwrightVerifyResult,
    WebwrightVisualJudgeParams, WebwrightVisualJudgeResult, WebwrightWorkspaceParams,
};

use crate::{CliOptions, build_runtime_from_config, decode_response, task_get};

pub(crate) async fn run_webwright_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("run") | Some("craft") => {
            let mode = args[0].clone();
            let options = parse_task_options(&args[1..], &mode)?;
            let task = options.task;
            if task.trim().is_empty() {
                anyhow::bail!("roder webwright {mode} requires a task");
            }
            let result: WebwrightSubmitResult = request(
                &client,
                "webwright/submit",
                serde_json::to_value(WebwrightSubmitParams {
                    task,
                    mode: Some(mode),
                    start_url: options.start_url,
                    task_id: None,
                    browser: options.browser,
                    headless: options.headless,
                    output_dir: None,
                    timeout_seconds: None,
                    thread_id: None,
                    turn_id: None,
                    workspace: None,
                })?,
            )
            .await?;
            println!(
                "{}\t{}\t{:?}\t{}",
                result.task.task_id,
                result.task.executor_id,
                result.task.state,
                result.task.spec.kind
            );
            wait_for_task(&client, &result.task.task_id).await?;
        }
        Some("setup") => {
            let options = parse_setup_options(&args[1..])?;
            let result: WebwrightSetupResult = request(
                &client,
                "webwright/setup",
                serde_json::to_value(WebwrightSetupParams {
                    python: options.python,
                    browser: options.browser,
                    dry_run: options.dry_run,
                })?,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Some("prepare") => {
            let options = parse_task_options(&args[1..], "prepare")?;
            let task = options.task;
            if task.trim().is_empty() {
                anyhow::bail!("roder webwright prepare requires a task");
            }
            let result: roder_protocol::WebwrightPrepareResult = request(
                &client,
                "webwright/prepare",
                serde_json::to_value(WebwrightPrepareParams {
                    task,
                    mode: Some("run".to_string()),
                    start_url: options.start_url,
                    task_id: None,
                    browser: options.browser,
                    headless: options.headless,
                    output_dir: None,
                    workspace: None,
                })?,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result.workspace)?);
        }
        Some("inspect") => {
            let Some(workspace) = args.get(1) else {
                anyhow::bail!("roder webwright inspect requires a workspace path");
            };
            let result: WebwrightArtifactsResult =
                workspace_request(&client, "webwright/artifacts", workspace).await?;
            println!("{}", serde_json::to_string_pretty(&result.workspace)?);
        }
        Some("verify") => {
            let Some(workspace) = args.get(1) else {
                anyhow::bail!("roder webwright verify requires a workspace path");
            };
            let result: WebwrightVerifyResult =
                workspace_request(&client, "webwright/verify", workspace).await?;
            println!("{}", serde_json::to_string_pretty(&result.verification)?);
        }
        Some("visual-judge") => {
            let Some(workspace) = args.get(1) else {
                anyhow::bail!("roder webwright visual-judge requires a workspace path");
            };
            let result: WebwrightVisualJudgeResult = request(
                &client,
                "webwright/visualJudge",
                serde_json::to_value(WebwrightVisualJudgeParams {
                    workspace: workspace.clone(),
                    workspace_root: None,
                    run_id: None,
                    enabled: Some(true),
                })?,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result.visual_judge)?);
        }
        Some("rerun") => {
            let Some(workspace) = args.get(1) else {
                anyhow::bail!("roder webwright rerun requires a workspace path");
            };
            let result: WebwrightRerunResult = request(
                &client,
                "webwright/rerun",
                serde_json::to_value(WebwrightRerunParams {
                    workspace: workspace.clone(),
                    workspace_root: None,
                    python: None,
                    thread_id: None,
                    turn_id: None,
                })?,
            )
            .await?;
            println!(
                "{}\t{}\t{:?}\trun_{:03}\t{}",
                result.task.task_id,
                result.task.executor_id,
                result.task.state,
                result.run_id,
                result.run_dir
            );
            wait_for_task(&client, &result.task.task_id).await?;
        }
        Some("export") => {
            let Some(workspace) = args.get(1) else {
                anyhow::bail!("roder webwright export requires a workspace path");
            };
            let Some(output_dir) = args.get(2) else {
                anyhow::bail!("roder webwright export requires an output directory");
            };
            let result: WebwrightExportResult = request(
                &client,
                "webwright/export",
                serde_json::to_value(WebwrightExportParams {
                    workspace: workspace.clone(),
                    workspace_root: None,
                    output_dir: output_dir.clone(),
                })?,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        _ => anyhow::bail!(
            "usage: roder webwright <setup [--browser firefox|chromium|webkit] [--python PYTHON] [--dry-run]|run [--browser BROWSER] TASK|craft [--browser BROWSER] TASK|prepare [--browser BROWSER] TASK|inspect WORKSPACE|verify WORKSPACE|visual-judge WORKSPACE|rerun WORKSPACE|export WORKSPACE OUTPUT_DIR>"
        ),
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WebwrightTaskCliOptions {
    browser: Option<String>,
    start_url: Option<String>,
    headless: Option<bool>,
    task: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WebwrightSetupCliOptions {
    browser: Option<String>,
    python: Option<String>,
    dry_run: bool,
}

fn parse_task_options(args: &[String], command: &str) -> anyhow::Result<WebwrightTaskCliOptions> {
    let mut options = WebwrightTaskCliOptions::default();
    let mut task = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--browser" => {
                index += 1;
                options.browser = Some(required_arg(args, index, "--browser")?.to_string());
            }
            "--start-url" => {
                index += 1;
                options.start_url = Some(required_arg(args, index, "--start-url")?.to_string());
            }
            "--headed" => {
                options.headless = Some(false);
            }
            "--headless" => {
                options.headless = Some(true);
            }
            "--" => {
                task.extend(args[index + 1..].iter().cloned());
                break;
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown roder webwright {command} option {value}");
            }
            value => task.push(value.to_string()),
        }
        index += 1;
    }
    options.task = task.join(" ");
    Ok(options)
}

fn parse_setup_options(args: &[String]) -> anyhow::Result<WebwrightSetupCliOptions> {
    let mut options = WebwrightSetupCliOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--browser" => {
                index += 1;
                options.browser = Some(required_arg(args, index, "--browser")?.to_string());
            }
            "--python" => {
                index += 1;
                options.python = Some(required_arg(args, index, "--python")?.to_string());
            }
            "--dry-run" => {
                options.dry_run = true;
            }
            "--" => {
                let rest = &args[index + 1..];
                if rest.len() > 1 || (rest.len() == 1 && options.browser.is_some()) {
                    anyhow::bail!("roder webwright setup accepts at most one browser");
                }
                if let Some(browser) = rest.first() {
                    options.browser = Some(browser.clone());
                }
                break;
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown roder webwright setup option {value}");
            }
            value => {
                if options.browser.is_some() {
                    anyhow::bail!("roder webwright setup accepts at most one browser");
                }
                options.browser = Some(value.to_string());
            }
        }
        index += 1;
    }
    Ok(options)
}

fn required_arg<'a>(args: &'a [String], index: usize, flag: &str) -> anyhow::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{flag} requires a value"))
}

async fn workspace_request<T>(
    client: &LocalAppClient,
    method: &str,
    workspace: &str,
) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    request(
        client,
        method,
        serde_json::to_value(WebwrightWorkspaceParams {
            workspace: workspace.to_string(),
            workspace_root: None,
        })?,
    )
    .await
}

async fn request<T>(
    client: &LocalAppClient,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(params),
        })
        .await;
    decode_response::<T>(res)
}

async fn wait_for_task(client: &LocalAppClient, task_id: &str) -> anyhow::Result<()> {
    loop {
        let result = task_get(client, task_id).await?;
        if matches!(
            result.task.state,
            TaskState::Completed | TaskState::Failed | TaskState::Cancelled
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_setup_options, parse_task_options};

    #[test]
    fn parse_setup_options_accepts_browser_python_and_dry_run() {
        let args = [
            "--browser",
            "chromium",
            "--python",
            "/tmp/python",
            "--dry-run",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();

        let parsed = parse_setup_options(&args).unwrap();

        assert_eq!(parsed.browser.as_deref(), Some("chromium"));
        assert_eq!(parsed.python.as_deref(), Some("/tmp/python"));
        assert!(parsed.dry_run);
    }

    #[test]
    fn parse_task_options_keeps_browser_out_of_task_text() {
        let args = ["--browser", "webkit", "Open", "the", "page"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();

        let parsed = parse_task_options(&args, "run").unwrap();

        assert_eq!(parsed.browser.as_deref(), Some("webkit"));
        assert_eq!(parsed.task, "Open the page");
    }
}
