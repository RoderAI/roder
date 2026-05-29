use std::sync::Arc;

use roder_api::dynamic_workflows::WorkflowApprovalDecision;
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::*;

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub(crate) async fn run_workflows_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("plan") => {
            let (prompt, script) = prompt_and_optional_script(&args[1..])?;
            let result: WorkflowsPlanResult = request(
                &client,
                "workflows/plan",
                WorkflowsPlanParams {
                    thread_id: None,
                    turn_id: None,
                    prompt,
                    workspace: None,
                    arguments: serde_json::Value::Object(Default::default()),
                    script,
                },
            )
            .await?;
            println!(
                "planned\t{}\t{}\t{:?}",
                result.run.run_id, result.run.script.name, result.run.status
            );
        }
        Some("run") => {
            let script = script_arg(&args[1..])?;
            let planned: WorkflowsPlanResult = request(
                &client,
                "workflows/plan",
                WorkflowsPlanParams {
                    thread_id: None,
                    turn_id: None,
                    prompt: "run saved workflow script".to_string(),
                    workspace: None,
                    arguments: serde_json::Value::Object(Default::default()),
                    script: Some(script),
                },
            )
            .await?;
            let approved: WorkflowsApproveResult = request(
                &client,
                "workflows/approve",
                WorkflowsApproveParams {
                    run_id: planned.run.run_id,
                    decision: WorkflowApprovalDecision::RunOnce,
                    reason: None,
                },
            )
            .await?;
            println!(
                "running\t{}\t{:?}",
                approved.run.run_id, approved.run.status
            );
        }
        Some("approve") => {
            let run_id = required_arg(args, 1, "usage: roder workflows approve RUN_ID [--deny]")?;
            let decision = if args.iter().any(|arg| arg == "--deny") {
                WorkflowApprovalDecision::Deny
            } else {
                WorkflowApprovalDecision::RunOnce
            };
            let result: WorkflowsApproveResult = request(
                &client,
                "workflows/approve",
                WorkflowsApproveParams {
                    run_id,
                    decision,
                    reason: None,
                },
            )
            .await?;
            println!("approved\t{}\t{:?}", result.run.run_id, result.run.status);
        }
        Some("list") => {
            let result: WorkflowsListResult = request(
                &client,
                "workflows/list",
                WorkflowsListParams {
                    thread_id: None,
                    include_terminal: args.iter().any(|arg| arg == "--all"),
                },
            )
            .await?;
            for run in result.runs {
                println!("{}\t{:?}\t{}", run.run_id, run.status, run.title);
            }
        }
        Some("get") => {
            let run_id = required_arg(args, 1, "usage: roder workflows get RUN_ID")?;
            let result: WorkflowsGetResult = request(
                &client,
                "workflows/get",
                WorkflowsGetParams {
                    run_id,
                    include_script_body: args.iter().any(|arg| arg == "--include-script-body"),
                    include_agents: true,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result.run)?);
        }
        Some("pause") => {
            let run_id = required_arg(args, 1, "usage: roder workflows pause RUN_ID")?;
            let result: WorkflowsPauseResult = request(
                &client,
                "workflows/pause",
                WorkflowsPauseParams {
                    run_id,
                    cancel_running_agents: false,
                    reason: None,
                },
            )
            .await?;
            println!("paused\t{}\t{:?}", result.run.run_id, result.run.status);
        }
        Some("resume") => {
            let run_id = required_arg(args, 1, "usage: roder workflows resume RUN_ID")?;
            let result: WorkflowsResumeResult = request(
                &client,
                "workflows/resume",
                WorkflowsResumeParams { run_id },
            )
            .await?;
            println!("resumed\t{}\t{:?}", result.run.run_id, result.run.status);
        }
        Some("stop") => {
            let run_id = required_arg(args, 1, "usage: roder workflows stop RUN_ID")?;
            let result: WorkflowsStopResult = request(
                &client,
                "workflows/stop",
                WorkflowsStopParams {
                    run_id,
                    reason: None,
                },
            )
            .await?;
            println!("stopped\t{}\t{:?}", result.run.run_id, result.run.status);
        }
        Some("restart-agent") => {
            let run_id = required_arg(
                args,
                1,
                "usage: roder workflows restart-agent RUN_ID AGENT_ID",
            )?;
            let agent_id = required_arg(
                args,
                2,
                "usage: roder workflows restart-agent RUN_ID AGENT_ID",
            )?;
            let result: WorkflowsRestartAgentResult = request(
                &client,
                "workflows/restartAgent",
                WorkflowsRestartAgentParams { run_id, agent_id },
            )
            .await?;
            println!(
                "restarted\t{}\t{}",
                result.run.run_id, result.agent.agent_id
            );
        }
        Some("save") => {
            let run_id = required_arg(
                args,
                1,
                "usage: roder workflows save RUN_ID NAME [--scope user|workspace] [--overwrite]",
            )?;
            let name = required_arg(
                args,
                2,
                "usage: roder workflows save RUN_ID NAME [--scope user|workspace] [--overwrite]",
            )?;
            let result: WorkflowsSaveResult = request(
                &client,
                "workflows/save",
                WorkflowsSaveParams {
                    run_id,
                    name,
                    scope: save_scope(args)?,
                    overwrite: args.iter().any(|arg| arg == "--overwrite"),
                },
            )
            .await?;
            println!(
                "saved\t{}\t{}",
                result.script.name,
                result.script.source.path.as_deref().unwrap_or("-")
            );
        }
        Some("scripts") => run_scripts_cli(&client, &args[1..]).await?,
        _ => anyhow::bail!(
            "usage: roder workflows <plan|run|list|get|pause|resume|stop|restart-agent|save|scripts>"
        ),
    }
    Ok(())
}

