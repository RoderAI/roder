use crate::{EvalFixture, EvalMetric, EvalMetricKind, EvalOutcome};

pub(super) fn fixture_command_check_metrics(
    fixture: &EvalFixture,
    outcome: &EvalOutcome,
) -> Vec<EvalMetric> {
    let required = fixture.expected.command_checks.len() as u64;
    let completed = if outcome == &EvalOutcome::Pass {
        required
    } else {
        0
    };
    vec![
        EvalMetric {
            name: "verifier_command_checks_required".to_string(),
            kind: EvalMetricKind::Count,
            value: required as f64,
            unit: None,
        },
        EvalMetric {
            name: "verifier_command_checks_completed".to_string(),
            kind: EvalMetricKind::Count,
            value: completed as f64,
            unit: None,
        },
    ]
}
