use serde::{Deserialize, Serialize};

use crate::events::{ThreadId, TurnId};

pub use crate::extension::{ContextPlannerId, ContextProviderId};

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

pub trait PolicyContributor: Send + Sync + 'static {}

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
