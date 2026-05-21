use std::path::Path;

use roder_protocol::{
    EvalReportReadParams, EvalReportReadResult, EvalReportSummary, EvalReportsListParams,
    EvalReportsListResult, JsonRpcError,
};

const DEFAULT_MAX_REPORT_BYTES: usize = 64 * 1024;
const MAX_REPORT_BYTES: usize = 256 * 1024;

pub(crate) fn handle_eval_reports_list(
    workspace: &Path,
    params: EvalReportsListParams,
) -> Result<serde_json::Value, JsonRpcError> {
    let report_dir = eval_report_dir(workspace);
    let mut reports = roder_evals::list_eval_reports(&report_dir)
        .map_err(|err| internal_error(format!("failed to list eval reports: {err:#}")))?;
    if let Some(limit) = params.limit {
        reports.truncate(limit);
    }
    Ok(serde_json::to_value(EvalReportsListResult {
        reports: reports.into_iter().map(summary).collect(),
    })
    .unwrap())
}

pub(crate) fn handle_eval_report_read(
    workspace: &Path,
    params: EvalReportReadParams,
) -> Result<serde_json::Value, JsonRpcError> {
    if params.report_id.contains("..")
        || params.report_id.starts_with('/')
        || params.report_id.starts_with('\\')
    {
        return Err(invalid_params(
            "report_id must be a report id from eval/reports/list",
        ));
    }
    let report_dir = eval_report_dir(workspace);
    let max_bytes = params
        .max_bytes
        .unwrap_or(DEFAULT_MAX_REPORT_BYTES)
        .min(MAX_REPORT_BYTES);
    let report = roder_evals::read_eval_report(&report_dir, &params.report_id, max_bytes)
        .map_err(|err| invalid_params(format!("failed to read eval report: {err:#}")))?;
    Ok(serde_json::to_value(EvalReportReadResult {
        summary: summary(report.summary),
        markdown: report.markdown,
        truncated: report.truncated,
    })
    .unwrap())
}

fn eval_report_dir(workspace: &Path) -> std::path::PathBuf {
    workspace.join("evals").join("reports")
}

fn summary(report: roder_evals::EvalReportSummary) -> EvalReportSummary {
    EvalReportSummary {
        id: report.id,
        suite_id: report.suite_id,
        fixture_count: report.fixture_count,
        passed: report.passed,
        failed: report.failed,
        generated_at: report.generated_at,
    }
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = err.to_string();
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_core::fake_provider::FakeInferenceEngine;
    use roder_core::{Runtime, RuntimeConfig};
    use roder_protocol::{EvalReportReadParams, EvalReportsListParams};

    use super::*;

    #[test]
    fn eval_report_methods_list_and_read_bounded_reports() {
        let root = std::env::temp_dir().join(format!("roder-app-evals-{}", uuid::Uuid::new_v4()));
        let report_dir = root.join("evals").join("reports");
        let report = roder_evals::EvalSuiteReport {
            suite_id: "tool-calls".to_string(),
            fixture_dir: root.join("evals").join("fixtures").join("tool-calls"),
            output_dir: report_dir.clone(),
            offline: true,
            generated_at: time::OffsetDateTime::UNIX_EPOCH,
            results: Vec::new(),
        };
        roder_evals::write_eval_report_files(&report, &report_dir).unwrap();

        let listed: EvalReportsListResult = serde_json::from_value(
            handle_eval_reports_list(&root, EvalReportsListParams { limit: Some(10) }).unwrap(),
        )
        .unwrap();
        assert_eq!(listed.reports.len(), 1);
        assert_eq!(listed.reports[0].id, "eval-run");

        let read: roder_protocol::EvalReportReadResult = serde_json::from_value(
            handle_eval_report_read(
                &root,
                EvalReportReadParams {
                    report_id: "eval-run".to_string(),
                    max_bytes: Some(32),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(read.summary.suite_id, "tool-calls");
        assert!(read.truncated);

        let err = handle_eval_report_read(
            &root,
            EvalReportReadParams {
                report_id: "../secret".to_string(),
                max_bytes: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, -32602);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn eval_report_json_rpc_methods_list_and_read_bounded_reports() {
        let root =
            std::env::temp_dir().join(format!("roder-app-evals-rpc-{}", uuid::Uuid::new_v4()));
        let report_dir = root.join("evals").join("reports");
        let report = roder_evals::EvalSuiteReport {
            suite_id: "tool-calls".to_string(),
            fixture_dir: root.join("evals").join("fixtures").join("tool-calls"),
            output_dir: report_dir.clone(),
            offline: true,
            generated_at: time::OffsetDateTime::UNIX_EPOCH,
            results: Vec::new(),
        };
        roder_evals::write_eval_report_files(&report, &report_dir).unwrap();
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    workspace: Some(root.display().to_string()),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let client = crate::LocalAppClient::new(Arc::new(crate::AppServer::new(runtime)));

        let listed = client
            .send_request(roder_protocol::JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("eval/list")),
                method: "eval/reports/list".to_string(),
                params: Some(
                    serde_json::to_value(EvalReportsListParams { limit: Some(10) }).unwrap(),
                ),
            })
            .await;
        assert!(listed.error.is_none(), "{:?}", listed.error);
        let listed: EvalReportsListResult = serde_json::from_value(listed.result.unwrap()).unwrap();
        assert_eq!(listed.reports[0].id, "eval-run");

        let read = client
            .send_request(roder_protocol::JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("eval/read")),
                method: "eval/report/read".to_string(),
                params: Some(
                    serde_json::to_value(EvalReportReadParams {
                        report_id: "eval-run".to_string(),
                        max_bytes: Some(32),
                    })
                    .unwrap(),
                ),
            })
            .await;
        assert!(read.error.is_none(), "{:?}", read.error);
        let read: EvalReportReadResult = serde_json::from_value(read.result.unwrap()).unwrap();
        assert_eq!(read.summary.suite_id, "tool-calls");
        assert!(read.truncated);
        let _ = std::fs::remove_dir_all(root);
    }
}
