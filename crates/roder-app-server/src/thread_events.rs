use crate::server::{AppServer, invalid_params_error};
use roder_protocol::{
    JsonRpcError, ThreadEventCancelParams, ThreadEventListParams, ThreadEventPublishParams,
    ThreadEventSubscribeParams,
};
use serde_json::{Value, json};

impl AppServer {
    pub(crate) async fn handle_thread_event_subscribe(
        &self,
        p: ThreadEventSubscribeParams,
    ) -> Result<Value, JsonRpcError> {
        let subscription = self
            .runtime
            .subscribe_thread_event(&p.thread_id, p.subscription)
            .await
            .map_err(invalid_params_error)?;
        Ok(json!({"subscription":subscription}))
    }
    pub(crate) async fn handle_thread_event_list(
        &self,
        p: ThreadEventListParams,
    ) -> Result<Value, JsonRpcError> {
        Ok(
            json!({"subscriptions":self.runtime.list_thread_event_subscriptions(&p.thread_id).await}),
        )
    }
    pub(crate) async fn handle_thread_event_cancel(
        &self,
        p: ThreadEventCancelParams,
    ) -> Result<Value, JsonRpcError> {
        Ok(
            json!({"cancelled":self.runtime.cancel_thread_event_subscription(&p.thread_id,&p.subscription_id).await}),
        )
    }
    pub(crate) async fn handle_thread_event_publish(
        &self,
        p: ThreadEventPublishParams,
    ) -> Result<Value, JsonRpcError> {
        Ok(
            json!({"queuedSubscriptions":self.runtime.publish_thread_event(&p.topic,&p.message).await.map_err(invalid_params_error)?}),
        )
    }
}
