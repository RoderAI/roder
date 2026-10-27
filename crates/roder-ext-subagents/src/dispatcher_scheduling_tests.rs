use std::sync::atomic::{AtomicUsize, Ordering};

use roder_api::tools::{
    ToolExecutionBarrier, ToolExecutionMode, ToolExecutor, ToolResult, ToolSpec,
};
use tokio::sync::{Barrier, Notify};

use super::*;

struct ChildProbe {
    mode: ToolExecutionMode,
    pair: Arc<Barrier>,
    writes: Arc<AtomicUsize>,
    entered: Arc<Notify>,
}

#[async_trait::async_trait]
impl ToolExecutor for ChildProbe {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "probe".into(),
            description: "child scheduling probe".into(),
            parameters: serde_json::json!({}),
        }
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        self.mode
    }

    async fn execute(&self, _: ToolExecutionContext, call: ToolCall) -> anyhow::Result<ToolResult> {
        match self.mode {
            ToolExecutionMode::SharedRead => {
                self.pair.wait().await;
                assert_eq!(self.writes.load(Ordering::SeqCst), 0);
            }
            ToolExecutionMode::Exclusive => {
                assert_eq!(self.writes.fetch_add(1, Ordering::SeqCst), 0);
                self.entered.notify_one();
                self.writes.fetch_sub(1, Ordering::SeqCst);
            }
            ToolExecutionMode::Coordinator => unreachable!(),
        }
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "ok".into(),
            data: serde_json::json!({}),
            is_error: false,
        })
    }
}

fn call(id: &str) -> roder_api::inference::ToolCallCompleted {
    roder_api::inference::ToolCallCompleted {
        id: id.into(),
        name: "probe".into(),
        arguments: "{}".into(),
    }
}

#[tokio::test]
async fn nested_tools_share_the_inherited_read_write_barrier() {
    let barrier = Arc::new(ToolExecutionBarrier::default());
    let handles = ToolExecutionHandles {
        execution_barrier: Some(barrier.clone()),
        ..ToolExecutionHandles::default()
    };
    let writes = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let mut reads = ToolRegistry::default();
    reads
        .register(Arc::new(ChildProbe {
            mode: ToolExecutionMode::SharedRead,
            pair: Arc::new(Barrier::new(2)),
            writes: writes.clone(),
            entered: entered.clone(),
        }))
        .unwrap();
    let thread = "thread".to_string();
    let turn = "turn".to_string();
    let (a, b) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            execute_tool(&reads, &thread, &turn, &handles, call("a")),
            execute_tool(&reads, &thread, &turn, &handles, call("b")),
        )
    })
    .await
    .expect("independent child reads must overlap");
    assert!(!a.unwrap().is_error);
    assert!(!b.unwrap().is_error);

    let mut tools = ToolRegistry::default();
    tools
        .register(Arc::new(ChildProbe {
            mode: ToolExecutionMode::Exclusive,
            pair: Arc::new(Barrier::new(1)),
            writes,
            entered: entered.clone(),
        }))
        .unwrap();
    let read_guard = barrier.acquire(ToolExecutionMode::SharedRead).await;
    let task = tokio::spawn(async move {
        execute_tool(&tools, &thread, &turn, &handles, call("mutation")).await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), entered.notified())
            .await
            .is_err(),
        "child writes must not bypass an active parent read"
    );
    drop(read_guard);
    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("releasing the parent read must admit the child mutation")
        .unwrap()
        .unwrap();
    assert!(!result.is_error);
}
