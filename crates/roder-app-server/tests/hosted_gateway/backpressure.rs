use super::*;
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId, ToolOutputDelta};
use roder_api::thread::{ThreadMetadata, ThreadSnapshot, ThreadStore, ThreadStoreFactory};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct GatedList {
    entered: Notify,
    release: Notify,
    completed: Notify,
}

#[async_trait::async_trait]
impl ThreadStore for GatedList {
    fn id(&self) -> String {
        "gated-list".into()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        self.entered.notify_one();
        self.release.notified().await;
        self.completed.notify_one();
        Ok(vec![])
    }

    async fn load_thread(&self, _: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        Ok(None)
    }

    async fn append_event(&self, _: &ThreadId, _: &EventEnvelope) -> anyhow::Result<()> {
        Ok(())
    }
}

struct GatedListFactory(Arc<GatedList>);

impl ThreadStoreFactory for GatedListFactory {
    fn id(&self) -> String {
        "gated-list".into()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        self.0.clone()
    }
}

#[tokio::test]
async fn overflow_closes_socket_while_admitted_request_finishes() {
    let gate = Arc::new(GatedList::default());
    let factory_gate = gate.clone();
    let pool = Arc::new(HostedRuntimePool::new(
        HostedRuntimeProfile {
            data_root: temp_dir("outbound-backpressure"),
            allow_local_workspaces: true,
            idle_ttl: Duration::from_secs(300),
        },
        Arc::new(move |_, _| {
            let gate = factory_gate.clone();
            Box::pin(async move {
                let mut builder = ExtensionRegistryBuilder::new();
                builder.inference_engine(Arc::new(FakeInferenceEngine));
                builder.thread_store_factory(Arc::new(GatedListFactory(gate)));
                let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default())?;
                Ok(Arc::new(AppServer::new(Arc::new(runtime))))
            })
        }),
    ));
    let fixture = fixture_with_pool(
        pool.clone(),
        RateLimitConfig::default(),
        Arc::new(AllowAllHostedRequestPolicy),
    )
    .await;
    let mut socket = connect(&fixture.url, "rk_test_tenant_a_writer")
        .await
        .unwrap();
    assert!(
        call(&mut socket, "initialize", serde_json::json!({}))
            .await
            .error
            .is_none()
    );
    socket
        .send(Message::Text(
            r#"{"jsonrpc":"2.0","id":"blocked","method":"thread/list","params":{}}"#.into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .expect("the request should enter the store");
    let lease = pool.lease("tenant-a").await.unwrap();
    lease
        .server
        .runtime
        .emit(RoderEvent::ToolOutputDelta(ToolOutputDelta {
            thread_id: "app-server".into(),
            turn_id: "test-turn".into(),
            tool_id: "test-tool".into(),
            delta: "x".repeat(17 * 1024 * 1024),
            timestamp: time::OffsetDateTime::now_utc(),
        }))
        .await;
    let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .expect("the socket should close without waiting for the blocked request");
    assert!(matches!(
        message,
        None | Some(Err(_)) | Some(Ok(Message::Close(_)))
    ));
    gate.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), gate.completed.notified())
        .await
        .expect("the admitted request should still finish");
    drop(lease);
    fixture.controller.stop().await.unwrap();
}
