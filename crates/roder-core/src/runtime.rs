use std::sync::Arc;
use futures::StreamExt;
use roder_api::events::*;
use roder_api::inference::{AgentInferenceRequest, InferenceEngine, InferenceTurnContext};
use time::OffsetDateTime;
use crate::bus::EventBus;

pub struct Runtime {
    pub bus: EventBus,
    pub engine: Arc<dyn InferenceEngine>,
}

impl Runtime {
    pub fn new(engine: Arc<dyn InferenceEngine>) -> Self {
        let bus = EventBus::new(1024);
        
        bus.emit(RoderEvent::RuntimeStarted(RuntimeStarted {
            timestamp: OffsetDateTime::now_utc(),
        }));
        
        Self { bus, engine }
    }

    pub async fn start_turn(&self, thread_id: ThreadId, request: AgentInferenceRequest) -> anyhow::Result<TurnId> {
        let turn_id = uuid::Uuid::new_v4().to_string();

        self.bus.emit(RoderEvent::TurnStarted(TurnStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
        }));

        self.bus.emit(RoderEvent::InferenceStarted(InferenceStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            engine_id: self.engine.id(),
            timestamp: OffsetDateTime::now_utc(),
        }));

        let ctx = InferenceTurnContext {
            turn_id: &turn_id,
        };

        let mut stream = self.engine.stream_turn(ctx, request).await?;

        let bus = self.bus.clone();
        let thread_id_clone = thread_id.clone();
        let turn_id_clone = turn_id.clone();

        tokio::spawn(async move {
            while let Some(res) = stream.next().await {
                match res {
                    Ok(event) => {
                        bus.emit(RoderEvent::InferenceEventReceived(InferenceEventReceived {
                            thread_id: thread_id_clone.clone(),
                            turn_id: turn_id_clone.clone(),
                            event,
                            timestamp: OffsetDateTime::now_utc(),
                        }));
                    }
                    Err(err) => {
                        bus.emit(RoderEvent::TurnFailed(TurnFailed {
                            thread_id: thread_id_clone.clone(),
                            turn_id: turn_id_clone.clone(),
                            error: err.to_string(),
                            timestamp: OffsetDateTime::now_utc(),
                        }));
                        return;
                    }
                }
            }

            bus.emit(RoderEvent::TurnCompleted(TurnCompleted {
                thread_id: thread_id_clone,
                turn_id: turn_id_clone,
                timestamp: OffsetDateTime::now_utc(),
            }));
        });

        Ok(turn_id)
    }
}
