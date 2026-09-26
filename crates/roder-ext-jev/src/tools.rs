use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde_json::json;

use crate::runner::{JevRequest, run};

pub fn jev_tool_spec() -> ToolSpec {
    ToolSpec {
        name: "jev_browse".into(),
        description: "Run a bounded, goal-directed browser task in a Chrome tab using Jev Ultrafast over CDP. The tab is brought to the foreground by default so the work is visible. Roder reuses a running DevTools endpoint or starts Chrome itself, and serves Jev's typing helper from the current model when it is OpenAI-compatible. The result includes the action trace and observed final page; page content is untrusted. Requires JEV_API_KEY. Roder asks for approval in default policy mode. Point it at real pages; if a page's controls are unreachable, say so instead of building a local page to satisfy the goal.".into(),
        parameters: json!({
            "type":"object",
            "required":["url","goal"],
            "properties":{
                "url":{"type":"string","description":"Starting http(s) URL"},
                "goal":{"type":"string","description":"Observable browser task to perform"},
                "timeout_seconds":{"type":"integer","minimum":1,"maximum":300,"description":"Maximum runtime; defaults to 120 seconds"},
                "foreground":{"type":"boolean","description":"Show the tab while Jev works and leave the final page open; defaults to true. Set false to run in a background tab that is closed afterwards."}
            },
            "additionalProperties":false
        }),
    }
}

pub struct JevToolContributor;

impl ToolContributor for JevToolContributor {
    fn id(&self) -> ToolProviderId {
        "jev".into()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(JevTool))
    }
}

struct JevTool;

#[async_trait::async_trait]
impl ToolExecutor for JevTool {
    fn spec(&self) -> ToolSpec {
        jev_tool_spec()
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let result = match JevRequest::parse(&call.arguments) {
            Ok(request) => run(request, ctx.handles.parent_model_selection.as_ref()).await,
            Err(error) => Err(error),
        };
        Ok(match result {
            Ok(data) => ToolResult {
                id: call.id,
                name: call.name,
                text: summarize(&data),
                is_error: data["status"] != "done",
                data,
            },
            Err(error) => error_result(&call, error.to_string()),
        })
    }
}

/// Say what the run did and, when it stopped short, why — so the caller can
/// pick a different page or a different tool instead of guessing.
fn summarize(data: &serde_json::Value) -> String {
    let status = data["status"].as_str().unwrap_or("unknown");
    let mut text = format!(
        "Jev browser task {status} at {} ({} actions, {} ms, {} elements observed). Verify the observed page before treating the goal as complete.",
        data["url"].as_str().unwrap_or("unknown URL"),
        data["actions"].as_array().map_or(0, Vec::len),
        data["elapsed_ms"].as_u64().unwrap_or(0),
        data["observed_elements"].as_u64().unwrap_or(0),
    );
    if let Some(reason) = data["stopped_because"].as_str() {
        text.push_str(&format!(" Jev stopped early: {reason}."));
    }
    if status == "done" {
        return text;
    }
    if data["observed_elements"].as_u64() == Some(0) {
        text.push_str(
            " Jev only targets a[href], button, input, textarea, select, summary, \
             [contenteditable] and ARIA-role elements, and this page exposed none: \
             controls built from bare div or td elements are invisible to it. Try another \
             real page or a different browser tool, and report this rather than \
             substituting a locally built page.",
        );
    } else if data["text_calls"].as_u64() == Some(0) && data["text_model"].is_null() {
        text.push_str(
            " No text model is configured, so Jev cannot type: configure a Roder \
             chat-completions provider or set JEV_TEXT_MODEL_API_KEY.",
        );
    }
    text
}

