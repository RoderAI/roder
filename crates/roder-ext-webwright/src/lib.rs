mod artifacts;
mod errors;
mod export;
mod extension;
mod playwright;
#[cfg(test)]
mod playwright_tests;
mod redaction;
mod run;
mod showcase;
mod task;
mod tools;
#[cfg(test)]
mod tools_tests;
mod verify;
mod visual;
mod workspace;
#[cfg(test)]
mod workspace_tests;

pub use artifacts::{
    WebwrightCriticalPoint, WebwrightLogSummary, WebwrightPlanSummary, WebwrightScriptSummary,
    WebwrightSelfReflectSummary,
};
pub use export::{WebwrightExportResult, export_workspace};
pub use extension::WebwrightExtension;
pub use playwright::{
    DependencyCheckMode, DependencyReport, WebwrightSetupOptions, WebwrightSetupReport,
    WebwrightSetupStepReport, preflight_local_dependencies, setup_webwright_runtime,
};
#[cfg(test)]
pub(crate) use playwright::{
    preflight_local_dependencies_in_roder_home, setup_webwright_runtime_in_roder_home,
};
pub use showcase::{
    ReportResult, ReportSection, WebwrightReport, WebwrightTaskDefinition, render_report_text,
};
pub use task::{WEBWRIGHT_TASK_EXECUTOR_ID, WebwrightTaskExecutor, WebwrightTaskInput};
pub use tools::{
    WEBWRIGHT_ALLOCATE_RUN_TOOL, WEBWRIGHT_LINT_SCRIPT_TOOL, WEBWRIGHT_LIST_ARTIFACTS_TOOL,
    WEBWRIGHT_PREPARE_WORKSPACE_TOOL, WEBWRIGHT_READ_LOG_TAIL_TOOL, WEBWRIGHT_RUN_SCRIPT_TOOL,
    WEBWRIGHT_SUMMARIZE_VERIFICATION_TOOL, WEBWRIGHT_VERIFY_RUN_TOOL, WebwrightToolContributor,
};
pub use verify::{VerificationCheck, VerificationResult, verify_workspace};
pub use visual::{
    VISUAL_JUDGE_DIR, WebwrightPreparedVisualJudge, WebwrightVisualJudgeRecord,
    prepare_visual_judge, store_visual_judge_record,
};
pub use workspace::{
    WebwrightManifest, WebwrightMode, WebwrightRunSummary, WebwrightWorkspace,
    WebwrightWorkspaceSummary, sanitize_task_id,
};
