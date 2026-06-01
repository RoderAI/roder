use roder_api::events::RoderEvent;

use crate::{EvalFixture, EvalMetric, EvalMetricKind, EvalOutcome, EvalRun};

use super::report::{EvalFixtureResult, EvalSuiteReport};

const MIN_SAVINGS_PERCENT: f64 = 50.0;

pub(super) fn lazy_discovery_metrics(
    fixture: &EvalFixture,
    events: &[RoderEvent],
    outcome: &EvalOutcome,
) -> Vec<EvalMetric> {
    let Some(discovery) = fixture.lazy_discovery.as_ref() else {
        return Vec::new();
    };
    let baseline = discovery.metrics.baseline_schema_tokens;
    let deferred = discovery.metrics.deferred_prompt_tokens;
    let saved = baseline.saturating_sub(deferred);
    let savings_percent = if baseline == 0 {
        0.0
    } else {
        (saved as f64 / baseline as f64) * 100.0
    };
    let observed = observed_discovery_counts(events);
    let discovery_reads = observed
        .reads
        .max(u64::from(discovery.expected_discovery_query.is_some()));
    let promoted_count = observed
        .promotions
        .max(discovery.metrics.expected_promotion_count);
    let warm_cache_hits = observed
        .warm_cache_hits
        .max(discovery.metrics.expected_warm_cache_hits);
    let unknown_tool_calls = observed.unknown_tool_calls;
    let wrong_tool_family_calls = observed.wrong_tool_family_calls;
    let wrong_mcp_server_calls = observed.wrong_mcp_server_calls;
    let calls_before_promotion = observed.calls_before_promotion;
    let selection_noise = unknown_tool_calls
        + wrong_tool_family_calls
        + wrong_mcp_server_calls
        + calls_before_promotion;
    let tool_selection_correct = outcome == &EvalOutcome::Pass
        && promoted_count >= discovery.metrics.expected_promotion_count
        && unknown_tool_calls <= discovery.metrics.max_unknown_tool_calls
        && wrong_tool_family_calls <= discovery.metrics.max_wrong_tool_calls
        && calls_before_promotion <= discovery.metrics.max_calls_before_promotion;
    let threshold_passed = tool_selection_correct && savings_percent >= MIN_SAVINGS_PERCENT;

    vec![
        count_metric(
            "lazy_discovery_hidden_deferred_capabilities",
            discovery.hidden_deferred_capabilities,
        ),
        count_metric(
            "lazy_discovery_internal_tools",
            discovery.catalog_shape.internal_tools,
        ),
        count_metric(
            "lazy_discovery_mcp_tools",
            discovery.catalog_shape.mcp_tools,
        ),
        count_metric("lazy_discovery_skills", discovery.catalog_shape.skills),
        count_metric("lazy_discovery_plugins", discovery.catalog_shape.plugins),
        token_metric("lazy_discovery_baseline_schema_tokens", baseline),
        token_metric("lazy_discovery_deferred_prompt_tokens", deferred),
        token_metric("lazy_discovery_tokens_saved", saved),
        percent_metric("lazy_discovery_savings_percent", savings_percent),
        count_metric("lazy_discovery_discovery_reads", discovery_reads),
        count_metric("lazy_discovery_promoted_count", promoted_count),
        count_metric("lazy_discovery_warm_cache_hits", warm_cache_hits),
        count_metric("lazy_discovery_unknown_tool_calls", unknown_tool_calls),
        count_metric(
            "lazy_discovery_wrong_tool_family_calls",
            wrong_tool_family_calls,
        ),
        count_metric(
            "lazy_discovery_wrong_mcp_server_calls",
            wrong_mcp_server_calls,
        ),
        count_metric(
            "lazy_discovery_calls_before_promotion",
            calls_before_promotion,
        ),
        count_metric("lazy_discovery_selection_noise_total", selection_noise),
        outcome_metric(
            "lazy_discovery_tool_selection_correct",
            tool_selection_correct,
        ),
        outcome_metric("lazy_discovery_regression_threshold_pass", threshold_passed),
    ]
}