async fn run_scripts_cli(client: &LocalAppClient, args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("list") => {
            let result: WorkflowsScriptsListResult = request(
                client,
                "workflows/scripts/list",
                WorkflowsScriptsListParams {
                    workspace: None,
                    include_user: true,
                    include_builtin: true,
                },
            )
            .await?;
            for script in result.scripts {
                println!(
                    "{}\t{:?}\t{}",
                    script.script_id, script.source.kind, script.name
                );
            }
        }
        Some("read") => {
            let name = required_arg(args, 1, "usage: roder workflows scripts read NAME")?;
            let result: WorkflowsScriptsReadResult = request(
                client,
                "workflows/scripts/read",
                WorkflowsScriptsReadParams {
                    script_id: None,
                    name: Some(name),
                    source: None,
                    include_body: args.iter().any(|arg| arg == "--include-body"),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result.script)?);
        }
        Some("delete") => {
            let script_id = required_arg(
                args,
                1,
                "usage: roder workflows scripts delete SCRIPT_ID --delete-file",
            )?;
            let result: WorkflowsScriptsDeleteResult = request(
                client,
                "workflows/scripts/delete",
                WorkflowsScriptsDeleteParams {
                    script_id,
                    delete_file: args.iter().any(|arg| arg == "--delete-file"),
                },
            )
            .await?;
            println!("deleted\t{}\t{}", result.script_id, result.deleted);
        }
        _ => anyhow::bail!("usage: roder workflows scripts <list|read NAME|delete SCRIPT_ID>"),
    }
    Ok(())
}

async fn request<T, P>(client: &LocalAppClient, method: &str, params: P) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
    P: serde::Serialize,
{
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(response)
}

fn prompt_and_optional_script(args: &[String]) -> anyhow::Result<(String, Option<String>)> {
    if let Some(index) = args.iter().position(|arg| arg == "--script") {
        let path = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("--script requires a path"))?;
        let prompt = args[..index].join(" ");
        return Ok((prompt, Some(std::fs::read_to_string(path)?)));
    }
    Ok((args.join(" "), None))
}

fn script_arg(args: &[String]) -> anyhow::Result<String> {
    let path = args
        .iter()
        .position(|arg| arg == "--script")
        .and_then(|index| args.get(index + 1))
        .or_else(|| args.first())
        .ok_or_else(|| anyhow::anyhow!("usage: roder workflows run --script PATH"))?;
    Ok(std::fs::read_to_string(path)?)
}

fn save_scope(args: &[String]) -> anyhow::Result<WorkflowsSaveScope> {
    let Some(index) = args.iter().position(|arg| arg == "--scope") else {
        return Ok(WorkflowsSaveScope::Workspace);
    };
    match args.get(index + 1).map(String::as_str) {
        Some("user") => Ok(WorkflowsSaveScope::User),
        Some("workspace") => Ok(WorkflowsSaveScope::Workspace),
        _ => anyhow::bail!("--scope requires user or workspace"),
    }
}

fn required_arg(args: &[String], index: usize, usage: &str) -> anyhow::Result<String> {
    args.get(index)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!(usage.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_cli_parses_save_scope_default_and_explicit() {
        assert_eq!(save_scope(&[]).unwrap(), WorkflowsSaveScope::Workspace);
        assert_eq!(
            save_scope(&["--scope".to_string(), "user".to_string()]).unwrap(),
            WorkflowsSaveScope::User
        );
    }

    #[test]
    fn workflow_cli_rejects_bad_scope() {
        let err = save_scope(&["--scope".to_string(), "team".to_string()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("user or workspace"), "{err}");
    }
}
