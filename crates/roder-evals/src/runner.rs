use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::events::{RoderEvent, ThreadId, TurnId};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    HostedWebSearchConfig, InferenceEvent, InstructionBundle, RuntimeProfile,
};
use roder_api::policy_mode::PolicyMode;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig, RuntimeSpeedPolicyConfig, StartTurnRequest};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::broadcast;
use tokio::time::{Duration, timeout};

mod report;
#[cfg(test)]
mod tests;
mod workspace;

pub use report::{
    EvalFixtureResult, EvalReportDocument, EvalReportSummary, EvalSuiteReport, list_eval_reports,
    read_eval_report, write_eval_report_files,
};

use report::{eval_metrics, trajectory_excerpt};
use workspace::{
    create_workspace, failure_class_for_fixture, grade_expected_evidence, run_workspace_setup,
};

use crate::{EvalFailureClass, EvalFixture, EvalOutcome, EvalReport, EvalRun, EvalTrajectory};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfflineEvalRunnerOptions {
    pub offline: bool,
    pub output_dir: PathBuf,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub runtime_profile: RuntimeProfile,
    #[serde(default)]
    pub speed_policy: EvalSpeedPolicyMode,
}

impl Default for OfflineEvalRunnerOptions {
    fn default() -> Self {
        Self {
            offline: true,
            output_dir: PathBuf::from("evals").join("reports"),
            provider: default_provider(),
            model: default_model(),
            runtime_profile: RuntimeProfile::Interactive,
            speed_policy: EvalSpeedPolicyMode::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvalSpeedPolicyMode {
    #[default]
    Off,
    On,
    Both,
}

impl EvalSpeedPolicyMode {
    fn runs(self, runtime_profile: RuntimeProfile) -> Vec<EvalSpeedPolicyRun> {
        match self {
            Self::Off => vec![EvalSpeedPolicyRun {
                label: "speed_policy:off",
                runtime_profile,
                enabled: false,
            }],
            Self::On => vec![EvalSpeedPolicyRun {
                label: "speed_policy:on",
                runtime_profile: RuntimeProfile::Eval,
                enabled: true,
            }],
            Self::Both => vec![
                EvalSpeedPolicyRun {
                    label: "speed_policy:off",
                    runtime_profile: RuntimeProfile::Eval,
                    enabled: false,
                },
                EvalSpeedPolicyRun {
                    label: "speed_policy:on",
                    runtime_profile: RuntimeProfile::Eval,
                    enabled: true,
                },
            ],
        }
    }
}

impl std::str::FromStr for EvalSpeedPolicyMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "on" => Ok(Self::On),
            "both" => Ok(Self::Both),
            other => anyhow::bail!("invalid --speed-policy {other:?}; expected off, on, or both"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct EvalSpeedPolicyRun {
    label: &'static str,
    runtime_profile: RuntimeProfile,
    enabled: bool,
}

pub fn load_eval_fixtures(dir: &Path) -> anyhow::Result<Vec<EvalFixture>> {
    let mut fixtures = Vec::new();
    load_eval_fixtures_from_dir(dir, &mut fixtures)
        .with_context(|| format!("failed to load eval fixtures from {}", dir.display()))?;
    fixtures.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(fixtures)
}

pub async fn run_offline_eval_suite(
    fixture_dir: &Path,
    options: OfflineEvalRunnerOptions,
) -> anyhow::Result<EvalSuiteReport> {
    if !options.offline {
        anyhow::bail!("offline eval runner requires --offline");
    }
    let fixtures = load_eval_fixtures(fixture_dir)?;
    if fixtures.is_empty() {
        anyhow::bail!(
            "no canonical eval fixtures found in {}",
            fixture_dir.display()
        );
    }
    let generated_at = OffsetDateTime::now_utc();
    let run_id = format!("eval-{}", uuid::Uuid::new_v4());
    let suite_id = fixture_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("fixtures")
        .to_string();
    let speed_runs = options.speed_policy.runs(options.runtime_profile);
    let mut results = Vec::with_capacity(fixtures.len() * speed_runs.len());
    for fixture in fixtures {
        for speed_run in &speed_runs {
            results.push(
                run_offline_fixture(
                    &suite_id,
                    &run_id,
                    &fixture,
                    &options.provider,
                    &options.model,
                    *speed_run,
                )
                .await?,
            );
        }
    }
    let report = EvalSuiteReport {
        suite_id,
        fixture_dir: fixture_dir.to_path_buf(),
        output_dir: options.output_dir.clone(),
        offline: options.offline,
        generated_at,
        results,
    };
    write_eval_report_files(&report, &options.output_dir)?;
    Ok(report)
}

fn load_eval_fixtures_from_dir(dir: &Path, fixtures: &mut Vec<EvalFixture>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            load_eval_fixtures_from_dir(&path, fixtures)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&text)?;
        if !value
            .get("expected")
            .is_some_and(serde_json::Value::is_object)
        {
            continue;
        }
        if let Ok(fixture) = serde_json::from_value::<EvalFixture>(value) {
            fixtures.push(fixture);
        }
    }
    Ok(())
}

async fn run_offline_fixture(
    suite_id: &str,
    run_id: &str,
    fixture: &EvalFixture,
    provider: &str,
    model: &str,
    speed_run: EvalSpeedPolicyRun,
) -> anyhow::Result<EvalFixtureResult> {
    let start = Instant::now();
    let workspace = create_workspace(fixture)?;
    let thread_id = format!("eval-{}", fixture.id);
    let mut events = Vec::new();
    let mut final_answer = String::new();
    let mut failure_message = None;
    let mut outcome = EvalOutcome::Pass;
    let mut failure_class = None;
    if let Err(err) = run_workspace_setup(fixture, &workspace.path) {
        outcome = EvalOutcome::HarnessError;
        failure_class = Some(EvalFailureClass::Environment);
        failure_message = Some(err.to_string());
    }
    let mut turn_id = "setup-failed".to_string();
    if outcome == EvalOutcome::Pass {
        let runtime = Arc::new(build_fake_runtime(
            &workspace.path,
            provider,
            model,
            speed_run.runtime_profile,
            speed_run.enabled,
            fixture.timeout_ms.map(deadline_seconds_from_timeout_ms),
        )?);
        let mut rx = runtime.subscribe_events();
        turn_id = runtime
            .start_turn(StartTurnRequest {
                thread_id: thread_id.clone(),
                message: fixture.prompt.clone(),
                images: Vec::new(),
                provider_override: Some(provider.to_string()),
                model_override: Some(model.to_string()),
                workspace: Some(workspace.path.display().to_string()),
                instructions: InstructionBundle::default(),
                task_ledger_required: fixture.expected.task_ledger_required,
            })
            .await?;
        let timeout_ms = fixture.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        match collect_turn_events(
            &mut rx,
            &thread_id,
            &turn_id,
            Duration::from_millis(timeout_ms),
            &mut final_answer,
        )
        .await
        {
            Ok(collected) => events = collected,
            Err(TurnCollectionError::Timeout { collected }) => {
                events = collected;
                outcome = EvalOutcome::Timeout;
                failure_class = Some(EvalFailureClass::Runtime);
                failure_message = Some(format!("fixture timed out after {timeout_ms}ms"));
            }
            Err(TurnCollectionError::Failed { error, collected }) => {
                events = collected;
                outcome = EvalOutcome::Fail;
                failure_class = Some(if error.contains("verification gaps remain") {
                    EvalFailureClass::Verifier
                } else {
                    EvalFailureClass::Runtime
                });
                failure_message = Some(error);
            }
        }
    }
    if outcome == EvalOutcome::Pass
        && let Err(err) = grade_expected_evidence(fixture, &workspace.path, &final_answer)
    {
        outcome = EvalOutcome::Fail;
        failure_class = Some(failure_class_for_fixture(fixture));
        failure_message = Some(err.to_string());
    }
    if outcome == EvalOutcome::Pass
        && let Err(err) = grade_task_ledger_requirement(fixture, &events)
    {
        outcome = EvalOutcome::Fail;
        failure_class = Some(EvalFailureClass::Verifier);
        failure_message = Some(err.to_string());
    }
    let trajectory = EvalTrajectory::from_events(thread_id.clone(), turn_id.clone(), &events);
    let trace_excerpt = trajectory_excerpt(&trajectory);
    let report = EvalReport {
        run: EvalRun {
            suite_id: suite_id.to_string(),
            run_id: run_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            started_at: OffsetDateTime::now_utc(),
            tags: {
                let mut tags = fixture.tags.clone();
                tags.push(speed_run.label.to_string());
                tags
            },
        },
        outcome: outcome.clone(),
        failure_class: failure_class.clone(),
        trajectory,
        metrics: eval_metrics(&events, start.elapsed().as_millis(), &outcome),
    };
    Ok(EvalFixtureResult {
        fixture_id: fixture.id.clone(),
        title: fixture.title.clone(),
        workspace: workspace.path.clone(),
        final_answer,
        report,
        trace_excerpt,
        failure_message,
    })
}

fn grade_task_ledger_requirement(
    fixture: &EvalFixture,
    events: &[RoderEvent],
) -> anyhow::Result<()> {
    if !fixture.expected.task_ledger_required {
        return Ok(());
    }
    let Some(snapshot) = events.iter().rev().find_map(|event| match event {
        RoderEvent::TaskLedgerUpdated(updated) => Some(updated),
        _ => None,
    }) else {
        anyhow::bail!("task ledger was required but was not created");
    };
    if snapshot.tasks.is_empty() {
        anyhow::bail!("task ledger was required but contained no tasks");
    }
    let incomplete = snapshot
        .tasks
        .iter()
        .filter(|task| {
            !matches!(
                task.status,
                roder_api::task_ledger::TaskLedgerStatus::Completed
            )
        })
        .map(|task| task.id.as_str())
        .collect::<Vec<_>>();
    if !incomplete.is_empty() {
        anyhow::bail!("task ledger incomplete: {}", incomplete.join(", "));
    }
    Ok(())
}

fn build_fake_runtime(
    workspace: &Path,
    provider: &str,
    model: &str,
    runtime_profile: RuntimeProfile,
    speed_policy_enabled: bool,
    turn_deadline_seconds: Option<u64>,
) -> anyhow::Result<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.tool_contributor(Arc::new(roder_tools::BuiltinCodingToolsContributor::new(
        workspace.to_path_buf(),
    )?));
    builder.tool_contributor(Arc::new(
        roder_ext_task_ledger::TaskLedgerToolContributor::default(),
    ));
    builder.tool_contributor(Arc::new(
        roder_ext_verification::VerificationToolContributor,
    ));
    builder.context_planner(Arc::new(roder_context::EntrypointContextPlanner::new(
        workspace.to_path_buf(),
    )));
    Runtime::new(
        builder.build()?,
        RuntimeConfig {
            default_provider: provider.to_string(),
            default_model: model.to_string(),
            hosted_web_search: HostedWebSearchConfig::disabled(),
            workspace: Some(workspace.display().to_string()),
            policy_mode: PolicyMode::Bypass,
            runtime_profile,
            speed_policy: RuntimeSpeedPolicyConfig {
                enabled: speed_policy_enabled,
                ..RuntimeSpeedPolicyConfig::default()
            },
            turn_deadline_seconds,
            ..RuntimeConfig::default()
        },
    )
}

fn deadline_seconds_from_timeout_ms(timeout_ms: u64) -> u64 {
    timeout_ms.div_ceil(1000).max(1)
}

async fn collect_turn_events(
    rx: &mut broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &ThreadId,
    turn_id: &TurnId,
    wait_for: Duration,
    final_answer: &mut String,
) -> Result<Vec<RoderEvent>, TurnCollectionError> {
    let mut events = Vec::new();
    let result = timeout(wait_for, async {
        loop {
            let envelope = match rx.recv().await {
                Ok(envelope) => envelope,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break Ok(()),
            };
            if envelope.thread_id.as_ref() != Some(thread_id)
                || envelope.turn_id.as_ref() != Some(turn_id)
            {
                continue;
            }
            if let RoderEvent::InferenceEventReceived(event) = &envelope.event
                && let InferenceEvent::MessageDelta(delta) = &event.event
            {
                final_answer.push_str(&delta.text);
            }
            let terminal = match &envelope.event {
                RoderEvent::TurnCompleted(_) => Some(Ok(())),
                RoderEvent::TurnFailed(event) => Some(Err(event.error.clone())),
                _ => None,
            };
            events.push(envelope.event);
            if let Some(done) = terminal {
                break done;
            }
        }
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(events),
        Ok(Err(error)) => Err(TurnCollectionError::Failed {
            error,
            collected: events,
        }),
        Err(_) => Err(TurnCollectionError::Timeout { collected: events }),
    }
}

enum TurnCollectionError {
    Timeout {
        collected: Vec<RoderEvent>,
    },
    Failed {
        error: String,
        collected: Vec<RoderEvent>,
    },
}

fn default_provider() -> String {
    PROVIDER_MOCK.to_string()
}

fn default_model() -> String {
    "mock".to_string()
}
