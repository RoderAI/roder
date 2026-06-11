//! App-server `stats/*` handlers for local usage analytics (roadmap
//! phase 73). Query windows and result sizes are bounded by the analytics
//! crate's hard `MAX_LIMIT`; workspace labels are transformed by the
//! configured label mode before leaving the store, and no response carries
//! prompt/assistant/tool-output text.

use std::path::PathBuf;
use std::sync::Arc;

use roder_protocol::JsonRpcError;
use roder_protocol::stats::{
    StatsBackfillParams, StatsBackfillResult, StatsExportParams, StatsExportResult,
    StatsQueryParams, StatsSessionsResult, StatsSummaryResult, StatsTokensParams,
    StatsTokensResult, StatsToolsParams, StatsToolsResult,
};
use roder_usage_analytics::{
    AnalyticsStore, BackfillOptions, MAX_LIMIT, StatsFilter, TokenGroup, WorkspaceLabelMode,
    backfill_analytics, sort_tool_summaries,
};

use crate::server::{AppServer, internal_error};

/// Test/host override for the analytics data directory; production resolves
/// the Roder config dir.
pub const STATS_DATA_DIR_ENV: &str = "RODER_STATS_DATA_DIR";

fn stats_data_dir() -> PathBuf {
    std::env::var_os(STATS_DATA_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(roder_config::config_dir)
}

fn open_store() -> Result<Arc<AnalyticsStore>, JsonRpcError> {
    let analytics = roder_config::load_config()
        .map(|config| config.analytics.unwrap_or_default())
        .unwrap_or_default();
    let mode =
        WorkspaceLabelMode::parse(&analytics.workspace_labels).map_err(internal_error)?;
    let base = stats_data_dir();
    let path = analytics
        .store
        .filter(|_| std::env::var_os(STATS_DATA_DIR_ENV).is_none())
        .map(PathBuf::from)
        .unwrap_or_else(|| AnalyticsStore::default_path(&base));
    Ok(Arc::new(
        AnalyticsStore::open(&path, mode).map_err(internal_error)?,
    ))
}

/// Bounds caller-provided limits to the analytics hard cap; oversized
/// limits are a typed validation error rather than a silent truncation.
fn bounded(filter: StatsFilter) -> Result<StatsFilter, JsonRpcError> {
    if let Some(limit) = filter.limit
        && limit > MAX_LIMIT
    {
        return Err(JsonRpcError {
            code: -32602,
            message: format!("limit {limit} exceeds the maximum of {MAX_LIMIT}"),
            data: None,
        });
    }
    Ok(filter)
}

impl AppServer {
    pub(crate) async fn handle_stats_summary(
        &self,
        params: StatsQueryParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = open_store()?;
        let summary = store
            .usage_summary(&bounded(params.filter)?)
            .map_err(internal_error)?;
        serde_json::to_value(StatsSummaryResult { summary }).map_err(internal_error)
    }

    pub(crate) async fn handle_stats_tools(
        &self,
        params: StatsToolsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = open_store()?;
        let mut tools = store
            .tool_summaries(&bounded(params.filter)?)
            .map_err(internal_error)?;
        sort_tool_summaries(&mut tools, params.sort.as_deref().unwrap_or("calls"));
        serde_json::to_value(StatsToolsResult { tools }).map_err(internal_error)
    }

    pub(crate) async fn handle_stats_tokens(
        &self,
        params: StatsTokensParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = open_store()?;
        let group = TokenGroup::parse(params.group.as_deref().unwrap_or("day"))
            .map_err(|error| JsonRpcError {
                code: -32602,
                message: error.to_string(),
                data: None,
            })?;
        let rows = store
            .token_summaries(group, &bounded(params.filter)?)
            .map_err(internal_error)?;
        serde_json::to_value(StatsTokensResult { rows }).map_err(internal_error)
    }

    pub(crate) async fn handle_stats_sessions(
        &self,
        params: StatsQueryParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = open_store()?;
        let sessions = store
            .session_summaries(&bounded(params.filter)?)
            .map_err(internal_error)?;
        serde_json::to_value(StatsSessionsResult { sessions }).map_err(internal_error)
    }

    pub(crate) async fn handle_stats_backfill(
        &self,
        params: StatsBackfillParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = open_store()?;
        let thread_root = stats_data_dir().join("threads");
        let report = tokio::task::spawn_blocking(move || {
            let report = backfill_analytics(
                &thread_root,
                &store,
                BackfillOptions {
                    rebuild: params.rebuild,
                    best_effort: params.best_effort,
                },
            )?;
            let rollup_rows = store.refresh_daily_rollups()?;
            anyhow::Ok((report, rollup_rows))
        })
        .await
        .map_err(internal_error)?
        .map_err(internal_error)?;
        let (report, rollup_rows) = report;
        serde_json::to_value(StatsBackfillResult {
            files_scanned: report.files_scanned,
            lines_ingested: report.lines_ingested,
            lines_skipped_by_offset: report.lines_skipped_by_offset,
            sessions_enriched: report.sessions_enriched,
            parse_error_count: report.parse_errors.len() as u64,
            rollup_rows,
        })
        .map_err(internal_error)
    }

    pub(crate) async fn handle_stats_export(
        &self,
        params: StatsExportParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = open_store()?;
        let output_path = params.output_path.clone();
        let path = PathBuf::from(&output_path);
        let records = tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::File::create(&path)?;
            store.export_jsonl(&mut file)
        })
        .await
        .map_err(internal_error)?
        .map_err(internal_error)?;
        serde_json::to_value(StatsExportResult {
            output_path,
            records,
        })
        .map_err(internal_error)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_core::fake_provider::FakeInferenceEngine;
    use roder_core::{Runtime, RuntimeConfig};
    use roder_protocol::JsonRpcRequest;
    use roder_usage_analytics::ToolCallRecord;

    use super::*;
    use crate::client::AppClient;
    use crate::{AppServer, LocalAppClient};

    fn seeded_data_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "roder-app-server-stats-{}",
            uuid::Uuid::new_v4()
        ));
        let store = AnalyticsStore::open(
            &AnalyticsStore::default_path(&dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap();
        for (index, duration) in (1..=20).enumerate() {
            store
                .upsert_tool_call(&ToolCallRecord {
                    thread_id: "t1".into(),
                    turn_id: "u1".into(),
                    tool_id: format!("call-{index}"),
                    tool_name: Some("read_file".into()),
                    started_at_ms: Some(1_000),
                    completed_at_ms: Some(1_000 + duration),
                    duration_ms: Some(duration),
                    status: "success".into(),
                    is_error: false,
                })
                .unwrap();
        }
        dir
    }

    fn client() -> LocalAppClient {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime =
            Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
        LocalAppClient::new(Arc::new(AppServer::new(runtime)))
    }

    async fn call(
        client: &LocalAppClient,
        method: &str,
        params: serde_json::Value,
    ) -> roder_protocol::JsonRpcResponse {
        client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: method.to_string(),
                params: Some(params),
            })
            .await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stats_methods_return_query_aggregates_and_bound_limits() {
        let data_dir = seeded_data_dir();
        // Env-based data-dir override; tests in this binary that touch
        // stats must run in this single test to avoid env races.
        unsafe { std::env::set_var(STATS_DATA_DIR_ENV, &data_dir) };

        let client = client();
        let response = call(&client, "stats/tools", serde_json::json!({})).await;
        assert!(response.error.is_none(), "{:?}", response.error);
        let result: StatsToolsResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.tools.len(), 1);
        assert_eq!(result.tools[0].tool_name, "read_file");
        assert_eq!(result.tools[0].call_count, 20);
        assert_eq!(result.tools[0].p95_duration_ms, Some(19));

        // Same aggregates as the direct query path the CLI uses.
        let store = AnalyticsStore::open(
            &AnalyticsStore::default_path(&data_dir),
            WorkspaceLabelMode::FullPath,
        )
        .unwrap();
        let direct = store.tool_summaries(&StatsFilter::default()).unwrap();
        assert_eq!(direct[0].p95_duration_ms, result.tools[0].p95_duration_ms);

        let response = call(&client, "stats/summary", serde_json::json!({})).await;
        assert!(response.error.is_none());
        let summary: StatsSummaryResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(summary.summary.tool_call_count, 20);

        // Oversized limits are a typed validation error.
        let response = call(
            &client,
            "stats/tools",
            serde_json::json!({ "filter": { "limit": 10_000 } }),
        )
        .await;
        let error = response.error.expect("limit validation error");
        assert_eq!(error.code, -32602);
        assert!(error.message.contains("exceeds the maximum"), "{error:?}");

        // Export writes a server-side artifact, never an inline payload.
        let export_path = data_dir.join("export.jsonl");
        let response = call(
            &client,
            "stats/export",
            serde_json::json!({ "outputPath": export_path.display().to_string() }),
        )
        .await;
        assert!(response.error.is_none(), "{:?}", response.error);
        let result: StatsExportResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.records, 20);
        let exported = std::fs::read_to_string(&export_path).unwrap();
        assert!(exported.contains("\"kind\":\"tool_call\""));
        for forbidden in ["output", "arguments", "prompt"] {
            assert!(!exported.contains(forbidden), "export contains {forbidden}");
        }

        unsafe { std::env::remove_var(STATS_DATA_DIR_ENV) };
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
