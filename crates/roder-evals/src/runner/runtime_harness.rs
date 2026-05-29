use std::path::Path;
use std::sync::Arc;

use roder_api::events::{RoderEvent, ThreadId, TurnId};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{HostedWebSearchConfig, InferenceEvent, RuntimeProfile};
use roder_api::policy_mode::PolicyMode;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig, RuntimeSpeedPolicyConfig};
use tokio::sync::broadcast;
use tokio::time::{Duration, timeout};

use crate::EvalFixture;

pub(super) fn build_fake_runtime(
    fixture: &EvalFixture,
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
    if fixture.tags.iter().any(|tag| tag == "zerolang") {
        builder.tool_contributor(Arc::new(roder_ext_zerolang::ZerolangToolContributor::new(
            roder_ext_zerolang::ZerolangConfig {
                binary: Some(workspace.join(".zero/fake-zero")),
                timeout_seconds: Some(5),
                artifact_dir: Some(Path::new(".zero/roder").to_path_buf()),
            },
        )));
    }
    if !fixture.tags.iter().any(|tag| tag == "router:off") {
        builder.context_planner(Arc::new(roder_context::RetrievalRouterPlanner));
    }
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

pub(super) fn deadline_seconds_from_timeout_ms(timeout_ms: u64) -> u64 {
    timeout_ms.div_ceil(1000).max(1)
}

pub(super) async fn collect_turn_events(
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

pub(super) enum TurnCollectionError {
    Timeout {
        collected: Vec<RoderEvent>,
    },
    Failed {
        error: String,
        collected: Vec<RoderEvent>,
    },
}
