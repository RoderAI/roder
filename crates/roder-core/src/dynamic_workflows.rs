use async_trait::async_trait;
use roder_api::catalog::REASONING_NONE;
use roder_api::dynamic_workflows::{
    WorkflowCostEstimate, WorkflowRunLimits, WorkflowRunStatus, WorkflowScript,
    WorkflowScriptSource, WorkflowScriptSourceKind,
};
use roder_api::inference::{InstructionBundle, ReasoningConfig, RuntimeProfile};
use roder_dynamic_workflows::{
    WorkflowDefinition, WorkflowRuntimeOptions, parse_workflow_definition, workflow_script_hash,
};
use time::OffsetDateTime;

use crate::speed_policy::reasoning_for_supported_effort;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DynamicWorkflowEffortProfile {
    #[default]
    Standard,
    Ultracode,
}

impl DynamicWorkflowEffortProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Ultracode => "ultracode",
        }
    }

    pub fn auto_workflows_enabled(self) -> bool {
        matches!(self, Self::Ultracode)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeDynamicWorkflowConfig {
    pub enabled: bool,
    pub trigger_word_enabled: bool,
    pub auto_with_ultracode: bool,
    pub effort_profile: DynamicWorkflowEffortProfile,
    pub limits: WorkflowRunLimits,
}

impl Default for RuntimeDynamicWorkflowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_word_enabled: true,
            auto_with_ultracode: true,
            effort_profile: DynamicWorkflowEffortProfile::Standard,
            limits: WorkflowRunLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTriggerKind {
    TriggerWord,
    UltracodeAuto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTriggerSuppression {
    Disabled,
    NoTrigger,
    SlashCommand,
    ApprovalReply,
    TinyCommand,
    PureChatQuestion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTriggerDecision {
    Plan(WorkflowTriggerKind),
    Ignore(WorkflowTriggerSuppression),
}

impl WorkflowTriggerDecision {
    pub fn should_plan(self) -> bool {
        matches!(self, Self::Plan(_))
    }
}

impl RuntimeDynamicWorkflowConfig {
    pub fn classify_trigger(&self, input: &str) -> WorkflowTriggerDecision {
        classify_workflow_trigger(input, self)
    }
}

pub fn classify_workflow_trigger(
    input: &str,
    config: &RuntimeDynamicWorkflowConfig,
) -> WorkflowTriggerDecision {
    if !config.enabled {
        return WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::Disabled);
    }

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::TinyCommand);
    }
    if trimmed.starts_with('/') {
        return WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::SlashCommand);
    }
    if is_approval_reply(trimmed) {
        return WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::ApprovalReply);
    }
    if is_pure_chat_question(trimmed) {
        return WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::PureChatQuestion);
    }

    let words = word_count(trimmed);
    if words <= 2 {
        return WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::TinyCommand);
    }

    if config.trigger_word_enabled && contains_word(trimmed, "workflow") {
        return WorkflowTriggerDecision::Plan(WorkflowTriggerKind::TriggerWord);
    }

    if config.auto_with_ultracode
        && config.effort_profile.auto_workflows_enabled()
        && is_substantive_task(trimmed, words)
    {
        return WorkflowTriggerDecision::Plan(WorkflowTriggerKind::UltracodeAuto);
    }

    WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::NoTrigger)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UltracodeReasoningDecision {
    pub desired_reasoning: String,
    pub applied_reasoning: Option<String>,
    pub supported: bool,
    pub reasoning: ReasoningConfig,
}

pub fn ultracode_reasoning_decision(
    model: &str,
    desired_reasoning: &str,
    fallback: ReasoningConfig,
) -> UltracodeReasoningDecision {
    let (reasoning, supported) = reasoning_for_supported_effort(model, desired_reasoning, fallback);
    UltracodeReasoningDecision {
        desired_reasoning: desired_reasoning.to_string(),
        applied_reasoning: supported.then(|| desired_reasoning.to_string()),
        supported,
        reasoning,
    }
}

