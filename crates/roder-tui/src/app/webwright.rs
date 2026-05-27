use roder_protocol::{JsonRpcRequest, WebwrightArtifactsResult, WebwrightWorkspaceParams};
use serde_json::Value;

use super::{AppClient, TuiApp, decode_response, truncate};

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_webwright_slash_command(&mut self, args: &str) {
        let parts = args.split_whitespace().collect::<Vec<_>>();
        match parts.as_slice() {
            ["inspect", workspace] | [workspace] => self.show_webwright_workspace(workspace).await,
            ["tail", workspace] => self.show_webwright_log_tail(workspace).await,
            _ => {
                self.timeline
                    .push_system("Usage: /webwright [inspect|tail] <workspace>");
                self.push_event("slash command: /webwright".to_string());
            }
        }
    }

    async fn show_webwright_workspace(&mut self, workspace: &str) {
        match self.webwright_artifacts(workspace).await {
            Ok(result) => {
                self.timeline
                    .push_system(render_webwright_progress(&result.workspace));
                self.push_event("slash command: /webwright inspect".to_string());
            }
            Err(err) => self.record_error(format!("webwright/artifacts failed: {err}")),
        }
    }

    async fn show_webwright_log_tail(&mut self, workspace: &str) {
        match self.webwright_artifacts(workspace).await {
            Ok(result) => {
                let Some(run) = latest_run(&result.workspace) else {
                    self.timeline
                        .push_system("Webwright workspace has no latest run.");
                    return;
                };
                let tail = lines_at(run, &["logTail"])
                    .or_else(|| {
                        run.pointer("/log/tail")
                            .and_then(Value::as_array)
                            .map(Vec::as_slice)
                    })
                    .map(render_string_lines)
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| "No retained Webwright log tail.".to_string());
                self.timeline
                    .push_system(format!("Webwright log tail:\n{tail}"));
                self.push_event("slash command: /webwright tail".to_string());
            }
            Err(err) => self.record_error(format!("webwright/artifacts failed: {err}")),
        }
    }

    async fn webwright_artifacts(
        &self,
        workspace: &str,
    ) -> anyhow::Result<WebwrightArtifactsResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("webwright/artifacts")),
                method: "webwright/artifacts".to_string(),
                params: Some(serde_json::to_value(WebwrightWorkspaceParams {
                    workspace: workspace.to_string(),
                    workspace_root: None,
                })?),
            })
            .await;
        decode_response(res)
    }
}

fn render_webwright_progress(workspace: &Value) -> String {
    let root = string_at(workspace, &["root"]).unwrap_or("-");
    let task_id = workspace
        .pointer("/manifest/taskId")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let mode = workspace
        .pointer("/manifest/mode")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let state = workspace
        .pointer("/manifest/verificationState")
        .and_then(Value::as_str)
        .unwrap_or("pending");
    let checked = workspace
        .pointer("/plan/checkedCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = workspace
        .pointer("/plan/totalCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut lines = vec![
        format!("Webwright {task_id} ({mode})"),
        format!("workspace: {}", truncate(root, 96)),
        format!("state: {state}"),
        format!("critical points: {checked}/{total}"),
    ];
    lines.extend(render_critical_points(workspace));
    if let Some(run) = latest_run(workspace) {
        lines.extend(render_latest_run(run));
    } else {
        lines.push("latest run: none".to_string());
    }
    let validation_errors = lines_at(workspace, &["validationErrors"])
        .map(render_string_lines)
        .filter(|text| !text.trim().is_empty());
    if let Some(errors) = validation_errors {
        lines.push("validation:".to_string());
        lines.push(errors);
    }
    lines.join("\n")
}

fn render_critical_points(workspace: &Value) -> Vec<String> {
    workspace
        .pointer("/plan/criticalPoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|point| {
            let mark = if point
                .get("checked")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "x"
            } else {
                " "
            };
            let text = point.get("text").and_then(Value::as_str).unwrap_or("");
            format!("- [{mark}] {}", truncate(text, 100))
        })
        .collect()
}

fn render_latest_run(run: &Value) -> Vec<String> {
    let run_id = run.get("runId").and_then(Value::as_u64).unwrap_or(0);
    let screenshots = lines_at(run, &["screenshots"]).unwrap_or(&[]);
    let final_datum = run
        .pointer("/log/finalDatum")
        .and_then(Value::as_str)
        .unwrap_or("missing");
    let mut lines = vec![
        format!("latest run: run_{run_id:03}"),
        format!("screenshots: {}", screenshots.len()),
    ];
    lines.extend(
        screenshots
            .iter()
            .filter_map(Value::as_str)
            .take(5)
            .map(|path| format!("- screenshot {}", truncate(path, 100))),
    );
    lines.push(format!("final datum: {}", truncate(final_datum, 120)));
    if let Some(tail) = lines_at(run, &["logTail"]) {
        lines.push("log tail:".to_string());
        lines.push(render_string_lines(tail));
    }
    lines
}

fn latest_run(workspace: &Value) -> Option<&Value> {
    let latest = workspace.get("latestRun").and_then(Value::as_u64)?;
    workspace
        .get("runs")
        .and_then(Value::as_array)?
        .iter()
        .find(|run| run.get("runId").and_then(Value::as_u64) == Some(latest))
}

fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))?
        .as_str()
}

fn lines_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a [Value]> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))?
        .as_array()
        .map(Vec::as_slice)
}

fn render_string_lines(lines: &[Value]) -> String {
    lines
        .iter()
        .filter_map(Value::as_str)
        .map(|line| truncate(line, 140))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_progress_with_critical_points_screenshots_and_final_datum() {
        let text = render_webwright_progress(&json!({
            "root": "/repo/.roder/webwright/fixture",
            "manifest": {
                "taskId": "fixture",
                "mode": "run",
                "verificationState": "success"
            },
            "plan": {
                "checkedCount": 1,
                "totalCount": 1,
                "criticalPoints": [{ "checked": true, "text": "Heading visible" }]
            },
            "latestRun": 1,
            "runs": [{
                "runId": 1,
                "screenshots": ["/repo/final_execution_001_ok.png"],
                "log": { "finalDatum": "final datum: Fixture Heading" },
                "logTail": ["step 1", "final datum: Fixture Heading"]
            }],
            "validationErrors": []
        }));

        assert!(text.contains("Webwright fixture (run)"));
        assert!(text.contains("critical points: 1/1"));
        assert!(text.contains("- [x] Heading visible"));
        assert!(text.contains("screenshots: 1"));
        assert!(text.contains("final datum: final datum: Fixture Heading"));
        assert!(text.contains("log tail:"));
    }
}
