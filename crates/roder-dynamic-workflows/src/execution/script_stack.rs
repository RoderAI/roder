use crate::host_api::WorkflowExecution;
use crate::model::{
    WorkflowRunInput, WorkflowRuntimeError, WorkflowRuntimeErrorKind, WorkflowRuntimeResult,
};
use crate::runner::WorkflowScriptRuntime;

const WORKFLOW_SCRIPT_STACK_BYTES: usize = 32 * 1024 * 1024;

pub(crate) async fn run_script_on_dedicated_stack(
    runtime: WorkflowScriptRuntime,
    source: String,
    input: WorkflowRunInput,
) -> WorkflowRuntimeResult<WorkflowExecution> {
    tokio::task::spawn_blocking(move || run_script_thread(runtime, source, input))
        .await
        .map_err(|err| {
            WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::ScriptExecution,
                format!("workflow script task failed: {err}"),
            )
        })?
}

fn run_script_thread(
    runtime: WorkflowScriptRuntime,
    source: String,
    input: WorkflowRunInput,
) -> WorkflowRuntimeResult<WorkflowExecution> {
    std::thread::Builder::new()
        .name("roder-workflow-script".to_string())
        .stack_size(WORKFLOW_SCRIPT_STACK_BYTES)
        .spawn(move || runtime.run(&source, input))
        .map_err(|err| {
            WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::ScriptExecution,
                format!("failed to launch workflow script thread: {err}"),
            )
        })?
        .join()
        .map_err(|_| {
            WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::ScriptExecution,
                "workflow script thread panicked",
            )
        })?
}
