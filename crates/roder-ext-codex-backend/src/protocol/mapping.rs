use roder_api::backend::{
    BackendEvent, BackendHistoryItem, BackendHistoryTurn, BackendThreadSnapshot,
};
use roder_api::policy_mode::PolicyMode;
use serde_json::Value;

pub(super) fn codex_policy(mode: PolicyMode) -> (&'static str, &'static str) {
    match mode {
        PolicyMode::Default => ("on-request", "workspaceWrite"),
        PolicyMode::AcceptAll => ("never", "workspaceWrite"),
        PolicyMode::Plan => ("on-request", "readOnly"),
        PolicyMode::Bypass => ("never", "dangerFullAccess"),
    }
}

pub(super) fn thread_sandbox(policy: &str) -> &'static str {
    match policy {
        "readOnly" => "read-only",
        "workspaceWrite" => "workspace-write",
        _ => "danger-full-access",
    }
}

pub(super) fn string_at(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

pub(super) fn approval_description(method: &str, params: &Value) -> String {
    let mut lines = vec![
        match method {
            "item/commandExecution/requestApproval" => "Codex requests command approval:",
            "item/permissions/requestApproval" => "Codex requests additional permissions:",
            _ => "Codex requests file-change approval:",
        }
        .to_owned(),
    ];
    if let Some(network) = params.get("networkApprovalContext") {
        let host = network
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or("unknown host");
        let protocol = network
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or("network");
        lines.push(format!("Network: {protocol} {host}"));
    }
    for key in ["command", "cwd", "reason", "grantRoot"] {
        if let Some(value) = params.get(key).and_then(Value::as_str) {
            lines.push(format!("{key}: {value}"));
        }
    }
    if let Some(permissions) = params.get("permissions") {
        lines.push(format!("permissions: {permissions}"));
    }
    lines.join("\n")
}

pub(super) fn tool_event(item: &Value, completed: bool) -> Option<BackendEvent> {
    let kind = item.get("type")?.as_str()?;
    let id = string_at(item, "id");
    let label = match kind {
        "commandExecution" => item
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("Command"),
        "fileChange" => "File changes",
        "mcpToolCall" => item
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("MCP tool"),
        "webSearch" => "Web search",
        "collabAgentToolCall" => "Agent",
        _ => return None,
    };
    if completed {
        Some(BackendEvent::ToolFinished {
            id,
            label: label.to_owned(),
            summary: format!(
                "{label}: {}",
                item.get("status").and_then(Value::as_str).unwrap_or("done")
            ),
        })
    } else {
        Some(BackendEvent::ToolStarted {
            id,
            label: label.to_owned(),
        })
    }
}

pub(super) fn snapshot_from_thread(thread: &Value) -> BackendThreadSnapshot {
    BackendThreadSnapshot {
        preview: string_at(thread, "preview"),
        cwd: string_at(thread, "cwd"),
        created_at: thread["createdAt"].as_i64().unwrap_or_default(),
        updated_at: thread["updatedAt"].as_i64().unwrap_or_default(),
        turns: thread["turns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|turn| {
                let id = turn["id"].as_str()?.to_owned();
                let items = turn["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(history_item)
                    .collect();
                Some(BackendHistoryTurn {
                    id,
                    status: string_at(turn, "status"),
                    started_at: turn["startedAt"].as_i64(),
                    completed_at: turn["completedAt"].as_i64(),
                    items,
                })
            })
            .collect(),
    }
}

fn history_item(item: &Value) -> Option<BackendHistoryItem> {
    let id = item["id"].as_str()?.to_owned();
    Some(match item["type"].as_str()? {
        "userMessage" => BackendHistoryItem::User {
            id,
            text: item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|content| content["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        },
        "agentMessage" => BackendHistoryItem::Agent {
            id,
            text: string_at(item, "text"),
        },
        "commandExecution" | "fileChange" | "mcpToolCall" | "webSearch" | "collabAgentToolCall" => {
            BackendHistoryItem::Tool {
                id,
                name: item["command"]
                    .as_str()
                    .or_else(|| item["tool"].as_str())
                    .unwrap_or_else(|| item["type"].as_str().unwrap_or("Tool"))
                    .to_owned(),
                output: item["aggregatedOutput"]
                    .as_str()
                    .or_else(|| item["output"].as_str())
                    .map(str::to_owned),
                status: string_at(item, "status"),
            }
        }
        _ => BackendHistoryItem::Other {
            id,
            payload: item.clone(),
        },
    })
}
