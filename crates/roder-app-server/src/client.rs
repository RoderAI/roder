use std::sync::Arc;

use async_trait::async_trait;
use roder_api::events::EventEnvelope;
use roder_protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use tokio::sync::broadcast;

use crate::AppServer;

#[async_trait]
pub trait AppClient: Clone + Send + Sync + 'static {
    type EventReceiver: AppEventReceiver;
    type NotificationReceiver: AppNotificationReceiver;

    async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse;
    fn subscribe_events(&self) -> Self::EventReceiver;
    fn subscribe_notifications(&self) -> Self::NotificationReceiver;
}

#[async_trait]
pub trait AppEventReceiver: Send {
    async fn recv(&mut self) -> Result<EventEnvelope, broadcast::error::RecvError>;
}

#[async_trait]
pub trait AppNotificationReceiver: Send {
    async fn recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::RecvError>;
}

#[async_trait]
impl AppEventReceiver for broadcast::Receiver<EventEnvelope> {
    async fn recv(&mut self) -> Result<EventEnvelope, broadcast::error::RecvError> {
        broadcast::Receiver::recv(self).await
    }
}

#[async_trait]
impl AppNotificationReceiver for broadcast::Receiver<JsonRpcNotification> {
    async fn recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::RecvError> {
        broadcast::Receiver::recv(self).await
    }
}

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

    pub fn app_server(&self) -> Arc<AppServer> {
        self.server.clone()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.server.subscribe_events()
    }

    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.server.subscribe_notifications()
    }
}

#[async_trait]
impl AppClient for LocalAppClient {
    type EventReceiver = broadcast::Receiver<EventEnvelope>;
    type NotificationReceiver = broadcast::Receiver<JsonRpcNotification>;

    async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        LocalAppClient::send_request(self, request).await
    }

    fn subscribe_events(&self) -> Self::EventReceiver {
        LocalAppClient::subscribe_events(self)
    }

    fn subscribe_notifications(&self) -> Self::NotificationReceiver {
        LocalAppClient::subscribe_notifications(self)
    }
}