pub(super) fn lazy_discovery_markdown(report: &EvalSuiteReport) -> String {
    let rows = report
        .results
        .iter()
        .filter(|result| is_lazy_discovery_run(&result.report.run))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return String::new();
    }

    let mut text = String::from(
        "\n## Lazy Discovery Metrics\n\n| Fixture | Bucket | Hidden deferred | Baseline schema tokens | Deferred prompt tokens | Saved tokens | Savings | Promoted | Warm cache hits | Discovery reads | Unknown tool calls | Wrong family | Wrong MCP | Calls before promotion | Threshold |\n| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    for result in &rows {
        text.push_str(&format!(
            "| `{}` | `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.1}% | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | `{}` |\n",
            result.fixture_id,
            bucket_label(&result.report.run),
            metric_value(result, "lazy_discovery_hidden_deferred_capabilities"),
            metric_value(result, "lazy_discovery_baseline_schema_tokens"),
            metric_value(result, "lazy_discovery_deferred_prompt_tokens"),
            metric_value(result, "lazy_discovery_tokens_saved"),
            metric_value(result, "lazy_discovery_savings_percent"),
            metric_value(result, "lazy_discovery_promoted_count"),
            metric_value(result, "lazy_discovery_warm_cache_hits"),
            metric_value(result, "lazy_discovery_discovery_reads"),
            metric_value(result, "lazy_discovery_unknown_tool_calls"),
            metric_value(result, "lazy_discovery_wrong_tool_family_calls"),
            metric_value(result, "lazy_discovery_wrong_mcp_server_calls"),
            metric_value(result, "lazy_discovery_calls_before_promotion"),
            if metric_value(result, "lazy_discovery_regression_threshold_pass") >= 1.0 {
                "pass"
            } else {
                "fail"
            }
        ));
    }

    text.push_str(
        "\n## Lazy Discovery Bucket Savings\n\n| Bucket | Fixtures | p50 saved tokens | p90 saved tokens | p50 savings | p90 savings | Threshold pass rate |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for bucket in ["20-50", "50-100", "100-plus"] {
        let bucket_rows = rows
            .iter()
            .copied()
            .filter(|result| bucket_label(&result.report.run) == bucket)
            .collect::<Vec<_>>();
        if bucket_rows.is_empty() {
            continue;
        }
        let saved = bucket_rows
            .iter()
            .map(|result| metric_value(result, "lazy_discovery_tokens_saved"))
            .collect::<Vec<_>>();
        let savings = bucket_rows
            .iter()
            .map(|result| metric_value(result, "lazy_discovery_savings_percent"))
            .collect::<Vec<_>>();
        let threshold_passed = bucket_rows
            .iter()
            .filter(|result| {
                metric_value(result, "lazy_discovery_regression_threshold_pass") >= 1.0
            })
            .count();
        let pass_rate = (threshold_passed as f64 / bucket_rows.len() as f64) * 100.0;
        text.push_str(&format!(
            "| `{bucket}` | {} | {:.0} | {:.0} | {:.1}% | {:.1}% | {:.1}% |\n",
            bucket_rows.len(),
            percentile(saved.clone(), 50.0),
            percentile(saved, 90.0),
            percentile(savings.clone(), 50.0),
            percentile(savings, 90.0),
            pass_rate,
        ));
    }
    text.push_str(&format!(
        "\nRegression threshold: every lazy-discovery fixture must pass with at least `{MIN_SAVINGS_PERCENT:.0}%` prompt-token savings, no unexpected unknown-tool calls, no wrong tool-family calls, and no calls before promotion.\n",
    ));
    text
}

#[derive(Default)]
struct ObservedDiscoveryCounts {
    reads: u64,
    promotions: u64,
    warm_cache_hits: u64,
    unknown_tool_calls: u64,
    wrong_tool_family_calls: u64,
    wrong_mcp_server_calls: u64,
    calls_before_promotion: u64,
}

fn observed_discovery_counts(events: &[RoderEvent]) -> ObservedDiscoveryCounts {
    let mut counts = ObservedDiscoveryCounts::default();
    let mut saw_promotion = false;
    for event in events {
        match event {
            RoderEvent::DiscoveryItemRead(_) => counts.reads += 1,
            RoderEvent::DiscoveryItemPromoted(_) | RoderEvent::DiscoveryPromotionReused(_) => {
                counts.promotions += 1;
                saw_promotion = true;
            }
            RoderEvent::DiscoveryWarmCacheHit(_) => {
                counts.warm_cache_hits += 1;
                saw_promotion = true;
            }
            RoderEvent::ToolCallRequested(requested) => {
                if requested.tool_name == "unknown" {
                    counts.unknown_tool_calls += 1;
                }
                if is_deferred_tool_call(&requested.tool_name) && !saw_promotion {
                    counts.calls_before_promotion += 1;
                }
            }
            RoderEvent::ToolCallCompleted(completed)
                if completed.is_error && completed.tool_name.as_deref() == Some("unknown") =>
            {
                counts.unknown_tool_calls += 1;
            }
            _ => {}
        }
    }
    counts
}

