use std::sync::Arc;
use tokio::sync::broadcast;
use roder_protocol::{JsonRpcRequest, JsonRpcResponse};
use roder_api::events::EventEnvelope;
use crate::AppServer;

pub struct LocalAppClient {
    pub server: Arc<AppServer>,
}

impl LocalAppClient {
    pub fn new(server: Arc<AppServer>) -> Self {
        Self { server }
    }

    pub async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        self.server.handle_request(request).await
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.server.subscribe_events()
    }
}
