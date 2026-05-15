use std::sync::Arc;

use roder_api::events::EventEnvelope;
use roder_protocol::{JsonRpcRequest, JsonRpcResponse};
use tokio::sync::broadcast;

use crate::AppServer;

#[derive(Clone)]
pub struct LocalAppClient {
    server: Arc<AppServer>,
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