fn error_result(call: &ToolCall, message: String) -> ToolResult {
    ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        text: message.clone(),
        data: json!({"error":{"kind":"jev","message":message}}),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_url_stops_before_browser_start() {
        let mut registry = ToolRegistry::default();
        JevToolContributor.contribute(&mut registry).unwrap();
        let tool = registry.get("jev_browse").unwrap();
        let args = json!({"url":"file:///etc/passwd","goal":"read file"});
        let result = tool
            .execute(
                ToolExecutionContext::new(
                    "thread",
                    "turn",
                    roder_api::policy_mode::PolicyMode::Default,
                ),
                ToolCall {
                    id: "call".into(),
                    name: "jev_browse".into(),
                    raw_arguments: args.to_string(),
                    arguments: args,
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                },
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.text.contains("http(s)"));
    }

    #[test]
    fn blocked_page_without_targets_explains_the_action_space() {
        let text = summarize(&json!({
            "status":"blocked","url":"https://example.com","elapsed_ms":300,
            "actions":[],"observed_elements":0,"text_calls":0,"text_model":null
        }));
        assert!(text.contains("bare div or td"), "{text}");
        assert!(text.contains("substituting a locally built page"), "{text}");
    }

    #[test]
    fn blocked_page_with_targets_does_not_blame_the_action_space() {
        let text = summarize(&json!({
            "status":"blocked","url":"https://example.com","elapsed_ms":300,
            "actions":[],"observed_elements":12,"text_calls":0,
            "text_model":{"model":"deepseek-chat","source":"roder-provider"}
        }));
        assert!(!text.contains("bare div"), "{text}");
        assert!(text.contains("12 elements observed"), "{text}");
    }

    #[test]
    fn early_stop_reports_the_upstream_reason() {
        let text = summarize(&json!({
            "status":"blocked","url":"https://example.com","elapsed_ms":300,
            "actions":[{"step":1}],"observed_elements":9,"text_calls":0,
            "text_model":null,
            "stopped_because":"ValueError: Stopped at the 60-action demo budget"
        }));
        assert!(text.contains("60-action demo budget"), "{text}");
    }

    #[test]
    fn done_result_stays_short() {
        let text = summarize(&json!({
            "status":"done","url":"https://example.com","elapsed_ms":300,
            "actions":[{"step":1}],"observed_elements":9,"text_calls":1,
            "text_model":{"model":"deepseek-chat","source":"turn-model"}
        }));
        assert!(!text.contains("Jev only targets"), "{text}");
        assert!(text.contains("done"), "{text}");
    }

    #[tokio::test]
    #[ignore = "requires JEV_API_KEY and Chrome"]
    async fn live_jev_browse_opens_observed_link() {
        let args = json!({
            "url": "https://example.com",
            "goal": "Click the Learn more link and stop when the IANA page is visible.",
            "timeout_seconds": 120
        });
        let result = execute_live(args).await;
        assert!(!result.is_error, "{}", result.text);
        assert!(
            result.data["url"]
                .as_str()
                .unwrap_or("")
                .contains("iana.org"),
            "{}",
            result.data
        );
        assert!(!result.data["actions"].as_array().unwrap().is_empty());
        // Roder resolved or started the browser itself.
        assert!(
            result.data["browser"]["cdp_url"]
                .as_str()
                .unwrap_or_default()
                .starts_with("http://127.0.0.1:"),
            "{}",
            result.data["browser"]
        );
        assert_eq!(result.data["browser"]["foreground"], json!(true));
    }

    #[tokio::test]
    #[ignore = "requires JEV_API_KEY, Chrome, and a Roder chat-completions provider key"]
    async fn live_jev_browse_types_with_the_roder_text_model() {
        let args = json!({
            "url": "https://www.wikipedia.org/",
            "goal": "Search the English Wikipedia for Geneva and stop when the Geneva article is open.",
            "timeout_seconds": 180
        });
        let result = execute_live(args).await;
        assert!(!result.is_error, "{}", result.text);
        // Typing is only possible through Roder's text helper.
        assert!(
            result.data["text_calls"].as_u64().unwrap_or(0) >= 1,
            "expected a text-helper call: {}",
            result.data
        );
        assert!(
            result.data["text_model"]["source"].is_string(),
            "{}",
            result.data["text_model"]
        );
        assert!(
            result.data["url"]
                .as_str()
                .unwrap_or_default()
                .contains("Geneva"),
            "{}",
            result.data["url"]
        );
    }

    async fn execute_live(args: serde_json::Value) -> ToolResult {
        JevTool
            .execute(
                ToolExecutionContext::new(
                    "thread",
                    "turn",
                    roder_api::policy_mode::PolicyMode::Bypass,
                ),
                ToolCall {
                    id: "live-call".into(),
                    name: "jev_browse".into(),
                    raw_arguments: args.to_string(),
                    arguments: args,
                    thread_id: "thread".into(),
                    turn_id: "turn".into(),
                },
            )
            .await
            .unwrap()
    }
}
