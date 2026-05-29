use std::sync::Arc;

use async_trait::async_trait;
use roder_api::{
    dynamic_workflows::{
        WorkflowCostEstimate, WorkflowPhase, WorkflowPhaseStatus, WorkflowRun, WorkflowRunId,
        WorkflowRunStatus, WorkflowRunSummary, WorkflowScript, WorkflowScriptSource,
        WorkflowScriptSourceKind,
    },
    events::{ThreadId, TurnId},
    inference::TokenUsage,
    subagents::{SubagentDispatcher, SubagentExitReason, SubagentRequest, SubagentResult},
};
use roder_dynamic_workflows::{
    WorkflowAgentExecutionContext, WorkflowAgentExecutionRequest, WorkflowAgentExecutor,
    WorkflowRuntimeOptions, parse_workflow_definition, workflow_script_hash,
};
use time::OffsetDateTime;

pub(super) struct AppWorkflowExecutor {
    pub(super) dispatcher: Option<Arc<dyn SubagentDispatcher>>,
}

#[async_trait]
impl WorkflowAgentExecutor for AppWorkflowExecutor {
    async fn execute_agent(
        &self,
        context: WorkflowAgentExecutionContext,
        request: WorkflowAgentExecutionRequest,
    ) -> anyhow::Result<SubagentResult> {
        if let Some(dispatcher) = &self.dispatcher {
            let parent_thread = context
                .thread_id
                .clone()
                .unwrap_or_else(|| context.run_id.clone());
            let parent_turn = context
                .turn_id
                .clone()
                .unwrap_or_else(|| context.agent_id.clone());
            return dispatcher
                .dispatch(parent_thread, parent_turn, request.subagent_request)
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(fake_subagent_result(&context, &request.subagent_request))
    }
}

pub(super) fn drafted_run(
    run_id: WorkflowRunId,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    script: WorkflowScript,
    cost_estimate: WorkflowCostEstimate,
) -> WorkflowRun {
    let now = OffsetDateTime::now_utc();
    let phases = phase_names(&script)
        .into_iter()
        .map(|name| WorkflowPhase {
            phase_id: name.clone(),
            name,
            status: WorkflowPhaseStatus::Queued,
            description: None,
            queued_agents: 0,
            completed_agents: 0,
            failed_agents: 0,
            started_at: None,
            completed_at: None,
        })
        .collect();
    WorkflowRun {
        run_id,
        thread_id,
        turn_id,
        script: script.clone(),
        status: WorkflowRunStatus::AwaitingApproval,
        limits: script.limits.clone(),
        phases,
        agents: Vec::new(),
        approval: None,
        cost_estimate: Some(cost_estimate),
        summary: None,
        error: None,
        created_at: now,
        updated_at: now,
        started_at: None,
        completed_at: None,
    }
}

pub(super) fn script_from_source(
    source: &str,
    kind: WorkflowScriptSourceKind,
    path: Option<String>,
    command_name: Option<String>,
) -> anyhow::Result<WorkflowScript> {
    let definition = parse_workflow_definition(source, &WorkflowRuntimeOptions::default())
        .map_err(|err| anyhow::anyhow!("workflow script failed validation: {err}"))?;
    let hash = workflow_script_hash(source);
    let now = OffsetDateTime::now_utc();
    Ok(WorkflowScript {
        script_id: format!("script-{}", hash.get(..12).unwrap_or(&hash)),
        name: definition.name,
        description: definition.description,
        source: WorkflowScriptSource {
            kind,
            path,
            command_name,
            extension_id: None,
        },
        hash,
        host_api_version: definition.host_api_version,
        arguments_schema: definition.arguments_schema,
        body: Some(source.to_string()),
        limits: definition.limits,
        created_at: now,
        updated_at: now,
    })
}

pub(super) fn prompt_workflow_source(prompt: &str) -> String {
    let prompt = serde_json::to_string(prompt).expect("prompt serializes");
    format!(
        r#"
workflow.define({{
  name: "planned-workflow",
  description: "Generated workflow plan",
  hostApiVersion: 1,
  argumentsSchema: {{ type: "object" }},
  phases: ["run"],
  limits: {{ maxConcurrentAgents: 1, maxAgentsPerRun: 1 }}
}}, async (ctx) => {{
  ctx.phase.start("run");
  const task = {prompt};
  const result = await ctx.agents.run("worker", {{
    lane: "scout",
    description: "run planned workflow",
    prompt: task,
    output: `result:${{task}}`
  }});
  return ctx.report.markdown(result.output);
}});
"#
    )
}

pub(super) fn cost_estimate_for_source(source: &str) -> WorkflowCostEstimate {
    WorkflowCostEstimate {
        min_child_agents: u32::from(source.contains("ctx.agents")),
        max_child_agents: if source.contains("ctx.agents") { 1 } else { 0 },
        estimated_prompt_tokens: Some((source.len() as u64 / 4).max(1)),
        estimated_completion_tokens: None,
        warning: None,
    }
}

pub(super) fn summary_for_run(run: &WorkflowRun) -> WorkflowRunSummary {
    let mut usage = TokenUsage::default();
    for agent in &run.agents {
        if let Some(agent_usage) = &agent.usage {
            usage.add_assign(agent_usage);
        }
    }
    WorkflowRunSummary {
        run_id: run.run_id.clone(),
        status: run.status,
        title: run.script.name.clone(),
        phase_count: run.phases.len() as u32,
        completed_phase_count: run
            .phases
            .iter()
            .filter(|phase| phase.status == WorkflowPhaseStatus::Completed)
            .count() as u32,
        agent_count: run.agents.len() as u32,
        completed_agent_count: run
            .agents
            .iter()
            .filter(|agent| {
                agent.status == roder_api::dynamic_workflows::WorkflowAgentStatus::Completed
            })
            .count() as u32,
        failed_agent_count: run
            .agents
            .iter()
            .filter(|agent| {
                matches!(
                    agent.status,
                    roder_api::dynamic_workflows::WorkflowAgentStatus::Failed
                        | roder_api::dynamic_workflows::WorkflowAgentStatus::Timeout
                        | roder_api::dynamic_workflows::WorkflowAgentStatus::Cancelled
                )
            })
            .count() as u32,
        concurrency_peak: run.limits.max_concurrent_agents,
        usage: (!usage.is_empty()).then_some(usage),
        elapsed_ms: elapsed_ms(run),
        report_preview: run
            .summary
            .as_ref()
            .and_then(|summary| summary.report_preview.clone()),
    }
}

fn elapsed_ms(run: &WorkflowRun) -> Option<u64> {
    let started = run.started_at?;
    let completed = run.completed_at.unwrap_or_else(OffsetDateTime::now_utc);
    Some((completed - started).whole_milliseconds().max(0) as u64)
}

pub(super) fn approval_id(run_id: &str) -> String {
    format!("approval-{run_id}")
}

pub(super) fn is_terminal(status: WorkflowRunStatus) -> bool {
    matches!(
        status,
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped
    )
}

fn phase_names(script: &WorkflowScript) -> Vec<String> {
    if let Some(body) = &script.body
        && let Ok(definition) = parse_workflow_definition(body, &WorkflowRuntimeOptions::default())
        && !definition.phases.is_empty()
    {
        return definition.phases;
    }
    vec![script.name.clone()]
}

fn fake_subagent_result(
    context: &WorkflowAgentExecutionContext,
    request: &SubagentRequest,
) -> SubagentResult {
    SubagentResult {
        thread_id: context
            .thread_id
            .clone()
            .unwrap_or_else(|| context.run_id.clone()),
        turn_id: context
            .turn_id
            .clone()
            .unwrap_or_else(|| context.agent_id.clone()),
        agent_type: request
            .subagent_type
            .clone()
            .unwrap_or_else(|| "workflow-agent".to_string()),
        model: request.model.clone(),
        final_message: format!("completed {}", request.description),
        usage: None,
        exit_reason: SubagentExitReason::Completed,
        transcript: None,
        metadata: serde_json::json!({ "executor": "app-server-fixture" }),
    }
}
