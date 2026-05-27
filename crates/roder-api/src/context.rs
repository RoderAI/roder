use serde::{Deserialize, Serialize};

use crate::events::{ThreadId, TurnId};
use crate::policy_mode::{PolicyDecision, PolicyMode};
use crate::tools::{ToolCall, ToolExecutionContext};

pub use crate::extension::{ContextPlannerId, ContextProviderId, PolicyContributorId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContextBlockKind {
    Instruction,
    RepositoryFact,
    Memory,
    RetrievedDocument,
    Environment,
    ToolAvailability,
    SafetyPolicy,
    TaskMetadata,
    PriorSummary,
    EntrypointHint,
    RetrievalHint,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextBlock {
    pub id: String,
    pub kind: ContextBlockKind,
    pub text: String,
    pub priority: i32,
    pub token_estimate: Option<u32>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextQuery {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub token_budget: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ContextPlan {
    pub blocks: Vec<ContextBlock>,
}

#[async_trait::async_trait]
pub trait ContextProvider: Send + Sync + 'static {
    fn id(&self) -> ContextProviderId;

    async fn blocks(&self, query: &ContextQuery) -> anyhow::Result<Vec<ContextBlock>>;
}

#[async_trait::async_trait]
pub trait ContextPlanner: Send + Sync + 'static {
    fn id(&self) -> ContextPlannerId;

    async fn plan(
        &self,
        query: &ContextQuery,
        provider_blocks: Vec<ContextBlock>,
    ) -> anyhow::Result<ContextPlan>;
}

#[derive(Debug, Clone)]
pub struct PolicyReview {
    pub call: ToolCall,
    pub mode: PolicyMode,
    pub context: ToolExecutionContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyContribution {
    Abstain,
    Allow { reason: Option<String> },
    RequireApproval { reason: Option<String> },
    Deny { reason: String },
}

#[async_trait::async_trait]
pub trait PolicyContributor: Send + Sync + 'static {
    fn id(&self) -> PolicyContributorId;

    async fn review_tool(&self, review: PolicyReview) -> anyhow::Result<PolicyContribution>;
}

pub trait PolicyGate: Send + Sync + 'static {
    fn decide(
        &self,
        call: &ToolCall,
        mode: PolicyMode,
        context: &ToolExecutionContext,
    ) -> PolicyDecision;
}

pub struct SimpleContextPlanner;

#[async_trait::async_trait]
impl ContextPlanner for SimpleContextPlanner {
    fn id(&self) -> ContextPlannerId {
        "default".to_string()
    }

    async fn plan(
        &self,
        _query: &ContextQuery,
        mut provider_blocks: Vec<ContextBlock>,
    ) -> anyhow::Result<ContextPlan> {
        provider_blocks.sort_by_key(|block| std::cmp::Reverse(block.priority));
        Ok(ContextPlan {
            blocks: provider_blocks,
        })
    }
}
