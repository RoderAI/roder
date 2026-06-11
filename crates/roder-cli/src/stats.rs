//! `roder stats ...` — local usage analytics CLI (roadmap phase 73).
//!
//! Everything operates on the local SQLite analytics store under the Roder
//! data dir (or `--data-dir` for fixtures/tests). No provider credentials
//! or network access; output excludes prompt/output bodies by design.

use std::path::PathBuf;
use std::sync::Arc;

use roder_usage_analytics::{
    AnalyticsStore, BackfillOptions, StatsFilter, TokenGroup, WorkspaceLabelMode,
    backfill_analytics, sort_tool_summaries,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Adds the passive analytics event-sink extension when `[analytics]` is
/// enabled (the default). Failures only warn: analytics must never block a
/// normal run.
pub(crate) fn extensions_with_usage_analytics(
    mut extra: roder_extension_host::ExtraExtensions,
    analytics: Option<roder_config::analytics::AnalyticsConfig>,
) -> roder_extension_host::ExtraExtensions {
    let analytics = analytics.unwrap_or_default();
    if !analytics.enabled {
        return extra;
    }
    let mode = match WorkspaceLabelMode::parse(&analytics.workspace_labels) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("warning: usage analytics disabled: {error}");
            return extra;
        }
    };
    let path = analytics
        .store
        .map(PathBuf::from)
        .unwrap_or_else(|| AnalyticsStore::default_path(&roder_config::config_dir()));
    match AnalyticsStore::open(&path, mode) {
        Ok(store) => {
            // Retention is enforced once per process start; 0 disables it.
            if analytics.retention_days > 0
                && let Err(error) = store.apply_retention(analytics.retention_days)
            {
                eprintln!("warning: analytics retention pruning failed: {error}");
            }
            extra.0.push(Arc::new(
                roder_usage_analytics::UsageAnalyticsExtension::new(Arc::new(store)),
            ));
        }
        Err(error) => eprintln!("warning: usage analytics disabled: {error}"),
    }
    extra
}

pub(crate) async fn run_stats_cli(args: &[String]) -> anyhow::Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_stats_help();
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "backfill" => stats_backfill(rest),
        "summary" => stats_summary(rest),
        "tools" => stats_tools(rest),
        "tokens" => stats_tokens(rest),
        "sessions" => stats_sessions(rest),
        "export" => stats_export(rest),
        _ => {
            print_stats_help();
            Ok(())
        }
    }
}

