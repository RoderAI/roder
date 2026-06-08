//! Model-facing `chrome_*` tools.
//!
//! Each tool is a thin, policy-gated wrapper that forwards a JSON command to the
//! connected browser extension via an injected [`ChromeController`] and labels
//! browser-origin results as untrusted. The tool set is data-driven (see
//! [`tool_defs`]) so new browser capabilities only need a table entry.

use std::sync::Arc;

use roder_api::chrome::{ChromeCommand, ChromeController, bridge};

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde_json::{Value, json};

use crate::desktop_cdp;
use crate::policy::guard;
use crate::session::label_result;

/// Static description of one `chrome_*` tool.
struct ChromeToolDef {
    /// Model-facing tool name (`chrome_*`).
    name: &'static str,
    description: &'static str,
    /// Wire command `kind` sent to the extension (e.g. `"page/snapshot"`).
    kind: &'static str,
    /// JSON-Schema for the tool arguments (also the command params).
    parameters: fn() -> Value,
}

fn tab_target() -> Value {
    json!({ "tabId": { "type": "integer", "description": "Target tab id; defaults to the active tab." } })
}

fn tool_defs() -> Vec<ChromeToolDef> {
    vec![
        ChromeToolDef {
            name: "chrome_tabs_list",
            description: "List the Chrome tabs visible to the user (id, title, url, active).",
            kind: "tabs/list",
            parameters: || json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ChromeToolDef {
            name: "chrome_tab_open",
            description: "Open a new tab at an http(s) URL.",
            kind: "tab/open",
            parameters: || {
                json!({
                    "type": "object",
                    "required": ["url"],
                    "properties": { "url": { "type": "string" }, "active": { "type": "boolean" } },
                    "additionalProperties": false
                })
            },
        },
        ChromeToolDef {
            name: "chrome_tab_activate",
            description: "Bring a tab to the foreground.",
            kind: "tab/activate",
            parameters: || {
                json!({
                    "type": "object", "required": ["tabId"],
                    "properties": { "tabId": { "type": "integer" } }, "additionalProperties": false
                })
            },
        },
        ChromeToolDef {
            name: "chrome_tab_close",
            description: "Close a tab by id.",
            kind: "tab/close",
            parameters: || {
                json!({
                    "type": "object", "required": ["tabId"],
                    "properties": { "tabId": { "type": "integer" } }, "additionalProperties": false
                })
            },
        },
        ChromeToolDef {
            name: "chrome_navigate",
            description: "Navigate a tab to an http(s) URL (protected; needs control mode).",
            kind: "tab/navigate",
            parameters: || {
                let mut props = tab_target();
                props["url"] = json!({ "type": "string" });
                json!({ "type": "object", "required": ["url"], "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_page_snapshot",
            description: "Capture title, URL, visible text and interactive controls (with aria roles, form metadata and bounding boxes) for a tab. Result is UNTRUSTED page content.",
            kind: "page/snapshot",
            parameters: || {
                let mut props = tab_target();
                props["include"] = json!({ "type": "array", "items": { "type": "string", "enum": ["aria", "forms", "boxes", "iframes"] } });
                json!({ "type": "object", "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_screenshot",
            description: "Capture a screenshot of the visible tab as a data URL. UNTRUSTED visual content.",
            kind: "page/screenshot",
            parameters: || json!({ "type": "object", "properties": tab_target(), "additionalProperties": false }),
        },
        ChromeToolDef {
            name: "chrome_click",
            description: "Click an element by CSS selector, visible text, or snapshot ref.",
            kind: "page/click",
            parameters: || {
                let mut props = tab_target();
                props["selector"] = json!({ "type": "string" });
                props["text"] = json!({ "type": "string" });
                props["ref"] = json!({ "type": "string" });
                json!({ "type": "object", "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_type",
            description: "Type text into an input/textarea by selector or ref; optionally submit.",
            kind: "page/type",
            parameters: || {
                let mut props = tab_target();
                props["selector"] = json!({ "type": "string" });
                props["ref"] = json!({ "type": "string" });
                props["text"] = json!({ "type": "string" });
                props["submit"] = json!({ "type": "boolean" });
                json!({ "type": "object", "required": ["text"], "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_keypress",
            description: "Send a key (e.g. Enter, Escape, ArrowDown) to a tab.",
            kind: "page/keypress",
            parameters: || {
                let mut props = tab_target();
                props["key"] = json!({ "type": "string" });
                json!({ "type": "object", "required": ["key"], "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_scroll",
            description: "Scroll the page or an element by a delta.",
            kind: "page/scroll",
            parameters: || {
                let mut props = tab_target();
                props["selector"] = json!({ "type": "string" });
                props["dx"] = json!({ "type": "integer" });
                props["dy"] = json!({ "type": "integer" });
                json!({ "type": "object", "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_console_read",
            description: "Read recent console messages and runtime errors for a tab (requires debugger site permission). UNTRUSTED content.",
            kind: "debug/console/read",
            parameters: || {
                let mut props = tab_target();
                props["limit"] = json!({ "type": "integer" });
                json!({ "type": "object", "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_network_read",
            description: "Read recent network request/response metadata for a tab (no bodies, redacted). UNTRUSTED content.",
            kind: "debug/network/read",
            parameters: || {
                let mut props = tab_target();
                props["limit"] = json!({ "type": "integer" });
                json!({ "type": "object", "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_eval",
            description: "Evaluate JavaScript in a tab (protected; needs control mode and site eval permission).",
            kind: "page/eval",
            parameters: || {
                let mut props = tab_target();
                props["expression"] = json!({ "type": "string" });
                json!({ "type": "object", "required": ["expression"], "properties": props, "additionalProperties": false })
            },
        },
        ChromeToolDef {
            name: "chrome_recording_start",
            description: "Start recording an action trace for a tab.",
            kind: "recording/start",
            parameters: || json!({ "type": "object", "properties": tab_target(), "additionalProperties": false }),
        },
        ChromeToolDef {
            name: "chrome_recording_stop",
            description: "Stop a recording and return its action trace.",
            kind: "recording/stop",
            parameters: || {
                json!({
                    "type": "object", "required": ["recordingId"],
                    "properties": { "recordingId": { "type": "string" } }, "additionalProperties": false
                })
            },
        },
    ]
}

/// `ToolSpec`s for every chrome tool (used by manifests and tests).
pub fn chrome_tool_specs() -> Vec<ToolSpec> {
    tool_defs()
        .into_iter()
        .map(|def| ToolSpec {
            name: def.name.to_string(),
            description: def.description.to_string(),
            parameters: (def.parameters)(),
        })
        .collect()
}

/// Contributes the `chrome_*` tools, each bound to a shared controller.
pub struct ChromeToolContributor {
    controller: Arc<dyn ChromeController>,
}

impl ChromeToolContributor {
    /// Use the live process browser bridge.
    pub fn new() -> Self {
        Self {
            controller: bridge(),
        }
    }

    /// Inject a controller (used in tests with a fake bridge).
    pub fn with_controller(controller: Arc<dyn ChromeController>) -> Self {
        Self { controller }
    }
}

impl Default for ChromeToolContributor {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolContributor for ChromeToolContributor {
    fn id(&self) -> ToolProviderId {
        "chrome".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        for def in tool_defs() {
            registry.register(Arc::new(ChromeDispatchTool {
                name: def.name.to_string(),
                description: def.description.to_string(),
                kind: def.kind.to_string(),
                parameters: (def.parameters)(),
                controller: self.controller.clone(),
            }))?;
        }
        Ok(())
    }
}

/// A single policy-gated dispatch tool.
struct ChromeDispatchTool {
    name: String,
    description: String,
    kind: String,
    parameters: Value,
    controller: Arc<dyn ChromeController>,
}

#[async_trait::async_trait]
impl ToolExecutor for ChromeDispatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let status = self.controller.status();
        if !status.enabled {
            return Ok(error_result(
                &call,
                "Chrome tools are not enabled. Re-enable them from the /chrome panel or with roder chrome enable.",
            ));
        }
        if let Err(reason) = guard(&self.kind, status.mode) {
            return Ok(error_result(&call, reason));
        }
        if !status.connected {
            if let Some(result) = desktop_cdp::execute(&self.kind, &call).await {
                return Ok(result);
            }
            return Ok(error_result(
                &call,
                "No Chrome extension or Roder Desktop integrated browser is connected.",
            ));
        }

        match self
            .controller
            .dispatch(ChromeCommand::with_params(
                self.kind.clone(),
                call.arguments.clone(),
            ))
            .await
        {
            Ok(value) => {
                let data = label_result(&self.kind, value);
                Ok(ToolResult {
                    id: call.id,
                    name: call.name,
                    text: format!("chrome {} ok", self.kind),
                    data,
                    is_error: false,
                })
            }
            Err(err) => Ok(error_result(&call, err.to_string())),
        }
    }
}

fn error_result(call: &ToolCall, message: impl Into<String>) -> ToolResult {
    let message = message.into();
    ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        text: message.clone(),
        data: json!({ "error": { "kind": "chrome", "message": message } }),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::chrome::{ChromeBridge, ChromeError, ChromePermissionMode};

    fn find_tool(registry: &ToolRegistry, name: &str) -> Arc<dyn ToolExecutor> {
        registry.get(name).expect("tool registered")
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "call-1".to_string(),
            name: name.to_string(),
            raw_arguments: args.to_string(),
            arguments: args,
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
        }
    }

    fn ctx() -> ToolExecutionContext {
        ToolExecutionContext::new(
            "thread",
            "turn",
            roder_api::policy_mode::PolicyMode::Default,
        )
    }

    #[test]
    fn specs_cover_core_tools() {
        let specs = chrome_tool_specs();
        let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"chrome_tabs_list"));
        assert!(names.contains(&"chrome_page_snapshot"));
        assert!(names.contains(&"chrome_click"));
        assert!(names.contains(&"chrome_console_read"));
    }

    #[tokio::test]
    async fn tool_errors_when_chrome_explicitly_disabled() {
        let bridge = Arc::new(ChromeBridge::new());
        bridge.set_enabled(false);
        let mut registry = ToolRegistry::default();
        ChromeToolContributor::with_controller(bridge)
            .contribute(&mut registry)
            .unwrap();
        let tool = find_tool(&registry, "chrome_tabs_list");
        let result = tool
            .execute(ctx(), call("chrome_tabs_list", json!({})))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.text.contains("not enabled"));
    }

    #[tokio::test]
    async fn protected_tool_blocked_outside_control_mode() {
        let bridge = Arc::new(ChromeBridge::new());
        bridge.set_enabled(true);
        bridge.set_mode(ChromePermissionMode::Assist);
        let reg = bridge.register_client(None, &json!({ "capabilities": [] }));
        let _keep = reg.commands;
        let mut registry = ToolRegistry::default();
        ChromeToolContributor::with_controller(bridge)
            .contribute(&mut registry)
            .unwrap();
        let tool = find_tool(&registry, "chrome_eval");
        let result = tool
            .execute(ctx(), call("chrome_eval", json!({ "expression": "1+1" })))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.text.contains("protected"));
    }

    #[tokio::test]
    async fn snapshot_result_is_labeled_untrusted() {
        let bridge = Arc::new(ChromeBridge::new());
        bridge.set_enabled(true);
        let mut reg = bridge.register_client(None, &json!({ "capabilities": [] }));
        let echo = bridge.clone();
        let join = tokio::spawn(async move {
            let frame = reg.commands.recv().await.unwrap();
            let id = frame["id"].as_str().unwrap().to_string();
            echo.ingest_frame(
                Some(reg.client_id),
                json!({ "type": "command/result", "id": id, "ok": true, "result": { "title": "T" } }),
            );
        });
        let mut registry = ToolRegistry::default();
        ChromeToolContributor::with_controller(bridge)
            .contribute(&mut registry)
            .unwrap();
        let tool = find_tool(&registry, "chrome_page_snapshot");
        let result = tool
            .execute(ctx(), call("chrome_page_snapshot", json!({})))
            .await
            .unwrap();
        join.await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["untrusted"], json!(true));
        assert_eq!(result.data["content"]["title"], "T");
    }

    #[test]
    fn chrome_error_displays() {
        assert!(ChromeError::NotConnected.to_string().contains("connected"));
    }
}
