//! Local long-term usage analytics for Roder (roadmap phase 73).
//!
//! Projects selected runtime events into a queryable SQLite database under
//! the Roder data directory (`<data-dir>/analytics/usage.sqlite3`).
//! Analytics are local-only, passive, idempotent, and redacted by default:
//! prompts, assistant text, tool output bodies, and secrets never reach the
//! store. Raw thread-event JSONL remains the durable audit/backfill source.

pub mod backfill;
pub mod ingest;
pub mod jsonl;
pub mod model;
pub mod query;
pub mod rollup;
mod schema;
pub mod sink;
pub mod store;

pub use backfill::{BackfillOptions, BackfillParseError, BackfillReport, backfill_analytics};
pub use ingest::AnalyticsIngestor;
pub use jsonl::AnalyticsJsonlRecord;
pub use model::{
    ANALYTICS_JSONL_SCHEMA_VERSION, DailyRollupRow, SessionRecord, SessionSummary, StatsFilter,
    TokenGroup, TokenSummaryRow, TokenUsageRecord, ToolCallRecord, ToolSummary, TurnRecord,
    UsageSummary, WorkspaceLabelMode,
};
pub use query::{DEFAULT_LIMIT, MAX_LIMIT, sort_tool_summaries};
pub use sink::{
    ANALYTICS_EXTENSION_ID, ANALYTICS_SINK_ID, UsageAnalyticsExtension, UsageAnalyticsSink,
};
pub use store::{AnalyticsStore, StoreCounts};
