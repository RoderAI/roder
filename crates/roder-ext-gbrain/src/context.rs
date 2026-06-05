//! Context provider: injects current-belief facts for the active thread into
//! the prompt, mirroring the default memory context provider but bi-temporally
//! aware (superseded/invalidated facts are excluded from "current belief").

use std::sync::Arc;

use roder_api::context::{
    ContextBlock, ContextBlockKind, ContextProvider, ContextProviderId, ContextQuery,
};
use roder_api::memory::MemoryScope;

use crate::model::AsOf;
use crate::store::{GbrainStore, RecallParams};

pub struct GbrainContextProvider {
    store: Arc<GbrainStore>,
}

impl GbrainContextProvider {
    pub fn new(store: Arc<GbrainStore>) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl ContextProvider for GbrainContextProvider {
    fn id(&self) -> ContextProviderId {
        "gbrain-context".to_string()
    }

    async fn blocks(&self, query: &ContextQuery) -> anyhow::Result<Vec<ContextBlock>> {
        let result = self
            .store
            .recall(RecallParams {
                query: query.prompt.clone(),
                as_of: AsOf::now(),
                scope: Some(MemoryScope::Workspace(query.thread_id.clone())),
                include_global: true,
                limit: 5,
            })
            .await?;
        Ok(result
            .hits
            .into_iter()
            .map(|hit| ContextBlock {
                id: hit.fact.id.clone(),
                kind: ContextBlockKind::Memory,
                text: hit.fact.text.clone(),
                priority: (hit.score * 100.0) as i32,
                token_estimate: None,
                metadata: serde_json::json!({
                    "scope": hit.fact.scope.stable_id(),
                    "subject": hit.fact.subject,
                    "validAt": crate::model::format_time(hit.fact.valid_at),
                    "status": crate::store::status_label(&hit.fact, result.now),
                    "provenance": hit.fact.provenance,
                }),
            })
            .collect())
    }
}
