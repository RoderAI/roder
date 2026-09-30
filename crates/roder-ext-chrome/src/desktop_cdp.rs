use std::sync::Arc;
use std::time::Duration;

use roder_api::tools::{ToolCall, ToolResult};
use serde_json::{Value, json};

use crate::desktop_scope::DesktopScope;
use crate::direct::{DirectGuard, DirectSession, DirectTab, TabClient, tool_result};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CdpTarget {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default)]
    #[allow(dead_code)]
    r#type: String,
    web_socket_debugger_url: String,
}

pub async fn execute(kind: &str, call: &ToolCall) -> Option<ToolResult> {
    if std::env::var_os("RODER_DESKTOP_CDP_DISABLE").is_some() {
        return None;
    }
    match kind {
        "tab/open" | "tab/navigate" => Some(direct(call, kind).await),
        "tabs/list" => Some(tabs_list(call).await),
        "page/snapshot" => Some(direct(call, kind).await),
        "page/screenshot" => Some(direct(call, kind).await),
        "page/eval" => Some(eval(call).await),
        "page/click" | "page/type" | "page/scroll" | "page/keypress" => {
            Some(direct(call, kind).await)
        }
        _ => None,
    }
}

async fn tabs_list(call: &ToolCall) -> ToolResult {
    match targets().await {
        Ok(targets) => {
            let data = crate::desktop_tabs::list(&call.thread_id, &targets);
            ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                text: format!(
                    "{}\n{}",
                    crate::session::UNTRUSTED_NOTE,
                    serde_json::to_string(&data).unwrap_or_default()
                ),
                data,
                is_error: false,
            }
        }
        Err(error) => error_result(call, error),
    }
}

/// Execute the canonical direct tools: real input, stable refs, and observed state.
async fn direct(call: &ToolCall, kind: &str) -> ToolResult {
    let tab = match desktop_tab(call).await {
        Ok(tab) => tab,
        Err(error) => return error_result(call, error),
    };
    let scope = match DesktopScope::from_env() {
        Ok(scope) => scope,
        Err(error) => return error_result(call, format!("{error:#}")),
    };
    let mut session = match DirectSession::attach(&tab, Arc::new(scope), false).await {
        Ok(session) => session,
        Err(error) => {
            return error_result(
                call,
                format!("connect to Desktop browser failed: {error:#}"),
            );
        }
    };
    let mut args = call.arguments.clone();
    let short = match kind {
        "tab/open" | "tab/navigate" => "navigate",
        "page/snapshot" => "look",
        "page/screenshot" => "screenshot",
        "page/click" => "click",
        "page/type" => "type",
        "page/scroll" => "scroll",
        "page/keypress" => "key",
        _ => return error_result(call, "Unsupported Desktop browser action"),
    };
    if matches!(short, "click" | "type" | "scroll") {
        let reference = args["ref"].as_str().filter(|r| !r.is_empty());
        let selector = args["selector"].as_str().unwrap_or_default();
        let label = if short == "click" {
            args["text"].as_str().unwrap_or_default()
        } else {
            ""
        };
        if reference.is_none() && (!selector.is_empty() || !label.is_empty()) {
            match session.resolve_ref(selector, label).await {
                Ok(reference) => args["ref"] = json!(reference),
                Err(error) => return error_result(call, format!("{error:#}")),
            }
        }
    }
    let step = session.run(short, &args).await;
    let mut result = tool_result(&call.id, &call.name, &step);
    result.data["fallback"] = json!("desktop-cdp");
    result
}

async fn eval(call: &ToolCall) -> ToolResult {
    let Some(expression) = call.arguments.get("expression").and_then(Value::as_str) else {
        return error_result(call, "chrome_eval requires expression");
    };
    let tab = match desktop_tab(call).await {
        Ok(tab) => tab,
        Err(error) => return error_result(call, error),
    };
    let result = async {
        let mut client = TabClient::attach(&tab).await?;
        let scope = DesktopScope::from_env()?;
        let url = client.evaluate_isolated("location.href").await?;
        if let Some(reason) = url.as_str().and_then(|url| scope.outside(url)) {
            anyhow::bail!(reason);
        }
        client.evaluate(expression).await
    }
    .await;
    match result {
        Ok(value) => {
            let data = crate::session::label_result(
                "page/eval",
                json!({"result": value, "browser": "roder-desktop"}),
            );
            ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                text: crate::session::result_text("page/eval", &data),
                data,
                is_error: false,
            }
        }
        Err(error) => error_result(call, format!("browser evaluation failed: {error:#}")),
    }
}

/// Respect the tab id returned by tabs/list; never silently act on another tab.
async fn desktop_tab(call: &ToolCall) -> Result<DirectTab, String> {
    let target = crate::desktop_tabs::target(
        &call.thread_id,
        call.arguments["tabId"].as_u64(),
        targets().await?,
    )?;
    Ok(DirectTab::Page {
        websocket: target.web_socket_debugger_url,
        target_id: target.id,
    })
}

async fn targets() -> Result<Vec<CdpTarget>, String> {
    let port = std::env::var("RODER_DESKTOP_CDP_PORT").unwrap_or_else(|_| "9334".to_string());
    let url = format!("http://127.0.0.1:{port}/json");
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?
        .get(&url)
        .send()
        .await
        .map_err(|error| {
            format!("Desktop integrated browser is not reachable at {url}: {error}")
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "Desktop integrated browser returned HTTP {} at {url}",
            response.status()
        ));
    }
    let mut listed = response
        .json::<Vec<CdpTarget>>()
        .await
        .map_err(|error| format!("invalid Desktop integrated browser target list: {error}"))?;
    listed.retain(|target| {
        target.r#type == "page"
            && !target.web_socket_debugger_url.is_empty()
            && !target.url.starts_with("devtools://")
            && !target.url.starts_with("chrome-extension://")
    });
    Ok(listed)
}

fn error_result(call: &ToolCall, message: impl Into<String>) -> ToolResult {
    let message = message.into();
    ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        text: message.clone(),
        data: json!({ "error": { "kind": "desktop-browser", "message": message } }),
        is_error: true,
    }
}