fn print_stats_help() {
    println!(
        "Usage:\n  roder stats backfill [--best-effort] [--rebuild] [--data-dir <dir>]\n  roder stats summary [--since <7d|date>] [--until <date>] [--json] [--data-dir <dir>]\n  roder stats tools [--tool <name>] [--sort calls|p95|errors|underused] [--json]\n  roder stats tokens [--group day|session|model|provider|workspace] [--json]\n  roder stats sessions [--json]\n  roder stats export --format jsonl --output <path>\n\nLocal-only usage analytics from <data-dir>/analytics/usage.sqlite3.\nBackfill replays <data-dir>/threads/*/events.jsonl idempotently."
    );
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn value_of(args: &[String], name: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == name {
            return iter.next().cloned();
        }
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn data_dir(args: &[String]) -> PathBuf {
    value_of(args, "--data-dir")
        .map(PathBuf::from)
        .unwrap_or_else(roder_config::config_dir)
}

fn open_store(args: &[String]) -> anyhow::Result<Arc<AnalyticsStore>> {
    let config = roder_config::load_config()
        .map(|config| config.analytics.unwrap_or_default())
        .unwrap_or_default();
    let mode = WorkspaceLabelMode::parse(&config.workspace_labels)?;
    let base = data_dir(args);
    let path = config
        .store
        .as_ref()
        .filter(|_| value_of(args, "--data-dir").is_none())
        .map(PathBuf::from)
        .unwrap_or_else(|| AnalyticsStore::default_path(&base));
    Ok(Arc::new(AnalyticsStore::open(&path, mode)?))
}

/// Parses `--since` values: `7d`, `24h`, `90m`, or an RFC3339/`YYYY-MM-DD`
/// date. Returns epoch milliseconds.
pub(crate) fn parse_since(value: &str, now_ms: i64) -> anyhow::Result<i64> {
    let trimmed = value.trim();
    if let Some(days) = trimmed.strip_suffix('d')
        && let Ok(days) = days.parse::<i64>()
    {
        return Ok(now_ms - days * 86_400_000);
    }
    if let Some(hours) = trimmed.strip_suffix('h')
        && let Ok(hours) = hours.parse::<i64>()
    {
        return Ok(now_ms - hours * 3_600_000);
    }
    if let Some(minutes) = trimmed.strip_suffix('m')
        && let Ok(minutes) = minutes.parse::<i64>()
    {
        return Ok(now_ms - minutes * 60_000);
    }
    parse_date_ms(trimmed)
}

pub(crate) fn parse_date_ms(value: &str) -> anyhow::Result<i64> {
    if let Ok(parsed) = OffsetDateTime::parse(value, &Rfc3339) {
        return Ok((parsed.unix_timestamp_nanos() / 1_000_000) as i64);
    }
    let date = time::Date::parse(
        value,
        time::macros::format_description!("[year]-[month]-[day]"),
    )
    .map_err(|_| anyhow::anyhow!("unrecognized date {value:?}; use 7d, 24h, or YYYY-MM-DD"))?;
    Ok((date.midnight().assume_utc().unix_timestamp_nanos() / 1_000_000) as i64)
}

fn filter_from(args: &[String]) -> anyhow::Result<StatsFilter> {
    let now_ms = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
    Ok(StatsFilter {
        since_ms: value_of(args, "--since")
            .map(|value| parse_since(&value, now_ms))
            .transpose()?,
        until_ms: value_of(args, "--until")
            .map(|value| parse_date_ms(&value))
            .transpose()?,
        thread_id: value_of(args, "--thread"),
        tool_name: value_of(args, "--tool"),
        provider: value_of(args, "--provider"),
        model: value_of(args, "--model"),
        workspace_key: None,
        min_calls: None,
        limit: value_of(args, "--limit")
            .map(|value| value.parse())
            .transpose()?,
    })
}

fn stats_backfill(args: &[String]) -> anyhow::Result<()> {
    let store = open_store(args)?;
    let thread_root = data_dir(args).join("threads");
    let report = backfill_analytics(
        &thread_root,
        &store,
        BackfillOptions {
            rebuild: flag(args, "--rebuild"),
            best_effort: flag(args, "--best-effort"),
        },
    )?;
    let retention_days = roder_config::load_config()
        .map(|config| config.analytics.unwrap_or_default().retention_days)
        .unwrap_or(0);
    let pruned = store.apply_retention(retention_days)?;
    let rolled = store.refresh_daily_rollups()?;
    println!(
        "scanned {} file(s), ingested {} line(s), skipped {} by offset, enriched {} session(s), pruned {} row(s), refreshed {} rollup row(s)",
        report.files_scanned,
        report.lines_ingested,
        report.lines_skipped_by_offset,
        report.sessions_enriched,
        pruned,
        rolled
    );
    for error in &report.parse_errors {
        eprintln!("warning: skipped {}:{}: {}", error.path, error.line, error.message);
    }
    Ok(())
}

fn stats_summary(args: &[String]) -> anyhow::Result<()> {
    let store = open_store(args)?;
    let summary = store.usage_summary(&filter_from(args)?)?;
    if flag(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    println!(
        "sessions {}  turns {} ({} completed, {} failed)\ntool calls {} ({} errors)\ntokens {} total ({} prompt, {} completion, {} cached prompt)",
        summary.session_count,
        summary.turn_count,
        summary.completed_turn_count,
        summary.failed_turn_count,
        summary.tool_call_count,
        summary.tool_error_count,
        summary.total_tokens,
        summary.prompt_tokens,
        summary.completion_tokens,
        summary.cached_prompt_tokens,
    );
    if let Some(tool) = &summary.most_called_tool {
        println!("most called tool: {tool}");
    }
    Ok(())
}

fn stats_tools(args: &[String]) -> anyhow::Result<()> {
    let store = open_store(args)?;
    let mut summaries = store.tool_summaries(&filter_from(args)?)?;
    let sort = value_of(args, "--sort").unwrap_or_else(|| "calls".to_string());
    sort_tool_summaries(&mut summaries, &sort);
    if flag(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&summaries)?);
        return Ok(());
    }
    println!("{:<24} {:>7} {:>7} {:>9} {:>9} {:>9}", "tool", "calls", "errors", "p50ms", "p95ms", "p99ms");
    for tool in summaries {
        println!(
            "{:<24} {:>7} {:>7} {:>9} {:>9} {:>9}",
            tool.tool_name,
            tool.call_count,
            tool.error_count,
            tool.p50_duration_ms.map_or("-".into(), |v| v.to_string()),
            tool.p95_duration_ms.map_or("-".into(), |v| v.to_string()),
            tool.p99_duration_ms.map_or("-".into(), |v| v.to_string()),
        );
    }
    Ok(())
}

fn stats_tokens(args: &[String]) -> anyhow::Result<()> {
    let store = open_store(args)?;
    let group = TokenGroup::parse(&value_of(args, "--group").unwrap_or_else(|| "day".to_string()))?;
    let rows = store.token_summaries(group, &filter_from(args)?)?;
    if flag(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!("{:<36} {:>10} {:>10} {:>10} {:>6}", "group", "prompt", "completion", "total", "turns");
    for row in rows {
        println!(
            "{:<36} {:>10} {:>10} {:>10} {:>6}",
            row.group, row.prompt_tokens, row.completion_tokens, row.total_tokens, row.turn_count
        );
    }
    Ok(())
}

fn stats_sessions(args: &[String]) -> anyhow::Result<()> {
    let store = open_store(args)?;
    let sessions = store.session_summaries(&filter_from(args)?)?;
    if flag(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    println!("{:<38} {:>7} {:>7} {:>8} {:>10}", "thread", "turns", "tools", "errors", "tokens");
    for session in sessions {
        println!(
            "{:<38} {:>7} {:>7} {:>8} {:>10}",
            session.thread_id,
            session.turn_count,
            session.tool_call_count,
            session.tool_error_count,
            session.total_tokens,
        );
    }
    Ok(())
}

fn stats_export(args: &[String]) -> anyhow::Result<()> {
    let format = value_of(args, "--format").unwrap_or_else(|| "jsonl".to_string());
    anyhow::ensure!(format == "jsonl", "only --format jsonl is supported");
    let store = open_store(args)?;
    let written = match value_of(args, "--output") {
        Some(path) => {
            let mut file = std::fs::File::create(&path)?;
            let written = store.export_jsonl(&mut file)?;
            println!("exported {written} record(s) to {path}");
            written
        }
        None => store.export_jsonl(&mut std::io::stdout().lock())?,
    };
    anyhow::ensure!(written > 0 || store.counts()?.turns == 0, "export wrote nothing");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_since_parsing_supports_durations_and_dates() {
        let now = 1_750_000_000_000_i64;
        assert_eq!(parse_since("7d", now).unwrap(), now - 7 * 86_400_000);
        assert_eq!(parse_since("24h", now).unwrap(), now - 86_400_000);
        assert_eq!(parse_since("90m", now).unwrap(), now - 90 * 60_000);
        // 2026-06-01T00:00:00Z
        assert_eq!(parse_since("2026-06-01", now).unwrap(), 1_780_272_000_000);
        assert!(parse_since("yesterday-ish", now).is_err());
    }
}