pub fn ultracode_reasoning_level_for_model(
    model: &str,
    desired_reasoning: &str,
    fallback_level: &str,
) -> String {
    let fallback = reasoning_config_from_level(fallback_level);
    ultracode_reasoning_decision(model, desired_reasoning, fallback)
        .reasoning
        .level
        .unwrap_or_else(|| REASONING_NONE.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowModelPolicy {
    pub provider: String,
    pub model: String,
    pub reasoning: ReasoningConfig,
    pub runtime_profile: RuntimeProfile,
    pub effort_profile: DynamicWorkflowEffortProfile,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowPlannerRequest {
    pub prompt: String,
    pub workspace: Option<String>,
    pub source_path: Option<String>,
    pub provider: String,
    pub model: String,
    pub reasoning: ReasoningConfig,
    pub runtime_profile: RuntimeProfile,
    pub effort_profile: DynamicWorkflowEffortProfile,
    pub trigger: WorkflowTriggerKind,
    pub instructions: InstructionBundle,
    pub limits: WorkflowRunLimits,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowPlanDraft {
    pub status: WorkflowRunStatus,
    pub script: WorkflowScript,
    pub definition: WorkflowDefinition,
    pub phase_names: Vec<String>,
    pub capability_scope: Vec<String>,
    pub model_policy: WorkflowModelPolicy,
    pub cost_estimate: WorkflowCostEstimate,
}

#[async_trait]
pub trait WorkflowScriptDraftSource: Send + Sync {
    async fn draft_workflow_script(
        &self,
        request: &WorkflowPlannerRequest,
    ) -> anyhow::Result<String>;
}

#[derive(Debug, Clone)]
pub struct WorkflowPlanner<P> {
    draft_source: P,
    options: WorkflowRuntimeOptions,
}

impl<P> WorkflowPlanner<P> {
    pub fn new(draft_source: P) -> Self {
        Self {
            draft_source,
            options: WorkflowRuntimeOptions::default(),
        }
    }

    pub fn with_options(mut self, options: WorkflowRuntimeOptions) -> Self {
        self.options = options;
        self
    }
}

impl<P> WorkflowPlanner<P>
where
    P: WorkflowScriptDraftSource,
{
    pub async fn plan(&self, request: WorkflowPlannerRequest) -> anyhow::Result<WorkflowPlanDraft> {
        let source = self.draft_source.draft_workflow_script(&request).await?;
        let mut options = self.options.clone();
        options.limits = request.limits.clone();
        let definition = parse_workflow_definition(&source, &options)
            .map_err(|err| anyhow::anyhow!("generated workflow script failed validation: {err}"))?;
        let hash = workflow_script_hash(&source);
        let now = OffsetDateTime::now_utc();
        let script = WorkflowScript {
            script_id: generated_script_id(&hash),
            name: definition.name.clone(),
            description: definition.description.clone(),
            source: WorkflowScriptSource {
                kind: WorkflowScriptSourceKind::Generated,
                path: request.source_path.clone(),
                command_name: None,
                extension_id: None,
            },
            hash,
            host_api_version: definition.host_api_version,
            arguments_schema: definition.arguments_schema.clone(),
            body: Some(source.clone()),
            limits: definition.limits.clone(),
            created_at: now,
            updated_at: now,
        };

        Ok(WorkflowPlanDraft {
            status: WorkflowRunStatus::AwaitingApproval,
            phase_names: phase_names(&definition),
            capability_scope: capability_scope(&source),
            model_policy: WorkflowModelPolicy {
                provider: request.provider,
                model: request.model,
                reasoning: request.reasoning,
                runtime_profile: request.runtime_profile,
                effort_profile: request.effort_profile,
            },
            cost_estimate: cost_estimate(&definition, &source),
            script,
            definition,
        })
    }
}

pub fn workflow_planner_instructions(prompt: &str) -> String {
    format!(
        "Draft a Roder workflow script for this task. Return only JavaScript that calls workflow.define with phase names, limits, and an async handler. The script may only coordinate ctx.agents, ctx.checkpoint, and ctx.report host APIs.\n\nTask:\n{prompt}"
    )
}

fn is_approval_reply(input: &str) -> bool {
    matches!(
        normalize_sentence(input).as_str(),
        "y" | "yes"
            | "ok"
            | "okay"
            | "approve"
            | "approved"
            | "run it"
            | "go ahead"
            | "continue"
            | "deny"
            | "denied"
            | "no"
    )
}

fn is_pure_chat_question(input: &str) -> bool {
    let normalized = normalize_sentence(input);
    if !normalized.ends_with('?') {
        return false;
    }
    normalized.starts_with("what ")
        || normalized.starts_with("what's ")
        || normalized.starts_with("how ")
        || normalized.starts_with("why ")
        || normalized.starts_with("can you explain ")
        || normalized.starts_with("explain ")
        || normalized.starts_with("tell me about ")
}

fn is_substantive_task(input: &str, words: usize) -> bool {
    if words < 8 {
        return false;
    }
    let normalized = normalize_sentence(input);
    [
        "audit",
        "build",
        "create",
        "implement",
        "investigate",
        "migrate",
        "plan",
        "research",
        "review",
        "verify",
    ]
    .iter()
    .any(|verb| contains_word(&normalized, verb))
}

fn contains_word(input: &str, needle: &str) -> bool {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case(needle))
}

fn word_count(input: &str) -> usize {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .count()
}

fn normalize_sentence(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn reasoning_config_from_level(level: &str) -> ReasoningConfig {
    match level {
        "" | REASONING_NONE => ReasoningConfig::default(),
        level => ReasoningConfig {
            enabled: true,
            level: Some(level.to_string()),
        },
    }
}

fn generated_script_id(hash: &str) -> String {
    let short_hash = hash.get(..12).unwrap_or(hash);
    format!("generated-{short_hash}")
}

fn phase_names(definition: &WorkflowDefinition) -> Vec<String> {
    if definition.phases.is_empty() {
        vec![definition.name.clone()]
    } else {
        definition.phases.clone()
    }
}

fn capability_scope(source: &str) -> Vec<String> {
    let mut capabilities = Vec::new();
    if source.contains("ctx.agents") {
        capabilities.push("childAgents".to_string());
    }
    if source.contains("ctx.checkpoint") {
        capabilities.push("checkpoints".to_string());
    }
    if source.contains("ctx.report") {
        capabilities.push("reports".to_string());
    }
    capabilities
}

fn cost_estimate(definition: &WorkflowDefinition, source: &str) -> WorkflowCostEstimate {
    let uses_agents = source.contains("ctx.agents");
    let phase_count = definition.phases.len().max(1) as u32;
    let min_child_agents = if uses_agents { phase_count } else { 0 };
    let max_child_agents = if uses_agents {
        definition.limits.max_agents_per_run
    } else {
        0
    };
    let estimated_prompt_tokens = Some((source.len() as u64 / 4).max(1));
    let warning = (max_child_agents >= 100).then(|| {
        format!(
            "This workflow may launch up to {max_child_agents} child agents; review cost and runtime before approving."
        )
    });

    WorkflowCostEstimate {
        min_child_agents,
        max_child_agents,
        estimated_prompt_tokens,
        estimated_completion_tokens: None,
        warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::catalog::REASONING_XHIGH;

    #[test]
    fn trigger_word_plans_but_chat_questions_and_commands_are_ignored() {
        let config = RuntimeDynamicWorkflowConfig::default();

        assert_eq!(
            classify_workflow_trigger(
                "Use a workflow to audit all crates for auth regressions",
                &config
            ),
            WorkflowTriggerDecision::Plan(WorkflowTriggerKind::TriggerWord)
        );
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
    fn ultracode_auto_plans_only_for_substantive_tasks() {
        let config = RuntimeDynamicWorkflowConfig {
            effort_profile: DynamicWorkflowEffortProfile::Ultracode,
            ..RuntimeDynamicWorkflowConfig::default()
        };

        assert_eq!(
            classify_workflow_trigger(
                "Audit the workspace for security issues and verify every finding with tests",
                &config
            ),
            WorkflowTriggerDecision::Plan(WorkflowTriggerKind::UltracodeAuto)
        );
        assert_eq!(
            classify_workflow_trigger("yes", &config),
            WorkflowTriggerDecision::Ignore(WorkflowTriggerSuppression::ApprovalReply)
        );
    }

    #[test]
    fn ultracode_reasoning_degrades_on_models_without_xhigh() {
        let fallback = ReasoningConfig::default();
        let decision = ultracode_reasoning_decision("mock", REASONING_XHIGH, fallback.clone());

        assert_eq!(decision.desired_reasoning, REASONING_XHIGH);
        assert_eq!(decision.applied_reasoning, None);
        assert!(!decision.supported);
        assert_eq!(decision.reasoning, fallback);
    }
}
