use std::sync::Arc;

use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{JsonRpcRequest, TasksCancelParams, TasksGetParams, TasksSubmitParams};

use crate::{CliOptions, build_runtime_from_config};

pub(crate) async fn run_tasks_cli(args: &[String], options: CliOptions) -> anyhow::Result<()> {
    let (runtime, _default_model, _tui_config) = build_runtime_from_config(options).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    let request = tasks_request(args)?;
    let response = client.send_request(request).await;
    if let Some(error) = response.error {
        anyhow::bail!("{}", error.message);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&response.result.unwrap_or_else(|| serde_json::json!({})))?
    );
    Ok(())
}

fn tasks_request(args: &[String]) -> anyhow::Result<JsonRpcRequest> {
    let Some(command) = args.first().map(String::as_str) else {
        anyhow::bail!("tasks requires a subcommand: list, show, cancel, or submit");
    };
    let (method, params) = match command {
        "list" => ("tasks/list", None),
        "show" => {
            let Some(task_id) = args.get(1) else {
                anyhow::bail!("tasks show requires a task id");
            };
            (
                "tasks/get",
                Some(serde_json::to_value(TasksGetParams {
                    task_id: task_id.clone(),
                })?),
            )
        }
        "cancel" => {
            let Some(task_id) = args.get(1) else {
                anyhow::bail!("tasks cancel requires a task id");
            };
            (
                "tasks/cancel",
                Some(serde_json::to_value(TasksCancelParams {
                    task_id: task_id.clone(),
                    reason: Some("cli".to_string()),
                })?),
            )
        }
        "submit" => {
            let Some(executor_id) = args.get(1) else {
                anyhow::bail!("tasks submit requires an executor id");
            };
            let input = args
                .get(2)
                .map(|raw| serde_json::from_str(raw))
                .transpose()?
                .unwrap_or_else(|| serde_json::json!({}));
            (
                "tasks/submit",
                Some(serde_json::to_value(TasksSubmitParams {
                    executor_id: executor_id.clone(),
                    input,
                    thread_id: None,
                    turn_id: None,
                })?),
            )
        }
        other => anyhow::bail!("unknown tasks subcommand {other:?}"),
    };
    Ok(JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(method)),
        method: method.to_string(),
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn maps_list_to_tasks_list() {
        let request = tasks_request(&args(&["list"])).unwrap();

        assert_eq!(request.method, "tasks/list");
        assert!(request.params.is_none());
    }

    #[test]
    fn maps_show_and_cancel_to_task_id_requests() {
        let show = tasks_request(&args(&["show", "task-1"])).unwrap();
        assert_eq!(show.method, "tasks/get");
        assert_eq!(show.params.unwrap()["task_id"], "task-1");

        let cancel = tasks_request(&args(&["cancel", "task-2"])).unwrap();
        assert_eq!(cancel.method, "tasks/cancel");
        let params = cancel.params.unwrap();
        assert_eq!(params["task_id"], "task-2");
        assert_eq!(params["reason"], "cli");
    }

    #[test]
    fn maps_submit_with_json_input() {
        let request = tasks_request(&args(&[
            "submit",
            "process",
            r#"{"command":"echo","args":["ok"]}"#,
        ]))
        .unwrap();

        assert_eq!(request.method, "tasks/submit");
        let params = request.params.unwrap();
        assert_eq!(params["executor_id"], "process");
        assert_eq!(params["input"]["command"], "echo");
        assert_eq!(params["input"]["args"][0], "ok");
    }

    #[test]
    fn rejects_missing_or_unknown_subcommands() {
        assert!(tasks_request(&args(&[])).is_err());
        assert!(tasks_request(&args(&["show"])).is_err());
        assert!(tasks_request(&args(&["wat"])).is_err());
    }
}
