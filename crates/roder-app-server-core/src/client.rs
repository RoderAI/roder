use async_trait::async_trait;
use roder_api::events::EventEnvelope;
use roder_protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use tokio::sync::broadcast;

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
    fn try_recv(&mut self) -> Result<EventEnvelope, broadcast::error::TryRecvError>;
}

#[async_trait]
pub trait AppNotificationReceiver: Send {
    async fn recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::RecvError>;
    fn try_recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::TryRecvError>;
}

#[async_trait]
impl AppEventReceiver for broadcast::Receiver<EventEnvelope> {
    async fn recv(&mut self) -> Result<EventEnvelope, broadcast::error::RecvError> {
        broadcast::Receiver::recv(self).await
    }

    fn try_recv(&mut self) -> Result<EventEnvelope, broadcast::error::TryRecvError> {
        broadcast::Receiver::try_recv(self)
    }
}

#[async_trait]
impl AppNotificationReceiver for broadcast::Receiver<JsonRpcNotification> {
    async fn recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::RecvError> {
        broadcast::Receiver::recv(self).await
    }

    fn try_recv(&mut self) -> Result<JsonRpcNotification, broadcast::error::TryRecvError> {
        broadcast::Receiver::try_recv(self)
    }
}
