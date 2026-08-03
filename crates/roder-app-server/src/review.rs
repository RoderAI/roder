//! `review/*` methods: run a read-only review sub-turn and hand its findings
//! to a review publisher.
//!
//! Publishers are extension services, so nothing here knows about GitHub or any
//! other platform: [`Runtime::publish_review`] resolves one by id from the
//! extension registry and calls it through the trait. With no publisher
//! installed, `review/publish` fails with a configuration error rather than
//! pretending to succeed.

use roder_api::review::ReviewDelivery;
use roder_core::review::{PublishReviewError, PublishReviewRequest, StartReviewRequest};
use roder_protocol::{
    JsonRpcError, ReviewPublishParams, ReviewPublishResult, ReviewPublishersListResult,
    ReviewStartParams, ReviewStartResult,
};

use crate::server::{AppServer, internal_error, invalid_params_error};

impl AppServer {
    pub(crate) async fn handle_review_start(
        &self,
        params: ReviewStartParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if params.publish.is_some() && params.delivery == ReviewDelivery::Detached {
            return Err(invalid_params_error(
                "publish requires inline delivery; detached callers should call review/publish when review/completed arrives",
            ));
        }
        let workspace = self.review_workspace(&params.thread_id).await?;
        let request = StartReviewRequest {
            thread_id: params.thread_id,
            target: params.target,
            workspace,
            provider_override: None,
            model_override: None,
        };

        let result = match params.delivery {
            ReviewDelivery::Detached => {
                let handle = self
                    .runtime
                    .start_review(request)
                    .await
                    .map_err(internal_error)?;
                ReviewStartResult {
                    turn_id: handle.turn_id,
                    review_id: handle.review_id,
                    review_thread_id: handle.review_thread_id,
                    output: None,
                    published: None,
                }
            }
            ReviewDelivery::Inline => {
                let completion = self
                    .runtime
                    .run_review(request)
                    .await
                    .map_err(internal_error)?;
                let published = match params.publish {
                    Some(destination) => Some(
                        self.publish_review(PublishReviewRequest {
                            review_id: completion.handle.review_id.clone(),
                            destination,
                            selected: None,
                            dry_run: false,
                        })
                        .await?,
                    ),
                    None => None,
                };
                ReviewStartResult {
                    turn_id: completion.handle.turn_id,
                    review_id: completion.handle.review_id,
                    review_thread_id: completion.handle.review_thread_id,
                    output: Some(completion.output),
                    published,
                }
            }
        };
        Ok(serde_json::to_value(result).unwrap())
    }

    /// The directory the review diffs. Threads created in this process are not
    /// always readable back from the store (an in-memory registry keeps none),
    /// so this falls back to the protocol thread map the same way `thread/read`
    /// does.
    async fn review_workspace(&self, thread_id: &str) -> Result<String, JsonRpcError> {
        if let Some(metadata) = self
            .runtime
            .load_thread_metadata(thread_id)
            .await
            .map_err(internal_error)?
        {
            return Ok(metadata.workspace);
        }
        self.protocol_threads
            .read()
            .await
            .get(thread_id)
            .map(|thread| thread.cwd.clone())
            .ok_or_else(|| invalid_params_error(format!("thread {thread_id} was not found")))
    }

    pub(crate) async fn handle_review_publish(
        &self,
        params: ReviewPublishParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut destination = params.destination.unwrap_or_default();
        if params.publisher_id.is_some() {
            destination.publisher_id = params.publisher_id;
        }
        let result = self
            .publish_review(PublishReviewRequest {
                review_id: params.review_id,
                destination,
                selected: params.finding_indexes,
                dry_run: params.dry_run,
            })
            .await?;
        Ok(serde_json::to_value(result).unwrap())
    }

    pub(crate) async fn handle_review_publishers_list(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let publishers = self
            .runtime
            .registry
            .review_publishers()
            .iter()
            .map(|publisher| publisher.descriptor())
            .collect();
        Ok(serde_json::to_value(ReviewPublishersListResult { publishers }).unwrap())
    }

    /// Anything the caller could have got right — an unknown review, an
    /// out-of-range finding, an uninstalled publisher — is a `-32602`; a remote
    /// refusal is not.
    async fn publish_review(
        &self,
        request: PublishReviewRequest,
    ) -> Result<ReviewPublishResult, JsonRpcError> {
        self.runtime
            .publish_review(request)
            .await
            .map_err(|error| match error {
                PublishReviewError::InvalidRequest(message) => invalid_params_error(message),
                PublishReviewError::Publish(error) => internal_error(error),
            })
    }
}
