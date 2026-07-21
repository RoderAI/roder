use crate::error::Result;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type HookFuture = Pin<Box<dyn Future<Output = Result<serde_json::Value>> + Send>>;
type HookFn = dyn Fn(serde_json::Value, Option<String>, HookContext) -> HookFuture + Send + Sync;

#[derive(Debug, Clone, Default)]
pub struct HookContext {}

#[derive(Clone)]
pub struct HookCallback(Arc<HookFn>);

impl std::fmt::Debug for HookCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("HookCallback").field(&"<callback>").finish()
    }
}

impl HookCallback {
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(serde_json::Value, Option<String>, HookContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<serde_json::Value>> + Send + 'static,
    {
        Self(Arc::new(move |input, tool_use_id, context| {
            Box::pin(callback(input, tool_use_id, context))
        }))
    }

    pub async fn call(
        &self,
        input: serde_json::Value,
        tool_use_id: Option<String>,
        context: HookContext,
    ) -> Result<serde_json::Value> {
        (self.0)(input, tool_use_id, context).await
    }
}

#[derive(Debug, Clone, Default)]
pub struct HookMatcher {
    pub matcher: Option<String>,
    pub hooks: Vec<HookCallback>,
    pub timeout: Option<f64>,
}

impl HookMatcher {
    pub fn new(callback: HookCallback) -> Self {
        Self {
            matcher: None,
            hooks: vec![callback],
            timeout: None,
        }
    }

    pub fn matcher(mut self, matcher: impl Into<String>) -> Self {
        self.matcher = Some(matcher.into());
        self
    }

    pub fn timeout(mut self, timeout: f64) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

pub type HookMap = HashMap<String, Vec<HookMatcher>>;

// ---------------------------------------------------------------------------
// Typed hook input/output payloads (parity with the Python SDK type surface).
//
// The hook callback itself stays generic over `serde_json::Value` for
// flexibility; these strongly-typed structs let callers deserialize a known
// event or construct a structured response, mirroring the Python TypedDicts.
// Input field names are snake_case on the wire; output field names are
// camelCase, matching the CLI exactly.
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

/// Base fields present across hook input events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseHookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

/// Input data for `PreToolUse` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreToolUseHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub tool_name: String,
    pub tool_input: serde_json::Map<String, serde_json::Value>,
    pub tool_use_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// Input data for `PostToolUse` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostToolUseHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub tool_name: String,
    pub tool_input: serde_json::Map<String, serde_json::Value>,
    pub tool_response: serde_json::Value,
    pub tool_use_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// Input data for `PostToolUseFailure` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostToolUseFailureHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub tool_name: String,
    pub tool_input: serde_json::Map<String, serde_json::Value>,
    pub tool_use_id: String,
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_interrupt: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// Input data for `UserPromptSubmit` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPromptSubmitHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub prompt: String,
}

/// Input data for `Stop` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub stop_hook_active: bool,
}

/// Input data for `SubagentStop` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStopHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub stop_hook_active: bool,
    pub agent_id: String,
    pub agent_transcript_path: String,
    pub agent_type: String,
}

/// Input data for `PreCompact` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreCompactHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub trigger: String,
    pub custom_instructions: Option<String>,
}

/// Input data for `Notification` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub notification_type: String,
}

/// Input data for `SubagentStart` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStartHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub agent_id: String,
    pub agent_type: String,
}

/// Input data for `PermissionRequest` hook events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestHookInput {
    #[serde(flatten)]
    pub base: BaseHookInput,
    pub tool_name: String,
    pub tool_input: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_suggestions: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// Strongly-typed union of hook inputs, discriminated by `hook_event_name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookInput {
    PreToolUse(PreToolUseHookInput),
    PostToolUse(PostToolUseHookInput),
    PostToolUseFailure(PostToolUseFailureHookInput),
    UserPromptSubmit(UserPromptSubmitHookInput),
    Stop(StopHookInput),
    SubagentStop(SubagentStopHookInput),
    PreCompact(PreCompactHookInput),
    Notification(NotificationHookInput),
    SubagentStart(SubagentStartHookInput),
    PermissionRequest(PermissionRequestHookInput),
}

/// Hook-specific output for `PostToolUseFailure` events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostToolUseFailureHookSpecificOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

/// Hook-specific output for `Notification` events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationHookSpecificOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

/// Hook-specific output for `SubagentStart` events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentStartHookSpecificOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

/// Hook-specific output for `PermissionRequest` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestHookSpecificOutput {
    pub decision: serde_json::Map<String, serde_json::Value>,
}
