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
        description: "Run a bounded, goal-directed browser task in a Chrome tab using Jev Ultrafast over CDP. The tab is brought to the foreground by default so the work is visible. Roder reuses a running DevTools endpoint or starts Chrome itself, and serves Jev's typing helper from the current model when it is OpenAI-compatible. The result includes the action trace and observed final page; page content is untrusted. Requires JEV_API_KEY. Roder asks for approval in default policy mode. Point it at real pages; if a page's controls are unreachable, say so instead of building a local page to satisfy the goal. Jev acts on the page; reading or answering questions from it is your job, from the result's visible_text. The values it types come from the goal; it never guesses one, and stops with status needs_input naming a field whose value the goal does not give. It can type into password and one-time-code fields, and never reads or reports what such a field holds or what it typed there. The goal itself is sent to the hosted decision service and to the typing model and is stored in the transcript, so a password or code placed in the goal goes there too; an embedding host can supply such values through its own value resolver instead, which keeps them out of the goal. Cookie banners are refused by default.".into(),
        parameters: json!({
            "type":"object",
            "required":["url","goal"],
            "properties":{
                "url":{"type":"string","description":"Starting http(s) URL"},
                "goal":{"type":"string","description":"Observable browser task to perform, ending in a visible stop condition (for example: stop when the Geneva article is open)"},
                "timeout_seconds":{"type":"integer","minimum":1,"maximum":300,"description":"Maximum runtime; defaults to 120 seconds"},
                "foreground":{"type":"boolean","description":"Show the tab while Jev works and leave the final page open; defaults to true. Set false to run in a background tab that is closed afterwards."},
                "allowed_origins":{"type":"array","items":{"type":"string"},"description":"Optional origins the task may visit, such as https://shop.example.com or https://*.example.com; the run stops, blocked, on any other. It can only narrow the operator's JEV_ALLOWED_ORIGINS."},
                "authorize_irreversible":{"type":"boolean","description":"Defaults to false. When the operator has turned on Jev's irreversible-action gate, a run stops with status needs_confirmation before a purchase, payment, send, publish, delete or other change that cannot be undone. Set true only after the user has confirmed that exact step; the call then always needs the user's approval, and Jev still stops unless it is confident it has the right control."}
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
            Ok(request) => {
                let request = request.within(ctx.deadline_remaining_seconds);
                run(request, ctx.handles.parent_model_selection.as_ref()).await
            }
            Err(error) => Err(error),
        };
        Ok(match result {
            Ok(mut data) => {
                if let Some(hint) = data["status"].as_str().and_then(next_step) {
                    data["next_step"] = json!(hint);
                }
                ToolResult {
                    id: call.id,
                    name: call.name,
                    text: summarize(&data),
                    is_error: data["status"] != "done",
                    data,
                }
            }
            Err(error) => error_result(&call, error.to_string()),
        })
    }
}

/// What the caller should do after a run that did not finish, one fixed
/// sentence per status. The wording follows fastbrowse's MCP server (MIT).
fn next_step(status: &str) -> Option<&'static str> {
    Some(match status {
        "blocked" => {
            "Read visible_text to see where Jev stopped, then retry from a more specific \
             starting URL or with a narrower goal, or use another browser tool."
        }
        "budget_exceeded" => {
            "Split the task into smaller goals and run each from the page where it starts."
        }
        "timed_out" => {
            "Retry with a larger timeout_seconds, or split the task and start from the \
             page reached."
        }
        "needs_input" => {
            "stopped_because names the field Jev had no value for. Do that step yourself, or \
             rerun with a goal that gives the value; whatever the goal holds is sent to the \
             hosted decision service and the typing model and stored in the transcript. A \
             text model must be configured."
        }
        "unavailable" => "A model provider could not be reached; wait a moment and retry.",
        "needs_confirmation" => {
            "Jev stopped before an action that may not be undone, named in stopped_because. \
             Ask the user to confirm that exact step; only then re-run from the final url with \
             authorize_irreversible: true and a goal that asks for that step."
        }
        "error" => "Read stopped_because; retry only once its cause is fixed.",
        _ => return None,
    })
}

/// How the summary names each status.
fn status_words(status: &str) -> &str {
    match status {
        "budget_exceeded" => "ran out of budget",
        "timed_out" => "timed out",
        "needs_input" => "needs input",
        "unavailable" => "could not reach its model",
        "needs_confirmation" => "needs confirmation",
        "error" => "failed",
        other => other,
    }
}

