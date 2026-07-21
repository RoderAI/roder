use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub mod messages;
pub use messages::*;
pub mod hooks;
pub use hooks::*;
pub mod config;
pub use config::*;
pub mod agent_options;
pub use agent_options::*;

// Enums and String Constants

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    BypassPermissions,
    DontAsk,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SdkBeta {
    #[serde(rename = "context-1m-2025-08-07")]
    Context1M20250807,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingSource {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThinkingConfigType {
    Adaptive,
    Enabled,
    Disabled,
}

/// Controls how much effort Claude puts into its response.
///
/// Mirrors the upstream Python `EffortLevel` literal
/// (`"low" | "medium" | "high" | "xhigh" | "max"`). Serializes to the bare
/// lowercase string the CLI expects for `--effort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl EffortLevel {
    /// The bare string value passed to the `--effort` CLI flag.
    pub fn as_cli(&self) -> &'static str {
        match self {
            EffortLevel::Low => "low",
            EffortLevel::Medium => "medium",
            EffortLevel::High => "high",
            EffortLevel::Xhigh => "xhigh",
            EffortLevel::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserContentKind {
    Text,
    Blocks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantMessageErrorKind {
    AuthenticationFailed,
    BillingError,
    RateLimit,
    InvalidRequest,
    ServerError,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskNotificationStatus {
    Completed,
    Failed,
    Stopped,
}

/// Status values reported inside a `task_updated` patch.
///
/// `pending`/`running`/`paused` are non-terminal; `completed`/`failed`/`killed`
/// are terminal. Note `task_updated` reports the raw `killed`; the CLI maps that
/// to `stopped` only when it emits a `task_notification`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskUpdatedStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Killed,
}

/// Task statuses that mean the task has finished and should be cleared from any
/// "active task" tracking. Spans both lifecycle vocabularies: `task_notification`
/// reports `stopped` (the CLI's mapped form of a killed task) while `task_updated`
/// reports the raw `killed`. Consumers should treat the status of a
/// `TaskNotificationMessage` and a `TaskUpdatedMessage` the same way.
pub const TERMINAL_TASK_STATUSES: [&str; 4] = ["completed", "failed", "stopped", "killed"];

/// Returns `true` when `status` is a terminal task status (see
/// [`TERMINAL_TASK_STATUSES`]).
pub fn is_terminal_task_status(status: &str) -> bool {
    TERMINAL_TASK_STATUSES.contains(&status)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RateLimitStatus {
    Allowed,
    #[serde(alias = "allowed_warning")]
    AllowedWarning,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitType {
    FiveHour,
    SevenDay,
    SevenDayOpus,
    SevenDaySonnet,
    Overage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MCPServerConnectionStatus {
    Connected,
    Failed,
    #[serde(rename = "needs-auth")]
    NeedsAuth,
    Pending,
    Disabled,
}

// MCP Server Config Types (tagged enum pattern)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MCPServerConfig {
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
    Sse {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    Http {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    #[serde(rename = "sdk")]
    Sdk { name: String },
}

// Simple Struct Types

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsPreset {
    pub r#type: String,
    pub preset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPromptPreset {
    pub r#type: String,
    pub preset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_dynamic_sections: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPromptFile {
    pub r#type: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingConfig {
    pub r#type: ThinkingConfigType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBudget {
    pub total: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRuleValue {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionUpdate {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<PermissionRuleValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<PermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolPermissionContext {
    pub suggestions: Vec<PermissionUpdate>,
    pub tool_use_id: Option<String>,
    pub agent_id: Option<String>,
    pub blocked_path: Option<String>,
    pub decision_reason: Option<String>,
    pub title: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub enum PermissionResult {
    Allow {
        updated_input: Option<serde_json::Map<String, serde_json::Value>>,
        updated_permissions: Option<Vec<PermissionUpdate>>,
    },
    Deny {
        message: String,
        interrupt: bool,
    },
}

impl PermissionResult {
    pub fn allow() -> Self {
        Self::Allow {
            updated_input: None,
            updated_permissions: None,
        }
    }

    pub fn deny(message: impl Into<String>) -> Self {
        Self::Deny {
            message: message.into(),
            interrupt: false,
        }
    }
}

pub type CanUseToolFuture = Pin<Box<dyn Future<Output = Result<PermissionResult>> + Send>>;
type CanUseToolFn = dyn Fn(
        String,
        serde_json::Map<String, serde_json::Value>,
        ToolPermissionContext,
    ) -> CanUseToolFuture
    + Send
    + Sync;

#[derive(Clone)]
pub struct CanUseToolCallback(Arc<CanUseToolFn>);

impl std::fmt::Debug for CanUseToolCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CanUseToolCallback")
            .field(&"<callback>")
            .finish()
    }
}

impl CanUseToolCallback {
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(String, serde_json::Map<String, serde_json::Value>, ToolPermissionContext) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = Result<PermissionResult>> + Send + 'static,
    {
        Self(Arc::new(move |tool_name, input, context| {
            Box::pin(callback(tool_name, input, context))
        }))
    }

    pub async fn call(
        &self,
        tool_name: String,
        input: serde_json::Map<String, serde_json::Value>,
        context: ToolPermissionContext,
    ) -> Result<PermissionResult> {
        (self.0)(tool_name, input, context).await
    }
}

type StderrFn = dyn Fn(String) + Send + Sync;

#[derive(Clone)]
pub struct StderrCallback(Arc<StderrFn>);

impl std::fmt::Debug for StderrCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StderrCallback")
            .field(&"<callback>")
            .finish()
    }
}

impl StderrCallback {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        Self(Arc::new(callback))
    }

    pub fn call(&self, line: String) {
        (self.0)(line);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SDKPluginConfig {
    pub r#type: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPToolAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_world: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPToolInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<MCPToolAnnotations>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPServerStatus {
    pub name: String,
    pub status: MCPServerConnectionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_info: Option<MCPServerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<MCPServerStatusConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<MCPToolInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPStatusResponse {
    pub mcp_servers: Vec<MCPServerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsageCategory {
    pub name: String,
    pub tokens: i32,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_deferred: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsageResponse {
    pub categories: Vec<ContextUsageCategory>,
    pub total_tokens: i32,
    pub max_tokens: i32,
    pub raw_max_tokens: i32,
    pub percentage: f64,
    pub model: String,
    pub is_auto_compact_enabled: bool,
    pub memory_files: Vec<serde_json::Map<String, serde_json::Value>>,
    pub mcp_tools: Vec<serde_json::Map<String, serde_json::Value>>,
    pub agents: Vec<serde_json::Map<String, serde_json::Value>>,
    pub grid_rows: Vec<Vec<serde_json::Map<String, serde_json::Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compact_threshold: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferred_builtin_tools: Option<Vec<serde_json::Map<String, serde_json::Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_tools: Option<Vec<serde_json::Map<String, serde_json::Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_sections: Option<Vec<serde_json::Map<String, serde_json::Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slash_commands: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUsage {
    #[serde(alias = "total_tokens")]
    pub total_tokens: i32,
    #[serde(alias = "tool_uses")]
    pub tool_uses: i32,
    #[serde(alias = "duration_ms")]
    pub duration_ms: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitInfo {
    pub status: RateLimitStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_type: Option<RateLimitType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_status: Option<RateLimitStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_resets_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_disabled_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Map<String, serde_json::Value>>,
}

// MCP Server Status Config (tagged enum)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MCPServerStatusConfig {
    #[serde(rename = "sdk")]
    Sdk { name: String },
    #[serde(rename = "claudeai-proxy")]
    ClaudeAiProxy { url: String, id: String },
    Sse {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    Http {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },
    #[serde(rename = "stdio")]
    Stdio {
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
}
