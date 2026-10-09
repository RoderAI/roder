use async_trait::async_trait;
use roder_api::policy_mode::PolicyMode;
use roder_api::remote_runner::*;
use roder_api::tools::*;
use roder_ext_cua::{CuaConfig, CuaTarget, CuaToolContributor, CuaTransport, DriverReply};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

const PNG: &str = include_str!("../capture.b64");
#[derive(Default)]
pub(super) struct Driver {
    pub(super) calls: Mutex<Vec<(String, Value)>>,
    pub(super) fail_input: AtomicBool,
    pub(super) fail_capture: AtomicBool,
    pub(super) pending_input: AtomicBool,
    pub(super) multiple_windows: AtomicBool,
    pub(super) scaled_desktop: AtomicBool,
}
#[async_trait]
impl CuaTransport for Driver {
    async fn call(
        &self,
        _: CuaTarget<'_>,
        tool: &str,
        args: Value,
        _: u64,
    ) -> anyhow::Result<DriverReply> {
        let mut calls = self.calls.lock().await;
        calls.push((tool.into(), args.clone()));
        if matches!(tool, "get_window_state" | "get_desktop_state") {
            let label = args["session"].as_str().unwrap();
            let capture_id = format!("{label}-capture-{}", calls.len());
            let snapshot_id = format!("s{:08x}", calls.len());
            let mut observation = json!({"capture_id":capture_id,"snapshot_id":snapshot_id,
                "screenshot_width":2,"screenshot_height":2,"screenshot_png_b64":if self.fail_capture.load(Ordering::SeqCst) { "bad" } else { PNG }});
            if tool == "get_desktop_state" && self.scaled_desktop.load(Ordering::SeqCst) {
                observation["screenshot_original_width"] = json!(20);
                observation["screenshot_original_height"] = json!(10);
            }
            Ok(DriverReply {
                observation,
                is_error: false,
            })
        } else if tool == "list_windows" {
            let mut windows = vec![json!({"pid":10,"window_id":20})];
            if self.multiple_windows.load(Ordering::SeqCst) {
                windows.push(json!({"pid":10,"window_id":21}));
            }
            Ok(DriverReply {
                observation: json!({"windows":windows}),
                is_error: false,
            })
        } else {
            if self.pending_input.load(Ordering::SeqCst) {
                drop(calls);
                std::future::pending::<()>().await;
            }
            Ok(DriverReply {
                observation: json!({"code":"background_unavailable"}),
                is_error: self.fail_input.load(Ordering::SeqCst),
            })
        }
    }
}
pub(super) struct Runner {
    id: String,
    pub(super) requests: Mutex<Vec<RunnerCommandRequest>>,
    pub(super) cancelled: AtomicBool,
    pub(super) pending: bool,
}
impl Runner {
    pub(super) fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            requests: Mutex::new(vec![]),
            cancelled: AtomicBool::new(false),
            pending: false,
        }
    }
}
#[async_trait]
impl RemoteRunnerSession for Runner {
    fn state(&self) -> RunnerSessionState {
        RunnerSessionState {
            provider_id: "fixture".into(),
            session_id: self.id.clone(),
            destination_id: "desktop".into(),
            snapshot: None,
            metadata: json!({}),
        }
    }
    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        let id = request.command_id.clone();
        let path = request.args.get(2).cloned();
        self.requests.lock().await.push(request);
        if self.pending {
            std::future::pending::<()>().await;
        }
        Ok(RunnerCommandResult {
            command_id: id,
            exit_code: Some(0),
            stdout: json!({"driver_version":"0.34.0","result_file":path}).to_string(),
            stderr: String::new(),
        })
    }
    async fn cancel_command(&self, _: &String) -> anyhow::Result<bool> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(true)
    }
    async fn read_file(
        &self,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        Ok(RunnerFileReadResult {
            path: request.path,
            contents: json!({"driver_version":"0.34.0","is_error":false,"observation":{}})
                .to_string()
                .into_bytes(),
        })
    }
    async fn write_file(&self, _: RunnerFileWriteRequest) -> anyhow::Result<()> {
        anyhow::bail!("unused")
    }
    async fn expose_port(&self, _: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        anyhow::bail!("unused")
    }
    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        Ok(None)
    }
    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
pub(super) fn context(thread: &str, runner: &str) -> ToolExecutionContext {
    ToolExecutionContext::new(thread, "turn", PolicyMode::Bypass).with_remote_workspace(Arc::new(
        RemoteWorkspace {
            session: Arc::new(Runner::new(runner)),
            root: "/workspace".into(),
            read_roots: vec![],
        },
    ))
}
pub(super) fn registry(driver: Arc<Driver>) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    CuaToolContributor::with_transport(CuaConfig::default(), driver)
        .contribute(&mut registry)
        .unwrap();
    registry
}
pub(super) async fn call(
    registry: &ToolRegistry,
    ctx: ToolExecutionContext,
    name: &str,
    args: Value,
) -> ToolResult {
    registry
        .get(name)
        .unwrap()
        .execute(
            ctx.clone(),
            ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments: args,
                raw_arguments: String::new(),
                thread_id: ctx.thread_id,
                turn_id: ctx.turn_id,
            },
        )
        .await
        .unwrap()
}
pub(super) async fn observe(registry: &ToolRegistry, ctx: ToolExecutionContext) -> ToolResult {
    call(
        registry,
        ctx,
        "cua_get_window_state",
        json!({"pid":10,"window_id":20}),
    )
    .await
}
