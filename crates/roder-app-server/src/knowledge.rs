//! App-server `knowledge/*` handlers (roadmap phase 93): client access to
//! the project knowledge base without direct filesystem access. Writes
//! through this surface are attributed to `KnowledgeSource::User`; agent
//! writes go through the `knowledge_*` tools instead.

use std::sync::Arc;

use roder_api::events::RoderEvent;
use roder_api::knowledge::{
    KnowledgeLinkRequest, KnowledgeListQuery, KnowledgeQuery, KnowledgeSaveRequest,
    KnowledgeSource, KnowledgeStore, KnowledgeUpdateRequest,
};
use roder_protocol::{
    JsonRpcError, KnowledgeDeleteParams, KnowledgeDeleteResult, KnowledgeLinkSetParams,
    KnowledgeListParams, KnowledgeListResult, KnowledgeReadParams, KnowledgeReadResult,
    KnowledgeRevisionsParams, KnowledgeRevisionsResult, KnowledgeSaveParams, KnowledgeSaveResult,
    KnowledgeSearchParams, KnowledgeSearchResults, KnowledgeUpdateParams,
};

use crate::server::{AppServer, internal_error};

const DEFAULT_LIST_LIMIT: usize = 50;
const DEFAULT_SEARCH_LIMIT: usize = 10;

impl AppServer {
    pub(crate) async fn handle_knowledge_list(
        &self,
        params: KnowledgeListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let documents = self
            .knowledge_store()?
            .list(KnowledgeListQuery {
                scope: params.scope,
                kind: params.kind,
                tag: params.tag,
                status: params.status,
                include_archived: params.include_archived,
                limit: params.limit.unwrap_or(DEFAULT_LIST_LIMIT),
            })
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(KnowledgeListResult { documents }).unwrap())
    }

    pub(crate) async fn handle_knowledge_read(
        &self,
        params: KnowledgeReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = self.knowledge_store()?;
        let document = match params.revision {
            Some(revision) => store.get_revision(&params.doc_id, revision).await,
            None => store.get(&params.doc_id).await,
        }
        .map_err(internal_error)?;
        Ok(serde_json::to_value(KnowledgeReadResult { document }).unwrap())
    }

    pub(crate) async fn handle_knowledge_save(
        &self,
        params: KnowledgeSaveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let document = self
            .knowledge_store()?
            .save(KnowledgeSaveRequest {
                scope: params.scope,
                kind: params.kind,
                title: params.title,
                tags: params.tags,
                body: params.body,
                source: KnowledgeSource::User,
            })
            .await
            .map_err(internal_error)?;
        self.runtime
            .emit(RoderEvent::KnowledgeSaved(
                roder_api::events::KnowledgeSaved {
                    document: document.summary(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(KnowledgeSaveResult { document }).unwrap())
    }

    pub(crate) async fn handle_knowledge_update(
        &self,
        params: KnowledgeUpdateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let document = self
            .knowledge_store()?
            .update(KnowledgeUpdateRequest {
                id: params.doc_id,
                title: params.title,
                body: params.body,
                status: params.status,
                tags: params.tags,
                source: KnowledgeSource::User,
            })
            .await
            .map_err(internal_error)?;
        self.runtime
            .emit(RoderEvent::KnowledgeUpdated(
                roder_api::events::KnowledgeUpdated {
                    document: document.summary(),
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(KnowledgeSaveResult { document }).unwrap())
    }

    pub(crate) async fn handle_knowledge_delete(
        &self,
        params: KnowledgeDeleteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let archived = self
            .knowledge_store()?
            .archive(&params.doc_id)
            .await
            .map_err(internal_error)?;
        if archived {
            self.runtime
                .emit(RoderEvent::KnowledgeArchived(
                    roder_api::events::KnowledgeArchived {
                        doc_id: params.doc_id.clone(),
                        timestamp: time::OffsetDateTime::now_utc(),
                    },
                ))
                .await;
        }
        Ok(serde_json::to_value(KnowledgeDeleteResult { archived }).unwrap())
    }

    pub(crate) async fn handle_knowledge_search(
        &self,
        params: KnowledgeSearchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let results = self
            .knowledge_store()?
            .search(KnowledgeQuery {
                scope: params.scope,
                text: params.text,
                kind: params.kind,
                limit: params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT),
                include_global: params.include_global,
            })
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(KnowledgeSearchResults { results }).unwrap())
    }

    pub(crate) async fn handle_knowledge_links_set(
        &self,
        params: KnowledgeLinkSetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let document = self
            .knowledge_store()?
            .set_link(KnowledgeLinkRequest {
                from: params.from.clone(),
                to: params.to.clone(),
                link_type: params.link_type,
                remove: params.remove,
            })
            .await
            .map_err(internal_error)?;
        self.runtime
            .emit(RoderEvent::KnowledgeLinked(
                roder_api::events::KnowledgeLinked {
                    from: params.from,
                    to: params.to,
                    link_type: params.link_type,
                    removed: params.remove,
                    timestamp: time::OffsetDateTime::now_utc(),
                },
            ))
            .await;
        Ok(serde_json::to_value(KnowledgeSaveResult { document }).unwrap())
    }

    pub(crate) async fn handle_knowledge_revisions_list(
        &self,
        params: KnowledgeRevisionsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let revisions = self
            .knowledge_store()?
            .revisions(&params.doc_id)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(KnowledgeRevisionsResult { revisions }).unwrap())
    }

    fn knowledge_store(&self) -> Result<Arc<dyn KnowledgeStore>, JsonRpcError> {
        self.runtime
            .registry()
            .knowledge_stores
            .first()
            .map(|factory| factory.create())
            .ok_or_else(|| JsonRpcError {
                code: -32000,
                message: "No knowledge store is registered".to_string(),
                data: None,
            })
    }
}
