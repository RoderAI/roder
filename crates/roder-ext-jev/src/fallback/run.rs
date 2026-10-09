//! The fallback's bounded loop: the model reads the tab, calls the full
//! browser tools, and ends with a verdict, within its own step, time and
//! token ceilings.
//!
//! It opens with a look the loop takes itself, so the model's first call
//! can already act. Each action's result carries a brief read of the page
//! after it; older page reads are cut to their first line before each model
//! call, and only the newest screenshot is still shown, which keeps a long
//! fallback's requests from growing with every step. A tool the owner's
//! rules stop (the gate, the allowed origins, an access block) ends the
//! fallback at once with that status; the model is not asked to go on.

use std::time::Instant;

use roder_api::inference::ToolCallCompleted;
use roder_api::tools::ToolSpec;
use roder_api::transcript::{
    AssistantMessage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    VIEW_IMAGE_DISPLAY_KEY, tool_display_payload,
};
use roder_ext_chrome::direct::{DirectSession, DirectStep, OpenedTab, StopKind, tool_result};
use serde::Serialize;
use serde_json::{Value, json};

use super::guard::JevGuard;
use super::model::{FallbackModel, Turn};
use super::offered::offered_tools;
use super::prompt::{Verdict, instructions, opening, page_url, verdict};
use super::trigger::Trigger;
use crate::engine::{JevRunResult, JevStatus};
use crate::session::cut;

/// The tool names the fallback model sees: the same as the hand-over's.
pub(crate) const PREFIX: &str = "jev_tab";
/// Tool results older than the last few page reads are cut to one line.
const KEPT_READS: usize = 2;

/// What the fallback goes on from: the goal, why Jev stopped, and Jev's run.
#[derive(Clone, Copy)]
pub(crate) struct Brief<'a> {
    pub(crate) goal: &'a str,
    pub(crate) trigger: Trigger,
    pub(crate) jev: &'a JevRunResult,
}

/// The fallback's own ceilings.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_steps: usize,
    pub(crate) max_tokens: u64,
    pub(crate) deadline: tokio::time::Instant,
}

/// Checked between steps: an embedder's own end (a benchmark that ended its
/// episode). The fallback then ends with what it has.
#[async_trait::async_trait]
pub(crate) trait EndCheck: Send + Sync {
    async fn ended(&self) -> bool;
}

/// One tool call the fallback made.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FallbackAction {
    pub(crate) step: usize,
    pub(crate) tool: String,
    /// What the call asked for, typed secrets shown as `[secret]`.
    pub(crate) args: Value,
    /// The control it pressed or read, as the page names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<String>,
    /// The result's first line.
    pub(crate) result: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    pub(crate) elapsed_ms: u64,
}

/// Tokens the fallback's model calls spent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct FallbackUsage {
    pub(crate) calls: usize,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    /// Some call did not report its usage, so the sums are a floor.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) incomplete: bool,
}

impl FallbackUsage {
    pub(crate) fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// How a fallback ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FallbackOutcome {
    pub(crate) status: JevStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stopped_because: Option<String>,
    /// The model's final message, after its verdict word (page-derived).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) message: String,
    pub(crate) model: String,
    pub(crate) actions: Vec<FallbackAction>,
    pub(crate) model_calls: usize,
    pub(crate) usage: FallbackUsage,
    pub(crate) elapsed_ms: u64,
    #[serde(skip)]
    pub(crate) opened_tabs: Vec<OpenedTab>,
    /// The tab the fallback ended on.
    #[serde(skip)]
    pub(crate) target_id: String,
    /// The page the last read saw, scrubbed.
    #[serde(skip)]
    pub(crate) last_page: Option<Value>,
}

struct Loop<'a> {
    /// This fallback's own conversation id (see [`Turn::conversation`]).
    conversation: String,
    session: &'a mut DirectSession,
    guard: &'a JevGuard,
    model: &'a dyn FallbackModel,
    /// The model is shown the pictures a tool returns, so it has the
    /// screenshot tool and hears of it.
    pictures: bool,
    instructions: String,
    tools: Vec<ToolSpec>,
    transcript: Vec<TranscriptItem>,
    outcome: FallbackOutcome,
    started: Instant,
}

/// Run the fallback on `session`'s tab until the model gives a verdict, a
/// rule stops it, or a ceiling is reached.
pub(crate) async fn run(
    session: &mut DirectSession,
    guard: &JevGuard,
    model: &dyn FallbackModel,
    brief: Brief<'_>,
    limits: Limits,
    end: Option<&dyn EndCheck>,
) -> FallbackOutcome {
    let pictures = model.sees_tool_result_images();
    let tools = offered_tools(pictures, session.may_authorize());
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let conversation = format!(
        "jev-fallback-{}-{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis())
    );
    let mut run = Loop {
        conversation,
        session,
        guard,
        model,
        pictures,
        instructions: instructions(pictures),
        tools,
        transcript: Vec::new(),
        outcome: FallbackOutcome {
            status: JevStatus::Blocked,
            stopped_because: None,
            message: String::new(),
            model: model.label(),
            actions: Vec::new(),
            model_calls: 0,
            usage: FallbackUsage::default(),
            elapsed_ms: 0,
            opened_tabs: Vec::new(),
            target_id: String::new(),
            last_page: None,
        },
        started: Instant::now(),
    };
    run.go(brief.goal, brief.trigger, brief.jev, limits, end)
        .await;
    run.outcome.elapsed_ms = run.started.elapsed().as_millis() as u64;
    run.outcome.target_id = run.session.target_id().to_string();
    run.outcome
}

