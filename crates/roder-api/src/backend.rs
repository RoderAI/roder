//! A complete agent runtime behind Roder's user interface.
//!
//! Backends own inference, tools, and conversation state. The UI only sends
//! user actions and renders normalized events. This is distinct from an
//! `InferenceEngine`, which participates in Roder's own tool loop.

use std::path::PathBuf;

use crate::inference::TokenUsage;
use crate::policy_mode::PolicyMode;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct BackendStart {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub resume_thread: Option<String>,
    pub policy_mode: PolicyMode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    TurnStarted {
        turn_id: String,
    },
    Text(String),
    Usage(TokenUsage),
    ProviderMetadata(serde_json::Value),
    ToolStarted {
        id: String,
        label: String,
    },
    ToolOutput {
        id: String,
        text: String,
    },
    ToolFinished {
        id: String,
        label: String,
        summary: String,
    },
    FileChanged {
        path: String,
        change_type: String,
    },
    Approval {
        request_id: String,
        description: String,
    },
    Question {
        request_id: String,
        questions: Vec<BackendQuestion>,
    },
    TurnFinished {
        turn_id: String,
        status: String,
    },
    Notice(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendQuestion {
    pub id: String,
    pub text: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BackendThreadSnapshot {
    pub preview: String,
    pub cwd: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub turns: Vec<BackendHistoryTurn>,
}

#[derive(Debug, Clone)]
pub struct BackendThreadSummary {
    pub id: String,
    pub model: String,
    pub snapshot: BackendThreadSnapshot,
}

#[derive(Debug, Clone)]
pub struct BackendModel {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub default_reasoning: String,
    pub reasoning_efforts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BackendHistoryTurn {
    pub id: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub items: Vec<BackendHistoryItem>,
}

#[derive(Debug, Clone)]
pub enum BackendHistoryItem {
    User {
        id: String,
        text: String,
    },
    Agent {
        id: String,
        text: String,
    },
    Tool {
        id: String,
        name: String,
        output: Option<String>,
        status: String,
    },
    Other {
        id: String,
        payload: serde_json::Value,
    },
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    fn id(&self) -> &'static str;

    async fn connect(&self, start: BackendStart) -> anyhow::Result<Box<dyn AgentBackendSession>>;
    async fn list_threads(
        &self,
        cwd: PathBuf,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<BackendThreadSummary>>;
    async fn list_models(&self, cwd: PathBuf) -> anyhow::Result<Vec<BackendModel>>;
}

#[async_trait]
pub trait AgentBackendSession: Send {
    fn thread_id(&self) -> &str;
    fn model(&self) -> &str;
    fn reasoning(&self) -> &str;
    fn snapshot(&self) -> BackendThreadSnapshot;
    async fn send(
        &mut self,
        text: String,
        policy_mode: Option<PolicyMode>,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> anyhow::Result<String>;
    async fn interrupt(&mut self) -> anyhow::Result<()>;
    async fn archive(&mut self) -> anyhow::Result<()>;
    async fn approve(&mut self, request_id: &str, approved: bool) -> anyhow::Result<()>;
    async fn answer(
        &mut self,
        request_id: &str,
        answers: Vec<(String, String)>,
    ) -> anyhow::Result<()>;
    async fn next_event(&mut self) -> anyhow::Result<BackendEvent>;
}