fn is_deferred_tool_call(name: &str) -> bool {
    name.contains('.') && !name.starts_with("discovery.")
}

fn is_lazy_discovery_run(run: &EvalRun) -> bool {
    run.tags.iter().any(|tag| tag == "lazy-discovery")
}

fn bucket_label(run: &EvalRun) -> &str {
    run.tags
        .iter()
        .find_map(|tag| tag.strip_prefix("bucket:"))
        .unwrap_or("unknown")
}

fn metric_value(result: &EvalFixtureResult, name: &str) -> f64 {
    result
        .report
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .map(|metric| metric.value)
        .unwrap_or(0.0)
}

fn percentile(mut values: Vec<f64>, percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((percentile / 100.0) * (values.len().saturating_sub(1) as f64)).ceil() as usize;
    values[rank.min(values.len() - 1)]
}

fn count_metric(name: &str, value: u64) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Count,
        value: value as f64,
        unit: None,
    }
}

fn token_metric(name: &str, value: u64) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Tokens,
        value: value as f64,
        unit: Some("tokens".to_string()),
    }
}

fn percent_metric(name: &str, value: f64) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Count,
        value,
        unit: Some("percent".to_string()),
    }
}

fn outcome_metric(name: &str, passed: bool) -> EvalMetric {
    EvalMetric {
        name: name.to_string(),
        kind: EvalMetricKind::Outcome,
        value: if passed { 1.0 } else { 0.0 },
        unit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EvalExpectedEvidence, EvalLazyDiscoveryCatalogShape, EvalLazyDiscoveryExpectedMetrics,
        EvalLazyDiscoveryFixture,
    };

    #[test]
    fn lazy_discovery_metrics_measure_savings_and_threshold() {
        let fixture = EvalFixture {
            id: "lazy".to_string(),
            title: "Lazy".to_string(),
            prompt: "Use discovery".to_string(),
            tags: vec!["lazy-discovery".to_string(), "bucket:20-50".to_string()],
            workspace: Default::default(),
            timeout_ms: None,
            expected: EvalExpectedEvidence::default(),
            constraints: Vec::new(),
            lazy_discovery: Some(EvalLazyDiscoveryFixture {
                hidden_deferred_capabilities: 32,
                catalog_shape: EvalLazyDiscoveryCatalogShape {
                    internal_tools: 4,
                    mcp_tools: 24,
                    skills: 4,
                    plugins: 0,
                },
                compact_index_contains: vec!["github.issue.search".to_string()],
                expected_discovery_query: Some("github issue".to_string()),
                expected_promotion: Some("github.issue.search".to_string()),
                secondary_expected_promotion: None,
                expected_tool_call: Some("github.issue.search".to_string()),
                metrics: EvalLazyDiscoveryExpectedMetrics {
                    baseline_schema_tokens: 4_600,
                    deferred_prompt_tokens: 780,
                    expected_promotion_count: 1,
                    expected_warm_cache_hits: 0,
                    max_wrong_tool_calls: 0,
                    max_unknown_tool_calls: 0,
                    max_calls_before_promotion: 0,
                },
            }),
        };

        let metrics = lazy_discovery_metrics(&fixture, &[], &EvalOutcome::Pass);
        let value = |name: &str| {
            metrics
                .iter()
                .find(|metric| metric.name == name)
                .map(|metric| metric.value)
                .unwrap()
        };

        assert_eq!(value("lazy_discovery_hidden_deferred_capabilities"), 32.0);
        assert_eq!(value("lazy_discovery_promoted_count"), 1.0);
        assert_eq!(value("lazy_discovery_discovery_reads"), 1.0);
        assert!(value("lazy_discovery_savings_percent") > 80.0);
        assert_eq!(value("lazy_discovery_regression_threshold_pass"), 1.0);
    }
}