impl Loop<'_> {
    async fn go(
        &mut self,
        goal: &str,
        trigger: Trigger,
        jev: &JevRunResult,
        limits: Limits,
        end: Option<&dyn EndCheck>,
    ) {
        let look = self.session.run("look", &json!({})).await;
        if look.is_error {
            return self.end(
                JevStatus::Error,
                format!("could not read the tab: {}", look.text),
            );
        }
        self.outcome.last_page = Some(look.data["page"].clone());
        let seconds = limits
            .deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .as_secs();
        self.transcript
            .push(TranscriptItem::UserMessage(UserMessage::text(opening(
                goal,
                trigger,
                jev,
                &look.text,
                limits.max_steps,
                seconds,
                self.pictures,
            ))));
        loop {
            if end.is_some() && end.unwrap().ended().await {
                return self.end(JevStatus::Blocked, "the embedder ended the task".into());
            }
            if self.outcome.actions.len() >= limits.max_steps {
                return self.end(
                    JevStatus::BudgetExceeded,
                    format!(
                        "the fallback reached its {}-step ceiling (JEV_FALLBACK_MAX_STEPS)",
                        limits.max_steps
                    ),
                );
            }
            if self.outcome.usage.tokens() >= limits.max_tokens {
                return self.end(
                    JevStatus::BudgetExceeded,
                    format!(
                        "the fallback spent its {}-token ceiling (JEV_FALLBACK_MAX_TOKENS)",
                        limits.max_tokens
                    ),
                );
            }
            compact(&mut self.transcript);
            let turn = Turn {
                conversation: &self.conversation,
                instructions: &self.instructions,
                transcript: &self.transcript,
                tools: &self.tools,
                last_page: self.outcome.last_page.as_ref(),
            };
            let reply = match tokio::time::timeout_at(limits.deadline, self.model.reply(turn)).await
            {
                Err(_) => return self.timed_out(),
                Ok(Err(error)) => {
                    return self.end(
                        JevStatus::Error,
                        format!("the fallback model failed: {error:#}"),
                    );
                }
                Ok(Ok(reply)) => reply,
            };
            self.outcome.model_calls += 1;
            match &reply.usage {
                Some(usage) => {
                    self.outcome.usage.input_tokens += u64::from(usage.prompt_tokens);
                    self.outcome.usage.output_tokens += u64::from(usage.completion_tokens);
                }
                None => self.outcome.usage.incomplete = true,
            }
            self.outcome.usage.calls = self.outcome.model_calls;
            // Calls the provider's own output items name, which every
            // request after this one must answer.
            let named = reply
                .items
                .iter()
                .filter(|item| {
                    matches!(
                        item["type"].as_str(),
                        Some("function_call" | "custom_tool_call")
                    )
                })
                .filter_map(|item| item["call_id"].as_str().map(str::to_string))
                .collect::<Vec<_>>();
            for item in reply.items {
                self.transcript
                    .push(TranscriptItem::ProviderMetadata(json!({"output": [item]})));
            }
            if reply.calls.is_empty() {
                return self.conclude(&reply.text);
            }
            if !reply.text.trim().is_empty() {
                self.transcript
                    .push(TranscriptItem::AssistantMessage(AssistantMessage {
                        text: reply.text.clone(),
                        phase: None,
                    }));
            }
            let mut answered = Vec::new();
            for call in reply.calls {
                if self.outcome.actions.len() >= limits.max_steps {
                    break;
                }
                answered.push(call.id.clone());
                let ran = tokio::time::timeout_at(limits.deadline, self.call(call)).await;
                match ran {
                    Err(_) => return self.timed_out(),
                    Ok(Some((status, reason))) => return self.end(status, reason),
                    Ok(None) => {}
                }
            }
            // A call left unrun (past the step ceiling, or one the provider
            // named but did not complete) still gets an answer, or the next
            // request is refused for a call with no output.
            for id in named.into_iter().filter(|id| !answered.contains(id)) {
                self.transcript
                    .push(TranscriptItem::ToolResult(ToolResultRecord {
                        id,
                        name: None,
                        result: "Not run.".into(),
                        display_payload: None,
                        is_error: true,
                    }));
            }
        }
    }

    /// Run one tool call and record it. `Some` when a rule stopped the
    /// fallback there.
    async fn call(&mut self, call: ToolCallCompleted) -> Option<(JevStatus, String)> {
        let args = serde_json::from_str::<Value>(&call.arguments).unwrap_or(json!({}));
        let short = call
            .name
            .strip_prefix(&format!("{PREFIX}_"))
            .unwrap_or(&call.name)
            .to_string();
        let started = Instant::now();
        let step = match self.tools.iter().any(|tool| tool.name == call.name) {
            true => self.session.run(&short, &args).await,
            false => DirectStep::error(format!("there is no tool {}", call.name)),
        };
        if let Some(secret) = &step.typed_secret {
            self.guard.remember(secret);
        }
        if let Some(opened) = &step.opened_tab {
            self.outcome.opened_tabs.push(opened.clone());
        }
        if step.data["page"].is_object() {
            self.outcome.last_page = Some(step.data["page"].clone());
        }
        let mut shown_args = args.clone();
        if step.typed_secret.is_some() {
            shown_args["text"] = json!(crate::secret::SECRET);
        }
        self.outcome.actions.push(FallbackAction {
            step: self.outcome.actions.len() + 1,
            tool: short,
            args: shown_args,
            target: target_label(&step.data),
            result: cut(step.text.lines().next().unwrap_or_default(), 200),
            error: step.is_error,
            url: page_url(&step.data),
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
        let result = tool_result(&call.id, &call.name, &step);
        let display = tool_display_payload(Some(&call.name), Some(&args), Some(&result.data));
        self.transcript
            .push(TranscriptItem::ToolCall(ToolCallRecord {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            }));
        self.transcript
            .push(TranscriptItem::ToolResult(ToolResultRecord {
                id: call.id,
                name: Some(call.name),
                result: step.text.clone(),
                display_payload: display,
                is_error: step.is_error,
            }));
        let stop = step.stop?;
        let status = match stop.kind {
            StopKind::NeedsConfirmation => JevStatus::NeedsConfirmation,
            StopKind::OutsideScope => JevStatus::Blocked,
            StopKind::AccessDenied => JevStatus::AccessDenied,
        };
        Some((status, stop.reason))
    }

    fn conclude(&mut self, message: &str) {
        let message = self.guard_scrub(message);
        let (verdict, rest) = verdict(&message);
        self.outcome.message = cut(&rest, 600);
        let (status, reason) = match verdict {
            Verdict::Done => (JevStatus::Done, None),
            Verdict::Blocked => (JevStatus::Blocked, Some(rest)),
            Verdict::NeedsInput => (JevStatus::NeedsInput, Some(rest)),
            Verdict::NeedsConfirmation => (JevStatus::NeedsConfirmation, Some(rest)),
            Verdict::Unsaid if rest.is_empty() => (
                JevStatus::Blocked,
                Some("the fallback model ended without a verdict".to_string()),
            ),
            Verdict::Unsaid => (
                JevStatus::Blocked,
                Some(format!("the fallback model ended without DONE: {rest}")),
            ),
        };
        self.outcome.status = status;
        self.outcome.stopped_because = reason.map(|reason| cut(&reason, 400));
    }

    fn guard_scrub(&self, text: &str) -> String {
        use roder_ext_chrome::direct::DirectGuard;
        self.guard.scrub(text)
    }

    fn end(&mut self, status: JevStatus, reason: String) {
        self.outcome.status = status;
        self.outcome.stopped_because = Some(cut(&self.guard_scrub(&reason), 400));
    }

    fn timed_out(&mut self) {
        self.end(
            JevStatus::TimedOut,
            "the fallback ran out of time (JEV_FALLBACK_MAX_SECONDS, or the call's deadline)"
                .into(),
        );
    }
}

/// What a step pressed or read, as the page names it.
fn target_label(data: &Value) -> Option<String> {
    let target = data.get("target").or_else(|| data.get("to"))?;
    let label = target["label"].as_str().filter(|label| !label.is_empty());
    let what = target["role"].as_str().or_else(|| target["tag"].as_str());
    match (what, label) {
        (Some(what), Some(label)) => Some(format!("{what} \"{}\"", cut(label, 80))),
        (None, Some(label)) => Some(cut(label, 80)),
        (Some(what), None) => Some(what.to_string()),
        (None, None) => None,
    }
}

/// Cut every page-bearing tool result but the last [`KEPT_READS`] to its
/// first line, and show only the newest screenshot.
pub(crate) fn compact(transcript: &mut [TranscriptItem]) {
    let mut reads = 0;
    let mut pictures = 0;
    for item in transcript.iter_mut().rev() {
        let TranscriptItem::ToolResult(result) = item else {
            continue;
        };
        if let Some(payload) = result.display_payload.as_mut()
            && payload.get(VIEW_IMAGE_DISPLAY_KEY).is_some()
        {
            pictures += 1;
            if pictures > 1
                && let Some(payload) = payload.as_object_mut()
            {
                payload.remove(VIEW_IMAGE_DISPLAY_KEY);
                result.result = format!("{} [picture no longer shown]", first_line(&result.result));
            }
            continue;
        }
        if result.result.lines().count() <= 1 {
            continue;
        }
        reads += 1;
        if reads > KEPT_READS {
            result.result = format!(
                "{} [page read elided; look again for the current page]",
                first_line(&result.result)
            );
        }
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}
