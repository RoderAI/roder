use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use roder_api::tools::{ToolCall, ToolResult};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpTarget {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
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
        "tab/open" | "tab/navigate" => Some(navigate(call).await),
        "tabs/list" => Some(tabs_list(call).await),
        "page/snapshot" => Some(page_snapshot(call).await),
        "page/eval" => Some(eval(call).await),
        "page/click" => Some(page_action(call, click_script(call)).await),
        "page/type" => Some(page_action(call, type_script(call)).await),
        "page/scroll" => Some(page_action(call, scroll_script(call)).await),
        "page/keypress" => Some(page_action(call, keypress_script(call)).await),
        _ => None,
    }
}

async fn navigate(call: &ToolCall) -> ToolResult {
    let Some(url) = call
        .arguments
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
    else {
        return error_result(call, "chrome_tab_open requires a url");
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return error_result(
            call,
            "Desktop integrated browser only supports http(s) URLs",
        );
    }
    match cdp_call("Page.navigate", json!({ "url": url })).await {
        Ok(_) => ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: format!("opened {url}"),
            data: json!({ "url": url, "opened": true, "fallback": "desktop-cdp" }),
            is_error: false,
        },
        Err(error) => error_result(call, error),
    }
}

async fn tabs_list(call: &ToolCall) -> ToolResult {
    match targets().await {
        Ok(targets) => {
            let active_id = targets.first().map(|target| target.id.clone());
            ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                text: format!("{} integrated browser target(s)", targets.len()),
                data: json!({
                    "tabs": targets.into_iter().enumerate().map(|(index, target)| json!({
                        "id": index,
                        "targetId": target.id,
                        "title": target.title,
                        "url": target.url,
                        "active": Some(&target.id) == active_id.as_ref(),
                        "browser": "roder-desktop"
                    })).collect::<Vec<_>>()
                }),
                is_error: false,
            }
        }
        Err(error) => error_result(call, error),
    }
}

async fn page_snapshot(call: &ToolCall) -> ToolResult {
    let script = r#"(() => {
  const controls = Array.from(document.querySelectorAll('a,button,input,textarea,select,[role],[aria-label],[contenteditable="true"]'))
    .filter((el) => {
      const rect = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
    })
    .slice(0, 200)
    .map((el, index) => {
      const rect = el.getBoundingClientRect();
      const tag = el.tagName.toLowerCase();
      const id = el.id ? `#${CSS.escape(el.id)}` : '';
      const name = el.getAttribute('name') ? `[name="${CSS.escape(el.getAttribute('name'))}"]` : '';
      const selector = id || `${tag}${name}`;
      return {
        ref: `c${index}`,
        tag,
        text: (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().slice(0, 500),
        ariaLabel: el.getAttribute('aria-label'),
        role: el.getAttribute('role'),
        type: el.getAttribute('type'),
        selector,
        box: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
      };
    });
  return { visible: true, url: location.href, title: document.title, text: document.body?.innerText || '', controls, untrusted: true, browser: 'roder-desktop' };
})()"#;
    match runtime_eval(script).await {
        Ok(value) => ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: "chrome page/snapshot ok".to_string(),
            data: value,
            is_error: false,
        },
        Err(error) => error_result(call, error),
    }
}

async fn eval(call: &ToolCall) -> ToolResult {
    let Some(expression) = call.arguments.get("expression").and_then(Value::as_str) else {
        return error_result(call, "chrome_eval requires expression");
    };
    match runtime_eval(expression).await {
        Ok(value) => ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: "chrome page/eval ok".to_string(),
            data: json!({ "result": value, "untrusted": true, "browser": "roder-desktop" }),
            is_error: false,
        },
        Err(error) => error_result(call, error),
    }
}

async fn page_action(call: &ToolCall, script: Result<String, String>) -> ToolResult {
    let script = match script {
        Ok(script) => script,
        Err(error) => return error_result(call, error),
    };
    match runtime_eval(&script).await {
        Ok(value) => ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            text: "chrome page action ok".to_string(),
            data: json!({ "result": value, "browser": "roder-desktop" }),
            is_error: false,
        },
        Err(error) => error_result(call, error),
    }
}

