use std::sync::Arc;

use roder_api::automations::{
    AutomationConcurrencyPolicy, AutomationProject, AutomationRunState, AutomationSchedule,
    CatchUpPolicy,
};
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    AutomationsCancelRunParams, AutomationsCreateParams, AutomationsDeleteParams,
    AutomationsListParams, AutomationsRunNowParams, AutomationsRunsParams, AutomationsUpdateParams,
    AutomationsUpdatePatch, JsonRpcRequest,
};

use crate::{CliOptions, build_runtime_from_config};

pub async fn run_automations_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    let value = match args.first().map(String::as_str) {
        Some("status") => request(&client, "automations/status", None).await?,
        Some("list") => {
            request(
                &client,
                "automations/list",
                Some(serde_json::to_value(AutomationsListParams::default())?),
            )
            .await?
        }
        Some("create") => {
            let params = parse_create(&args[1..])?;
            request(
                &client,
                "automations/create",
                Some(serde_json::to_value(params)?),
            )
            .await?
        }
        Some("enable") | Some("disable") => {
            let Some(id) = args.get(1) else {
                anyhow::bail!("automations {} requires an automation id", args[0]);
            };
            let params = AutomationsUpdateParams {
                automation_id: id.clone(),
                patch: AutomationsUpdatePatch {
                    enabled: Some(args[0] == "enable"),
                    ..AutomationsUpdatePatch::default()
                },
            };
            request(
                &client,
                "automations/update",
                Some(serde_json::to_value(params)?),
            )
            .await?
        }
        Some("run-now") => {
            let Some(id) = args.get(1) else {
                anyhow::bail!("automations run-now requires an automation id");
            };
            request(
                &client,
                "automations/runNow",
                Some(serde_json::to_value(AutomationsRunNowParams {
                    automation_id: id.clone(),
                    prompt_override: None,
                })?),
            )
            .await?
        }
        Some("runs") => {
            let Some(id) = args.get(1) else {
                anyhow::bail!("automations runs requires an automation id");
            };
            request(
                &client,
                "automations/runs",
                Some(serde_json::to_value(AutomationsRunsParams {
                    automation_id: id.clone(),
                    state: args
                        .iter()
                        .position(|arg| arg == "--state")
                        .and_then(|idx| args.get(idx + 1))
                        .map(|state| parse_run_state(state))
                        .transpose()?,
                    limit: None,
                })?),
            )
            .await?
        }
        Some("delete") => {
            let Some(id) = args.get(1) else {
                anyhow::bail!("automations delete requires an automation id");
            };
            request(
                &client,
                "automations/delete",
                Some(serde_json::to_value(AutomationsDeleteParams {
                    automation_id: id.clone(),
                })?),
            )
            .await?
        }
        Some("cancel-run") => {
            let Some(id) = args.get(1) else {
                anyhow::bail!("automations cancel-run requires a run id");
            };
            request(
                &client,
                "automations/cancelRun",
                Some(serde_json::to_value(AutomationsCancelRunParams {
                    run_id: id.clone(),
                    reason: Some("cancelled from CLI".to_string()),
                })?),
            )
            .await?
        }
        _ => {
            anyhow::bail!(
                "usage: roder automations <status|list|create|enable|disable|run-now|runs|delete|cancel-run>"
            );
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn parse_create(args: &[String]) -> anyhow::Result<AutomationsCreateParams> {
    let name = flag(args, "--name")?;
    let cwd = flag(args, "--cwd")?;
    let prompt = flag(args, "--prompt")?;
    let interval = flag(args, "--interval-seconds")?.parse::<u64>()?;
    Ok(AutomationsCreateParams {
        name,
        project: AutomationProject {
            cwd,
            display_name: None,
        },
        schedule: AutomationSchedule::Interval { seconds: interval },
        prompt,
        enabled: true,
        model_provider: None,
        model: None,
        policy_mode: None,
        catch_up: CatchUpPolicy::RunLatestOnly,
        concurrency: AutomationConcurrencyPolicy::Forbid,
    })
}

fn flag(args: &[String], name: &str) -> anyhow::Result<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{name} requires a value"))
}

fn parse_run_state(value: &str) -> anyhow::Result<AutomationRunState> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(Into::into)
}

async fn request(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> anyhow::Result<serde_json::Value> {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: method.to_string(),
            params,
        })
        .await;
    if let Some(error) = response.error {
        anyhow::bail!("{}: {}", error.code, error.message);
    }
    response
        .result
        .ok_or_else(|| anyhow::anyhow!("missing result for {method}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automations_create_cli_parses_interval_shape() {
        let params = parse_create(&[
            "--name".to_string(),
            "Hourly".to_string(),
            "--cwd".to_string(),
            "/tmp/project".to_string(),
            "--prompt".to_string(),
            "summarize".to_string(),
            "--interval-seconds".to_string(),
            "3600".to_string(),
        ])
        .unwrap();

        assert_eq!(params.name, "Hourly");
        assert_eq!(params.project.cwd, "/tmp/project");
        assert_eq!(
            params.schedule,
            AutomationSchedule::Interval { seconds: 3600 }
        );
    }
}