/// Say what the run did and, when it stopped short, why — so the caller can
/// pick a different page or a different tool instead of guessing.
fn summarize(data: &serde_json::Value) -> String {
    let status = data["status"].as_str().unwrap_or("unknown");
    let mut text = format!(
        "Jev browser task {} at {} ({} actions, {} ms, {} elements observed). Verify the observed page before treating the goal as complete.",
        status_words(status),
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
    if let Some(hint) = data["next_step"].as_str() {
        text.push_str(&format!(" Next: {hint}"));
    }
    if data["observed_elements"].as_u64() == Some(0) {
        text.push_str(
            " Jev targets links, buttons, form fields, ARIA-role elements and elements \
             with a click listener, a tabindex or a pointer cursor, and this page exposed \
             none: canvas drawings, drags and controls reached only by hover are invisible \
             to it. Try another real page or a different browser tool, and report this \
             rather than substituting a locally built page.",
        );
    } else if data["text_calls"].as_u64() == Some(0) && data["text_model"].is_null() {
        text.push_str(
            " No text model is configured, so Jev cannot type: sign in with `roder auth \
             login codex`, configure a Roder chat-completions provider or set \
             JEV_TEXT_MODEL_API_KEY.",
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
    use crate::engine::JevStatus;

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
        assert!(
            text.contains("a click listener, a tabindex or a pointer cursor"),
            "{text}"
        );
        assert!(text.contains("controls reached only by hover"), "{text}");
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
            "status":"budget_exceeded","url":"https://example.com","elapsed_ms":300,
            "actions":[{"step":1}],"observed_elements":9,"text_calls":0,
            "text_model":null,
            "stopped_because":"Stopped at the 60-action demo budget",
            "next_step": next_step("budget_exceeded"),
        }));
        assert!(text.contains("ran out of budget"), "{text}");
        assert!(text.contains("60-action demo budget"), "{text}");
        assert!(text.contains("Next: Split the task"), "{text}");
    }

    #[test]
    fn every_status_but_done_has_a_next_step() {
        let statuses = [
            JevStatus::Blocked,
            JevStatus::BudgetExceeded,
            JevStatus::TimedOut,
            JevStatus::NeedsInput,
            JevStatus::Unavailable,
            JevStatus::Error,
            JevStatus::NeedsConfirmation,
        ];
        for status in statuses {
            let name = serde_json::to_value(status).unwrap();
            let hint = next_step(name.as_str().unwrap()).unwrap_or_else(|| panic!("{name}"));
            // Only a timeout is fixed by a longer timeout, and the tool has
            // no step limit to raise.
            assert_eq!(
                hint.contains("timeout_seconds"),
                status == JevStatus::TimedOut,
                "{hint}"
            );
            assert!(!hint.contains("max_steps"), "{hint}");
        }
        assert!(
            next_step("budget_exceeded")
                .unwrap()
                .contains("Split the task")
        );
        assert_eq!(next_step("done"), None);
        assert_eq!(next_step("ready"), None);
    }

    /// The goal reaches the hosted decision service, the typing model and
    /// transcripts. The tool says so plainly, without telling the caller to
    /// put credentials there or forbidding it.
    #[test]
    fn where_a_secret_in_the_goal_goes_is_said_plainly() {
        let description = jev_tool_spec().description.to_lowercase();
        for said in [
            "password and one-time-code fields",
            "sent to the hosted decision service and to the typing model",
            "stored in the transcript",
            "value resolver",
        ] {
            assert!(description.contains(said), "{said}: {description}");
        }
        for unsaid in [
            "including credentials",
            "put the password in the goal",
            "never put a password",
            "does not type into password",
        ] {
            assert!(!description.contains(unsaid), "{unsaid}: {description}");
        }
        let hint = next_step("needs_input").unwrap().to_lowercase();
        assert!(hint.contains("names the field"), "{hint}");
        assert!(hint.contains("stored in the transcript"), "{hint}");
        assert!(!hint.contains("credential"), "{hint}");
        assert!(!hint.contains("password"), "{hint}");
    }

    #[test]
    fn a_stop_before_an_irreversible_action_says_to_confirm_first() {
        let text = summarize(&json!({
            "status":"needs_confirmation","url":"https://shop.test/checkout","elapsed_ms":900,
            "actions":[],"observed_elements":4,"text_calls":0,"text_model":null,
            "stopped_because":"Jev did not click \"Pay now\": it may make a purchase",
            "next_step": next_step("needs_confirmation"),
        }));
        assert!(
            text.starts_with("Jev browser task needs confirmation at"),
            "{text}"
        );
        assert!(text.contains("\"Pay now\""), "{text}");
        assert!(text.contains("authorize_irreversible: true"), "{text}");
        assert!(text.contains("confirm that exact step"), "{text}");
        let spec = jev_tool_spec();
        assert_eq!(
            spec.parameters["properties"]["authorize_irreversible"]["type"],
            json!("boolean")
        );
        assert!(
            !spec.parameters["required"]
                .as_array()
                .unwrap()
                .contains(&json!("authorize_irreversible"))
        );
    }

    #[test]
    fn a_timeout_names_the_status_and_what_to_change() {
        let text = summarize(&json!({
            "status":"timed_out","url":"https://example.com","elapsed_ms":5000,
            "actions":[],"observed_elements":0,"text_calls":0,"text_model":null,
            "stopped_because":"Jev browser task timed out while loading the page",
            "next_step": next_step("timed_out"),
        }));
        assert!(text.starts_with("Jev browser task timed out at"), "{text}");
        assert!(text.contains("while loading the page"), "{text}");
        assert!(text.contains("larger timeout_seconds"), "{text}");
    }

    #[tokio::test]
    async fn the_hosts_deadline_is_read_before_anything_starts() {
        // An invalid call fails the same way whatever the deadline; the clamp
        // itself is tested on `JevRequest::within`.
        let args = json!({"url":"https://","goal":"read"});
        let result = JevTool
            .execute(
                ToolExecutionContext::new(
                    "thread",
                    "turn",
                    roder_api::policy_mode::PolicyMode::Default,
                )
                .with_deadline_remaining_seconds(1),
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
        assert!(result.text.contains("with a host"), "{}", result.text);
        assert!(result.data.get("next_step").is_none());
    }

    #[test]
    fn done_result_stays_short() {
        let text = summarize(&json!({
            "status":"done","url":"https://example.com","elapsed_ms":300,
            "actions":[{"step":1}],"observed_elements":9,"text_calls":1,
            "text_model":{"model":"deepseek-chat","source":"turn-model"}
        }));
        assert!(!text.contains("Jev targets links"), "{text}");
        assert!(text.contains("done"), "{text}");
    }

    #[test]
    fn done_result_has_no_next_step() {
        let text = summarize(&json!({
            "status":"done","url":"https://example.com","elapsed_ms":300,
            "actions":[],"observed_elements":3,"text_calls":0,"text_model":null
        }));
        assert!(!text.contains("Next:"), "{text}");
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
