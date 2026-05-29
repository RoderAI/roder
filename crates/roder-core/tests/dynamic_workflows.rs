use async_trait::async_trait;
use roder_api::catalog::{PROVIDER_MOCK, REASONING_NONE, REASONING_XHIGH};
use roder_api::dynamic_workflows::WorkflowRunLimits;
use roder_api::inference::{InstructionBundle, ReasoningConfig, RuntimeProfile};
use roder_core::{
    DynamicWorkflowEffortProfile, RuntimeConfig, RuntimeDynamicWorkflowConfig, WorkflowPlanner,
    WorkflowPlannerRequest, WorkflowScriptDraftSource, WorkflowTriggerDecision,
    WorkflowTriggerKind, WorkflowTriggerSuppression, classify_workflow_trigger,
};
use roder_dynamic_workflows::{
    WorkflowApprovalScope, WorkflowConsentKey, WorkflowConsentStore, workflow_script_hash,
};
use time::OffsetDateTime;

const FIXTURE_WORKFLOW: &str = r#"
workflow.define({
  name: "Crate Audit",
  description: "Audit crates and review findings.",
  hostApiVersion: 1,
  phases: ["scout", "review"],
  limits: { maxAgentsPerRun: 12, maxConcurrentAgents: 4 }
}, async (ctx) => {
  const findings = await ctx.agents.map("scout", ["api", "core"], (crateName) => ({
    lane: "scout",
    prompt: `Audit ${crateName}`
  }));
  ctx.checkpoint.save("findings", findings);
  return ctx.report.markdown(findings.map((finding) => finding.output).join("\n"));
});
"#;

#[derive(Clone)]
struct StaticDraftSource {
    source: String,
}

#[async_trait]
impl WorkflowScriptDraftSource for StaticDraftSource {
    async fn draft_workflow_script(
        &self,
        request: &WorkflowPlannerRequest,
    ) -> anyhow::Result<String> {
        assert!(request.prompt.contains("audit"));
        assert_eq!(request.provider, PROVIDER_MOCK);
        Ok(self.source.clone())
    }
}

#[tokio::test]
async fn prompt_triggered_planning_uses_fake_model_and_validates_script() {
    let config = RuntimeDynamicWorkflowConfig::default();
    let trigger = classify_workflow_trigger(
        "Use a workflow to audit the crates and review findings",
        &config,
    );
    assert_eq!(
        trigger,
        WorkflowTriggerDecision::Plan(WorkflowTriggerKind::TriggerWord)
    );

    let planner = WorkflowPlanner::new(StaticDraftSource {
        source: FIXTURE_WORKFLOW.to_string(),
    });
    let draft = planner
        .plan(WorkflowPlannerRequest {
            prompt: "Use a workflow to audit the crates".to_string(),
            workspace: Some("/workspace".to_string()),
            source_path: None,
            provider: PROVIDER_MOCK.to_string(),
            model: "mock".to_string(),
            reasoning: ReasoningConfig::default(),
            runtime_profile: RuntimeProfile::Interactive,
            effort_profile: DynamicWorkflowEffortProfile::Standard,
            trigger: WorkflowTriggerKind::TriggerWord,
            instructions: InstructionBundle::default(),
            limits: WorkflowRunLimits::default(),
        })
        .await
        .unwrap();

    assert_eq!(draft.script.name, "Crate Audit");
    assert_eq!(draft.script.hash, workflow_script_hash(FIXTURE_WORKFLOW));
    assert_eq!(
        draft.phase_names,
        vec!["scout".to_string(), "review".to_string()]
    );
    assert_eq!(
        draft.capability_scope,
        vec![
            "childAgents".to_string(),
            "checkpoints".to_string(),
            "reports".to_string()
        ]
    );
    assert_eq!(draft.cost_estimate.max_child_agents, 12);
}

#[test]
fn ignored_trigger_word_does_not_plan_for_chat_or_slash_commands() {
    let config = RuntimeDynamicWorkflowConfig::default();

    assert_eq!(
        classify_workflow_trigger("What is a workflow?", &config),
        WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::PureChatQuestion)
    );
    assert_eq!(
        classify_workflow_trigger("/workflows", &config),
        WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::SlashCommand)
    );
}

#[test]
fn consent_reuse_is_scoped_to_script_workspace_source_and_capabilities() {
    let now = OffsetDateTime::UNIX_EPOCH;
    let hash = workflow_script_hash(FIXTURE_WORKFLOW);
    let key = WorkflowConsentKey::new(
        hash.clone(),
        Some("/workspace"),
        Some(".agents/workflows/audit.workflow.js"),
        WorkflowApprovalScope::ScriptAndWorkspace,
    );
    let mut store = WorkflowConsentStore::default();
    store.record(
        key.clone(),
        vec!["childAgents".to_string(), "reports".to_string()],
        now,
        None,
    );

    assert!(
        store
            .reusable_consent(&key, now, &["childAgents".to_string()])
            .is_some()
    );

    let other_source = WorkflowConsentKey::new(
        hash,
        Some("/workspace"),
        Some(".agents/workflows/other.workflow.js"),
        WorkflowApprovalScope::ScriptAndWorkspace,
    );
    assert!(
        store
            .reusable_consent(&other_source, now, &["childAgents".to_string()])
            .is_none()
    );
    assert!(
        store
            .reusable_consent(&key, now, &["shell".to_string()])
            .is_none()
    );
}

#[test]
fn ultracode_degrades_on_models_without_xhigh() {
    let mock = RuntimeConfig {
        dynamic_workflows: RuntimeDynamicWorkflowConfig {
            effort_profile: DynamicWorkflowEffortProfile::Ultracode,
            ..RuntimeDynamicWorkflowConfig::default()
        },
        ..RuntimeConfig::default()
    };
    assert_eq!(
        roder_core::Runtime::effective_reasoning_for_config(&mock),
        REASONING_NONE
    );

    let supported = RuntimeConfig {
        default_model: "gpt-5.5".to_string(),
        dynamic_workflows: RuntimeDynamicWorkflowConfig {
            effort_profile: DynamicWorkflowEffortProfile::Ultracode,
            ..RuntimeDynamicWorkflowConfig::default()
        },
        ..RuntimeConfig::default()
    };
    assert_eq!(
        roder_core::Runtime::effective_reasoning_for_config(&supported),
        REASONING_XHIGH
    );
}
