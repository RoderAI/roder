use roder_api::thread_events::ThreadEventSubscribeRequest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadEventSubscribeParams {
    pub thread_id: String,
    #[serde(flatten)]
    pub subscription: ThreadEventSubscribeRequest,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadEventListParams {
    pub thread_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadEventCancelParams {
    pub thread_id: String,
    pub subscription_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadEventPublishParams {
    pub topic: String,
    pub message: String,
}