async fn runtime_eval(expression: &str) -> Result<Value, String> {
    let value = cdp_call(
        "Runtime.evaluate",
        json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
    )
    .await?;
    if let Some(exception) = value.get("exceptionDetails") {
        return Err(format!("browser evaluation failed: {exception}"));
    }
    Ok(value
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

async fn cdp_call(method: &str, params: Value) -> Result<Value, String> {
    let target = active_target().await?;
    let request_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (mut ws, _) = connect_async(&target.web_socket_debugger_url)
        .await
        .map_err(|error| format!("connect to Desktop integrated browser failed: {error}"))?;
    let request = json!({ "id": request_id, "method": method, "params": params });
    ws.send(Message::Text(request.to_string().into()))
        .await
        .map_err(|error| format!("send browser command failed: {error}"))?;
    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            _ = &mut timeout => return Err("Desktop integrated browser did not respond in time".to_string()),
            message = ws.next() => {
                let Some(message) = message else { return Err("Desktop integrated browser connection closed".to_string()); };
                let message = message.map_err(|error| format!("read browser response failed: {error}"))?;
                let Message::Text(text) = message else { continue; };
                let value: Value = serde_json::from_str(&text).map_err(|error| format!("invalid browser response: {error}"))?;
                if value.get("id").and_then(Value::as_u64) != Some(request_id) {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    return Err(format!("browser command failed: {error}"));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}

async fn active_target() -> Result<CdpTarget, String> {
    let mut targets = targets().await?;
    targets
        .drain(..)
        .find(|target| {
            !target.web_socket_debugger_url.is_empty()
                && !target.url.starts_with("devtools://")
                && !target.url.starts_with("chrome-extension://")
        })
        .ok_or_else(|| "Desktop integrated browser target is not available".to_string())
}

async fn targets() -> Result<Vec<CdpTarget>, String> {
    let port = std::env::var("RODER_DESKTOP_CDP_PORT").unwrap_or_else(|_| "9334".to_string());
    let url = format!("http://127.0.0.1:{port}/json");
    let response = reqwest::get(&url).await.map_err(|error| {
        format!("Desktop integrated browser is not reachable at {url}: {error}")
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "Desktop integrated browser returned HTTP {} at {url}",
            response.status()
        ));
    }
    response
        .json::<Vec<CdpTarget>>()
        .await
        .map_err(|error| format!("invalid Desktop integrated browser target list: {error}"))
}

fn click_script(call: &ToolCall) -> Result<String, String> {
    let target = target_expression(call)?;
    Ok(format!(
        "(() => {{ const el = {target}; if (!el) return false; el.click(); return true; }})()"
    ))
}

fn type_script(call: &ToolCall) -> Result<String, String> {
    let text = call
        .arguments
        .get("text")
        .and_then(Value::as_str)
        .ok_or("chrome_type requires text")?;
    let submit = call
        .arguments
        .get("submit")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let target = target_expression(call)?;
    Ok(format!(
        "(() => {{ const el = {target}; if (!el) return false; el.focus(); el.value = {}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }})); if ({submit}) {{ const form = el.form || el.closest('form'); if (form) form.requestSubmit ? form.requestSubmit() : form.submit(); }} return true; }})()",
        json!(text)
    ))
}

fn scroll_script(call: &ToolCall) -> Result<String, String> {
    let dx = call
        .arguments
        .get("dx")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let dy = call
        .arguments
        .get("dy")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if let Some(selector) = call.arguments.get("selector").and_then(Value::as_str) {
        Ok(format!(
            "(() => {{ const el = document.querySelector({}); if (!el) return false; el.scrollBy({}, {}); return true; }})()",
            json!(selector),
            dx,
            dy
        ))
    } else {
        Ok(format!(
            "(() => {{ window.scrollBy({}, {}); return true; }})()",
            dx, dy
        ))
    }
}

fn keypress_script(call: &ToolCall) -> Result<String, String> {
    let key = call
        .arguments
        .get("key")
        .and_then(Value::as_str)
        .ok_or("chrome_keypress requires key")?;
    Ok(format!(
        "(() => {{ const event = new KeyboardEvent('keydown', {{ key: {}, bubbles: true }}); (document.activeElement || document.body).dispatchEvent(event); return true; }})()",
        json!(key)
    ))
}

fn target_expression(call: &ToolCall) -> Result<String, String> {
    if let Some(selector) = call.arguments.get("selector").and_then(Value::as_str) {
        return Ok(format!("document.querySelector({})", json!(selector)));
    }
    if let Some(text) = call.arguments.get("text").and_then(Value::as_str) {
        return Ok(format!(
            "Array.from(document.querySelectorAll('a,button,input,textarea,select,[role],[aria-label],[contenteditable=\\\"true\\\"]')).find((el) => ((el.innerText || el.value || el.getAttribute('aria-label') || '').trim()).includes({}))",
            json!(text)
        ));
    }
    if let Some(reference) = call.arguments.get("ref").and_then(Value::as_str) {
        let index = reference.strip_prefix('c').and_then(|value| value.parse::<usize>().ok()).ok_or("Desktop integrated browser refs must come from chrome_page_snapshot (for example c0)")?;
        return Ok(format!(
            "Array.from(document.querySelectorAll('a,button,input,textarea,select,[role],[aria-label],[contenteditable=\\\"true\\\"]')).filter((el) => {{ const rect = el.getBoundingClientRect(); const style = getComputedStyle(el); return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none'; }})[{index}]"
        ));
    }
    Err("browser action requires selector, text, or ref".to_string())
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
